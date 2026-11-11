//! The three-way war (`CThreeWayWar`, `G/threeway_war.cpp`): its two GM lines and the
//! death hook on the forked maps.
//!
//! `/threeway_info` (`G/cmd_gm.cpp:4204-4226`) speaks the progress header, the map
//! line (the event SungZi map and the three pass maps), the `ThreewayPhase` regen
//! flag, and one `%s killscore %d usercount %d` line per empire: the CP949 nation
//! name, the kill score, and the PCs of that empire on the SungZi map.
//! `/threeway_myinfo` (`:4228-4233`) speaks the header and the caller's `Deadcount`.
//! Their `LC_TEXT` formats hit no country table, so every language hears the source
//! bytes.
//!
//! No owner quest starts the war (every `forked.*` name is `unused`, ledger 401), so
//! the state is `Initialize`'s: regen flag 0, scores 0, no tokens, and the event
//! indices read the unset flags as entry 0 of each list. A player dying on a forked map
//! runs [`GameState::threeway_dead`], though the owner's map index loads no forked map.

use common::vid::Vid;

use super::GameState;
use crate::chat_line::Arg;
use crate::game_loop_messages::GmLine;
use crate::item_move::Mover;
use protocol::gc_chat::{CHAT_TYPE_INFO, CHAT_TYPE_NOTICE};

impl GameState {
    /// The three-way lines, split out so the dispatcher stays under the line limit.
    /// They change no row.
    pub(super) fn run_threeway_line(
        &self,
        vid: Vid,
        line: GmLine,
        mover: Mover,
    ) -> Option<super::gm_lines::GmRows> {
        match line {
            GmLine::ThreewayInfo => self.threeway_info(vid, mover),
            GmLine::ThreewayMyinfo => self.threeway_myinfo(vid, mover),
            _ => {}
        }
        None
    }

