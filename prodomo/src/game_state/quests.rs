//! The quests: a click on an NPC runs its quests before its click trigger, and the dialog a script
//! sends is answered with `CG_SCRIPT_ANSWER` and `CG_QUEST_INPUT_STRING`
//! (`CInputMain::ScriptAnswer` and `QuestInputString`, `G/input_main.cpp:2200-2234`).
//!
//! The runtime is [`quest::manager::Manager`]; this is the world around it. It finds the
//! character's player id and the NPCs of its map, and it turns what a script sends into the
//! records its client is sent, in the order the script sent them: a `GC_SCRIPT` for each dialog
//! and a `GC_CHAT` for each chat line, whose text is formatted as `ChatPacket(type, "%s", text)`.
//!
//! The click (`CHARACTER::OnClick`, `G/char.cpp:6181-6352`) records the NPC and asks
//! `CQuestManager::Click` (`G/questmanager.cpp:918-990`) with the NPC's race, which is
//! `GetRaceNum`, a `WORD`. When a quest takes the click the NPC's click trigger does not run, so a
//! keeper whose quest answers opens no shop. A character that leaves the world ends its running
//! script unfinished (`CQuestManager::DisconnectPC`).
//!
//! # Divergences
//!
//! - **A dialog too long for `GC_SCRIPT`.** A dialog longer than 65529 bytes is logged and not
//!   sent; legacy sent it with sizes that had wrapped around (a Defect, see
//!   [`protocol::gc_script`]).
//! - **Where the quest's NPC is found.** `npc.get_race` finds the clicked NPC on the character's
//!   own Channel and map, where legacy's `CHARACTER_MANAGER::Find` looked across the process.
//! - **`mob_name` in a language past the last** reads English, where legacy read past its table
//!   ([`gamedata::mob_locale_names::MobNamesByLanguage`]).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use common::vid::Vid;
use gamedata::locale_string::LocaleStrings;
use gamedata::mob_locale_names::MobNamesByLanguage;
use gamedata::mob_proto::MobProtos;
use protocol::cg_quest_text::{CgQuestInputString, CG_QUEST_TEXT_FIELD_SIZE};
use protocol::gc_chat::{CHAT_TYPE_COMMAND, CHAT_TYPE_INFO, CHAT_TYPE_NOTICE, CHAT_TYPE_TALKING};
use protocol::gc_script::GcScript;
use quest::manager::{ChatKind, Game, Manager};
use tracing::warn;
use world::character::{number, Pcg32};
use world::npc::MapNpcs;

use super::GameState;
use crate::chat_line::{chat_packet, Arg};
use crate::game_loop_messages::GroundPlace;
use crate::item_move::{MoveItemRefused, Mover};

/// The quests boot loaded, with the monsters their `mob_name` names.
#[derive(Debug)]
pub struct Quests {
    /// The scripts, the NPCs they belong to and every character's running script.
    manager: Manager,
    /// `CMobManager`: a vnum with no proto has no name.
    protos: Arc<MobProtos>,
    /// Every language's mob names.
    names: MobNamesByLanguage,
}

impl Quests {
    /// The quests of `manager`, whose `mob_name` names the monsters of `protos` from `names`.
    #[must_use]
    pub fn new(manager: Manager, protos: Arc<MobProtos>, names: MobNamesByLanguage) -> Self {
        Self {
            manager,
            protos,
            names,
        }
    }

    /// The manager, which a test reads a flag or the running script from.
    #[must_use]
    pub fn manager(&self) -> &Manager {
        &self.manager
    }

    /// The manager, to change: a test sets `noguard` through it.
    pub fn manager_mut(&mut self) -> &mut Manager {
        &mut self.manager
    }
}

/// One quest request of a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestStep {
    /// `CG_SCRIPT_ANSWER`: above 250 continues a `wait`, otherwise picks a menu entry.
    Answer(u8),
    /// `CG_QUEST_INPUT_STRING`: the text an `input()` waits for.
    Input(Vec<u8>),
    /// `CG_SCRIPT_BUTTON`: the quest whose button the client pressed. The word is the
    /// quest's number (`makequestbutton`, `quest/questlib.lua:117-122`), and the high bit
    /// asks for the quest's information instead of its button
    /// (`CInputMain::ScriptButton`, `G/input_main.cpp:2188-2196`).
    Button(u32),
}

