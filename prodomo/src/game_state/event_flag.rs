//! Event flags: `/eventflag`, `/geteventflag` (`G/cmd_gm.cpp:2265-2299`), the commands
//! that set one named flag (`/xmas_boom`, `/xmas_snow`, `/xmas_santa`, `/eclipse`), and
//! `/eventhelper`.
//!
//! [`GameState::run_event_flag`] is `ACMD(do_event_flag)`: two words (the name and the
//! value), silent unless both are there, and the caller hears `RequestSetEventFlag
//! <name> <value>`. The nine `mob_*` names clamp to 0..1000. `RequestSetEventFlag`'s
//! round trip through the retired DB peer (ADR-0001) is a direct call of
//! [`GameState::set_event_flag`] after the answer, which is where legacy's echo lands.
//! [`GameState::run_get_event_flag`] is `ACMD(do_get_event_flag)`
//! (`CQuestManager::SendEventFlagList`, `G/questmanager.cpp:1308-1325`): one `name
//! value` line per flag, in name order, with the three `prob` lines off the test
//! server (`test_server` is on here, so they never show).
//!
//! [`GameState::set_event_flag`] is `CQuestManager::SetEventFlag`
//! (`G/questmanager.cpp:1335-1688`): the table takes the value and the name's arm runs.
//! Legacy's DB process forwards every set to every core before it compares
//! (`D/ClientManagerEventFlag.cpp:31-63`), so an arm runs even when the value did not
//! change. The ported arms are the six rates, the Christmas arms of `xmas::ProcessEventFlag`
//! (`G/xmas_event.cpp`), `newyear_boom`, `eclipse`, `day`, `pre_event_hc`,
//! `new_xmas_event`, `daily_gift_event` and `world_boss_event`. A line every client hears
//! goes to every in-game character of every Channel: each legacy core told its own. A
//! spawn or a removal runs once per Channel world hosting the map, as each core hosting
//! it ran it.
//!
//! Not ported, so their names only set the table: the premium (`_buyer`) rates, which no
//! character can use (there is no premium time); `crcdisconnect`; the drop events
//! (`drop_char_stone*`, `refine_box*`); the four time limits and the three `no_*` switches
//! (their readers keep the compiled-in values).
//!
//! The table is the quest manager's (`Manager::event_flags`): the same flags
//! `game.get_event_flag` reads. When boot loaded no quests, the set is kept nowhere and
//! the list is silent, but the arms still run.

use common::vid::Vid;

use protocol::gc_chat::{CHAT_TYPE_COMMAND, CHAT_TYPE_INFO};

use crate::chat_line::{chat_packet, Arg, Recipient};
use crate::game_loop_messages::EventLine;
use crate::game_state::GameState;
use crate::item_move::Mover;

/// `xmas::MOB_SANTA_VNUM` (`G/xmas_event.h`). `pub(super)` for the NPC idle's hop.
pub(super) const SANTA: u32 = 20_031;
/// `xmas::MOB_XMAS_TREE_VNUM`.
const XMAS_TREE: u32 = 20_032;
/// `xmas::MOB_XMAS_FIRWORK_SELLER_VNUM`.
const FIREWORK_SELLER: u32 = 9_004;
/// `pre_event_hc`'s `EventNPC` (`G/questmanager.cpp:1522`).
const PRE_EVENT_NPC: u32 = 20_090;
/// `new_xmas_event`'s new Santa (`G/questmanager.cpp:1630`).
const NEW_SANTA: u32 = 20_126;
/// Seohan mountain, where the Santa and the tree stand (`G/xmas_event.cpp:45`, `:81`).
const SANTA_MAP: i32 = 61;

/// The firework sellers of `SpawnEventHelper` and the `day` arm: map and metres from the
/// map's base (`G/xmas_event.cpp:150-157`, `G/questmanager.cpp:1471-1478`).
const FIREWORK_SELLERS: [(i32, i32, i32); 6] = [
    (1, 615, 618),
    (3, 500, 625),
    (21, 598, 665),
    (23, 476, 360),
    (41, 318, 629),
    (43, 478, 375),
];

/// `pre_event_hc`'s NPCs: map and metres from the map's base (`G/questmanager.cpp:1530-1535`).
const PRE_EVENT_NPCS: [(i32, i32, i32); 3] = [(3, 588, 617), (23, 397, 250), (43, 567, 426)];

/// `new_xmas_event`'s Santas: map, centimetres from the map's base, and facing
/// (`G/questmanager.cpp:1659-1674`). Each stands with no spawn motion.
const NEW_SANTAS: [(i32, i32, i32, i32); 3] = [
    (1, 60_800, 61_700, 90),
    (21, 59_600, 61_000, 110),
    (41, 35_700, 74_300, 140),
];

