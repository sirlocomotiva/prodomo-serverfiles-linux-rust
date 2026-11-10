//! The siege and kingdom war events (`CEventManager`'s 13 and 14,
//! `G/event_manager.cpp:1531-1743`): their stones, their notices, their `OnDead` kill
//! accounting (`:803-926`) and the kingdom score record (`SendKingdomWarScorePacket`,
//! `:1732-1742`).
//!
//! Both run on map 103 (`SIEGE_MAP_INDEX`, `KINGDOM_WAR_MAP_INDEX`,
//! `G/event_manager.h:36-37`), which the owner's index lists (`metin2_map_t1`) and
//! every Channel hosts; the start/end shells live in `start_game_event` and
//! `end_game_event`, and this module holds the pieces they call.
//!
//! The siege start stands the three kingdom stones (8020-8022) at the base plus
//! offsets, clears the defeat count and the three score flags, chats the start and
//! raises `siege_war_event`. A stone's death on 103 chats its empire's line, counts
//! the defeat, warps that empire's players on 103 home 3 s later, and counts the
//! second defeat's score flag at once before ending the war (`CQuestManager::SetEventFlag`,
//! since `RequestSetEventFlag` only wrote the DB in the retired peer). The end pays
//! each empire 25, 50 or 100 exp (the winner's 3 also 50 loot) for 48 hours, clears
//! the flags, chats the end, warps every player on 103 home 60 s later, kills the
//! monsters, stones and veins on 103, and chats the teleport line to the map.
//!
//! The kingdom start zeroes the four score rows (all-empires first) and chats the
//! start; a player a PC of another empire kills on 103 counts one for the killer's
//! empire and fans the score record to every PC on 103. The end pays the same table
//! by the sorted rows' positions (all-empires holds 0 and sorts first, so the
//! kingdoms take positions 1-3), clears the rows, chats the end, warps 103 home
//! and chats the map its teleport line.
//! An entrant on 103 while the war runs hears its score record ahead of the time
//! record (`G/input_login.cpp:680-683`).
//!
//! The 48-hour grants and the priv notices are `give_empire_priv`'s; the `m_bSiegeScore + 1`
//! fallback answers an undefeated empire when the war is forced to end. The end keeps
//! legacy's order: flags first, the teleport line last.
//!
//! # What this build does not carry
//!
//! The DB peer is retired (ADR-0001), so `RequestSetEventFlag` writes the flag
//! directly, as the world has no other process to tell. The P2P siege notice never
//! existed for these two arms: both run whole on the one process.

use common::vid::Vid;
use gamedata::mob_proto::{CHAR_TYPE_MONSTER, CHAR_TYPE_STONE};
use protocol::gc_chat::{CHAT_TYPE_BIG_NOTICE, CHAT_TYPE_NOTICE};
use protocol::gc_vid::GcEventKwScore;

use super::empire_priv::{PRIV_EXP_PCT, PRIV_ITEM_DROP};
use super::GameState;
use crate::chat_line::Arg;

/// `SIEGE_MAP_INDEX` and `KINGDOM_WAR_MAP_INDEX`.
pub(super) const WAR_MAP: i32 = 103;
/// The three kingdom stones (`SetSiegeWarEvent`'s `8020 + i`).
const STONE_VNUMS: [u32; 3] = [8020, 8021, 8022];
/// The stones' offsets in map units (`s_aStonePos`, each `* 100`).
const STONE_OFFSETS: [(i32, i32); 3] = [(224, 302), (433, 566), (467, 216)];
/// The stone's empire, by stone slot: Shinsoo, Jinno, Chunjo.
const STONE_EMPIRES: [u8; 3] = [1, 3, 2];
/// The defeat count's empire score flag, by stone slot.
const STONE_FLAGS: [&[u8]; 3] = [
    b"siege_shinsoo_score",
    b"siege_jinno_score",
    b"siege_chunjo_score",
];
/// The stone-destroyed line, by stone slot.
const STONE_LINES: [&[u8]; 3] = [
    b"Shinsoo Stone has been destroyed. Shinsoo players will be teleported to their villages.",
    b"Jinno Stone has been destroyed. Jinno players will be teleported to their villages.",
    b"Chunjo Stone has been destroyed. Chunjo players will be teleported to their villages.",
];
/// The 48-hour reward, in seconds.
const REWARD_SECS: u64 = 48 * 60 * 60;
/// The stone-kill home warp's delay, in seconds.
const STONE_WARP_SECS: u64 = 3;
/// The forced-end home warp's delay, in seconds.
const END_WARP_SECS: u64 = 60;
/// The teleported line (`SendNoticeMap`'s `LC_TEXT`, read from the owner's
/// `locale_string.txt` through the hearer's language).
const TELEPORT_LINE: &[u8] = b"Everyone will be teleported into the city shortly.";