impl QuestStep {
    /// The text `CInputMain::QuestInputString` passes on: `strlcpy` into 65 bytes keeps at most
    /// 64 and stops at the first NUL.
    #[must_use]
    pub fn input(record: &CgQuestInputString) -> Self {
        let msg = &record.msg[..CG_QUEST_TEXT_FIELD_SIZE - 1];
        let end = msg.iter().position(|&byte| byte == 0).unwrap_or(msg.len());
        Self::Input(msg[..end].to_vec())
    }
}

/// What a quest step reports: the records its scripts sent for the client, the GRAND
/// climbs its reads landed (each for the store's `store_skill`), and the `guild.*`
/// writers the scripts ran (each for the descriptor to drain after the step, in order).
pub type QuestStepOutcome = (
    Vec<Vec<u8>>,
    Vec<(u8, u8, i32)>,
    Vec<super::guild::GuildQuestStore>,
);

/// What a quest run reports (230s): the records, the GRAND climbs, and the `guild.*`
/// calls the scripts made (each for [`GameState::answer_guild_quest`] after the step).
type QuestRun = (
    Vec<Vec<u8>>,
    Vec<(u8, u8, i32)>,
    Vec<super::guild::GuildQuestOp>,
);

/// `get_global_time()` in seconds: the wall clock a quest step judges against.
pub(super) fn quest_now() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u32::try_from(since.as_secs()).unwrap_or(u32::MAX)
        })
}

/// What a running script reaches: the character's map, its client and the world's dice.
pub(super) struct Session<'s> {
    /// `GetLevel()` of the character the script runs for.
    pub(super) level: u8,
    /// The NPCs of the character's map.
    pub(super) npcs: Option<&'s MapNpcs>,
    /// Which monsters exist.
    pub(super) protos: &'s MobProtos,
    /// Their names.
    pub(super) names: &'s MobNamesByLanguage,
    /// The chat lines' formats.
    pub(super) locale: &'s LocaleStrings,
    /// The character's language and empire.
    pub(super) mover: Mover,
    /// `GetMapIndex()` of the character the script runs for: the map it stands on, the
    /// private index when it stands on a private map.
    pub(super) map_index: i32,
    /// The war map the character stands on, and 0 when it stands on none
    /// (`CWarMapManager::Find(GetMapIndex())`, `G/input_login.cpp:780`).
    pub(super) war_map: i32,
    /// The skill rows, for the GRAND read's gates.
    pub(super) tables: &'s gamedata::pc_skill_data::PcSkills,
    /// `number()`'s draw, and the GRAND read's.
    pub(super) dice: &'s mut Pcg32,
    /// The character the script runs for.
    pub(super) reader: Vid,
    /// Its job and skill group, for the GRAND read's `can_use` arm.
    pub(super) card: (u8, u8),
    /// Its level, for the GRAND read's level-limit arm.
    pub(super) pc_level: u8,
    /// The reader's PID, when it is online: the `guild.*` calls' caller.
    pub(super) caller: Option<u32>,
    /// Its skill slots, which the GRAND read climbs and stamps.
    pub(super) slots: &'s mut super::pc_skill::PcSkillState,
    /// Its timed affects, which the GRAND read's book bonus consumes.
    pub(super) affects: Option<&'s mut super::affect::AffectEvent>,
    /// Its points, which the GRAND read's 141 recompute rewrites.
    pub(super) points: Option<&'s mut world::character::Points>,
    /// What the GRAND read climbed, for the store's `store_skill`: the slot, level
    /// and stamp of each climb, in order.
    pub(super) climbs: Vec<(u8, u8, i32)>,
    /// The `guild.*` calls the scripts made, in order, each for
    /// [`GameState::answer_guild_quest`] after the step.
    pub(super) guild_ops: Vec<super::guild::GuildQuestOp>,
    /// The shadow the `guild.*` calls read: the guilds as the world holds them at the
    /// step's start, so a call answers the same rows the world would judge.
    pub(super) guilds: super::guild::GuildShadow,
    /// What the client is sent, in order.
    pub(super) records: Vec<Vec<u8>>,
}