/// The rates `CHARACTER_MANAGER` keeps (`G/char_manager.cpp:51-62`): each starts at 100,
/// and its event flag sets it to the value, 0 included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MobRates {
    /// `m_iMobItemRate`: `mob_item`.
    pub(super) item: i32,
    /// `m_iMobDamageRate`: `mob_dam`.
    pub(super) damage: i32,
    /// `m_iMobGoldAmountRate`: `mob_gold`.
    pub(super) gold_amount: i32,
    /// `m_iMobGoldDropRate`: `mob_gold_pct`.
    pub(super) gold_drop: i32,
    /// `m_iMobExpRate`: `mob_exp`.
    pub(super) exp: i32,
    /// `m_iUserDamageRate`: `user_dam`.
    pub(super) user_damage: i32,
}

impl Default for MobRates {
    fn default() -> Self {
        Self {
            item: 100,
            damage: 100,
            gold_amount: 100,
            gold_drop: 100,
            exp: 100,
            user_damage: 100,
        }
    }
}

/// The `mob_*` names `do_event_flag` clamps to 0..1000 (`G/cmd_gm.cpp:2276-2288`).
const CLAMPED_NAMES: [&[u8]; 9] = [
    b"mob_item",
    b"mob_exp",
    b"mob_gold",
    b"mob_dam",
    b"mob_gold_pct",
    b"mob_item_buyer",
    b"mob_exp_buyer",
    b"mob_gold_buyer",
    b"mob_gold_pct_buyer",
];

/// `(int)strtol(word)` (`common/utils.h:44-50`), saturating past `i32`'s ends.
fn parse_flag_value(word: &[u8]) -> i32 {
    let (negative, digits) = match word.split_first() {
        Some((b'-', rest)) => (true, rest),
        Some((b'+', rest)) => (false, rest),
        _ => (false, word),
    };
    let mut value: i64 = 0;
    let mut any = false;
    for &byte in digits {
        let digit = byte.wrapping_sub(b'0');
        if digit >= 10 {
            break;
        }
        any = true;
        value = value.saturating_mul(10).saturating_add(i64::from(digit));
    }
    if !any {
        return 0;
    }
    let signed = if negative { -value } else { value };
    i32::try_from(signed.clamp(i64::from(i32::MIN), i64::from(i32::MAX)))
        .expect("a clamped value fits")
}

