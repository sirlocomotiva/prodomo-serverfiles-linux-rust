//! The dungeon board (`__DUNGEON_INFO__`, `G/constants.cpp:1339-1348`,
//! `G/char.cpp:11809-11863`, `G/char_manager.cpp:1309-1388`): six bosses with their level
//! windows and quest names, the cooldown lines and the three ranks.
//!
//! Legacy has the six rows compiled in, reads the caller's quest flags for the cooldown
//! line (`<quest>.cooldown - time(0)`, `dungeon.<boss>_completed/fastest/damage`), and
//! queries `player.quest` for each rank, caching it for 5 minutes. Here the rows are the
//! same six, the flags are the quest manager's, and the ranks are rows the descriptor
//! reads once per line — no 5-minute cache, since the flags the ranks read also change
//! only through saves (a recorded Divergence, ledger 463).

use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_COMMAND;

use crate::chat_line::Arg;
use crate::game_state::GameState;
use crate::item_move::Mover;

/// One dungeon row: the boss race (`mobIdx`), the level window and the quest name whose
/// `<quest>.cooldown` flag the line reads (`m_mapDungeonList`,
/// `G/constants.cpp:1340-1347`).
pub struct DungeonRow {
    /// The boss race.
    pub boss: u32,
    /// The window the caller's level must sit in for the full line.
    pub window: (u8, u8),
    /// The quest name of the `<quest>.cooldown` flag.
    pub quest: &'static str,
}

/// The six rows, in the map's (`mobIdx`) order.
pub const DUNGEON_ROWS: [DungeonRow; 6] = [
    DungeonRow {
        boss: 1092,
        window: (60, 75),
        quest: "SpiderDungeon",
    },
    DungeonRow {
        boss: 1093,
        window: (40, 75),
        quest: "deviltower_zone",
    },
    DungeonRow {
        boss: 2598,
        window: (75, 90),
        quest: "devilcatacomb_zone",
    },
    DungeonRow {
        boss: 2692,
        window: (100, 120),
        quest: "meley",
    },
    DungeonRow {
        boss: 6001,
        window: (100, 120),
        quest: "hydra",
    },
    DungeonRow {
        boss: 6418,
        window: (100, 120),
        quest: "jotun",
    },
];

impl GameState {
    /// The dungeon row of race `boss`, if it is one of the six.
    #[must_use]
    pub fn dungeon_row(boss: u32) -> Option<&'static DungeonRow> {
        DUNGEON_ROWS.iter().find(|row| row.boss == boss)
    }

    /// Whether the race is one of the six dungeon bosses.
    #[must_use]
    pub fn is_dungeon_boss(race: u32) -> bool {
        Self::dungeon_row(race).is_some()
    }

    /// A quest flag of the character under `vid`, 0 when the quests are not loaded, the
    /// character is offline, or the flag is not set (`GetQuestFlag`, `G/char.cpp:7726-7738`).
    pub(super) fn dungeon_flag(&self, vid: Vid, name: &[u8]) -> i32 {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return 0;
        };
        let pid = character.player_id();
        self.quests()
            .and_then(|quests| quests.manager().quest_flag(pid, name))
            .unwrap_or(0)
    }
}

impl GameState {
    /// `/dungeon_info ...` (`do_dungeon_info`, `G/cmd_general.cpp:4095-4117`): fewer than
    /// two words stays silent; `rank <mob> <rank>` answers on the descriptor's rank
    /// read (see [`DungeonRankAsk`]); `test_cooldown` and `update` stay silent for a
    /// caller the world cannot call a GM (see below), else stamp the three cooldowns
    /// or fan the full line.
    ///
    /// Words split on spaces with the command's own name first, as `do_reward_system`
    /// does (`split_argument`, `G/utils.cpp:178-182`). The `rank` arm runs on its own
    /// [`GameCommand::DungeonRank`], not here, so it stays silent on this path.
    pub(super) fn run_dungeon_info(&mut self, vid: Vid, rest: &[u8], mover: Mover) {
        let mut words: Vec<&[u8]> = vec![b"dungeon_info".as_slice()];
        words.extend(
            rest.split(|byte| *byte == b' ')
                .filter(|word| !word.is_empty()),
        );
        if words.len() < 2 {
            return;
        }
        if words[1] == b"rank" {
            return;
        }
        if !self.dungeon_info_is_gm() {
            return;
        }
        if words[1] == b"test_cooldown" {
            self.stamp_dungeon_test_cooldowns(vid);
            self.send_dungeon_cooldown(vid, mover, 0);
        } else if words[1] == b"update" {
            self.send_dungeon_cooldown(vid, mover, 0);
        }
    }