    /// `/threeway_info` (`do_threeway_war_info`, `G/cmd_gm.cpp:4204-4226`).
    fn threeway_info(&self, vid: Vid, mover: Mover) {
        let channel = self.bodies.get(&vid).map_or(0, |body| body.channel);
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"Progress information for each empire",
            &[],
        );
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"Select Map Information Holy Land %d Passage %d %d %d",
            &[
                Arg::Int(i64::from(self.threeway_sungzi())),
                Arg::Int(i64::from(self.threeway_pass(1))),
                Arg::Int(i64::from(self.threeway_pass(2))),
                Arg::Int(i64::from(self.threeway_pass(3))),
            ],
        );
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"ThreewayPhase %d",
            &[Arg::Int(i64::from(self.threeway_regen))],
        );
        for empire in 1..=3u8 {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"%s killscore %d usercount %d",
                &[
                    Arg::Text(Self::threeway_empire_name(empire)),
                    Arg::Int(i64::from(self.threeway_score(empire))),
                    Arg::Int(i64::from(self.threeway_users(channel, empire))),
                ],
            );
        }
    }

    /// `GetSungziMapIndex` (`G/threeway_war.cpp:647-650`): the event SungZi map, the
    /// `threeway_war_sungzi_idx` flag's entry of the SungZi list. An unset or
    /// out-of-range flag reads 0, against legacy's out-of-bounds index — a Defect
    /// the map lookup below would follow into unowned memory, so this build refuses.
    fn threeway_sungzi(&self) -> i32 {
        let index = self.threeway_event_index(b"threeway_war_sungzi_idx");
        self.forked_maps.sungzi().get(index).copied().unwrap_or(0)
    }

    /// `GetPassMapIndex` (`G/threeway_war.cpp:617-625`): the event pass map of
    /// `empire`, the `threeway_war_pass_idx` flag's triple's entry. An unset or
    /// out-of-range flag reads 0, as above.
    fn threeway_pass(&self, empire: u8) -> i32 {
        let index = self.threeway_event_index(b"threeway_war_pass_idx");
        let slot = usize::from(empire.saturating_sub(1));
        self.forked_maps
            .passes()
            .get(index)
            .and_then(|pass| pass.get(slot))
            .copied()
            .unwrap_or(0)
    }

    /// The event flag's list index: 0 when the quests are absent or the flag is
    /// negative, else the flag's value.
    fn threeway_event_index(&self, name: &[u8]) -> usize {
        let flag = self
            .quests
            .as_ref()
            .map_or(0, |quests| quests.manager().event_flag(name));
        usize::try_from(flag).unwrap_or(0)
    }

    /// `EMPIRE_NAME` (`G/start_position.h:14-17`): `LC_TEXT(g_nation_name[e])`, the
    /// CP949 nation name (`G/start_position.cpp:5-11`). `LC_TEXT` is the one-argument
    /// `locale_find`, whose language defaults to `LOCALE_YMIR` (`G/locale.hpp:10,31`),
    /// and a `LOCALE_YMIR` lookup answers its string as it is: the raw bytes speak.
    fn threeway_empire_name(empire: u8) -> &'static [u8] {
        match empire {
            1 => NATION_NAMES[0],
            2 => NATION_NAMES[1],
            _ => NATION_NAMES[2],
        }
    }

    /// `GetKillScore` (`G/threeway_war.cpp:90-99`): the empire's score, 0 outside
    /// 1..=3, as legacy's wrong-empire arm answers it.
    fn threeway_score(&self, empire: u8) -> i32 {
        if (1..=3).contains(&empire) {
            self.threeway_scores[usize::from(empire - 1)]
        } else {
            0
        }
    }

    /// `FCountInMap` on the event SungZi map (`G/cmd_gm.cpp:4188-4201`): the PCs of
    /// `empire` whose body stands on it, on the caller's Channel (legacy's process).
    /// `bodies` holds PCs only.
    fn threeway_users(&self, channel: u8, empire: u8) -> i32 {
        let map = self.threeway_sungzi();
        let mut count = 0;
        for body in self.bodies.values() {
            if body.channel == channel && body.map == map && body.card.empire == empire {
                count += 1;
            }
        }
        count
    }

    /// `CThreeWayWar::onDead` (`G/threeway_war.cpp:290-554`): the forked death hook.
    ///
    /// A GM's death stops at once unless the server is a test server; so does any
    /// death once the regen flag is -1. Every other death spends one of the victim's
    /// revive tokens (an unregistered PID goes from 0 to -1), then stops unless the
    /// victim died on the event SungZi map to a PC of another empire. Such a kill
    /// scores `GetKillValue` on the killer's empire unless it is eliminated (-1), and
    /// a nonzero score is noticed to the SungZi map: on every kill on a test server,
    /// else on every fifth point.
    ///
    /// The elimination and victory arms (`:332-552`) are not ported. The owner's map
    /// index loads none of the forked maps 114-128, so no death reaches this hook on
    /// the owner's data (ledger 466); the arms wait on those map folders.
    pub(super) fn threeway_dead(&mut self, vid: Vid, killer: Option<super::death::Attacker>) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        let gm = character.points().map_or(
            crate::command::GM_PLAYER,
            world::character::Points::gm_level,
        );
        // `GM_PLAYER != GetGMLevel() && false == test_server` (`:295-296`).
        if gm != crate::command::GM_PLAYER && !self.test_server {
            return;
        }
        // `-1 == GetRegenFlag()` (`:298-299`): the war is over.
        if self.threeway_regen == -1 {
            return;
        }
        // `DecreaseReviveTokenForPlayer` (`:301`, `:128-134`): PID 0 is skipped.
        if pid != 0 {
            let left = self.threeway_tokens.get(&pid).copied().unwrap_or(0) - 1;
            self.threeway_tokens.insert(pid, left);
        }
        // `IsSungZiMapIndex` (`:303-304`) reads every SungZi map, not the event's.
        let Some(victim) = self.bodies.get(&vid) else {
            return;
        };
        if !self.forked_maps.sungzi().contains(&victim.map) {
            return;
        }
        let Some(killer) = killer
            .filter(|killer| killer.pc)
            .and_then(|killer| self.bodies.get(&killer.vid))
        else {
            return;
        };
        let (victim_empire, killer_empire, level) =
            (victim.card.empire, killer.card.empire, victim.card.level);
        let channel = victim.channel;
        // `GetEmpire() == pkKiller->GetEmpire()` (`:309-311`).
        if victim_empire == killer_empire {
            return;
        }
        let mut score = self.threeway_score(killer_empire);
        if score >= 0 {
            score += threeway_kill_value(level);
            if (1..=3).contains(&killer_empire) {
                self.threeway_scores[usize::from(killer_empire - 1)] = score;
            }
        }
        if score != 0 && (self.test_server || score % 5 == 0) {
            self.threeway_score_notice(channel);
        }
    }

    /// The score notice (`G/threeway_war.cpp:321-330`): `snprintf` into 65 bytes,
    /// then `SendNoticeMap` on the event SungZi map (`G/cmd_gm.cpp:1453-1484`), which
    /// uses the text as each recipient's format and says it as `CHAT_TYPE_NOTICE`.
    /// Legacy walks one process's descriptors, which is one Channel.
    fn threeway_score_notice(&self, channel: u8) {
        let mut text = SCORE_NOTICE[0].to_vec();
        for (index, score) in self.threeway_scores.iter().enumerate() {
            if index > 0 {
                text.extend_from_slice(SCORE_NOTICE[index]);
            }
            text.extend_from_slice(score.to_string().as_bytes());
        }
        text.truncate(SCORE_NOTICE_MAX);
        let map = self.threeway_sungzi();
        for (vid, body) in &self.bodies {
            if body.channel != channel || body.map != map {
                continue;
            }
            let line = crate::chat_line::chat_packet(
                crate::chat_line::Recipient {
                    strings: &self.locale,
                    language: body.card.language,
                    empire: body.card.empire,
                },
                CHAT_TYPE_NOTICE,
                &text,
                &[],
            );
            let _sent = self.write_to_client(*vid, line);
        }
    }

    /// `/threeway_myinfo` (`do_threeway_war_myinfo`, `G/cmd_gm.cpp:4228-4233`).
    fn threeway_myinfo(&self, vid: Vid, mover: Mover) {
        self.tell(vid, mover, CHAT_TYPE_INFO, b"My three-way progress", &[]);
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        let dead = self.threeway_tokens.get(&pid).copied().unwrap_or(0);
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"Deadcount %d",
            &[Arg::Int(i64::from(dead))],
        );
    }
}