/// The wars' scores: `m_bSiegeScore` and `m_aKingdomWarScore`, with the home warps
/// the end-arms and the stone kills queue as Pulses.
#[derive(Debug, Default)]
pub(super) struct SiegeWar {
    /// The siege defeats so far.
    siege_score: u8,
    /// The kingdom kills by empire slot (0 is all-empires, always 0).
    kingdom: [KingdomRow; 4],
    /// The queued home warps: the Pulse and the empire (0 is every empire).
    warps: Vec<(u64, u8)>,
}

/// `mining::IsVeinOfOre` (`G/mining.cpp:469-480`): the veins of `mining::info`.
fn is_vein(vnum: u32) -> bool {
    (20_047..=20_059).contains(&vnum) || (30_301..=30_306).contains(&vnum)
}

/// One `TKingdomWarCount`: the kingdom and its kills.
#[derive(Clone, Copy, Debug, Default)]
struct KingdomRow {
    /// `bKingdom`.
    kingdom: u8,
    /// `wCount`.
    count: u16,
}

impl GameState {
    /// `SetSiegeWarEvent(true)` (`:1531-1565`): the stones stand on every Channel
    /// hosting 103, the count and the three flags clear, the start chats and the
    /// flag rises. A 103 with no base on a Channel stands nothing there, as
    /// legacy's null map returns false.
    pub(super) fn start_siege_war(&mut self) {
        let places = self.places_hosting(WAR_MAP);
        let mut stood_any = false;
        for place in &places {
            let Some((bx, by)) = self.map_base(*place) else {
                continue;
            };
            for (slot, vnum) in STONE_VNUMS.iter().enumerate() {
                let (dx, dy) = STONE_OFFSETS[slot];
                let _stood =
                    self.spawn_event_mob(*place, *vnum, (bx + dx * 100, by + dy * 100), 0, false);
                stood_any = true;
            }
        }
        let _ = stood_any;
        self.siege_war.siege_score = 0;
        self.set_event_flag(b"siege_shinsoo_score", 0);
        self.set_event_flag(b"siege_chunjo_score", 0);
        self.set_event_flag(b"siege_jinno_score", 0);
        self.broadcast_notice_text(
            b"Siege War Event has started. Coordinate with the kingdom to become victorious.",
        );
        self.set_event_flag(b"siege_war_event", 1);
    }

    /// `SetSiegeWarEvent(false)` (`:1566-1658`): each empire's score pays, the count
    /// and the flags clear, the flag falls, the end chats, the 60 s home warp
    /// queues, the monsters, stones and veins on 103 die, and the map hears the
    /// teleport line.
    pub(super) fn end_siege_war(&mut self) {
        let fallback = i32::from(self.siege_war.siege_score) + 1;
        for empire in 1..=3u8 {
            let flag = match empire {
                1 => b"siege_shinsoo_score".as_slice(),
                2 => b"siege_chunjo_score".as_slice(),
                _ => b"siege_jinno_score".as_slice(),
            };
            let mut score = self.event_rate(flag);
            if score <= 0 {
                score = fallback;
            }
            self.pay_war_rank(empire, score);
        }
        self.siege_war.siege_score = 0;
        self.set_event_flag(b"siege_shinsoo_score", 0);
        self.set_event_flag(b"siege_chunjo_score", 0);
        self.set_event_flag(b"siege_jinno_score", 0);
        self.set_event_flag(b"siege_war_event", 0);
        self.broadcast_notice_text(b"Siege War Event has ended. Thank you for participating!");
        self.queue_war_home(END_WARP_SECS, 0);
        self.kill_war_map();
        self.tell_war_map(TELEPORT_LINE, false);
    }