    /// `ch->IsGM()` for the `test_cooldown` and `update` arms (`G/char.cpp:6373-6380`):
    /// a non-player GM level, or the test-server mode — which is always on in this
    /// build (`GameState::test_server`), so both arms always run.
    fn dungeon_info_is_gm(&self) -> bool {
        self.test_server
    }

    /// The `test_cooldown` stamps (`G/cmd_general.cpp:4111-4113`): `jotun.cooldown` now
    /// + 30, `hydra.cooldown` now + 60, `meley.cooldown` now + 90.
    fn stamp_dungeon_test_cooldowns(&mut self, vid: Vid) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        let now = i64::from(super::quests::quest_now());
        let Some(quests) = self.quests.as_mut() else {
            return;
        };
        let manager = quests.manager_mut();
        for (name, ahead) in [
            (b"jotun.cooldown".as_slice(), 30),
            (b"hydra.cooldown".as_slice(), 60),
            (b"meley.cooldown".as_slice(), 90),
        ] {
            let end = i32::try_from(now.saturating_add(ahead)).unwrap_or(i32::MAX);
            manager.set_quest_flag(pid, name, end);
        }
    }

    /// `CHARACTER::SendDungeonCooldown` (`G/char.cpp:11818-11863`): for boss 0, every
    /// row the caller's level sits in; else the one row. Each cell is
    /// `<boss>|<cooldown-now>|<completed>|<fastest>|<damage>#`, and the line is
    /// `dungeon_info_cooldown <boss == 0> <cells or ->`.
    fn send_dungeon_cooldown(&mut self, vid: Vid, mover: Mover, boss: u32) {
        let Some(body) = self.bodies.get(&vid) else {
            return;
        };
        let level = body.card.level;
        let rows: Vec<&DungeonRow> = if boss == 0 {
            DUNGEON_ROWS
                .iter()
                .filter(|row| level >= row.window.0 && level <= row.window.1)
                .collect()
        } else {
            Self::dungeon_row(boss).into_iter().collect()
        };
        let now = i64::from(super::quests::quest_now());
        let mut cells = Vec::new();
        for row in rows {
            let name = format!("{}.cooldown", row.quest);
            let left = i64::from(self.dungeon_flag(vid, name.as_bytes())).saturating_sub(now);
            for (index, value) in [
                (0, row.boss.to_string()),
                (1, left.to_string()),
                (
                    2,
                    self.dungeon_count(vid, row.boss, "completed").to_string(),
                ),
                (3, self.dungeon_count(vid, row.boss, "fastest").to_string()),
                (4, self.dungeon_count(vid, row.boss, "damage").to_string()),
            ] {
                cells.extend_from_slice(value.as_bytes());
                cells.push(if index == 4 { b'#' } else { b'|' });
            }
        }
        if cells.is_empty() {
            cells.extend_from_slice(b"-");
        }
        let flag = i64::from(boss == 0);
        self.tell(
            vid,
            mover,
            CHAT_TYPE_COMMAND,
            b"dungeon_info_cooldown %d %s",
            &[Arg::Int(flag), Arg::Text(&cells)],
        );
    }

    /// One of the caller's three `dungeon.<boss>_<what>` flags, 0 unset.
    fn dungeon_count(&self, vid: Vid, boss: u32, what: &str) -> i32 {
        let name = format!("dungeon.{boss}_{what}");
        self.dungeon_flag(vid, name.as_bytes())
    }

    /// The `__DUNGEON_INFO__` arm of `Damage` (`G/char_battle.cpp:2322-2336`): when the
    /// attacker is a player and the victim one of the six bosses, a hit above the
    /// attacker's `dungeon.<race>_damage` flag raises the flag and fans the boss's
    /// single line. Runs before the damage record, as the arm does.
    pub(super) fn dungeon_damage(&mut self, attacker: Vid, victim: u32, damage: i32) {
        let Some(npc) = self.npc_by_vid(victim) else {
            return;
        };
        let race = u32::from(npc.race);
        if Self::dungeon_row(race).is_none() {
            return;
        }
        let Ok(character) = self.characters.find_by_vid(attacker) else {
            return;
        };
        let pid = character.player_id();
        let name = format!("dungeon.{race}_damage");
        let flag = name.as_bytes();
        let current = self
            .quests()
            .and_then(|quests| quests.manager().quest_flag(pid, flag))
            .unwrap_or(0);
        if current >= damage {
            return;
        }
        if let Some(quests) = self.quests.as_mut() {
            quests.manager_mut().set_quest_flag(pid, flag, damage);
        }
        let Some(mover) = self.own_mover(attacker) else {
            return;
        };
        self.send_dungeon_cooldown(attacker, mover, race);
    }

    /// Answers one `SendDungeonRank` ask for the descriptor's store read
    /// (`G/char_manager.cpp:1311-1388`): `None` past a miss, else the mover, the mob,
    /// the rank and the flag to read.
    pub(super) fn ask_dungeon_rank(vid: Vid, rest: &[u8], mover: Mover) -> Option<DungeonRankAsk> {
        let mut words: Vec<&[u8]> = vec![b"dungeon_info".as_slice()];
        words.extend(
            rest.split(|byte| *byte == b' ')
                .filter(|word| !word.is_empty()),
        );
        if words.len() < 4 || words[1] != b"rank" {
            return None;
        }
        let (Some(mob), Some(rank)) = (uint_word(words[2]), uint_word(words[3])) else {
            return None;
        };
        #[allow(clippy::cast_possible_truncation)]
        let rank = rank as u8;
        Self::dungeon_row(mob)?;
        let flag = match rank {
            0 => format!("dungeon.{mob}_completed"),
            1 => format!("dungeon.{mob}_fastest"),
            2 => format!("dungeon.{mob}_damage"),
            _ => return None,
        };
        Some(DungeonRankAsk {
            vid,
            mover,
            mob,
            rank,
            flag: flag.into_bytes(),
        })
    }
}