impl GameState {
    /// `ACMD(do_event_flag)`: the name takes the value, and the caller hears the
    /// `RequestSetEventFlag` line. A missing word is silent, as legacy's early return
    /// is. With no quests the set lands nowhere: the answer still goes out.
    pub(super) fn run_event_flag(&mut self, vid: Vid, rest: &[u8], mover: Mover) {
        let (name, back) = crate::command::one_argument(rest);
        let (value_word, _) = crate::command::one_argument(back);
        if name.is_empty() || value_word.is_empty() {
            return;
        }
        let mut value = parse_flag_value(&value_word);
        if CLAMPED_NAMES.contains(&name.as_slice()) {
            value = value.clamp(0, 1000);
        }
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"RequestSetEventFlag %s %d",
            &[Arg::Text(&name), Arg::Int(i64::from(value))],
        );
        self.set_event_flag(&name, value);
    }

    /// `ACMD(do_xmas)` (`G/cmd_gm.cpp:3270-3294`), `ACMD(do_eclipse)` (`:4460-4473`) and
    /// `ACMD(do_event_helper)` (`:4502-4520`). The `xmas_*` lines set their flag to the
    /// word's `strtol`, 0 when it is missing; `/eclipse` sets 1 for a word reading 1 and 0
    /// otherwise. All three are silent. `/eventhelper 1` spawns the firework sellers and
    /// any other word removes them, each answered with its line.
    pub(super) fn run_event_line(&mut self, vid: Vid, line: EventLine, rest: &[u8], mover: Mover) {
        let (word, _) = crate::command::one_argument(rest);
        let value = parse_flag_value(&word);
        match line {
            EventLine::XmasBoom => self.set_event_flag(b"xmas_boom", value),
            EventLine::XmasSnow => self.set_event_flag(b"xmas_snow", value),
            EventLine::XmasSanta => self.set_event_flag(b"xmas_santa", value),
            EventLine::Eclipse => self.set_event_flag(b"eclipse", i32::from(value == 1)),
            EventLine::EventHelper => {
                let spawn = value == 1;
                self.spawn_event_helper(spawn);
                let text: &[u8] = if spawn {
                    b"Event Helper Spawn"
                } else {
                    b"Event Helper Delete"
                };
                self.tell(vid, mover, CHAT_TYPE_INFO, text, &[]);
            }
        }
    }

    /// `CQuestManager::SetEventFlag` (`G/questmanager.cpp:1335-1688`): the table takes the
    /// value, then the name's arm runs with the value it replaced (0 for a new name).
    pub(super) fn set_event_flag(&mut self, name: &[u8], value: i32) {
        let prev = self
            .quests
            .as_ref()
            .map_or(0, |quests| quests.manager().event_flag(name));
        if let Some(quests) = self.quests.as_mut() {
            quests.manager_mut().set_event_flag(name, value);
        }
        match name {
            b"mob_item" => self.mob_rates.item = value,
            b"mob_dam" => self.mob_rates.damage = value,
            b"mob_gold" => self.mob_rates.gold_amount = value,
            b"mob_gold_pct" => self.mob_rates.gold_drop = value,
            b"user_dam" => self.mob_rates.user_damage = value,
            b"mob_exp" => self.mob_rates.exp = value,
            b"xmas_snow" | b"xmas_boom" | b"xmas_song" | b"xmas_tree" => {
                self.command_to_everyone(b"%s %d", &[Arg::Text(name), Arg::Int(value.into())]);
                self.xmas_arm(name, prev, value);
            }
            b"xmas_santa" => self.xmas_santa_arm(value),
            b"newyear_boom" => {
                self.command_to_everyone(b"newyear_boom %d", &[Arg::Int(value.into())]);
            }
            b"eclipse" => {
                let mode: &[u8] = if value == 1 { b"dark" } else { b"light" };
                self.command_to_everyone(b"DayMode %s", &[Arg::Text(mode)]);
            }
            b"day" => {
                let mode: &[u8] = if value != 0 { b"dark" } else { b"light" };
                self.command_to_everyone(b"DayMode %s", &[Arg::Text(mode)]);
                if value != 0 && prev == 0 {
                    self.spawn_event_helper(true);
                } else if value == 0 && prev != 0 {
                    self.spawn_event_helper(false);
                }
            }
            b"pre_event_hc" => self.pre_event_arm(prev, value),
            b"new_xmas_event" => self.new_xmas_arm(value),
            b"daily_gift_event" => self.daily_gift_event_arm(value),
            // `world_boss_event` (`G/questmanager.cpp:1344-1348`): the flag
            // takes the value, then `Enable` runs — even when the value did
            // not change, as the DB peer forwards every set before the compare
            // (`D/ClientManagerEventFlag.cpp:31-63`).
            b"world_boss_event" => self.world_boss_enable(value != 0),
            _ => {}
        }
    }

    /// `xmas_boom` and `xmas_tree`'s spawns and removals (`G/xmas_event.cpp:25-58`).
    fn xmas_arm(&mut self, name: &[u8], prev: i32, value: i32) {
        match name {
            b"xmas_boom" if value != 0 && prev == 0 => self.spawn_event_helper(true),
            b"xmas_boom" if value == 0 && prev != 0 => self.spawn_event_helper(false),
            b"xmas_tree" if value > 0 && prev == 0 => {
                // One tree per Channel world hosting Seohan, when it has none.
                for place in self.places_hosting(SANTA_MAP) {
                    if self.vids_of_race(Some(place.0), XMAS_TREE).is_empty() {
                        let _stood = self.spawn_event_mob(
                            place,
                            XMAS_TREE,
                            (76_500 + 358_400, 60_900 + 153_600),
                            -1,
                            false,
                        );
                    }
                }
            }
            b"xmas_tree" if prev > 0 && value == 0 => self.destroy_race(XMAS_TREE, &[]),
            _ => {}
        }
    }

    /// `xmas_santa`'s arm (`G/xmas_event.cpp:60-90`): 0 removes every Santa; 1, where
    /// Seohan is hosted, sets the flag to 2 and stands a Santa at a random point of it when
    /// none stands. Legacy spawns only when one already stands, against its own comment
    /// ("create one if there is none"), so the first Santa never comes: a Defect the
    /// Rewrite does not reproduce. 2 does nothing.
    fn xmas_santa_arm(&mut self, value: i32) {
        match value {
            0 => self.destroy_race(SANTA, &[]),
            1 => {
                let places = self.places_hosting(SANTA_MAP);
                if places.is_empty() {
                    return;
                }
                self.set_event_flag(b"xmas_santa", 2);
                for place in places {
                    if self.vids_of_race(Some(place.0), SANTA).is_empty() {
                        self.spawn_event_mob_randomly(place, SANTA);
                    }
                }
            }
            _ => {}
        }
    }

    /// `pre_event_hc`'s arm (`G/questmanager.cpp:1520-1574`): the three NPCs stand when the
    /// flag turns on, and those on maps 3, 23 and 43 go when it turns off.
    fn pre_event_arm(&mut self, prev: i32, value: i32) {
        if value != 0 && prev == 0 {
            for (map, x, y) in PRE_EVENT_NPCS {
                self.spawn_at_base(map, PRE_EVENT_NPC, (x * 100, y * 100), -1);
            }
        } else if value == 0 && prev != 0 {
            self.destroy_race(PRE_EVENT_NPC, &[3, 23, 43]);
        }
    }

    /// `new_xmas_event`'s arm (`G/questmanager.cpp:1627-1687`): any non-zero value stands
    /// the new Santa on each of maps 1, 21 and 41 where a Channel world hosts the map and
    /// none stands there yet; 0 removes every one.
    fn new_xmas_arm(&mut self, value: i32) {
        if value == 0 {
            self.destroy_race(NEW_SANTA, &[]);
            return;
        }
        for (map, dx, dy, dir) in NEW_SANTAS {
            for place in self.places_hosting(map) {
                let standing = self
                    .vids_of_race(Some(place.0), NEW_SANTA)
                    .iter()
                    .any(|&(_, on)| on == map);
                if standing {
                    continue;
                }
                if let Some((bx, by)) = self.map_base(place) {
                    let _stood =
                        self.spawn_event_mob(place, NEW_SANTA, (bx + dx, by + dy), dir, false);
                }
            }
        }
    }

    /// `xmas::SpawnEventHelper` (`G/xmas_event.cpp:140-185`): `true` stands a firework
    /// seller on each hosted map of its list, with no check for one standing already;
    /// `false` removes every seller.
    pub(super) fn spawn_event_helper(&mut self, spawn: bool) {
        if spawn {
            for (map, x, y) in FIREWORK_SELLERS {
                self.spawn_at_base(map, FIREWORK_SELLER, (x * 100, y * 100), -1);
            }
        } else {
            self.destroy_race(FIREWORK_SELLER, &[]);
        }
    }

    /// `SpawnMob(vnum, map, base + offset, 0, false, dir)` on every Channel world hosting
    /// `map`. A world with no base for the map logs and skips it, as legacy's `sys_err` does.
    fn spawn_at_base(&mut self, map: i32, vnum: u32, (dx, dy): (i32, i32), dir: i32) {
        for place in self.places_hosting(map) {
            let Some((bx, by)) = self.map_base(place) else {
                tracing::warn!(?place, "cannot get map base position");
                continue;
            };
            let _stood = self.spawn_event_mob(place, vnum, (bx + dx, by + dy), dir, false);
        }
    }

    /// `GetCharactersByRaceNum(race)` on one Channel world, or on all with `None`: each
    /// character's VID and map. Every race the arms ask for is registered (an NPC proto or
    /// a Christmas race).
    pub(super) fn vids_of_race(&self, channel: Option<u8>, race: u32) -> Vec<(u32, i32)> {
        self.npcs
            .iter()
            .filter(|(&(on, _), _)| channel.is_none_or(|wanted| wanted == on))
            .flat_map(|(&(_, map), list)| {
                list.npcs
                    .iter()
                    .filter(|npc| u32::from(npc.race) == race)
                    .map(move |npc| (npc.vid, map))
            })
            .collect()
    }

    /// `M2_DESTROY_CHARACTER` of every character of `race`, or of those on `maps` when the
    /// list is not empty.
    fn destroy_race(&mut self, race: u32, maps: &[i32]) {
        for (vid, map) in self.vids_of_race(None, race) {
            if maps.is_empty() || maps.contains(&map) {
                self.destroy_npc(vid);
            }
        }
    }

    /// One `CHAT_TYPE_COMMAND` line to every in-game character
    /// (`DESC_MANAGER::GetClientSet`, each with a character), rendered for each hearer.
    fn command_to_everyone(&self, format: &[u8], args: &[Arg<'_>]) {
        let Some(clients) = self.clients.as_ref() else {
            return;
        };
        clients.deliver_everywhere(|entry| {
            Some(chat_packet(
                Recipient {
                    strings: &self.locale,
                    language: entry.language,
                    empire: entry.empire,
                },
                CHAT_TYPE_COMMAND,
                format,
                args,
            ))
        });
    }

    /// `BroadcastEventFlagOnLogin` (`G/questmanager.cpp:1701-1744`): the lines a character
    /// hears while it loads, one per non-zero flag of the seven, in legacy's order.
    #[must_use]
    pub(super) fn login_event_lines(&self) -> Vec<Vec<u8>> {
        let Some(quests) = self.quests.as_ref() else {
            return Vec::new();
        };
        let flag = |name: &[u8]| quests.manager().event_flag(name);
        let mut lines = Vec::new();
        for (name, word) in [
            (b"world_boss_event".as_slice(), b"worldboss".as_slice()),
            (b"xmas_snow", b"xmas_snow"),
            (b"xmas_boom", b"xmas_boom"),
            (b"xmas_tree", b"xmas_tree"),
        ] {
            let value = flag(name);
            if value != 0 {
                lines.push(format!("{} {value}", String::from_utf8_lossy(word)).into_bytes());
            }
        }
        if flag(b"day") != 0 {
            lines.push(b"DayMode dark".to_vec());
        }
        let value = flag(b"newyear_boom");
        if value != 0 {
            lines.push(format!("newyear_boom {value}").into_bytes());
        }
        let value = flag(b"eclipse");
        if value != 0 {
            let mode = if value == 1 { "dark" } else { "light" };
            lines.push(format!("DayMode {mode}").into_bytes());
        }
        lines
    }

    /// `ACMD(do_get_event_flag)`: one `name value` line per flag on the caller, in
    /// name order. The `BTreeMap` walks sorted, as `std::map` does. The three `prob`
    /// lines are dead here: they need the test server off, and it is on.
    pub(super) fn run_get_event_flag(&self, vid: Vid, mover: Mover) {
        let Some(quests) = self.quests.as_ref() else {
            return;
        };
        for (name, value) in quests.manager().event_flags() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"%s %d",
                &[Arg::Text(name), Arg::Int(i64::from(*value))],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_flag_value, FIREWORK_SELLER, NEW_SANTA, SANTA, XMAS_TREE};
    use crate::client_registry::{ChannelClients, ClientEntry, Lease};
    use crate::game_loop_messages::EventLine;
    use crate::game_state::fixtures::{a_world, enter, TestClock, PLACE};
    use crate::game_state::GameState;
    use common::vid::Vid;
    use gamedata::map_atlas::MapRegion;
    use gamedata::mob_locale_names::MobNamesByLanguage;
    use gamedata::server_attr::{CellBlock, SectreeGrid, ServerAttr, SECTREE_CELLS};
    use std::sync::Arc;

    /// `strtol`'s walk: the sign, the digit run, the saturation, and the no-digit zero.
    #[test]
    fn the_flag_value_reads_strtol() {
        assert_eq!(parse_flag_value(b"123"), 123);
        assert_eq!(parse_flag_value(b"+123abc"), 123);
        assert_eq!(parse_flag_value(b"  123"), 0);
        assert_eq!(parse_flag_value(b"-5"), -5);
        assert_eq!(parse_flag_value(b"99999999999999999999"), i32::MAX);
        assert_eq!(parse_flag_value(b"-99999999999999999999"), i32::MIN);
        assert_eq!(parse_flag_value(b"no digits"), 0);
        assert_eq!(parse_flag_value(b""), 0);
    }

    const VID: u32 = 0x6001;

    fn mover() -> crate::item_move::Mover {
        crate::item_move::Mover {
            recently_fought: false,
            empire: 1,
            language: 0,
            pk_mode: 4,
            affect_flags: [0; 2],
            dead: false,
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

    /// The set lands and answers; a `mob_*` name clamps to 0..1000; a missing word
    /// is silent; the list walks the table in name order.
    #[test]
    fn the_set_lands_and_the_list_walks_in_name_order() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = enter(&mut world, VID, (3200, 3200), 0);
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager =
            quest::manager::Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        world.quests = Some(crate::game_state::quests::Quests::new(
            manager,
            std::sync::Arc::default(),
            MobNamesByLanguage::default(),
        ));
        world.run_event_flag(Vid::new(VID), b"cards_event 1", mover());
        assert_eq!(
            text_of(&inbox.try_recv().expect("the set answers")),
            b"RequestSetEventFlag cards_event 1"
        );
        world.run_event_flag(Vid::new(VID), b"mob_gold 5000", mover());
        assert_eq!(
            text_of(&inbox.try_recv().expect("the clamp answers")),
            b"RequestSetEventFlag mob_gold 1000"
        );
        world.run_event_flag(Vid::new(VID), b"cards_event", mover());
        assert!(inbox.try_recv().is_err(), "a missing word is silent");
        world.run_get_event_flag(Vid::new(VID), mover());
        // The boot's two seeds sort with the two sets.
        for expected in [
            b"cards_event 1".as_slice(),
            b"guild_disband_delay 1",
            b"guild_withdraw_delay 1",
            b"mob_gold 1000",
        ] {
            assert_eq!(text_of(&inbox.try_recv().expect("a flag lists")), expected);
        }
        assert!(inbox.try_recv().is_err(), "the list ends");
        // The quest reader sees the set flag.
        assert_eq!(
            world
                .quests
                .as_ref()
                .expect("quests loaded")
                .manager()
                .event_flag(b"cards_event"),
            1
        );
    }

    /// With no quests the set still answers and the list stays silent.
    #[test]
    fn without_quests_the_set_answers_and_the_list_stays_silent() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = enter(&mut world, VID, (3200, 3200), 0);
        world.run_event_flag(Vid::new(VID), b"cards_event 1", mover());
        assert_eq!(
            text_of(&inbox.try_recv().expect("the set answers")),
            b"RequestSetEventFlag cards_event 1"
        );
        world.run_get_event_flag(Vid::new(VID), mover());
        assert!(inbox.try_recv().is_err(), "no table, no lines");
    }

    /// A world hosting [`PLACE`] (map 41) as twelve sectrees square, all cells open, whose
    /// respawner knows the event NPCs, with the client registry every broadcast goes to.
    fn event_world(clock: &TestClock) -> (GameState, Arc<ChannelClients>) {
        event_world_on(clock, PLACE.1, (0, 0))
    }

    /// As [`event_world`], hosting `map` on Channel 1 with its corner at `base` instead.
    fn event_world_on(
        clock: &TestClock,
        map: i32,
        (sx, sy): (i32, i32),
    ) -> (GameState, Arc<ChannelClients>) {
        use gamedata::mob_proto::{MobProto, MobProtos, CHAR_TYPE_NPC};
        use world::cells::{HostedCells, MapCells};
        use world::npc::{MapNpcs, NpcVids, Respawner, FIRST_NPC_VID};
        let clients = Arc::new(ChannelClients::new());
        let mut world = a_world(clock)
            .with_clients(Arc::clone(&clients))
            .with_quests(quests());
        let region = MapRegion {
            index: map,
            name: b"test".to_vec(),
            sx,
            sy,
            ex: sx + 12 * 6_400,
            ey: sy + 12 * 6_400,
            spawn: (0, 0),
            empire_spawns: None,
        };
        let grid = SectreeGrid::of(&region).expect("the region is whole sectrees");
        world.host_map(PLACE.0, map, grid);
        let blocks = (0..144)
            .map(|_| CellBlock::from_cells(&vec![0; SECTREE_CELLS * SECTREE_CELLS]))
            .collect();
        let attr = ServerAttr::from_blocks(12, 12, blocks).expect("144 blocks");
        let mut hosted = HostedCells::default();
        hosted.insert(map, MapCells::new(grid, Arc::new(attr)));
        let rows = [FIREWORK_SELLER, SANTA, NEW_SANTA, XMAS_TREE]
            .into_iter()
            .map(|vnum| MobProto {
                line: 2,
                table: gamedata::records::MobTableRecord {
                    vnum,
                    mob_type: CHAR_TYPE_NPC,
                    max_hp: 100,
                    ..gamedata::records::MobTableRecord::default()
                },
            })
            .collect();
        let respawner = Respawner::new(
            Arc::new(MobProtos::from_rows(rows)),
            Arc::default(),
            Arc::new(gamedata::mob_locale_names::MobLocaleNames::parse(b"VNUM\tNAME\n").unwrap()),
            NpcVids::starting_at(FIRST_NPC_VID + 100),
        );
        world.install_respawner(respawner, Arc::new(hosted));
        world.stand_npcs(PLACE.0, &region, MapNpcs::default());
        (world, clients)
    }

    /// The points of every standing character of `race` on [`PLACE`].
    fn standing(world: &GameState, race: u32) -> Vec<(i32, i32)> {
        standing_on(world, PLACE, race)
    }

    /// The points of every standing character of `race` on `place`.
    fn standing_on(world: &GameState, place: (u8, i32), race: u32) -> Vec<(i32, i32)> {
        world.npcs[&place]
            .npcs
            .iter()
            .filter(|npc| u32::from(npc.race) == race)
            .map(|npc| (npc.x, npc.y))
            .collect()
    }

    /// On a world hosting Seohan (61, base 358400, 153600): `xmas_santa 1` turns the flag
    /// to 2 and stands one Santa somewhere on the map; a second 1 stands no second Santa
    /// (the Defect's inverted check is not reproduced); 0 removes it. `xmas_tree` stands
    /// the tree at its fixed point once and removes it at 0.
    #[test]
    fn the_santa_and_the_tree_stand_on_seohan() {
        let clock = TestClock::default();
        let (mut world, _clients) = event_world_on(&clock, 61, (358_400, 153_600));
        let seohan = (PLACE.0, 61);
        world.set_event_flag(b"xmas_santa", 1);
        assert_eq!(flag(&world, b"xmas_santa"), 2);
        assert_eq!(
            standing_on(&world, seohan, SANTA).len(),
            1,
            "one Santa comes"
        );
        world.set_event_flag(b"xmas_santa", 1);
        assert_eq!(
            standing_on(&world, seohan, SANTA).len(),
            1,
            "never a second"
        );
        world.set_event_flag(b"xmas_santa", 0);
        assert!(standing_on(&world, seohan, SANTA).is_empty());
        world.set_event_flag(b"xmas_tree", 1);
        world.set_event_flag(b"xmas_tree", 2);
        assert_eq!(
            standing_on(&world, seohan, XMAS_TREE),
            vec![(76_500 + 358_400, 60_900 + 153_600)]
        );
        world.set_event_flag(b"xmas_tree", 0);
        assert!(standing_on(&world, seohan, XMAS_TREE).is_empty());
    }

    /// The chat type and text of every chat record in `inbox`.
    fn chats(inbox: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) -> Vec<(u8, Vec<u8>)> {
        let mut lines = Vec::new();
        while let Ok(record) = inbox.try_recv() {
            if record[0] == 4 {
                lines.push((record[3], text_of(&record)));
            }
        }
        lines
    }

    /// `vid`'s seat in the registry, as the descriptor of an in-game character holds it.
    fn seat(clients: &Arc<ChannelClients>, vid: u32) -> Lease {
        clients.join(ClientEntry {
            channel: PLACE.0,
            map: PLACE.1,
            name: format!("P{vid}"),
            vid,
            empire: 1,
            language: 0,
        })
    }

    /// The chat type and text of every record a registry seat was sent.
    fn heard(lease: &mut Lease) -> Vec<(u8, Vec<u8>)> {
        std::iter::from_fn(|| lease.try_next())
            .map(|record| (record[3], text_of(&record)))
            .collect()
    }

    /// `xmas_boom` rings for every in-game character on every set, unchanged or not; the
    /// sellers stand only as it turns on (map 41's at base + 318 m, 629 m) and go as it
    /// turns off.
    #[test]
    fn the_boom_rings_everyone_and_brings_the_sellers_once() {
        use protocol::gc_chat::CHAT_TYPE_COMMAND;
        let clock = TestClock::default();
        let (mut world, clients) = event_world(&clock);
        let mut alpha_inbox = enter(&mut world, VID, (31_800, 62_900), 0);
        let mut alpha = seat(&clients, VID);
        let mut beta = seat(&clients, VID + 1);
        chats(&mut alpha_inbox);
        world.run_event_line(Vid::new(VID), EventLine::XmasBoom, b"1", mover());
        let rung = vec![(CHAT_TYPE_COMMAND, b"xmas_boom 1".to_vec())];
        assert_eq!(heard(&mut alpha), rung, "the caller hears it");
        assert_eq!(heard(&mut beta), rung, "every character hears it");
        assert!(
            chats(&mut alpha_inbox).is_empty(),
            "no line answers the caller"
        );
        assert_eq!(standing(&world, FIREWORK_SELLER), vec![(31_800, 62_900)]);
        world.run_event_line(Vid::new(VID), EventLine::XmasBoom, b"1", mover());
        assert_eq!(heard(&mut beta), rung, "an unchanged set rings again");
        assert_eq!(
            standing(&world, FIREWORK_SELLER).len(),
            1,
            "no second seller"
        );
        world.run_event_line(Vid::new(VID), EventLine::XmasBoom, b"", mover());
        assert_eq!(
            heard(&mut beta),
            vec![(CHAT_TYPE_COMMAND, b"xmas_boom 0".to_vec())],
            "a missing word is 0"
        );
        assert!(
            standing(&world, FIREWORK_SELLER).is_empty(),
            "the sellers go"
        );
    }

    /// `/eventhelper 1` stands a seller every time, with no check; any other word removes
    /// them all. Each answers its line.
    #[test]
    fn the_event_helper_spawns_unchecked_and_any_other_word_removes() {
        let clock = TestClock::default();
        let (mut world, _clients) = event_world(&clock);
        let mut alpha = enter(&mut world, VID, (3_200, 3_200), 0);
        chats(&mut alpha);
        world.run_event_line(Vid::new(VID), EventLine::EventHelper, b"1", mover());
        world.run_event_line(Vid::new(VID), EventLine::EventHelper, b"1", mover());
        assert_eq!(standing(&world, FIREWORK_SELLER).len(), 2);
        world.run_event_line(Vid::new(VID), EventLine::EventHelper, b"2", mover());
        assert!(standing(&world, FIREWORK_SELLER).is_empty());
        let texts: Vec<Vec<u8>> = chats(&mut alpha)
            .into_iter()
            .map(|(_, text)| text)
            .collect();
        assert_eq!(
            texts,
            vec![
                b"Event Helper Spawn".to_vec(),
                b"Event Helper Spawn".to_vec(),
                b"Event Helper Delete".to_vec(),
            ]
        );
    }

    /// `new_xmas_event` stands one new Santa on map 41 at base + (357 m, 743 m) when none
    /// stands, whatever the non-zero value, and 0 removes it. `day` darkens and lightens.
    #[test]
    fn the_new_santa_stands_once_and_the_day_turns() {
        let clock = TestClock::default();
        let (mut world, clients) = event_world(&clock);
        let mut alpha = seat(&clients, VID);
        world.set_event_flag(b"new_xmas_event", 1);
        world.set_event_flag(b"new_xmas_event", 5);
        assert_eq!(standing(&world, NEW_SANTA), vec![(35_700, 74_300)]);
        world.set_event_flag(b"new_xmas_event", 0);
        assert!(standing(&world, NEW_SANTA).is_empty());
        world.set_event_flag(b"day", 3);
        world.set_event_flag(b"day", 0);
        world.set_event_flag(b"eclipse", 2);
        let texts: Vec<Vec<u8>> = heard(&mut alpha)
            .into_iter()
            .map(|(_, text)| text)
            .collect();
        assert_eq!(
            texts,
            vec![
                b"DayMode dark".to_vec(),
                b"DayMode light".to_vec(),
                b"DayMode light".to_vec(),
            ],
            "eclipse is dark only at 1"
        );
        assert!(
            standing(&world, FIREWORK_SELLER).is_empty(),
            "day's sellers stood and went, map 41's point lies in the region"
        );
    }

    /// The respawn's gates (`G/xmas_event.cpp:115-141`): at the flag 0 nothing comes and at
    /// a standing Santa no second comes, each leaving nothing armed; with the flag on and
    /// no Santa standing, Santa comes to the map and nothing re-arms; on a map none hosts
    /// the spawn re-arms 5 s on.
    #[test]
    fn the_santa_respawn_checks_the_flag_the_standing_and_the_map() {
        use crate::game_loop::PulseProcessor;
        use crate::game_state::regen::Due;
        let clock = TestClock::default();
        let (mut world, _clients) = event_world_on(&clock, 61, (358_400, 153_600));
        let seohan = (PLACE.0, 61);
        // At the flag 0 nothing comes.
        world.spawn_santa(61, 50 * 60, 0);
        assert_eq!(world.regens.due.len(), 1);
        let (when, _) = world.regens.due.iter().next().copied().expect("armed");
        world.process_pulse(when);
        assert!(world.regens.due.is_empty(), "a lapsed flag-0 run ends it");
        assert!(standing_on(&world, seohan, SANTA).is_empty());
        // With the flag on Santa comes.
        world.set_event_flag(b"xmas_santa", 2);
        world.spawn_santa(61, 50 * 60, when);
        let (due, _) = world
            .regens
            .due
            .iter()
            .next()
            .copied()
            .expect("armed again");
        assert_eq!(
            due,
            when + crate::game_state::regen::pulses(50),
            "50 s, the test server"
        );
        world.process_pulse(due);
        assert_eq!(standing_on(&world, seohan, SANTA).len(), 1);
        assert!(
            world.regens.due.is_empty(),
            "a landed Santa re-arms nothing"
        );
        // At a standing Santa no second comes.
        world.spawn_santa(61, 50 * 60, due);
        let (next, _) = world.regens.due.iter().next().copied().expect("armed");
        world.process_pulse(next);
        assert_eq!(standing_on(&world, seohan, SANTA).len(), 1);
        assert!(world.regens.due.is_empty(), "a standing Santa ends it");
        // On a map none hosts the spawn re-arms 5 s on.
        world.set_event_flag(b"xmas_santa", 0);
        assert!(standing_on(&world, seohan, SANTA).is_empty());
        world.set_event_flag(b"xmas_santa", 2);
        world.spawn_santa(62, 50 * 60, next);
        let (third, _) = world.regens.due.iter().next().copied().expect("armed");
        world.process_pulse(third);
        assert_eq!(
            world
                .regens
                .due
                .iter()
                .filter_map(|&(when, ref what)| match what {
                    Due::SantaSpawn { map } => Some((when, *map)),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            [(third + crate::game_state::regen::pulses(5), 62)],
            "a map none hosts re-arms 5 s on"
        );
    }

    /// `xmas_santa 1` on a world not hosting Seohan does nothing: the flag stays 1.
    #[test]
    fn the_santa_needs_seohan() {
        let clock = TestClock::default();
        let (mut world, _clients) = event_world(&clock);
        world.run_event_line(Vid::new(VID), EventLine::XmasSanta, b"1", mover());
        assert_eq!(flag(&world, b"xmas_santa"), 1);
        assert!(standing(&world, SANTA).is_empty());
    }

    /// The rates start at 100 and take any set value, 0 included; a set of an unrelated
    /// name leaves them.
    #[test]
    fn the_rates_take_the_set_value_zero_included() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        assert_eq!(world.mob_rates.exp, 100);
        world.set_event_flag(b"mob_exp", 0);
        world.set_event_flag(b"mob_item", 250);
        world.set_event_flag(b"mob_exp_buyer", 300);
        assert_eq!(world.mob_rates.exp, 0, "0 is a rate, not 'unset'");
        assert_eq!(world.mob_rates.item, 250);
        assert_eq!(world.mob_rates.gold_amount, 100);
    }

    /// The login lines: one per non-zero flag of the seven, in legacy's order; `day` is
    /// always dark, `eclipse` dark only at 1.
    #[test]
    fn the_login_lines_follow_legacys_order() {
        let clock = TestClock::default();
        let mut world = a_world(&clock).with_quests(quests());
        assert!(world.login_event_lines().is_empty(), "every flag is 0");
        for (name, value) in [
            (b"eclipse".as_slice(), 2),
            (b"newyear_boom", 4),
            (b"day", 7),
            (b"xmas_tree", 0),
            (b"xmas_snow", 3),
            (b"world_boss_event", 1),
        ] {
            world.set_event_flag(name, value);
        }
        let lines = world.login_event_lines();
        let lines: Vec<&[u8]> = lines.iter().map(Vec::as_slice).collect();
        assert_eq!(
            lines,
            [
                b"worldboss 1".as_slice(),
                b"xmas_snow 3",
                b"DayMode dark",
                b"newyear_boom 4",
                b"DayMode light",
            ]
        );
    }

    /// The quest runtime over the legacy locale.
    fn quests() -> crate::game_state::quests::Quests {
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

    fn flag(world: &GameState, name: &[u8]) -> i32 {
        world
            .quests
            .as_ref()
            .expect("quests loaded")
            .manager()
            .event_flag(name)
    }
}