impl<'s> Session<'s> {
    /// The skill rows, for the GRAND read's gates.
    pub(super) fn skill_tables(&self) -> &'s gamedata::pc_skill_data::PcSkills {
        self.tables
    }

    /// The reader's level of `vnum` (`GetSkillLevel`): 0 past vnum 254, clamped to 40.
    pub(super) fn slot(&self, vnum: u32) -> u8 {
        usize::try_from(vnum)
            .ok()
            .and_then(|slot| self.slots.levels.get(slot))
            .copied()
            .unwrap_or(0)
            .min(40)
    }

    /// `IsLearnableSkill` (`G/char_skill.cpp:260-339`) on the live slots.
    pub(super) fn learnable(&self, row: &gamedata::records::SkillTableRecord) -> bool {
        let level = self.slot(row.vnum);
        if level >= 40 {
            return false;
        }
        if row.skill_type == 0 {
            return level < row.max_level;
        }
        if row.skill_type == 5 {
            return row.vnum != super::pc_skill::SKILL_HORSE_WILDATTACK_RANGE
                || self.card.0 == super::pc_skill::JOB_ASSASSIN;
        }
        if self.card.1 == 0 {
            return false;
        }
        if row.skill_type.wrapping_sub(1) == self.card.0 {
            return true;
        }
        if row.skill_type == 6 {
            if (super::pc_skill::SKILL_ANTI_FIRST..super::pc_skill::SKILL_ANTI_FIRST + 4)
                .contains(&row.vnum)
            {
                return (super::pc_skill::SKILL_ANTI_FIRST..super::pc_skill::SKILL_ANTI_FIRST + 4)
                    .filter(|sibling| *sibling != row.vnum)
                    .all(|sibling| self.slot(sibling) == 0);
            }
            if (super::pc_skill::SKILL_ANTI_FIRST + 4..=super::pc_skill::SKILL_ANTI_LAST)
                .contains(&row.vnum)
            {
                return (super::pc_skill::SKILL_ANTI_FIRST + 4..=super::pc_skill::SKILL_ANTI_LAST)
                    .filter(|sibling| *sibling != row.vnum)
                    .all(|sibling| self.slot(sibling) == 0);
            }
            if (super::pc_skill::SKILL_NEW_FIRST..=super::pc_skill::SKILL_NEW_LAST)
                .contains(&row.vnum)
                || (super::pc_skill::SKILL_HELP_FIRST..=super::pc_skill::SKILL_HELP_LAST)
                    .contains(&row.vnum)
            {
                return level != 0;
            }
        }
        false
    }

    /// One `ChatPacket` line to the reader, queued for its client.
    pub(super) fn say(&mut self, chat_type: u8, format: &[u8]) {
        let to = self.mover.recipient(self.locale);
        self.records.push(chat_packet(to, chat_type, format, &[]));
    }

    /// Whether the reader holds a timed affect of `kind`.
    pub(super) fn has_affect(&self, kind: u32) -> bool {
        self.affects
            .as_deref()
            .is_some_and(|event| event.affects.iter().any(|affect| affect.kind == kind))
    }

    /// Take every timed affect of `kind`, queuing the `GC_AFFECT_REMOVE`s and the
    /// point records their values take back. The flags' `GC_CHARACTER_UPDATE` is
    /// broadcast by the world's next view: the affect words are in `packet_around`.
    pub(super) fn take_affect(&mut self, kind: u32) {
        let Some(event) = self.affects.as_deref_mut() else {
            return;
        };
        let taken = super::affect::take_affects_of_kind(&mut event.affects, kind);
        for affect in &taken {
            self.records
                .push(super::affect::affect_remove_frame(affect));
            if let Some(points) = self.points.as_deref_mut() {
                let records = points
                    .point_change(usize::from(affect.apply_on), -affect.value, false, false)
                    .unwrap_or_default();
                self.records.extend(crate::loading_phase::point_changes(
                    &records,
                    self.reader.raw(),
                ));
            }
        }
    }

    /// The world's dice, for the GRAND read's draws.
    pub(super) fn dice_mut(&mut self) -> &mut Pcg32 {
        self.dice
    }

    /// `SkillLevelUp(vnum, SKILL_UP_BY_QUEST)`'s climb half (`:802-907`): the
    /// gates (usable, anti-first, row, below 255, learnable, GRAND, level limit,
    /// prereq, grouped), the climb (GRAND at 40 stays 40), the 141 recompute,
    /// and byte 76 with all 255 slots. Queues the climb for the store's
    /// `store_skill`. Answers whether the level changed.
    pub(super) fn climb_grand(
        &mut self,
        vnum: u32,
        row: &gamedata::records::SkillTableRecord,
    ) -> bool {
        use super::pc_skill::{master_of, SKILL_GRAND_MASTER, SKILL_MAX_NUM};
        let (job, group) = self.card;
        let usable = gamedata::pc_skill_data::can_use(job, group, false, vnum)
            || matches!(
                vnum,
                137..=140
                    | super::pc_skill::SKILL_ANTI_FIRST..=super::pc_skill::SKILL_ANTI_LAST
                    | super::pc_skill::SKILL_ADD_HP
                    | super::pc_skill::SKILL_RESIST_PENETRATE
                    | super::pc_skill::SKILL_NEW_FIRST..=super::pc_skill::SKILL_NEW_LAST
                    | super::pc_skill::SKILL_HELP_FIRST..=super::pc_skill::SKILL_HELP_LAST,
            );
        if !usable {
            return false;
        }
        if (super::pc_skill::SKILL_ANTI_FIRST..=super::pc_skill::SKILL_ANTI_LAST).contains(&vnum)
            && self.slot(vnum) == 0
        {
            return false;
        }
        if vnum >= SKILL_MAX_NUM {
            return false;
        }
        if !self.learnable(row) {
            return false;
        }
        let level = self.slot(vnum);
        if row.skill_type != 0 && master_of(level) != SKILL_GRAND_MASTER {
            return false;
        }
        if self.pc_level < row.level_limit {
            return false;
        }
        if row.pre_skill_vnum != 0
            && master_of(self.slot(row.pre_skill_vnum)) == super::pc_skill::SKILL_NORMAL
            && self.slot(row.pre_skill_vnum) < row.pre_skill_level
        {
            return false;
        }
        if row.skill_type != 0 && group == 0 {
            return false;
        }
        let climbed = if master_of(level) == SKILL_GRAND_MASTER && level.saturating_add(1) >= 40 {
            40
        } else {
            level.saturating_add(1).min(40)
        };
        if climbed == level {
            return false;
        }
        if let Some(slot) = usize::try_from(vnum)
            .ok()
            .and_then(|slot| self.slots.levels.get_mut(slot))
        {
            *slot = climbed;
        }
        if vnum == super::pc_skill::SKILL_ADD_HP {
            self.refresh_passive_hp();
        }
        self.send_skill_levels();
        if let Ok(slot) = u8::try_from(vnum) {
            let stamp = usize::try_from(vnum)
                .ok()
                .and_then(|slot| self.slots.next_read.get(slot))
                .copied()
                .unwrap_or(0);
            self.climbs
                .push((slot, climbed, i32::from_ne_bytes(stamp.to_ne_bytes())));
        }
        true
    }

    /// `ComputePoints`' `SKILL_ADD_HP` arm (`G/char.cpp:2956-2966`) after a 141
    /// up: the row's point poly at `k = power / 100` refreshes the passive
    /// max-HP bonus, recomputes the points, and queues the records.
    fn refresh_passive_hp(&mut self) {
        let Some(row) = self.tables.get(super::pc_skill::SKILL_ADD_HP).cloned() else {
            return;
        };
        let level = self.slot(super::pc_skill::SKILL_ADD_HP);
        let power = self.tables.power(self.card.0, self.card.1, level, false);
        let bonus = common::cfloat::f64_to_i32(
            row.point
                .eval(&BTreeMap::from([("k", f64::from(power) / 100.0)])),
        );
        let Some(points) = self.points.as_deref_mut() else {
            return;
        };
        points.set_passive_bonuses(world::character::PassiveBonuses {
            max_hp: bonus,
            ..world::character::PassiveBonuses::default()
        });
        let records = points.compute_points();
        self.records.extend(crate::loading_phase::point_changes(
            &records,
            self.reader.raw(),
        ));
    }

    /// `SkillLevelPacket` (`G/char_skill.cpp:170-181`): byte 76 with all 255 slots.
    fn send_skill_levels(&mut self) {
        use protocol::gc_nested::{GcSkill, GcSkillLevelNew};
        let mut skills = crate::loading_phase::skill_levels();
        for (slot, entry) in skills.iter_mut().enumerate() {
            let (level, next_read) = self
                .slots
                .levels
                .get(slot)
                .zip(self.slots.next_read.get(slot))
                .map_or((0, 0), |(level, stamp)| (*level, *stamp));
            *entry = GcSkill {
                master_type: super::pc_skill::master_of(level),
                level,
                next_read: i32::from_ne_bytes(next_read.to_ne_bytes()),
            };
        }
        let frame = GcSkillLevelNew {
            header: GcSkillLevelNew::header(),
            skills,
        };
        let mut out = Vec::new();
        frame.encode_into(&mut out);
        self.records.push(out);
    }

    /// Stamp the read time of `vnum` and send byte 76 with all 255 slots.
    pub(super) fn stamp(&mut self, vnum: u32, stamp: u32) {
        if let Some(slot) = usize::try_from(vnum)
            .ok()
            .and_then(|slot| self.slots.next_read.get_mut(slot))
        {
            *slot = stamp;
        }
        self.send_skill_levels();
    }
}