    /// `SetKingdomWarEvent(true)` (`:1662-1680`): the four rows zero with their
    /// kingdoms, the start chats and the flag rises.
    pub(super) fn start_kingdom_war(&mut self) {
        for (slot, row) in self.siege_war.kingdom.iter_mut().enumerate() {
            row.kingdom = u8::try_from(slot).unwrap_or(0);
            row.count = 0;
        }
        self.broadcast_notice_text(
            b"Kingdom War Event has started. Sharpen your weapons and join the battlefield.",
        );
        self.set_event_flag(b"kingdom_war_event", 1);
    }

    /// `SetKingdomWarEvent(false)` (`:1681-1726`): the rows sort low to high and each
    /// kingdom's position pays, the rows clear, the flag falls, the end chats and
    /// the 60 s home warp queues.
    pub(super) fn end_kingdom_war(&mut self) {
        let mut rows = self.siege_war.kingdom;
        rows.sort_by_key(|row| row.count);
        for (position, row) in rows.iter().enumerate().skip(1) {
            let Ok(rank) = i32::try_from(position) else {
                continue;
            };
            self.pay_war_rank(row.kingdom, rank);
        }
        self.siege_war.kingdom = [KingdomRow::default(); 4];
        self.set_event_flag(b"kingdom_war_event", 0);
        self.broadcast_notice_text(b"Kingdom War Event has ended. Thank you for participating!");
        self.queue_war_home(END_WARP_SECS, 0);
        self.tell_war_map(TELEPORT_LINE, false);
    }

    /// The war reward table (`SetSiegeWarEvent`'s end arms, `SetKingdomWarEvent`'s
    /// end arms): rank 1 pays 25 exp, 2 pays 50 exp, 3 pays 100 exp and 50 loot,
    /// each for 48 hours. Any other rank pays nothing.
    fn pay_war_rank(&mut self, empire: u8, rank: i32) {
        let pulses = REWARD_SECS.saturating_mul(u64::from(crate::save::PASSES_PER_SEC));
        let ends = self.last_pulse.saturating_add(pulses);
        match rank {
            1 => self.give_empire_priv(empire, PRIV_EXP_PCT, 25, ends),
            2 => self.give_empire_priv(empire, PRIV_EXP_PCT, 50, ends),
            3 => {
                self.give_empire_priv(empire, PRIV_EXP_PCT, 100, ends);
                self.give_empire_priv(empire, PRIV_ITEM_DROP, 50, ends);
            }
            _ => {}
        }
    }

    /// `OnDead`'s stone arms (`:833-923`) for the stone of `vnum` that died on 103:
    /// the empire's line chats, the defeat counts, that empire's 3 s home warp
    /// queues, and the second defeat writes its score flag at once, then ends the
    /// war. Any other vnum is quiet.
    pub(super) fn siege_stone_dead(&mut self, vnum: u32) {
        let Some(slot) = STONE_VNUMS.iter().position(|stone| *stone == vnum) else {
            return;
        };
        self.broadcast_notice_text(STONE_LINES[slot]);
        self.siege_war.siege_score = self.siege_war.siege_score.saturating_add(1);
        self.queue_war_home(STONE_WARP_SECS, STONE_EMPIRES[slot]);
        let count = i32::from(self.siege_war.siege_score);
        if count == 2 {
            self.set_event_flag(STONE_FLAGS[slot], count);
            self.end_siege_war();
        } else {
            self.set_event_flag(STONE_FLAGS[slot], count);
        }
    }