/// One `SendDungeonRank` the descriptor reads and fans: the caller, its mover, the mob,
/// the rank and the flag name to read.
#[derive(Debug, Clone)]
pub struct DungeonRankAsk {
    /// The asking character.
    pub vid: Vid,
    /// What the descriptor knows of the character.
    pub mover: Mover,
    /// The boss race.
    pub mob: u32,
    /// The rank, 0 to 2.
    pub rank: u8,
    /// The flag to read, such as `dungeon.1092_completed`.
    pub flag: Vec<u8>,
}

/// `str_to_number` on a `DWORD` or `BYTE` (`C/utils.h:20-26`, `:52-58`): `None` for an
/// empty word, which is the only `false` it answers; else `strtoul` base 10, garbage
/// reading 0, saturating past `ULONG_MAX`.
fn uint_word(word: &[u8]) -> Option<u32> {
    if word.is_empty() {
        return None;
    }
    Some(super::gm_lines::unsigned_word(word))
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc::UnboundedReceiver;

    use super::super::fixtures::{a_fighter, enter, open_cells, stand_field, TestClock};
    use super::*;
    use crate::game_loop::PulseProcessor;

    /// The player.
    const NEAR: (i32, i32) = (3100, 3200);

    /// The quest runtime over the legacy locale.
    fn quests() -> crate::game_state::quests::Quests {
        use gamedata::mob_locale_names::MobNamesByLanguage;
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager =
            quest::manager::Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        crate::game_state::quests::Quests::new(
            manager,
            std::sync::Arc::default(),
            MobNamesByLanguage::default(),
        )
    }

    fn drained(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// The player 1 [`NEAR`], after Pulse 1; the inbox is the player's, drained.
    fn a_field() -> (TestClock, GameState, UnboundedReceiver<Vec<u8>>) {
        let clock = TestClock::default();
        clock.set(1_000);
        let mut world = stand_field(&clock, Vec::new(), open_cells(), true);
        let mut inbox = enter(&mut world, 1, NEAR, 12_345);
        world.process_pulse(1);
        drained(&mut inbox);
        (clock, world, inbox)
    }

    /// `ask_dungeon_rank`'s parse (`G/cmd_general.cpp:4099-4107`, `C/utils.h:20-26`,
    /// `:52-58`): a short line, a non-`rank` word and an empty number stay silent; a
    /// boss off the six stays silent; ranks past 2 stay silent; else the flag the
    /// descriptor reads.
    #[test]
    fn the_rank_ask_parses_the_line_and_the_table() {
        let (_clock, world, _inbox) = a_field();
        let vid = Vid::new(1);
        let mover = world.own_mover(vid).expect("the player stands entered");
        for rest in [
            &b"".as_slice(),
            &b"rank".as_slice(),
            &b"rank 1092".as_slice(),
        ] {
            assert!(
                GameState::ask_dungeon_rank(vid, rest, mover).is_none(),
                "a short line stays silent: {rest:?}"
            );
        }
        assert!(
            GameState::ask_dungeon_rank(vid, b"rank ", mover).is_none(),
            "an empty mob stays silent"
        );
        assert!(
            GameState::ask_dungeon_rank(vid, b"update", mover).is_none(),
            "a non-rank word stays silent"
        );
        assert!(
            GameState::ask_dungeon_rank(vid, b"rank 101 0", mover).is_none(),
            "a boss off the six stays silent"
        );
        assert!(
            GameState::ask_dungeon_rank(vid, b"rank 1092 3", mover).is_none(),
            "a rank past 2 stays silent"
        );
        let ask =
            GameState::ask_dungeon_rank(vid, b"rank 1092 1", mover).expect("the table holds 1092");
        assert_eq!(ask.mob, 1092);
        assert_eq!(ask.rank, 1);
        assert_eq!(ask.flag, b"dungeon.1092_fastest");
    }

    /// The `__DUNGEON_INFO__` arm of `Damage` (`G/char_battle.cpp:2322-2336`) does not
    /// run here: only `fight.rs` calls [`GameState::dungeon_damage`], before the damage
    /// record, and this module's tests pin the table the arm reads.
    #[test]
    fn the_six_bosses_are_the_legacy_rows() {
        for (boss, window, quest) in [
            (1092, (60, 75), "SpiderDungeon"),
            (1093, (40, 75), "deviltower_zone"),
            (2598, (75, 90), "devilcatacomb_zone"),
            (2692, (100, 120), "meley"),
            (6001, (100, 120), "hydra"),
            (6418, (100, 120), "jotun"),
        ] {
            let row = GameState::dungeon_row(boss).expect("the table holds the boss");
            assert_eq!(row.window, window);
            assert_eq!(row.quest, quest);
        }
        assert!(GameState::dungeon_row(101).is_none(), "the dog is no boss");
        assert_eq!(
            DUNGEON_ROWS.iter().map(|row| row.boss).collect::<Vec<_>>(),
            vec![1092, 1093, 2598, 2692, 6001, 6418],
            "the map's order"
        );
    }

    /// `dungeon_damage`'s miss: no NPC under the victim, so nothing marks and nothing
    /// fans. The hit path is the scenario's and `fight.rs`'s, not this module's.
    #[test]
    fn a_hit_on_nothing_marks_nothing() {
        let (_clock, mut world, mut alpha) = a_field();
        world.dungeon_damage(Vid::new(1), 0, 100);
        assert!(
            drained(&mut alpha).is_empty(),
            "no NPC under 0: nothing fans"
        );
    }

    /// The arm's comparison is strict (`<`, `G/char_battle.cpp:2330`): a hit above the
    /// flag raises it and fans the boss's single line; an equal hit marks nothing and
    /// fans nothing.
    #[test]
    fn a_hit_above_the_flag_raises_it_and_an_equal_hit_marks_nothing() {
        use protocol::gc_chat::GcChat;
        use protocol::gc_chat::CHAT_TYPE_COMMAND;
        let clock = TestClock::default();
        clock.set(1_000);
        let boss = a_fighter(
            world::npc::FIRST_NPC_VID,
            1092,
            0,
            (175, 2000),
            0,
            (3200, 3200),
            10_000,
        );
        let mut world = stand_field(&clock, vec![boss], open_cells(), true).with_quests(quests());
        let mut inbox = enter(&mut world, 1, NEAR, 12_345);
        world.process_pulse(1);
        drained(&mut inbox);
        let vid = Vid::new(1);
        let pid = world.characters.find_by_vid(vid).unwrap().player_id();
        world.dungeon_damage(vid, world::npc::FIRST_NPC_VID, 322);
        let flag = world
            .quests()
            .and_then(|quests| quests.manager().quest_flag(pid, b"dungeon.1092_damage"))
            .unwrap_or(0);
        assert_eq!(flag, 322, "the hit raises the flag");
        let heard = drained(&mut inbox);
        assert_eq!(heard.len(), 1, "exactly the single fans: {heard:02x?}");
        let chat = GcChat::decode(&heard[0]).expect("a chat record");
        assert_eq!(chat.chat_type, CHAT_TYPE_COMMAND);
        assert!(
            chat.text.starts_with(b"dungeon_info_cooldown 0 1092|"),
            "the boss's single line: {:?}",
            chat.text
        );
        world.dungeon_damage(vid, world::npc::FIRST_NPC_VID, 322);
        assert!(
            drained(&mut inbox).is_empty(),
            "an equal hit marks nothing and fans nothing"
        );
    }
}