impl Game for Session<'_> {
    fn level(&self) -> u8 {
        self.level
    }

    fn race_of(&self, vid: u32) -> Option<u32> {
        self.npcs?
            .npcs
            .iter()
            .find(|npc| npc.vid == vid)
            .map(|npc| u32::from(npc.race))
    }

    fn mob_name(&self, vnum: u32) -> Option<Vec<u8>> {
        self.protos.get(vnum)?;
        Some(self.names.find(vnum, self.mover.language).to_vec())
    }

    fn chat(&mut self, kind: ChatKind, text: &[u8]) {
        let chat_type = match kind {
            ChatKind::Talking => CHAT_TYPE_TALKING,
            ChatKind::Info => CHAT_TYPE_INFO,
            ChatKind::Notice => CHAT_TYPE_NOTICE,
            ChatKind::Command => CHAT_TYPE_COMMAND,
        };
        let to = self.mover.recipient(self.locale);
        self.records
            .push(chat_packet(to, chat_type, b"%s", &[Arg::Text(text)]));
    }

    fn script(&mut self, skin: u8, script: &[u8]) {
        let mut record = Vec::new();
        match GcScript::new(skin, script.to_vec()).encode_into(&mut record) {
            Ok(()) => self.records.push(record),
            Err(error) => warn!(%error, "A quest dialog is too long to send; it is dropped"),
        }
    }

    fn global_time(&self) -> u32 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                u32::try_from(since.as_secs()).unwrap_or(u32::MAX)
            })
    }

    fn number(&mut self, from: i32, to: i32) -> i32 {
        number(self.dice, from, to)
    }

    fn learn_grand_master_skill(&mut self, vnum: u32, total: i32) -> bool {
        super::pc_skill::grand_master_read(self, vnum, total)
    }

    fn skill_level(&self, vnum: u32) -> u8 {
        // Live: the slots are borrowed, so a climb earlier in this script shows.
        usize::try_from(vnum)
            .ok()
            .and_then(|slot| self.slots.levels.get(slot))
            .copied()
            .unwrap_or(0)
            .min(40)
    }

    fn map_index(&self) -> i32 {
        self.map_index
    }

    fn war_map(&self) -> i32 {
        self.war_map
    }

    fn guild_call(
        &mut self,
        name: &[u8],
        args: &[quest::manager::GuildArg],
    ) -> quest::manager::GuildAnswer {
        // The shadow answers now, so the script's control flow sees the pushed value;
        // the world judges the same call again after the step for its records and
        // stores.
        self.guild_ops.push(super::guild::GuildQuestOp {
            name: name.to_vec(),
            args: args.to_vec(),
        });
        let caller = self.caller;
        self.guilds.judge_shadow(caller, name, args)
    }
}