    /// `OnDead`'s kingdom arm (`:807-828`) for the player `victim` the PC `killer`
    /// killed on 103: a cross-empire kill counts one for the killer's empire and
    /// fans the score record to every PC on 103. A same-empire kill is quiet.
    pub(super) fn kingdom_kill(&mut self, killer: Vid, victim: Vid) {
        let Some(killer_body) = self.bodies.get(&killer) else {
            return;
        };
        let Some(victim_body) = self.bodies.get(&victim) else {
            return;
        };
        if killer_body.card.empire == victim_body.card.empire {
            return;
        }
        let slot = usize::from(killer_body.card.empire);
        if let Some(row) = self.siege_war.kingdom.get_mut(slot) {
            row.count = row.count.saturating_add(1);
        }
        let scores = self.kingdom_scores();
        self.fan_kingdom_scores(scores);
    }

    /// The score record's three words: Shinsoo, Chunjo, Jinno.
    fn kingdom_scores(&self) -> [u16; 3] {
        let at = |slot: usize| self.siege_war.kingdom.get(slot).map_or(0, |row| row.count);
        [at(1), at(2), at(3)]
    }

    /// `SendKingdomWarScorePacket` to every PC standing on 103.
    fn fan_kingdom_scores(&mut self, scores: [u16; 3]) {
        let mut record = Vec::with_capacity(GcEventKwScore::WIRE_SIZE);
        GcEventKwScore::new(scores).encode_into(&mut record);
        let mut vids: Vec<Vid> = self
            .bodies
            .iter()
            .filter(|(_, body)| body.map == WAR_MAP)
            .map(|(vid, _)| *vid)
            .collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            let _sent = self.write_to_client(vid, record.clone());
        }
    }

    /// The login's score record (`G/input_login.cpp:680-683`): an entrant on 103
    /// while the kingdom war runs hears its words. The burst carries it before the
    /// time record, where legacy sends it before `TPacketGCTime`.
    pub(super) fn kingdom_score_line(&self, to: Vid) -> Option<Vec<u8>> {
        let on_war = self.bodies.get(&to).is_some_and(|body| body.map == WAR_MAP);
        if !on_war || self.event_rate(b"kingdom_war_event") == 0 {
            return None;
        }
        let mut record = Vec::with_capacity(GcEventKwScore::WIRE_SIZE);
        GcEventKwScore::new(self.kingdom_scores()).encode_into(&mut record);
        Some(record)
    }

    /// Queue a home warp `secs` seconds out for `empire` (0 is every empire).
    fn queue_war_home(&mut self, secs: u64, empire: u8) {
        let pulses = secs.saturating_mul(u64::from(crate::save::PASSES_PER_SEC));
        let due = self.last_pulse.saturating_add(pulses);
        self.siege_war.warps.push((due, empire));
    }

    /// The queued home warps' Pulse arm: every player on 103 of the queued empire
    /// (every empire for 0) warps to its empire start. An empire-0 player stays,
    /// as legacy's `sys_err` arm returns without warping it.
    pub(super) fn run_war_home(&mut self, pulse: u64) {
        let mut index = 0;
        while index < self.siege_war.warps.len() {
            let (due, empire) = self.siege_war.warps[index];
            if pulse < due {
                index += 1;
                continue;
            }
            self.siege_war.warps.remove(index);
            let mut vids: Vec<Vid> = self
                .bodies
                .iter()
                .filter(|(_, body)| {
                    body.map == WAR_MAP && (empire == 0 || body.card.empire == empire)
                })
                .map(|(vid, _)| *vid)
                .collect();
            vids.sort_by_key(|vid| vid.raw());
            for vid in vids {
                let Some(body) = self.bodies.get(&vid) else {
                    continue;
                };
                if body.card.empire == 0 {
                    continue;
                }
                if let Some(point) = crate::warp::go_home(body.card.empire) {
                    self.order_warp(vid, point);
                }
            }
        }
    }

    /// `FKillSectree` on 103 (`:194-204`): every monster, stone and vein there dies.
    /// `M2_DESTROY_CHARACTER` removes the dead outright; the corpses' timers never
    /// arm.
    fn kill_war_map(&mut self) {
        let mut vids: Vec<u32> = Vec::new();
        for ((_, map), list) in &self.npcs {
            if *map != WAR_MAP {
                continue;
            }
            for npc in &list.npcs {
                if !npc.dead
                    && (npc.char_type == CHAR_TYPE_MONSTER
                        || npc.char_type == CHAR_TYPE_STONE
                        || is_vein(npc.vnum))
                {
                    vids.push(npc.vid);
                }
            }
        }
        vids.sort_unstable();
        for vid in vids {
            self.dead_npc(vid, None);
            self.destroy_npc(vid);
        }
    }

    /// `SendNoticeMap(line, 103, big)` (`G/cmd_gm.cpp:1453-1500`): every PC on 103
    /// hears the line as a notice, or a big notice.
    fn tell_war_map(&self, line: &[u8], big: bool) {
        let chat = if big {
            CHAT_TYPE_BIG_NOTICE
        } else {
            CHAT_TYPE_NOTICE
        };
        let mut vids: Vec<Vid> = self
            .bodies
            .iter()
            .filter(|(_, body)| body.map == WAR_MAP)
            .map(|(vid, _)| *vid)
            .collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            if let Some(mover) = self.own_mover(vid) {
                self.tell(vid, mover, chat, b"%s", &[Arg::Text(line)]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{STONE_OFFSETS, STONE_VNUMS, WAR_MAP};
    use crate::client_registry::{ChannelClients, ClientEntry, ClientOrder};
    use crate::game_state::empire_priv::{PRIV_EXP_PCT, PRIV_ITEM_DROP};
    use crate::game_state::fixtures::{a_world, enter_on, TestClock};
    use common::vid::Vid;
    use gamedata::map_atlas::MapRegion;
    use gamedata::mob_proto::{MobProto, MobProtos, CHAR_TYPE_STONE};
    use gamedata::server_attr::{CellBlock, SectreeGrid, ServerAttr, SECTREE_CELLS};
    use std::sync::Arc;
    use world::cells::{HostedCells, MapCells};
    use world::npc::{MapNpcs, NpcVids, Respawner, FIRST_NPC_VID};

    /// The owner's map 103 base (`Setting.txt`).
    const BASE: (i32, i32) = (0, 25_600);

    /// A world hosting map 103 on Channel 1 as twelve sectrees square from the
    /// owner's base, all cells open, whose respawner knows the three stones.
    fn war_world(clock: &TestClock) -> crate::game_state::GameState {
        let mut world = a_world(clock)
            .with_clients(Arc::new(ChannelClients::new()))
            .with_quests(quests());
        let region = MapRegion {
            index: WAR_MAP,
            name: b"test".to_vec(),
            sx: BASE.0,
            sy: BASE.1,
            ex: BASE.0 + 12 * 6_400,
            ey: BASE.1 + 12 * 6_400,
            spawn: (0, 0),
            empire_spawns: None,
        };
        let grid = SectreeGrid::of(&region).expect("the region is whole sectrees");
        world.host_map(1, WAR_MAP, grid);
        let blocks = (0..144)
            .map(|_| CellBlock::from_cells(&vec![0; SECTREE_CELLS * SECTREE_CELLS]))
            .collect();
        let attr = ServerAttr::from_blocks(12, 12, blocks).expect("144 blocks");
        let mut hosted = HostedCells::default();
        hosted.insert(WAR_MAP, MapCells::new(grid, Arc::new(attr)));
        let rows = STONE_VNUMS
            .into_iter()
            .map(|vnum| MobProto {
                line: 2,
                table: gamedata::records::MobTableRecord {
                    vnum,
                    mob_type: CHAR_TYPE_STONE,
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
        world.stand_npcs(1, &region, MapNpcs::default());
        world
    }

    /// The quest runtime over the legacy locale.
    fn quests() -> crate::game_state::quests::Quests {
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager =
            quest::manager::Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        crate::game_state::quests::Quests::new(
            manager,
            Arc::default(),
            gamedata::mob_locale_names::MobNamesByLanguage::default(),
        )
    }

    /// The points of every live standing character of `race` on map 103.
    fn standing(world: &crate::game_state::GameState, race: u32) -> Vec<(i32, i32)> {
        world.npcs[&(1, WAR_MAP)]
            .npcs
            .iter()
            .filter(|npc| u32::from(npc.race) == race && !npc.dead)
            .map(|npc| (npc.x, npc.y))
            .collect()
    }

    /// The VID of the standing character of `race` on map 103.
    fn standing_vid(world: &crate::game_state::GameState, race: u32) -> u32 {
        world.npcs[&(1, WAR_MAP)]
            .npcs
            .iter()
            .find(|npc| u32::from(npc.race) == race)
            .map(|npc| npc.vid)
            .expect("the stone stands")
    }

    /// The text of a `GC_CHAT` line.
    fn text_of(line: &[u8]) -> Vec<u8> {
        assert_eq!(line[0], 4, "HEADER_GC_CHAT");
        line[10..].to_vec()
    }

    /// The start stands the three stones at the base plus offsets, clears the
    /// count and the score flags, chats the start and raises the flag.
    #[test]
    fn the_siege_start_stands_the_three_stones_and_chats_the_start() {
        let clock = TestClock::default();
        let mut world = war_world(&clock);
        let mut inbox = enter_on(&mut world, (1, WAR_MAP), 0x6001, (6400, 25_600 + 6400), 0);
        while inbox.try_recv().is_ok() {}
        world.start_game_event(13, 0, 0);
        for (slot, vnum) in STONE_VNUMS.iter().enumerate() {
            let (dx, dy) = STONE_OFFSETS[slot];
            assert_eq!(
                standing(&world, *vnum),
                [(BASE.0 + dx * 100, BASE.1 + dy * 100)],
                "stone {vnum}"
            );
        }
        assert_eq!(
            text_of(&inbox.try_recv().expect("the start")),
            b"Siege War Event has started. Coordinate with the kingdom to become victorious."
        );
        assert!(inbox.try_recv().is_err());
        assert_eq!(world.event_rate(b"siege_war_event"), 1);
        assert_eq!(world.event_rate(b"siege_shinsoo_score"), 0);
        assert_eq!(world.siege_war.siege_score, 0);
        while inbox.try_recv().is_ok() {}
    }

    /// The first stone's death chats its empire's line, counts the defeat and its
    /// score flag, and warps that empire's players on 103 home 3 s later; the
    /// other empires stay.
    #[test]
    fn the_first_stone_counts_and_warps_its_empire_home() {
        let clock = TestClock::default();
        let mut world = war_world(&clock);
        let place = (1, WAR_MAP);
        let mut shinsoo = enter_on(&mut world, place, 0x6001, (6400, 32_000), 0);
        let mut chunjo = enter_on(&mut world, place, 0x6002, (6500, 32_000), 0);
        world
            .bodies
            .get_mut(&Vid::new(0x6001))
            .expect("entered")
            .card
            .empire = 1;
        world
            .bodies
            .get_mut(&Vid::new(0x6002))
            .expect("entered")
            .card
            .empire = 2;
        while shinsoo.try_recv().is_ok() {}
        while chunjo.try_recv().is_ok() {}
        world.start_game_event(13, 0, 0);
        while shinsoo.try_recv().is_ok() {}
        while chunjo.try_recv().is_ok() {}
        let stone = standing_vid(&world, 8020);
        world.dead_npc(stone, Some(Vid::new(0x6001)));
        assert_eq!(
            text_of(&shinsoo.try_recv().expect("the line")),
            b"Shinsoo Stone has been destroyed. Shinsoo players will be teleported to their villages."
        );
        assert_eq!(
            text_of(&chunjo.try_recv().expect("the line")),
            b"Shinsoo Stone has been destroyed. Shinsoo players will be teleported to their villages."
        );
        assert_eq!(world.siege_war.siege_score, 1);
        assert_eq!(world.event_rate(b"siege_shinsoo_score"), 1);
        assert_eq!(world.event_rate(b"siege_war_event"), 1, "the war runs on");
        let due = 3 * u64::from(crate::save::PASSES_PER_SEC);
        assert_eq!(
            world.siege_war.warps,
            [(due, 1)],
            "Shinsoo's 3 s warp queues"
        );
        world.run_war_home(due - 1);
        assert_eq!(world.bodies[&Vid::new(0x6001)].map, WAR_MAP, "not yet");
        assert_eq!(world.siege_war.warps, [(due, 1)], "still queued");
        world.run_war_home(due);
        assert!(world.siege_war.warps.is_empty(), "the warp fires");
        while shinsoo.try_recv().is_ok() {}
        while chunjo.try_recv().is_ok() {}
    }

    /// The second stone's death writes its score flag at once, then ends the war:
    /// the empires are paid by rank, the flags fall, the map dies and the teleport
    /// line rides last.
    #[test]
    fn the_second_stone_ends_the_war_and_pays_the_ranks() {
        let clock = TestClock::default();
        let mut world = war_world(&clock);
        let place = (1, WAR_MAP);
        let mut inbox = enter_on(&mut world, place, 0x6001, (6400, 32_000), 0);
        while inbox.try_recv().is_ok() {}
        world.start_game_event(13, 0, 0);
        while inbox.try_recv().is_ok() {}
        world.dead_npc(standing_vid(&world, 8020), Some(Vid::new(0x6001)));
        while inbox.try_recv().is_ok() {}
        world.dead_npc(standing_vid(&world, 8021), Some(Vid::new(0x6001)));
        // The stone line, the end line, then the map's teleport line. The priv
        // grants fan through the client registry, which the entered inbox is not
        // on, so the test reads the rows, not the notices.
        let mut texts = Vec::new();
        while let Ok(line) = inbox.try_recv() {
            if line[0] == 4 {
                texts.push(text_of(&line));
            }
        }
        assert_eq!(
            texts[0],
            b"Jinno Stone has been destroyed. Jinno players will be teleported to their villages."
        );
        assert_eq!(
            texts[texts.len() - 2],
            b"Siege War Event has ended. Thank you for participating!"
        );
        assert_eq!(
            texts[texts.len() - 1],
            b"Everyone will be teleported into the city shortly."
        );
        // Shinsoo's 1 pays 25 exp, Jinno's 2 pays 50 exp, and Chunjo's 0
        // takes the fallback 3 and pays 100 exp with 50 loot.
        assert_eq!(world.empire_privs[usize::from(PRIV_EXP_PCT)][1].value, 25);
        assert_eq!(world.empire_privs[usize::from(PRIV_EXP_PCT)][3].value, 50);
        assert_eq!(world.empire_privs[usize::from(PRIV_EXP_PCT)][2].value, 100);
        assert_eq!(world.empire_privs[usize::from(PRIV_ITEM_DROP)][2].value, 50);
        assert_eq!(world.event_rate(b"siege_war_event"), 0);
        assert_eq!(world.event_rate(b"siege_shinsoo_score"), 0);
        assert_eq!(world.event_rate(b"siege_jinno_score"), 0);
        assert!(standing(&world, 8020).is_empty(), "the map died");
        assert!(standing(&world, 8021).is_empty(), "the map died");
        assert!(standing(&world, 8022).is_empty(), "the map died");
        while inbox.try_recv().is_ok() {}
    }

    /// The fired warp orders the empire's players home and skips an empire-0 one,
    /// as legacy's `sys_err` arm returns without warping it.
    #[test]
    fn the_fired_warp_orders_the_empire_home_and_skips_empire_zero() {
        let clock = TestClock::default();
        let clients = Arc::new(ChannelClients::new());
        let mut world = war_world(&clock).with_clients(Arc::clone(&clients));
        let place = (1, WAR_MAP);
        let mut shinsoo = clients.join(ClientEntry {
            channel: 1,
            map: WAR_MAP,
            name: "Shinsoo".to_string(),
            vid: 0x6001,
            empire: 1,
            language: 0,
        });
        let mut stray = clients.join(ClientEntry {
            channel: 1,
            map: WAR_MAP,
            name: "Stray".to_string(),
            vid: 0x6002,
            empire: 0,
            language: 0,
        });
        let _in1 = enter_on(&mut world, place, 0x6001, (6400, 32_000), 0);
        let _in2 = enter_on(&mut world, place, 0x6002, (6500, 32_000), 0);
        world
            .bodies
            .get_mut(&Vid::new(0x6002))
            .expect("entered")
            .card
            .empire = 0;
        world.start_game_event(13, 0, 0);
        world.dead_npc(standing_vid(&world, 8020), Some(Vid::new(0x6001)));
        let due = 3 * u64::from(crate::save::PASSES_PER_SEC);
        world.run_war_home(due);
        let mut shinsoo_orders = shinsoo.take_orders().expect("the queue is home");
        let ClientOrder::Warp { x, y, .. } = shinsoo_orders.try_recv().expect("the warp") else {
            panic!("a warp order");
        };
        assert_eq!((x, y), crate::warp::go_home(1).expect("Shinsoo's start"));
        assert!(shinsoo_orders.try_recv().is_err(), "one warp only");
        let mut stray_orders = stray.take_orders().expect("the queue is home");
        assert!(stray_orders.try_recv().is_err(), "empire 0 stays");
    }

    /// A killerless stone death stays quiet, as `OnDead`'s killer gate does: the
    /// quest and `FKillSectree` deaths never count.
    #[test]
    fn a_killerless_stone_death_stays_quiet() {
        let clock = TestClock::default();
        let mut world = war_world(&clock);
        let place = (1, WAR_MAP);
        let mut inbox = enter_on(&mut world, place, 0x6001, (6400, 32_000), 0);
        while inbox.try_recv().is_ok() {}
        world.start_game_event(13, 0, 0);
        while inbox.try_recv().is_ok() {}
        world.dead_npc(standing_vid(&world, 8020), None);
        while inbox.try_recv().is_ok() {}
        assert_eq!(world.siege_war.siege_score, 0, "no defeat counted");
        assert_eq!(world.event_rate(b"siege_shinsoo_score"), 0, "no flag set");
        assert_eq!(world.event_rate(b"siege_war_event"), 1, "the war runs on");
        while inbox.try_recv().is_ok() {}
    }

    /// A cross-empire kill on 103 counts for the killer's empire and fans the
    /// score record; a same-empire kill is quiet.
    #[test]
    fn the_kingdom_kill_counts_and_fans_the_score() {
        let clock = TestClock::default();
        let mut world = war_world(&clock);
        let place = (1, WAR_MAP);
        let mut killer = enter_on(&mut world, place, 0x6001, (6400, 32_000), 0);
        let mut victim = enter_on(&mut world, place, 0x6002, (6500, 32_000), 0);
        world
            .bodies
            .get_mut(&Vid::new(0x6002))
            .expect("entered")
            .card
            .empire = 2;
        while killer.try_recv().is_ok() {}
        while victim.try_recv().is_ok() {}
        world.start_game_event(14, 0, 0);
        assert_eq!(
            text_of(&killer.try_recv().expect("the start")),
            b"Kingdom War Event has started. Sharpen your weapons and join the battlefield."
        );
        while killer.try_recv().is_ok() {}
        while victim.try_recv().is_ok() {}
        world.kingdom_kill(Vid::new(0x6001), Vid::new(0x6002));
        let score = killer.try_recv().expect("the score");
        assert_eq!(score, [0x9d, 1, 0, 0, 0, 0, 0], "Shinsoo's kill fans 1/0/0");
        let score = victim.try_recv().expect("the score");
        assert_eq!(score, [0x9d, 1, 0, 0, 0, 0, 0], "the victim hears it too");
        world.kingdom_kill(Vid::new(0x6001), Vid::new(0x6001));
        assert!(killer.try_recv().is_err(), "same empire is quiet");
        assert_eq!(world.siege_war.kingdom[1].count, 1);
        world.end_game_event(14);
        assert_eq!(
            text_of(&killer.try_recv().expect("the end")),
            b"Kingdom War Event has ended. Thank you for participating!"
        );
        assert_eq!(world.event_rate(b"kingdom_war_event"), 0);
        while killer.try_recv().is_ok() {}
        while victim.try_recv().is_ok() {}
    }
}