/// `g_nation_name[1..=3]` (`G/start_position.cpp:5-11`), CP949.
const NATION_NAMES: [&[u8]; 3] = [
    b"\xbd\xc5\xbc\xf6\xb1\xb9",
    b"\xc3\xb5\xc1\xb6\xb1\xb9",
    b"\xc1\xf8\xb3\xeb\xb1\xb9",
];

/// The score notice's format (`G/threeway_war.cpp:326`), CP949, cut at each `%d`:
/// "current score Shinsoo:%d Chunjo:%d Jinno:%d".
const SCORE_NOTICE: [&[u8]; 3] = [
    b"\xc7\xf6\xc0\xe7 \xbd\xba\xc4\xda\xbe\xee \xbd\xc5\xbc\xf6\xb1\xb9:",
    b" \xc3\xb5\xc1\xb6\xb1\xb9:",
    b" \xc1\xf8\xb3\xeb\xb1\xb9:",
];

/// `char szBuf[64 + 1]` (`G/threeway_war.cpp:324`): `snprintf` keeps 64 bytes.
const SCORE_NOTICE_MAX: usize = 64;

/// `GetKillValue` (`G/threeway_war.cpp:264-288`): 1/2/3 by the victim's level band
/// 30-39, 40-49, 50-99, else 0.
fn threeway_kill_value(level: u8) -> i32 {
    if (30..=39).contains(&level) {
        1
    } else if (40..=49).contains(&level) {
        2
    } else if (50..=99).contains(&level) {
        3
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use common::vid::Vid;
    use protocol::gc_chat::CHAT_TYPE_NOTICE;

    use crate::game_loop_messages::GmLine;
    use gamedata::mob_locale_names::MobNamesByLanguage;
    use gamedata::server_attr::SectreeGrid;

    use super::super::fixtures::{a_world, enter_on, TestClock};

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

    fn text_of(line: &[u8]) -> Vec<u8> {
        let size = usize::from(u16::from_le_bytes([line[1], line[2]]));
        assert_eq!(size, line.len());
        assert_eq!(line[0], 4, "HEADER_GC_CHAT");
        line[10..].to_vec()
    }

    /// The forked maps on Channel 1, with the quests installed.
    fn threeway_world(clock: &TestClock) -> super::super::GameState {
        let mut world = a_world(clock);
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let bytes = std::fs::read(locale_dir.join(gamedata::forked::FORKED_FILE))
            .expect("the owner's forked index reads");
        world = world.with_forked_maps(std::sync::Arc::new(gamedata::forked::ForkedMaps::read(
            &bytes,
        )));
        for channel in [1, 2] {
            for map in [114, 118, 119, 120] {
                world.host_map(
                    channel,
                    map,
                    SectreeGrid {
                        x: 0,
                        y: 0,
                        columns: 2,
                        rows: 2,
                    },
                );
            }
        }
        let manager =
            quest::manager::Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        world.quests = Some(super::super::quests::Quests::new(
            manager,
            std::sync::Arc::default(),
            MobNamesByLanguage::default(),
        ));
        world
    }

    fn kill(world: &mut super::super::GameState) {
        world.threeway_dead(
            Vid::new(7),
            Some(super::super::death::Attacker {
                vid: Vid::new(8),
                pc: true,
            }),
        );
    }

    /// `/threeway_info` speaks the header, the map line, the phase and the three
    /// empire lines, with the raw CP949 nation names and the caller's Channel's
    /// count; `/threeway_myinfo` speaks the header and the dead count.
    #[test]
    fn the_lines_speak_the_progress_and_the_dead_count() {
        let clock = TestClock::default();
        let mut world = threeway_world(&clock);
        let mut caller = enter_on(&mut world, (1, 114), 7, (3200, 3200), 820);
        while caller.try_recv().is_ok() {}
        // An empire-1 PC on Channel 2's map 114 is another process's: not counted.
        let _elsewhere = enter_on(&mut world, (2, 114), 9, (3200, 3200), 820);
        while caller.try_recv().is_ok() {}
        world.threeway_scores = [3, 0, -1];
        world.threeway_regen = -1;
        world.threeway_tokens.insert(7, 4);
        world.run_threeway_line(Vid::new(7), GmLine::ThreewayInfo, mover());
        for line in [
            b"Progress information for each empire".as_slice(),
            b"Select Map Information Holy Land 114 Passage 118 119 120",
            b"ThreewayPhase -1",
            b"\xbd\xc5\xbc\xf6\xb1\xb9 killscore 3 usercount 1",
            b"\xc3\xb5\xc1\xb6\xb1\xb9 killscore 0 usercount 0",
            b"\xc1\xf8\xb3\xeb\xb1\xb9 killscore -1 usercount 0",
        ] {
            let got = caller.try_recv().expect("the info speaks");
            assert_eq!(text_of(&got), line);
        }
        world.run_threeway_line(Vid::new(7), GmLine::ThreewayMyinfo, mover());
        for line in [b"My three-way progress".as_slice(), b"Deadcount 4"] {
            let got = caller.try_recv().expect("the myinfo speaks");
            assert_eq!(text_of(&got), line);
        }
    }

    /// A forked death spends a token and scores the killer's empire by the victim's
    /// level band; a same-empire kill and a pass-map death spend only; every scoring
    /// kill notices the SungZi map on a test server; an ended war spends nothing.
    #[test]
    fn a_forked_death_spends_a_token_and_scores_the_killer() {
        let clock = TestClock::default();
        let mut world = threeway_world(&clock);
        let mut victim = enter_on(&mut world, (1, 114), 7, (3200, 3200), 820);
        while victim.try_recv().is_ok() {}
        let mut killer = enter_on(&mut world, (1, 114), 8, (3300, 3200), 820);
        while killer.try_recv().is_ok() {}
        while victim.try_recv().is_ok() {}
        world
            .bodies
            .get_mut(&Vid::new(8))
            .expect("the killer stands")
            .card
            .empire = 2;
        world
            .bodies
            .get_mut(&Vid::new(7))
            .expect("the victim stands")
            .card
            .level = 45;
        world.threeway_tokens.insert(7, 4);
        kill(&mut world);
        assert_eq!(world.threeway_tokens.get(&7), Some(&3));
        assert_eq!(world.threeway_scores, [0, 2, 0]);
        let notice = victim.try_recv().expect("the SungZi map hears the score");
        assert_eq!(notice[3], CHAT_TYPE_NOTICE);
        assert_eq!(
            text_of(&notice),
            b"\xc7\xf6\xc0\xe7 \xbd\xba\xc4\xda\xbe\xee \xbd\xc5\xbc\xf6\xb1\xb9:0 \
              \xc3\xb5\xc1\xb6\xb1\xb9:2 \xc1\xf8\xb3\xeb\xb1\xb9:0"
        );
        assert!(killer.try_recv().is_ok(), "the killer stands there too");
        // A same-empire kill spends a token, scores nothing and says nothing.
        world
            .bodies
            .get_mut(&Vid::new(8))
            .expect("the killer stands")
            .card
            .empire = 1;
        kill(&mut world);
        assert_eq!(world.threeway_tokens.get(&7), Some(&2));
        assert_eq!(world.threeway_scores, [0, 2, 0]);
        assert!(victim.try_recv().is_err());
        // The level bands: 30 pays 1, 50 pays 3, 100 pays nothing.
        world
            .bodies
            .get_mut(&Vid::new(8))
            .expect("the killer stands")
            .card
            .empire = 2;
        for (level, score) in [(30, 3), (50, 6), (100, 6)] {
            world
                .bodies
                .get_mut(&Vid::new(7))
                .expect("the victim stands")
                .card
                .level = level;
            kill(&mut world);
            assert_eq!(world.threeway_scores, [0, score, 0], "level {level}");
        }
        assert_eq!(world.threeway_tokens.get(&7), Some(&-1));
        // A pass map is forked but no SungZi map: the hook spends and stops.
        world
            .bodies
            .get_mut(&Vid::new(7))
            .expect("the victim stands")
            .map = 118;
        kill(&mut world);
        assert_eq!(world.threeway_tokens.get(&7), Some(&-2));
        assert_eq!(world.threeway_scores, [0, 6, 0]);
        // Any SungZi map scores, not only the event's (`IsSungZiMapIndex`): 121.
        world
            .bodies
            .get_mut(&Vid::new(7))
            .expect("the victim stands")
            .map = 121;
        world
            .bodies
            .get_mut(&Vid::new(7))
            .expect("the victim stands")
            .card
            .level = 30;
        kill(&mut world);
        assert_eq!(world.threeway_scores, [0, 7, 0]);
        // An eliminated killer's empire scores nothing.
        world
            .bodies
            .get_mut(&Vid::new(7))
            .expect("the victim stands")
            .map = 114;
        world.threeway_scores = [0, -1, 0];
        kill(&mut world);
        assert_eq!(world.threeway_scores, [0, -1, 0]);
        // An ended war spends no token.
        world.threeway_regen = -1;
        kill(&mut world);
        assert_eq!(world.threeway_tokens.get(&7), Some(&-4));
    }
}