impl GameState {
    /// Share the quests boot loaded. Without them no NPC has a quest.
    #[must_use]
    pub fn with_quests(mut self, quests: Quests) -> Self {
        self.quests = Some(quests);
        self
    }

    /// The quests, when boot loaded them.
    #[must_use]
    pub fn quests(&self) -> Option<&Quests> {
        self.quests.as_ref()
    }

    /// Run one quest step for the character online under `vid`, standing at `place`, and answer
    /// the records its script sent and the GRAND climbs its reads landed (each for the
    /// store's `store_skill`), in order.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when no character is online under `vid`.
    pub fn quest(
        &mut self,
        vid: Vid,
        step: &QuestStep,
        place: GroundPlace,
        mover: Mover,
    ) -> Result<QuestStepOutcome, MoveItemRefused> {
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let pid = character.player_id();
        let level = character
            .points()
            .map_or(1, world::character::Points::level);
        let answered = self.run_quests(place, mover, level, pid, |manager, game| match step {
            QuestStep::Answer(answer) => manager.answer(pid, *answer, game),
            QuestStep::Input(text) => manager.input(pid, text, game),
            QuestStep::Button(quest) => {
                manager.button(pid, *quest, game);
            }
        });
        let Some(((), (mut records, climbs, ops))) = answered else {
            return Ok(QuestStepOutcome::default());
        };
        let now = quest_now();
        let mut stores = Vec::new();
        for op in &ops {
            let judged = self.answer_guild_quest(vid, &op.name, &op.args, now);
            records.extend(judged.own);
            stores.extend(judged.stores);
        }
        Ok((records, climbs, stores))
    }

    /// `CQuestManager::Letter` (`G/questmanager.cpp:502-514`) for the character under
    /// `vid`: the named quest's letter event runs at its start state, and the records its
    /// script sent go to that character's own client.
    ///
    /// The war asks its members to join it this way (`CGuild::GuildWarEntryAsk`,
    /// `G/guild_war.cpp:680-727`), so the world runs the letter for a character that is
    /// not the one that started it.
    pub(super) fn quest_letter(&mut self, vid: Vid, quest: u32) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        let level = character
            .points()
            .map_or(1, world::character::Points::level);
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let Some((channel, map)) = self.place_of(vid.raw()) else {
            return;
        };
        let (x, y) = self
            .spot_of(vid.raw())
            .map_or((0, 0), |spot| (spot.x, spot.y));
        let place = GroundPlace { channel, map, x, y };
        let Some(((), (records, _, _))) =
            self.run_quests(place, mover, level, pid, |manager, game| {
                manager.letter(pid, quest, 0, game);
            })
        else {
            return;
        };
        for record in records {
            let _sent = self.write_to_client(vid, record);
        }
    }

    /// `CQuestManager::Click` for the character under `vid` clicking the NPC `target` of race
    /// `race`: whether a quest took the click, the records its scripts sent either way, and
    /// the GRAND climbs its reads landed (each for the store's `store_skill`).
    pub(super) fn quest_click(
        &mut self,
        vid: Vid,
        (target, race): (u32, u16),
        place: GroundPlace,
        mover: Mover,
    ) -> (bool, QuestStepOutcome) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return (false, QuestStepOutcome::default());
        };
        let pid = character.player_id();
        let level = character
            .points()
            .map_or(1, world::character::Points::level);
        let Some((taken, (mut records, climbs, ops))) =
            self.run_quests(place, mover, level, pid, |manager, game| {
                manager.click(pid, target, u32::from(race), game)
            })
        else {
            return (false, QuestStepOutcome::default());
        };
        let now = quest_now();
        let mut stores = Vec::new();
        for op in &ops {
            let judged = self.answer_guild_quest(vid, &op.name, &op.args, now);
            records.extend(judged.own);
            stores.extend(judged.stores);
        }
        (taken, (records, climbs, stores))
    }

    /// `CQuestManager::GetPCForce(pid)->IsRunning()` for the character under `vid`: whether a
    /// script of it waits for its client.
    pub(super) fn quest_running(&self, vid: Vid) -> bool {
        match (self.quests.as_ref(), self.characters.find_by_vid(vid)) {
            (Some(quests), Ok(character)) => quests.manager.is_running(character.player_id()),
            _ => false,
        }
    }

    /// End the running script of the character under `vid`, which is leaving the world.
    pub(super) fn end_quests(&mut self, vid: Vid) {
        if let (Some(quests), Ok(character)) =
            (self.quests.as_mut(), self.characters.find_by_vid(vid))
        {
            quests.manager.logout(character.player_id());
        }
    }
    /// `CQuestManager::LevelUp` for the character under `vid`, which just leveled up: the
    /// records its `levelup` scripts sent (`G/questmanager.cpp:667-681`). The manager must have
    /// seen the character already (a click), else it misses as legacy's `GetPC` returns NULL.
    pub(super) fn quest_levelup(
        &mut self,
        vid: Vid,
        place: GroundPlace,
        mover: Mover,
    ) -> Vec<Vec<u8>> {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return Vec::new();
        };
        let pid = character.player_id();
        let level = character
            .points()
            .map_or(1, world::character::Points::level);
        self.run_quests(place, mover, level, pid, |manager, game| {
            manager.levelup(pid, game);
        })
        .map_or_else(Vec::new, |((), (records, _, _))| records)
    }

    /// `CQuestManager::Kill` for the character under `vid`, which killed race `race` under
    /// `victim` (`G/questmanager.cpp:574-611`): `SetQuestNPCID` first, then the records its
    /// `kill` scripts sent and its leader's `party_kill` scripts sent. The manager must have
    /// seen the character already, else it misses as legacy's `GetPC` returns NULL. The
    /// leader is `GetLeaderCharacter`'s PID when the killer is partied and its leader is
    /// linked, the killer's own PID when partyless, and absent when the leader is offline
    /// (no `party_kill` then).
    pub(super) fn quest_kill(
        &mut self,
        vid: Vid,
        victim: u32,
        race: u32,
        place: GroundPlace,
        mover: Mover,
    ) -> Vec<Vec<u8>> {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return Vec::new();
        };
        let pid = character.player_id();
        let level = character
            .points()
            .map_or(1, world::character::Points::level);
        let partied = self.party_leader_of(pid).is_some();
        let leader = self
            .quest_kill_leader(vid)
            .or(if partied { None } else { Some(pid) });
        self.run_quests(place, mover, level, pid, |manager, game| {
            manager.set_quest_npc(victim);
            manager.kill(pid, race, leader, game);
        })
        .map_or_else(Vec::new, |((), (records, _, _))| records)
    }

    /// Run `op` on the manager with a [`Game`] for the character of `pid` standing
    /// at `place` (`GetCurrentCharacterPtr` is it). The session borrows the skill
    /// slots, the timed affects and the points, so a GRAND read runs now, while the
    /// script that asked still waits for the answer — and the climbs it landed come
    /// back for the store's `store_skill`.
    fn run_quests<R>(
        &mut self,
        place: GroundPlace,
        mover: Mover,
        level: u8,
        pid: u32,
        op: impl FnOnce(&mut Manager, &mut dyn Game) -> R,
    ) -> Option<(R, QuestRun)> {
        let reader = self
            .characters
            .find_by_pid(pid)
            .map_or(Vid::new(0), world::character::Character::vid);
        let GameState {
            characters,
            npcs,
            quests,
            locale,
            dice,
            bodies,
            war_maps,
            pc_skills,
            affect_events,
            pc_skill_tables,
            guilds,
            ..
        } = self;
        let quests = quests.as_mut()?;
        let card = bodies
            .get(&reader)
            .map_or((0, 0), |body| (body.card.job, body.card.skill_group));
        let pc_level = characters
            .find_by_vid(reader)
            .ok()
            .and_then(|character| character.points())
            .map_or(0, world::character::Points::level);
        let caller = characters
            .find_by_vid(reader)
            .ok()
            .map(world::character::Character::player_id);
        // `GetMapIndex()` and `GetWarMap()` (`G/input_login.cpp:780`): the map the
        // character stands on, and the war map it stands on, which is the map the
        // world holds a war for.
        let map_index = bodies.get(&reader).map_or(0, |body| body.map);
        let war_map = war_maps.get(&map_index).map_or(0, |war| war.map_index);
        let points = characters
            .find_by_vid_mut(reader)
            .ok()
            .and_then(|character| character.items_and_points_mut().1);
        let mut session = Session {
            level,
            npcs: map_npcs(npcs, place),
            protos: &quests.protos,
            names: &quests.names,
            locale,
            mover,
            map_index,
            war_map,
            tables: pc_skill_tables,
            dice,
            reader,
            card,
            pc_level,
            caller,
            slots: pc_skills.entry(reader.raw()).or_default(),
            affects: affect_events.get_mut(&reader),
            points,
            climbs: Vec::new(),
            guild_ops: Vec::new(),
            guilds: guilds.clone(),
            records: Vec::new(),
        };
        let answer = op(&mut quests.manager, &mut session);
        let Session {
            records,
            climbs,
            guild_ops,
            ..
        } = session;
        Some((answer, (records, climbs, guild_ops)))
    }
}

#[cfg(test)]
impl GameState {
    /// Load the Locale's quests, unless they are loaded, and click the OX manager for the
    /// character under `vid`, whose script then waits in its menu.
    pub(super) fn start_a_quest(&mut self, vid: Vid) {
        if self.quests.is_none() {
            let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../legacy/gamedata/locale/europe");
            let manager = Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
            let quests = Quests::new(manager, Arc::default(), MobNamesByLanguage::default());
            self.quests = Some(quests);
        }
        let (taken, _) = self.quest_click(
            vid,
            (1, test_place::OX_MANAGER),
            test_place::AT,
            test_place::MOVER,
        );
        assert!(
            taken && self.quest_running(vid),
            "the OX manager's menu waits"
        );
    }

    /// Answer the OX manager's menu with its last entry, which ends the script.
    pub(super) fn end_the_quest(&mut self, vid: Vid) {
        let answer = QuestStep::Answer(1);
        let _records = self.quest(vid, &answer, test_place::AT, test_place::MOVER);
        assert!(
            !self.quest_running(vid),
            "the menu's last entry ends the script"
        );
    }
}

/// Where the quest tests click.
#[cfg(test)]
mod test_place {
    use super::{GroundPlace, Mover};

    /// The OX manager, an NPC whose click opens a menu.
    pub const OX_MANAGER: u16 = 20_011;
    /// Somewhere on map 1.
    pub const AT: GroundPlace = GroundPlace {
        channel: 1,
        map: 1,
        x: 0,
        y: 0,
    };
    /// A Shinsoo character.
    pub const MOVER: Mover = Mover {
        recently_fought: false,
        empire: 1,
        language: 0,
        pk_mode: crate::loading_phase::PK_MODE_PEACE,
        affect_flags: [0; 2],
        dead: false,
        alignment: 0,
        view_guild: (0, 0),
    };
}

/// The NPCs of the map at `place`.
fn map_npcs(npcs: &BTreeMap<(u8, i32), Arc<MapNpcs>>, place: GroundPlace) -> Option<&MapNpcs> {
    npcs.get(&(place.channel, place.map)).map(Arc::as_ref)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(msg: &[u8]) -> QuestStep {
        let mut field = [0; CG_QUEST_TEXT_FIELD_SIZE];
        field[..msg.len()].copy_from_slice(msg);
        QuestStep::input(&CgQuestInputString { msg: field })
    }

    /// `strlcpy` into 65 bytes: the text ends at its first NUL and at 64 bytes.
    #[test]
    fn the_input_is_cut_as_strlcpy_cut_it() {
        assert_eq!(input(b"abc"), QuestStep::Input(b"abc".to_vec()));
        assert_eq!(input(b"ab\0cd"), QuestStep::Input(b"ab".to_vec()));
        assert_eq!(input(b""), QuestStep::Input(Vec::new()));
        let full = [b'x'; CG_QUEST_TEXT_FIELD_SIZE];
        assert_eq!(input(&full), QuestStep::Input(vec![b'x'; 64]));
    }

    /// `mob_name` names only a monster `CMobManager::Get` finds (`questlua_global.cpp:673`), and
    /// each chat line goes out as `ChatPacket(type, "%s", text)` of its own type.
    #[test]
    fn a_script_names_existing_monsters_and_chats_in_its_lines_type() {
        use gamedata::mob_locale_names::MobLocaleNames;
        use gamedata::mob_proto::{MobProto, CHAR_TYPE_NPC};
        use gamedata::records::MobTableRecord;

        let protos = MobProtos::from_rows(vec![MobProto {
            line: 2,
            table: MobTableRecord {
                vnum: 20_016,
                mob_type: CHAR_TYPE_NPC,
                ..MobTableRecord::default()
            },
        }]);
        let english = MobLocaleNames::parse(b"VNUM\tNAME\n20016\tSmith\n20017\tGhost\n").unwrap();
        let names = MobNamesByLanguage::from_tables(vec![english]);
        let locale = LocaleStrings::default();
        let mut dice = Pcg32::new(1, 1);
        let tables = gamedata::pc_skill_data::PcSkills::default();
        let mut slots = super::super::pc_skill::PcSkillState::default();
        let mut session = Session {
            map_index: 0,
            war_map: 0,
            level: 1,
            npcs: None,
            protos: &protos,
            names: &names,
            locale: &locale,
            mover: test_place::MOVER,
            tables: &tables,
            dice: &mut dice,
            reader: Vid::new(1),
            card: (0, 1),
            pc_level: 10,
            caller: None,
            slots: &mut slots,
            affects: None,
            points: None,
            climbs: Vec::new(),
            guild_ops: Vec::new(),
            guilds: super::super::guild::GuildShadow::default(),
            records: Vec::new(),
        };
        assert_eq!(session.mob_name(20_016), Some(b"Smith".to_vec()));
        assert_eq!(session.mob_name(20_017), None, "a name with no proto");
        let kinds = [
            (ChatKind::Talking, CHAT_TYPE_TALKING),
            (ChatKind::Info, CHAT_TYPE_INFO),
            (ChatKind::Notice, CHAT_TYPE_NOTICE),
            (ChatKind::Command, CHAT_TYPE_COMMAND),
        ];
        for (kind, _) in kinds {
            session.chat(kind, b"hi");
        }
        let to = test_place::MOVER.recipient(&locale);
        let expected: Vec<Vec<u8>> = kinds
            .iter()
            .map(|(_, chat_type)| chat_packet(to, *chat_type, b"%s", &[Arg::Text(b"hi")]))
            .collect();
        assert_eq!(session.records, expected);
    }
}
