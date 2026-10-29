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
use gamedata::item_proto::ItemProtos;
use gamedata::locale_string::LocaleStrings;
use gamedata::mob_locale_names::{ItemNamesByLanguage, MobNamesByLanguage};
use gamedata::mob_proto::{MobProtos, CHAR_TYPE_GOTO, CHAR_TYPE_NPC, CHAR_TYPE_WARP};
use protocol::cg_quest_text::{CgQuestInputString, CG_QUEST_TEXT_FIELD_SIZE};
use protocol::gc_chat::{CHAT_TYPE_COMMAND, CHAT_TYPE_INFO, CHAT_TYPE_NOTICE, CHAT_TYPE_TALKING};
use protocol::gc_quest_info::{c_field, GcQuestInfo};
use protocol::gc_script::GcScript;
use quest::manager::{ChatKind, Game, Manager, QuestInfo, Target};
use tracing::warn;
use world::character::{number, Pcg32};
use world::npc::MapNpcs;

use super::quest_target::QuestTargets;

/// `ITEM_FLAG_QUEST_USE` and `ITEM_FLAG_QUEST_USE_MULTIPLE` (`common/item_length.h:369-370`):
/// an `ITEM_QUEST` with either runs no quest when used. The text protos' `QUEST_USE` name sets
/// bit 9, not 10 (`ProtoReader.cpp:398-400`); legacy tests the bits, and so does the Rewrite.
const ITEM_FLAG_QUEST_USE: u32 = 1 << 10;
const ITEM_FLAG_QUEST_USE_MULTIPLE: u32 = 1 << 11;
use super::GameState;
use crate::chat_line::{chat_packet, Arg};
use crate::game_loop_messages::GroundPlace;
use crate::item_move::{MoveItemRefused, Mover};

/// The quests boot loaded, with the monsters their `mob_name` names and the items their
/// `item_name` names.
#[derive(Debug)]
pub struct Quests {
    /// The scripts, the NPCs they belong to and every character's running script.
    manager: Manager,
    /// `CMobManager`: a vnum with no proto has no name.
    protos: Arc<MobProtos>,
    /// Every language's mob names.
    names: MobNamesByLanguage,
    /// Every language's item names, which name the items of the world's item protos.
    item_names: ItemNamesByLanguage,
}

impl Quests {
    /// The quests of `manager`, whose `mob_name` names the monsters of `protos` from `names`.
    #[must_use]
    pub fn new(manager: Manager, protos: Arc<MobProtos>, names: MobNamesByLanguage) -> Self {
        Self {
            manager,
            protos,
            names,
            item_names: ItemNamesByLanguage::default(),
        }
    }

    /// These quests with `item_names` answering `item_name`; without them every item it finds
    /// is `NoName`.
    #[must_use]
    pub fn with_item_names(self, item_names: ItemNamesByLanguage) -> Self {
        Self { item_names, ..self }
    }

    /// The item names `item_name` answers from, which a test reads.
    #[must_use]
    pub fn item_names(&self) -> &ItemNamesByLanguage {
        &self.item_names
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
    /// (`CInputMain::ScriptButton`, `G/input_main.cpp:2188-2196`): the quest window's
    /// entry, which a `GC_QUEST_INFO` made.
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
/// climbs its reads landed (each for the store's `store_skill`), the `guild.*`
/// writers the scripts ran (each for the descriptor to drain after the step, in order), and
/// the item rows its gives changed (for the descriptor to store before it sends).
pub type QuestStepOutcome = (
    Vec<Vec<u8>>,
    Vec<(u8, u8, i32)>,
    Vec<super::guild::GuildQuestStore>,
    Vec<db::items::RowChange>,
);

/// What a quest run reports: the records, the GRAND climbs, the `guild.*`
/// calls the scripts made (each for [`GameState::answer_guild_quest`] after the step),
/// and the last `pc.send_block_mode` byte the scripts set, for the block byte.
type QuestRun = (
    Vec<Vec<u8>>,
    Vec<(u8, u8, i32)>,
    Vec<super::guild::GuildQuestOp>,
    Option<u8>,
);

/// The bit of a `CG_SCRIPT_BUTTON` word that asks for the quest's information
/// (`p->idx & 0x80000000`, `G/input_main.cpp:2188`).
const QUEST_INFO_BIT: u32 = 0x8000_0000;

/// `MOB_SANTA_VNUM`, `MOB_XMAS_TREE_VNUM` and `MOB_XMAS_FIRWORK_SELLER_VNUM`
/// (`G/xmas_event.h:8-10`), whose races `CHARACTER_MANAGER` registers whatever their protos.
const XMAS_RACES: [u32; 3] = [20_031, 20_032, 9_004];

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
    /// `ITEM_MANAGER::GetTable`: an item with no proto has no name.
    pub(super) items: &'s ItemProtos,
    /// The items' names.
    pub(super) item_names: &'s ItemNamesByLanguage,
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
    /// The `pc.send_block_mode` byte the scripts set, the last one winning;
    /// `SetBlockModeForce` writes the byte beside the quest flags, and no flag moves.
    pub(super) block_mode: Option<u8>,
    /// `GetPlayerID()` of the character the script runs for, whose targets the scripts mark.
    pub(super) pid: u32,
    /// Every character's quest targets.
    pub(super) targets: &'s mut QuestTargets,
    /// The Pulse the step runs at, from which a new target's event counts.
    pub(super) pulse: u64,
    /// The character's items, which `pc.count_item` reads, and which `pc.give_item2` and
    /// `pc.remove_item` change only on the steps whose rows the descriptor stores
    /// ([`Giver::stores`]). `None` when the character is not in the world.
    pub(super) giver: Option<Giver<'s>>,
    /// The item rows the gives changed, in order, for the descriptor to store.
    pub(super) item_rows: Vec<db::items::RowChange>,
    /// The made items no cell took, for the ground at the character's feet.
    pub(super) laid: Vec<world::item::Item>,
}

/// What a give, a count and a removal reach: the character's items and quickslots, the rules
/// its cells follow, and what `CreateItem` makes an item with.
pub(super) struct Giver<'s> {
    /// Whether the step's descriptor stores the item rows, so the step may give and remove;
    /// without it the step only counts.
    pub(super) stores: bool,
    /// The character's items.
    pub(super) items: &'s mut world::character::CharacterItems,
    /// Its quickslots, which a given potion fills.
    pub(super) slots: &'s mut world::character::Quickslots,
    /// The stack limit and the unlocked cells.
    pub(super) rules: world::character::MoveRules,
    /// The item IDs, when the world has them.
    pub(super) ids: Option<&'s mut world::item::ItemIds>,
    /// The magic roll's cached Gaussian.
    pub(super) gauss: &'s mut world::character::Gauss,
    /// The `item_attr` rows the magic rolls against.
    pub(super) attrs: &'s [gamedata::records::ItemAttrRecord],
    /// The mask table.
    pub(super) masks: &'s [(u32, u32)],
    /// `GetPlayerID()` of the character, whose rows the gives write.
    pub(super) owner_id: u32,
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

    fn playtime(&self) -> i32 {
        self.points.as_deref().map_or(0, |points| {
            points.real_point(common::point_slot::POINT_PLAYTIME)
        })
    }

    fn item_vnum(&self, name: &[u8]) -> Option<u32> {
        self.items.vnum_by_locale_name(name)
    }

    /// `AutoGiveItem(vnum, count)` for the reader: the stacks, a free cell, or the ground.
    /// `CreateItem` is the drop's ([`world::character::make_drop_item`]) with the proto's
    /// magic percent. Its records reach the client at once, as `ChatPacket` and
    /// `AddToCharacter` send them, ahead of the dialog the script says after. A step whose
    /// rows no descriptor stores (a letter, the login, a level, a kill, a target's event)
    /// gives nothing.
    fn give_item(&mut self, vnum: u32, count: u16) -> u32 {
        use world::character::{auto_give_item, make_drop_item, AutoGiven, DropAsk, DropMade};
        let Some(giver) = self.giver.as_mut().filter(|giver| giver.stores) else {
            warn!(
                vnum,
                count, "a quest gave an item where no descriptor stores it; none given"
            );
            return 0;
        };
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
            });
        let drop_rules = DropMade {
            attrs: giver.attrs,
            masks: giver.masks,
            now_secs,
            count_limit: giver.rules.count_limit,
        };
        let protos = self.items;
        let dice = &mut *self.dice;
        let gauss = &mut *giver.gauss;
        let ids = giver.ids.as_deref_mut();
        let create = |left: u16| {
            let ids = ids?;
            let ask = DropAsk {
                count: i32::from(left),
                rare_pct: -1,
            };
            match make_drop_item(dice, gauss, ids, protos, &drop_rules, vnum, ask) {
                Ok(item) => Some(item),
                Err(refused) => {
                    warn!(vnum, %refused, "cannot create item by vnum");
                    None
                }
            }
        };
        let (outcome, done) = auto_give_item(
            giver.items,
            giver.slots,
            &giver.rules,
            protos,
            (vnum, count),
            create,
        );
        let moved = crate::item_move::MovedItems::new(
            giver.owner_id,
            self.reader.raw(),
            done,
            self.mover,
            protos,
            self.locale,
        );
        self.records.extend(moved.records);
        self.item_rows.extend(moved.changes);
        match outcome {
            AutoGiven::NoProto | AutoGiven::NotMade => 0,
            AutoGiven::Merged(id) | AutoGiven::Placed { id, .. } => id,
            AutoGiven::Unplaced(item) => {
                let id = item.id;
                self.laid.push(item);
                id
            }
        }
    }

    fn skill_group(&self) -> u8 {
        self.card.1
    }

    fn held_vnum(&self, id: u32) -> Option<u32> {
        let giver = self.giver.as_ref()?;
        giver.items.item(id).map(|item| item.vnum)
    }

    fn count_item(&self, vnum: u32) -> i32 {
        self.giver.as_ref().map_or(0, |giver| {
            world::character::count_specify_item(giver.items, giver.rules.usable_cells, vnum)
        })
    }

    /// `RemoveSpecifyItem(vnum, count)` for the reader. Its records reach the client at once,
    /// as `SetCount` sends them, and its rows are answered for the store. A step whose rows no
    /// descriptor stores takes nothing.
    fn remove_item(&mut self, vnum: u32, count: u32) {
        let Some(giver) = self.giver.as_mut().filter(|giver| giver.stores) else {
            warn!(
                vnum,
                count, "a quest removed items where no descriptor stores it; none removed"
            );
            return;
        };
        let done = world::character::remove_specify_item(
            giver.items,
            giver.slots,
            self.items,
            giver.rules.usable_cells,
            (vnum, count),
        );
        let moved = crate::item_move::MovedItems::new(
            giver.owner_id,
            self.reader.raw(),
            done,
            self.mover,
            self.items,
            self.locale,
        );
        self.records.extend(moved.records);
        self.item_rows.extend(moved.changes);
    }

    fn race_of(&self, vid: u32) -> Option<u32> {
        self.npcs?
            .npcs
            .iter()
            .find(|npc| npc.vid == vid)
            .map(|npc| u32::from(npc.race))
    }

    /// `_item_name` (`G/questlua_global.cpp:649-671`): the proto, an exact vnum or one inside a
    /// range row ([`ItemProtos::get`]), then `LC_LOCALE_ITEM_TEXT` in the character's language,
    /// `NoName` when its file does not name the vnum.
    fn item_name(&self, vnum: u32) -> Option<Vec<u8>> {
        self.items.get(vnum)?;
        Some(self.item_names.find(vnum, self.mover.language).to_vec())
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

    fn send_block_mode(&mut self, mode: u8) {
        // `SetBlockModeForce` (`G/char.cpp:7658-7661`): the byte lands now, the
        // client hears `setblockmode %d` in this step's records, and no quest
        // flag moves — so the pending byte rides the session and lands after
        // the step, next to where the world's flags would have landed.
        let to = self.mover.recipient(self.locale);
        self.records.push(chat_packet(
            to,
            CHAT_TYPE_COMMAND,
            b"setblockmode %d",
            &[Arg::Int(i64::from(mode))],
        ));
        self.block_mode = Some(mode);
    }

    fn map_index(&self) -> i32 {
        self.map_index
    }

    fn quest_info(&mut self, info: &QuestInfo) {
        let record = GcQuestInfo {
            index: info.index,
            category: info.category,
            begin: info.begin,
            title: info.title.as_deref().map(c_field),
            ..GcQuestInfo::default()
        };
        let mut out = Vec::new();
        record.encode_into(&mut out);
        self.records.push(out);
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

    fn find_npc(&self, race: u32) -> u32 {
        find_by_race(self.npcs, self.protos, race)
    }

    fn create_target(&mut self, target: &Target) {
        let records = self
            .targets
            .create(self.pid, target, (self.map_index, self.pulse));
        self.records.extend(records);
    }

    fn delete_target(&mut self, quest: u32, name: Option<&[u8]>) {
        let records = self.targets.delete(self.pid, quest, name);
        self.records.extend(records);
    }

    fn target_vid(&self, quest: u32, name: &[u8]) -> u32 {
        self.targets.vid(self.pid, quest, name)
    }

    fn target_at(&self, vid: u32) -> Option<(u32, Vec<u8>)> {
        self.targets.at(self.pid, vid)
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
        let answered =
            self.run_quests_giving(
                place,
                (mover, level, pid),
                true,
                |manager, game| match step {
                    QuestStep::Answer(answer) => manager.answer(pid, *answer, game),
                    QuestStep::Input(text) => manager.input(pid, text, game),
                    QuestStep::Button(quest) if quest & QUEST_INFO_BIT != 0 => {
                        manager.info(pid, quest & !QUEST_INFO_BIT, game);
                    }
                    QuestStep::Button(quest) => {
                        manager.button(pid, *quest, game);
                    }
                },
            );
        let Some(((), (mut records, climbs, ops, block_mode), items)) = answered else {
            return Ok(QuestStepOutcome::default());
        };
        if let Some(mode) = block_mode {
            // `pc.send_block_mode` landed after the script's own records and after
            // the guild drains: `SetBlockModeForce` writes the byte beside the
            // flags (`G/char.cpp:7658-7661`), never through them.
            self.block_modes.insert(vid, mode);
        }
        let now = quest_now();
        let mut stores = Vec::new();
        for op in &ops {
            let judged = self.answer_guild_quest(vid, &op.name, &op.args, now);
            records.extend(judged.own);
            stores.extend(judged.stores);
        }
        Ok((records, climbs, stores, items))
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
        let Some(((), (records, _, _, block_mode))) =
            self.run_quests(place, mover, level, pid, |manager, game| {
                manager.letter(pid, quest, 0, game);
            })
        else {
            return;
        };
        if let Some(mode) = block_mode {
            self.block_modes.insert(vid, mode);
        }
        for record in records {
            let _sent = self.write_to_client(vid, record);
        }
    }

    /// `CQuestManager::Login` (`G/questmanager.cpp:541-556`) for the character under `vid`,
    /// which just entered the game with its quest flags loaded: its `login` scripts run, then
    /// its `letter` scripts, and the records they sent go to its own client after the
    /// enter's.
    ///
    /// Legacy runs it from `quest_login_event` (`G/input_db.cpp:995-1050`) once the
    /// descriptor is in `PHASE_GAME`, or at once when `QuestLoad` finds it there already
    /// (`:1104-1115`). The Rewrite loads the flags as the world admits the character, which
    /// is the `CG_ENTER_GAME` that sets that phase, so the event runs there.
    pub(super) fn quest_login(&mut self, vid: Vid) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        let _ran = self.run_quest_event(vid, |manager, game| manager.login(pid, game));
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
        let Some((taken, (mut records, climbs, ops, block_mode), items)) =
            self.run_quests_giving(place, (mover, level, pid), true, |manager, game| {
                manager.click(pid, target, u32::from(race), game)
            })
        else {
            return (false, QuestStepOutcome::default());
        };
        if let Some(mode) = block_mode {
            self.block_modes.insert(vid, mode);
        }
        let now = quest_now();
        let mut stores = Vec::new();
        for op in &ops {
            let judged = self.answer_guild_quest(vid, &op.name, &op.args, now);
            records.extend(judged.own);
            stores.extend(judged.stores);
        }
        (taken, (records, climbs, stores, items))
    }

    /// `case ITEM_QUEST` of `UseItemEx` (`G/char_item.cpp:2842-2888`) after the world's checks
    /// passed for the item at `at` of the character under `vid`: unless the item's proto carries
    /// a quest-use flag, `CQuestManager::UseItem` runs its `use` scripts. What they sent and
    /// changed joins `step`, whose descriptor stores and sends it. The quickslots are taken
    /// again, since a removal may have synced them.
    ///
    /// `GetSIGVnum()` is 0 for every item on the owner's data, whose one special item group has
    /// no `quest` type, so `SIGUse` never runs.
    pub(super) fn use_quest_item(
        &mut self,
        vid: Vid,
        at: protocol::item_pos::ItemPos,
        mover: Mover,
        step: &mut crate::item_move::MovedItems,
    ) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let Some(item) = character.items().item_at(at) else {
            return;
        };
        let (id, vnum) = (item.id, item.vnum);
        let flags = self.protos.get(vnum).map_or(0, |proto| proto.flags);
        if flags & (ITEM_FLAG_QUEST_USE | ITEM_FLAG_QUEST_USE_MULTIPLE) != 0 {
            return;
        }
        let pid = character.player_id();
        let level = character
            .points()
            .map_or(1, world::character::Points::level);
        let Some((channel, map)) = self.place_of(vid.raw()) else {
            return;
        };
        let (x, y) = self
            .spot_of(vid.raw())
            .map_or((0, 0), |spot| (spot.x, spot.y));
        let place = GroundPlace { channel, map, x, y };
        let Some(((), (records, climbs, ops, block_mode), items)) =
            self.run_quests_giving(place, (mover, level, pid), true, |manager, game| {
                manager.use_item(pid, (id, vnum), game);
            })
        else {
            return;
        };
        if let Some(mode) = block_mode {
            self.block_modes.insert(vid, mode);
        }
        step.records.extend(records);
        step.changes.extend(items);
        let now = quest_now();
        let mut stores = Vec::new();
        for op in &ops {
            let judged = self.answer_guild_quest(vid, &op.name, &op.args, now);
            step.records.extend(judged.own);
            stores.extend(judged.stores);
        }
        step.quest = (climbs, stores);
        if let Ok(character) = self.characters.find_by_vid(vid) {
            step.quickslots = Some(character.quickslots().clone());
        }
    }

    /// `CQuestManager::GetPCForce(pid)->IsRunning()` for the character under `vid`: whether a
    /// script of it waits for its client.
    pub(super) fn quest_running(&self, vid: Vid) -> bool {
        match (self.quests.as_ref(), self.characters.find_by_vid(vid)) {
            (Some(quests), Ok(character)) => quests.manager.is_running(character.player_id()),
            _ => false,
        }
    }

    /// End the running script of the character under `vid`, which is leaving the world, and
    /// drop its quest targets (`CTargetManager::Logout`, `G/char.cpp:1815`).
    pub(super) fn end_quests(&mut self, vid: Vid) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        if let Some(quests) = self.quests.as_mut() {
            quests.manager.logout(pid);
        }
        self.quest_targets.logout(pid);
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
        let landed = self.run_quests(place, mover, level, pid, |manager, game| {
            manager.levelup(pid, game);
        });
        let Some(((), (records, _, _, block_mode))) = landed else {
            return Vec::new();
        };
        if let Some(mode) = block_mode {
            self.block_modes.insert(vid, mode);
        }
        records
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
        let landed = self.run_quests(place, mover, level, pid, |manager, game| {
            manager.set_quest_npc(victim);
            manager.kill(pid, race, leader, game);
        });
        let Some(((), (records, _, _, block_mode))) = landed else {
            return Vec::new();
        };
        if let Some(mode) = block_mode {
            self.block_modes.insert(vid, mode);
        }
        records
    }

    /// Run `op` on the manager with a [`Game`] for the character of `pid` standing
    /// at `place` (`GetCurrentCharacterPtr` is it). The session borrows the skill
    /// slots, the timed affects and the points, so a GRAND read runs now, while the
    /// script that asked still waits for the answer — and the climbs it landed come
    /// back for the store's `store_skill`.
    pub(super) fn run_quests<R>(
        &mut self,
        place: GroundPlace,
        mover: Mover,
        level: u8,
        pid: u32,
        op: impl FnOnce(&mut Manager, &mut dyn Game) -> R,
    ) -> Option<(R, QuestRun)> {
        self.run_quests_giving(place, (mover, level, pid), false, op)
            .map(|(answer, run, _)| (answer, run))
    }

    /// [`Self::run_quests`], with `pc.give_item2` able to give when `stores_items`: the step's
    /// descriptor then stores the item rows this also answers. The made items no cell took
    /// lie at the character's feet after the step, with its ownership for 300 seconds when
    /// they cannot be dropped and 60 otherwise (`G/char_item.cpp:9196-9218`).
    pub(super) fn run_quests_giving<R>(
        &mut self,
        place: GroundPlace,
        (mover, level, pid): (Mover, u8, u32),
        stores_items: bool,
        op: impl FnOnce(&mut Manager, &mut dyn Game) -> R,
    ) -> Option<(R, QuestRun, Vec<db::items::RowChange>)> {
        let reader = self
            .characters
            .find_by_pid(pid)
            .map_or(Vid::new(0), world::character::Character::vid);
        let GameState {
            characters,
            npcs,
            quests,
            protos: items,
            locale,
            dice,
            bodies,
            war_maps,
            pc_skills,
            affect_events,
            pc_skill_tables,
            guilds,
            quest_targets,
            last_pulse,
            item_ids,
            item_count_limit,
            item_attrs,
            item_masks,
            gauss,
            ..
        } = self;
        let quests = quests.as_mut()?;
        let context = reader_context(characters, bodies, war_maps, reader);
        let character = characters.find_by_vid_mut(reader).ok();
        let (points, item_giver) = match character {
            Some(character) => {
                let usable_cells =
                    common::item_slots::usable_inventory_cells(character.inven_point());
                let (items, points, slots) = character.items_points_and_quickslots_mut();
                let armed = Some(Giver {
                    stores: stores_items,
                    items,
                    slots,
                    rules: world::character::MoveRules {
                        count_limit: *item_count_limit,
                        usable_cells,
                        belt_grade: None,
                        questing: true,
                    },
                    ids: item_ids.as_mut(),
                    gauss,
                    attrs: item_attrs,
                    masks: item_masks.pairs(),
                    owner_id: pid,
                });
                (points, armed)
            }
            None => (None, None),
        };
        let mut session = Session {
            level,
            npcs: map_npcs(npcs, place),
            protos: &quests.protos,
            names: &quests.names,
            items,
            item_names: &quests.item_names,
            locale,
            mover,
            map_index: context.map_index,
            war_map: context.war_map,
            tables: pc_skill_tables,
            dice,
            reader,
            card: context.card,
            pc_level: context.pc_level,
            caller: context.caller,
            slots: pc_skills.entry(reader.raw()).or_default(),
            affects: affect_events.get_mut(&reader),
            points,
            climbs: Vec::new(),
            guild_ops: Vec::new(),
            block_mode: None,
            guilds: guilds.clone(),
            records: Vec::new(),
            pid,
            targets: quest_targets,
            pulse: *last_pulse,
            giver: item_giver,
            item_rows: Vec::new(),
            laid: Vec::new(),
        };
        let answer = op(&mut quests.manager, &mut session);
        let Session {
            records,
            climbs,
            guild_ops,
            block_mode,
            item_rows,
            laid,
            ..
        } = session;
        for item in laid {
            self.lay_given(reader, place, item);
        }
        Some((answer, (records, climbs, guild_ops, block_mode), item_rows))
    }

    /// `AddToGround` at the character's feet for a given item no cell took, with the
    /// 300-second destroy event (`ITEM_DESTROY_TIME_AUTOGIVE`, `G/config.cpp:56`), then
    /// `SetOwnership(ch, 300)` for an `ANTI_DROP` item and `SetOwnership(ch, 60)` otherwise.
    /// It has no row until it is picked up, as a monster's drop has none.
    fn lay_given(&mut self, reader: Vid, place: GroundPlace, item: world::item::Item) {
        let z = self.spot_of(reader.raw()).map_or(0, |spot| spot.z);
        let secs = given_ownership_secs(item.anti_flags);
        let at = (place.x, place.y, z);
        let Some(ground) = self.lay_drop_hit(place.channel, place.map, at, item) else {
            return;
        };
        let Ok(character) = self.characters.find_by_vid(reader) else {
            return;
        };
        let pid = character.player_id();
        let Some(body) = self.bodies.get(&reader) else {
            return;
        };
        let name = crate::loading_phase::name_field(&body.card.name);
        self.set_ground_ownership(ground, Some((pid, name)), secs);
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

/// `_find_npc_by_vnum`'s search (`G/questlua_global.cpp:792-823`): the VID of a character of
/// race `race` among `npcs`, the characters on the map of the character the script runs for,
/// and 0 when there is none.
///
/// `GetCharactersByRaceNum` holds the characters whose `GetRaceNum`, the low 16 bits of the
/// vnum, is a registered race (`RegisterRaceNumMap`, `G/char_manager.cpp:868-877`): the vnum of
/// an NPC, warp or goto proto (`G/mob_manager.cpp:83-84`) or of a Christmas character
/// (`G/char_manager.cpp:47-49`). Of those the first spawned answers, where legacy's
/// `unordered_set` had no order (a Divergence).
fn find_by_race(npcs: Option<&MapNpcs>, protos: &MobProtos, race: u32) -> u32 {
    let registered = XMAS_RACES.contains(&race)
        || protos.get(race).is_some_and(|proto| {
            matches!(
                proto.mob_type,
                CHAR_TYPE_NPC | CHAR_TYPE_WARP | CHAR_TYPE_GOTO
            )
        });
    if !registered {
        return 0;
    }
    npcs.and_then(|map| map.npcs.iter().find(|npc| u32::from(npc.race) == race))
        .map_or(0, |npc| npc.vid)
}

/// The seconds a given item laid on the ground stays its taker's: `SetOwnership(ch, 300)` for an
/// `ANTI_DROP` item and `SetOwnership(ch, 60)` otherwise (`G/char_item.cpp:9214-9217`).
fn given_ownership_secs(anti_flags: u32) -> i32 {
    if anti_flags & world::item::ITEM_ANTIFLAG_DROP != 0 {
        300
    } else {
        60
    }
}

/// What a session knows of its reader before it borrows the reader mutably.
struct ReaderContext {
    card: (u8, u8),
    pc_level: u8,
    caller: Option<u32>,
    map_index: i32,
    war_map: i32,
}

/// The reader's job and skill group, level and player ID, and `GetMapIndex()` and `GetWarMap()`
/// (`G/input_login.cpp:780`): the map the character stands on, and the war map it stands on,
/// which is the map the world holds a war for.
fn reader_context(
    characters: &world::character::CharacterManager,
    bodies: &std::collections::HashMap<Vid, super::motion::Body>,
    war_maps: &BTreeMap<i32, super::war_map::WarMap>,
    reader: Vid,
) -> ReaderContext {
    let card = bodies
        .get(&reader)
        .map_or((0, 0), |body| (body.card.job, body.card.skill_group));
    let character = characters.find_by_vid(reader).ok();
    let pc_level = character
        .and_then(|character| character.points())
        .map_or(0, world::character::Points::level);
    let caller = character.map(world::character::Character::player_id);
    let map_index = bodies.get(&reader).map_or(0, |body| body.map);
    let war_map = war_maps.get(&map_index).map_or(0, |war| war.map_index);
    ReaderContext {
        card,
        pc_level,
        caller,
        map_index,
        war_map,
    }
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

    /// Runs `f` with a session of character 1, in the mover's language, that names the
    /// monsters of `protos` from `names` and the items of `items` from `item_names`.
    fn with_session<R>(
        (protos, names): (&MobProtos, &MobNamesByLanguage),
        (items, item_names): (&ItemProtos, &ItemNamesByLanguage),
        f: impl FnOnce(&mut Session<'_>) -> R,
    ) -> R {
        let locale = LocaleStrings::default();
        let mut dice = Pcg32::new(1, 1);
        let tables = gamedata::pc_skill_data::PcSkills::default();
        let mut slots = super::super::pc_skill::PcSkillState::default();
        let mut targets = QuestTargets::default();
        let mut session = Session {
            map_index: 0,
            war_map: 0,
            level: 1,
            npcs: None,
            protos,
            names,
            items,
            item_names,
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
            block_mode: None,
            pid: 1,
            targets: &mut targets,
            pulse: 0,
            giver: None,
            item_rows: Vec::new(),
            laid: Vec::new(),
        };
        f(&mut session)
    }

    /// `pc.get_skill_group` reads the card's group, and with no character in the world the
    /// quest item is never held.
    #[test]
    fn the_session_reads_the_group_and_holds_no_item_without_a_character() {
        let (protos, names) = (MobProtos::default(), MobNamesByLanguage::default());
        let (items, item_names) = (ItemProtos::default(), ItemNamesByLanguage::default());
        with_session((&protos, &names), (&items, &item_names), |session| {
            assert_eq!(session.skill_group(), 1);
            assert_eq!(session.held_vnum(1000), None);
        });
    }

    /// `pc.give_item2`'s `AutoGiveItem` in a session that may give: a vnum with no proto
    /// gives nothing and answers 0, as 85000 does on the owner's data; a sword is made with
    /// the next ID, named, then set in the first free cell, and its row is answered for the
    /// store; with every cell taken it is laid at the character's feet, still answering its
    /// ID. The count reads the held swords and a removal takes one with its row. A session
    /// whose rows are not stored counts but neither gives nor removes, and one with no
    /// character answers 0 and sends nothing.
    #[test]
    fn a_give_names_sets_and_stores_the_item_or_lays_it() {
        use gamedata::item_proto::ItemProto;
        use world::character::{CharacterItems, Gauss, MoveRules, Quickslots};
        use world::item::{ItemIdRange, ItemIds};

        const SWORD: u32 = 19;
        let items_protos = ItemProtos::from_rows(vec![ItemProto::for_category_rule(SWORD, 1, 0)]);
        let (protos, names) = (MobProtos::default(), MobNamesByLanguage::default());
        let item_names = ItemNamesByLanguage::default();
        let locale = LocaleStrings::default();
        let mut dice = Pcg32::new(1, 1);
        let tables = gamedata::pc_skill_data::PcSkills::default();
        let mut slots = super::super::pc_skill::PcSkillState::default();
        let mut targets = QuestTargets::default();
        let mut held = CharacterItems::new();
        let mut quickslots = Quickslots::default();
        let mut ids = ItemIds::new(ItemIdRange::new(1000, 2000, 1000).unwrap());
        let mut gauss = Gauss::new();
        let mut session = Session {
            map_index: 0,
            war_map: 0,
            level: 1,
            npcs: None,
            protos: &protos,
            names: &names,
            items: &items_protos,
            item_names: &item_names,
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
            block_mode: None,
            pid: 1,
            targets: &mut targets,
            pulse: 0,
            giver: Some(Giver {
                stores: true,
                items: &mut held,
                slots: &mut quickslots,
                rules: MoveRules {
                    count_limit: 200,
                    usable_cells: 2,
                    belt_grade: None,
                    questing: true,
                },
                ids: Some(&mut ids),
                gauss: &mut gauss,
                attrs: &[],
                masks: &[],
                owner_id: 42,
            }),
            item_rows: Vec::new(),
            laid: Vec::new(),
        };
        assert_eq!(session.give_item(85_000, 1), 0);
        assert!(session.records.is_empty() && session.item_rows.is_empty());

        assert_eq!(session.give_item(SWORD, 1), 1000);
        assert_eq!(session.records.len(), 2, "the name, then the item set");
        assert_eq!(session.held_vnum(1000), Some(SWORD));
        assert!(matches!(
            session.item_rows.as_slice(),
            [db::items::RowChange::Created(row)] if row.id == 1000 && row.owner_id == Some(42)
        ));
        assert_eq!(session.give_item(SWORD, 1), 1001);
        assert!(session.laid.is_empty());

        // Both unlocked cells are taken: the third is laid, with no record and no row.
        assert_eq!(session.give_item(SWORD, 1), 1002);
        assert_eq!(session.records.len(), 4);
        assert_eq!(session.item_rows.len(), 2);
        assert_eq!(
            session.laid.iter().map(|item| item.id).collect::<Vec<_>>(),
            [1002]
        );

        // The count reads the two held swords; the removal takes the first, sends its clear
        // and answers its row.
        assert_eq!(session.count_item(SWORD), 2);
        session.remove_item(SWORD, 1);
        assert_eq!(session.count_item(SWORD), 1);
        assert_eq!(session.records.len(), 5);
        assert!(matches!(
            session.item_rows.last(),
            Some(db::items::RowChange::Destroyed { id: 1000 })
        ));

        // A step whose rows no descriptor stores still counts, but neither gives nor removes.
        if let Some(giver) = session.giver.as_mut() {
            giver.stores = false;
        }
        assert_eq!(session.give_item(SWORD, 1), 0);
        session.remove_item(SWORD, 1);
        assert_eq!(session.count_item(SWORD), 1);
        assert_eq!((session.records.len(), session.item_rows.len()), (5, 3));

        session.giver = None;
        assert_eq!(session.give_item(SWORD, 1), 0);
        assert_eq!(session.count_item(SWORD), 0);
        assert_eq!(session.records.len(), 5);
    }

    /// A laid `ANTI_DROP` give stays its taker's for 300 seconds, any other for 60, whatever
    /// other anti-flags it carries.
    #[test]
    fn a_laid_undroppable_give_stays_its_takers_five_minutes() {
        use world::item::{ITEM_ANTIFLAG_DROP, ITEM_ANTIFLAG_GIVE};

        assert_eq!(given_ownership_secs(0), 60);
        assert_eq!(given_ownership_secs(ITEM_ANTIFLAG_GIVE), 60);
        assert_eq!(given_ownership_secs(ITEM_ANTIFLAG_DROP), 300);
        assert_eq!(
            given_ownership_secs(ITEM_ANTIFLAG_DROP | ITEM_ANTIFLAG_GIVE),
            300
        );
    }

    /// `mob_name` names only a monster `CMobManager::Get` finds (`questlua_global.cpp:673`), and
    /// `item_name` only an item `GetTable` finds, a range row's vnums strictly inside it
    /// (`questlua_global.cpp:649`), each in the character's language. Each chat line goes out as
    /// `ChatPacket(type, "%s", text)` of its own type.
    #[test]
    fn a_script_names_existing_monsters_and_items_and_chats_in_its_lines_type() {
        use gamedata::item_proto::ItemProto;
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
        let ymir = MobLocaleNames::parse(b"VNUM\tNAME\n20016\tSmith\n20017\tGhost\n").unwrap();
        let english = MobLocaleNames::parse(b"VNUM\tNAME\n20016\tBlacksmith\n").unwrap();
        let names = MobNamesByLanguage::from_tables(vec![ymir, english]);
        let range = ItemProto {
            vnum_range: 99,
            ..ItemProto::for_category_rule(110_000, 0, 0)
        };
        let items = ItemProtos::from_rows(vec![ItemProto::for_category_rule(11_901, 0, 0), range]);
        let ymir = MobLocaleNames::parse(b"VNUM\tNAME\n11901\tSmoking\n110050\tBook\n30\tX\n");
        let english = MobLocaleNames::parse(b"VNUM\tNAME\n11901\tTuxedo\n").unwrap();
        let item_names = ItemNamesByLanguage::from_tables(vec![ymir.unwrap(), english]);
        with_session((&protos, &names), (&items, &item_names), |session| {
            assert_eq!(session.mob_name(20_016), Some(b"Smith".to_vec()));
            assert_eq!(session.mob_name(20_017), None, "a name with no proto");
            assert_eq!(session.item_name(11_901), Some(b"Smoking".to_vec()));
            assert_eq!(
                session.item_name(110_050),
                Some(b"Book".to_vec()),
                "inside the range"
            );
            assert_eq!(
                session.item_name(110_051),
                Some(b"NoName".to_vec()),
                "a range's, unnamed"
            );
            assert_eq!(
                session.item_name(110_000),
                Some(b"NoName".to_vec()),
                "the range's own row"
            );
            assert_eq!(
                session.item_name(110_099),
                None,
                "a range's end is outside it"
            );
            assert_eq!(session.item_name(30), None, "a name with no proto");
            let kinds = [
                (ChatKind::Talking, CHAT_TYPE_TALKING),
                (ChatKind::Info, CHAT_TYPE_INFO),
                (ChatKind::Notice, CHAT_TYPE_NOTICE),
                (ChatKind::Command, CHAT_TYPE_COMMAND),
            ];
            for (kind, _) in kinds {
                session.chat(kind, b"hi");
            }
            let to = test_place::MOVER.recipient(session.locale);
            let expected: Vec<Vec<u8>> = kinds
                .iter()
                .map(|(_, chat_type)| chat_packet(to, *chat_type, b"%s", &[Arg::Text(b"hi")]))
                .collect();
            assert_eq!(session.records, expected);

            session.mover.language = common::enums::LOCALE_DEFAULT as u8;
            assert_eq!(
                session.mob_name(20_016),
                Some(b"Blacksmith".to_vec()),
                "the character's language"
            );
            assert_eq!(session.item_name(11_901), Some(b"Tuxedo".to_vec()));
            assert_eq!(session.item_name(110_050), Some(b"NoName".to_vec()));
        });
    }

    /// `SendQuestInfoPakcet` (`G/questpc.cpp:231-308`): the window a script changed goes
    /// out in this step's records as one `GC_QUEST_INFO`, in `packet_quest_info` field
    /// order, with the parts its bits name and the size of the whole record.
    #[test]
    fn the_quest_window_goes_out_as_one_quest_info_record() {
        let protos = MobProtos::from_rows(Vec::new());
        let names = MobNamesByLanguage::default();
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
            items: &ItemProtos::default(),
            item_names: &ItemNamesByLanguage::default(),
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
            block_mode: None,
            pid: 1,
            targets: &mut QuestTargets::default(),
            pulse: 0,
            giver: None,
            item_rows: Vec::new(),
            laid: Vec::new(),
        };
        session.quest_info(&QuestInfo {
            index: 0x0128,
            category: 0x0203,
            begin: Some(true),
            title: Some(b"Bonus".to_vec()),
        });
        session.quest_info(&QuestInfo {
            index: 0x0128,
            category: 0,
            begin: Some(false),
            title: None,
        });
        let mut titled = vec![81, 40, 0, 0x28, 0x01, 0x03, 0x02, 0b11, 1];
        let mut title = [0; 31];
        title[..5].copy_from_slice(b"Bonus");
        titled.extend_from_slice(&title);
        assert_eq!(
            session.records,
            [titled, vec![81, 9, 0, 0x28, 0x01, 0, 0, 0b01, 0]]
        );
    }

    /// `SetBlockModeForce` (`G/char.cpp:7658-7661`): the byte lands now, the
    /// client hears `setblockmode %d` in this step's records, the last call
    /// winning, and no quest flag moves — the pending byte rides the session.
    #[test]
    fn send_block_mode_lands_the_line_and_the_last_byte_wins() {
        use gamedata::mob_locale_names::{MobLocaleNames, MobNamesByLanguage};
        use gamedata::mob_proto::MobProto;
        use gamedata::mob_proto::CHAR_TYPE_NPC;
        use gamedata::records::MobTableRecord;

        let protos = MobProtos::from_rows(vec![MobProto {
            line: 2,
            table: MobTableRecord {
                vnum: 20_016,
                mob_type: CHAR_TYPE_NPC,
                ..MobTableRecord::default()
            },
        }]);
        let english = MobLocaleNames::parse(b"VNUM\tNAME\n20016\tSmith\n").unwrap();
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
            items: &ItemProtos::default(),
            item_names: &ItemNamesByLanguage::default(),
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
            block_mode: None,
            pid: 1,
            targets: &mut QuestTargets::default(),
            pulse: 0,
            giver: None,
            item_rows: Vec::new(),
            laid: Vec::new(),
        };
        assert_eq!(session.block_mode, None);
        session.send_block_mode(8);
        session.send_block_mode(24);
        assert_eq!(session.block_mode, Some(24), "the last call wins");
        assert_eq!(session.records.len(), 2);
        let to = test_place::MOVER.recipient(&locale);
        assert_eq!(
            session.records[1],
            chat_packet(to, CHAT_TYPE_COMMAND, b"setblockmode %d", &[Arg::Int(24)]),
        );
    }

    /// `CQuestManager::Login` (`G/questmanager.cpp:541-556`) on the world: the login
    /// scripts run for the character with the flags its load set, `game_option` says what
    /// it blocks and sends `setblockmode`, the byte lands on the character, and the lines go
    /// to its own client.
    #[test]
    fn a_login_lands_the_block_byte_and_sends_its_lines() {
        use crate::game_state::fixtures::{a_world, enter, TestClock};

        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager = Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        world.quests = Some(Quests::new(
            manager,
            Arc::default(),
            MobNamesByLanguage::default(),
        ));
        let mut inbox = enter(&mut world, 1, (500, 500), 0);
        while inbox.try_recv().is_ok() {}
        let vid = Vid::new(1);
        if let Some(quests) = world.quests.as_mut() {
            quests
                .manager_mut()
                .load_flags(1, [(b"game_option.block_whisper".to_vec(), 1)]);
        }

        world.quest_login(vid);
        assert_eq!(world.block_modes.get(&vid), Some(&8));
        let mut lines = Vec::new();
        while let Ok(line) = inbox.try_recv() {
            lines.push(line);
        }
        let to = world
            .own_mover(vid)
            .expect("in the world")
            .recipient(&world.locale);
        let command = chat_packet(to, CHAT_TYPE_COMMAND, b"setblockmode %d", &[Arg::Int(8)]);
        let at = lines
            .iter()
            .position(|line| *line == command)
            .expect("the client hears the byte");
        assert!(
            lines[..at]
                .iter()
                .any(|line| line.get(3) == Some(&CHAT_TYPE_INFO)),
            "the syschat that names the block comes first"
        );
    }

    /// `send_letter` at login titles its quest: the `GC_QUEST_INFO` the script's end sent
    /// (`quest::PC::EndRunning`, `G/questpc.cpp:342-346`) follows the button's `GC_SCRIPT`,
    /// and the quest window's entry (`CInputMain::ScriptButton` with the high bit,
    /// `G/input_main.cpp`) runs the quest's `info` script.
    #[test]
    fn the_letter_titles_its_quest_and_the_window_runs_its_info() {
        use crate::game_state::fixtures::{a_world, enter, points_at, TestClock, PLACE};

        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager = Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        let quest = manager
            .host()
            .quest_index(b"item_attr67add")
            .expect("the letter's quest");
        world.quests = Some(Quests::new(
            manager,
            Arc::default(),
            MobNamesByLanguage::default(),
        ));
        let mut inbox = enter(&mut world, 1, (500, 500), 0);
        while inbox.try_recv().is_ok() {}
        let vid = Vid::new(1);
        world
            .characters
            .find_by_vid_mut(vid)
            .expect("entered")
            .set_points(Some(points_at(30, 0, 0)));
        if let Some(quests) = world.quests.as_mut() {
            quests.manager_mut().load_flags(1, Vec::new());
        }
        let place = GroundPlace {
            channel: PLACE.0,
            map: PLACE.1,
            x: 500,
            y: 500,
        };
        let mover = world.own_mover(vid).expect("in the world");
        // Before the login no state names the quest, so the window's entry runs nothing,
        // where the letter's own button would have started it (`NPC::OnInfo`).
        let (records, _climbs, _stores, _items) = world
            .quest(
                vid,
                &QuestStep::Button(quest | QUEST_INFO_BIT),
                place,
                mover,
            )
            .expect("the world holds the character");
        assert!(records.is_empty(), "{records:?}");

        world.quest_login(vid);
        let mut lines = Vec::new();
        while let Ok(line) = inbox.try_recv() {
            lines.push(line);
        }
        let button = format!("[QUESTBUTTON idx;{quest}|name;Sixth and Seventh Bonus][DONE]");
        let at = lines
            .iter()
            .position(|line| {
                GcScript::decode(line).is_ok_and(|script| {
                    script.skin == 0 && script.script.starts_with(button.as_bytes())
                })
            })
            .expect("the letter's button");
        // `packet_quest_info` in source field order: header, size, index, c_index, flag,
        // then the 31-byte title the TITLE bit names.
        let index = u16::try_from(quest).expect("a 16-bit quest number");
        let mut golden = vec![81, 39, 0];
        golden.extend_from_slice(&index.to_le_bytes());
        golden.extend_from_slice(&[0, 0, 0b10]);
        let mut title = [0; 31];
        title[..23].copy_from_slice(b"Sixth and Seventh Bonus");
        golden.extend_from_slice(&title);
        assert_eq!(
            lines.get(at + 1),
            Some(&golden),
            "the title follows its button"
        );

        let (records, _climbs, _stores, _items) = world
            .quest(
                vid,
                &QuestStep::Button(quest | QUEST_INFO_BIT),
                place,
                mover,
            )
            .expect("the world holds the character");
        let info = records
            .iter()
            .find_map(|record| GcScript::decode(record).ok())
            .expect("the info dialog");
        assert_eq!(info.skin, 1, "QUEST_SKIN_NORMAL");
        assert!(
            info.script
                .starts_with(b"It is possible to add a 6th or 7th bonus"),
            "{info:?}"
        );
        // The dialog moved the quest to `talk`; a second press finds no script there.
        let (records, _climbs, _stores, _items) = world
            .quest(
                vid,
                &QuestStep::Button(quest | QUEST_INFO_BIT),
                place,
                mover,
            )
            .expect("the world holds the character");
        assert!(records.is_empty(), "{records:?}");
        let (records, _climbs, _stores, _items) = world
            .quest(vid, &QuestStep::Button(999 | QUEST_INFO_BIT), place, mover)
            .expect("the world holds the character");
        assert!(records.is_empty(), "an unknown quest is quiet: {records:?}");
    }

    /// `pc.get_playtime` reads `GetRealPoint(POINT_PLAYTIME)` (`G/questlua_pc.cpp:874-878`):
    /// the minutes the points loaded with, and once a save banks the minutes played
    /// (`CreatePlayerProto`, `G/char.cpp:1519-1545`) the banked ones. The sash quest's
    /// `time_to_wait` window says how many of its wait's minutes are left.
    #[test]
    fn the_sash_wait_counts_down_the_minutes_a_save_banks() {
        use crate::game_loop_messages::GameCommand;
        use crate::game_state::fixtures::{a_world, enter, points_at, TestClock, PLACE};

        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager = Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        let quest = manager
            .host()
            .quest_index(b"sash_mission")
            .expect("the sash quest");
        world.quests = Some(Quests::new(
            manager,
            Arc::default(),
            MobNamesByLanguage::default(),
        ));
        let mut inbox = enter(&mut world, 1, (500, 500), 0);
        while inbox.try_recv().is_ok() {}
        let vid = Vid::new(1);
        let mut points = points_at(154, 0, 0);
        points.set_playtime(300);
        world
            .characters
            .find_by_vid_mut(vid)
            .expect("entered")
            .set_points(Some(points));
        if let Some(quests) = world.quests.as_mut() {
            // The CRC32 of `time_to_wait`, 3464966988, stored as the `int` it is.
            quests.manager_mut().load_flags(
                1,
                [
                    (b"sash_mission.__status".to_vec(), -830_000_308),
                    (b"sash_mission.time_to_wait".to_vec(), 480),
                ],
            );
        }
        let place = GroundPlace {
            channel: PLACE.0,
            map: PLACE.1,
            x: 500,
            y: 500,
        };
        let mover = world.own_mover(vid).expect("in the world");
        let left = |world: &mut GameState, minutes: i32| {
            let (records, _climbs, _stores, _items) = world
                .quest(
                    vid,
                    &QuestStep::Button(quest | QUEST_INFO_BIT),
                    place,
                    mover,
                )
                .expect("the world holds the character");
            let info = records
                .iter()
                .find_map(|record| GcScript::decode(record).ok())
                .expect("the info dialog");
            let line = format!("Remaining time: {minutes} minutes.");
            assert!(
                info.script
                    .windows(line.len())
                    .any(|window| window == line.as_bytes()),
                "{line} in {:?}",
                String::from_utf8_lossy(&info.script)
            );
        };
        let bank = |world: &mut GameState, playtime: Option<i32>| {
            let (reply, answer) = tokio::sync::oneshot::channel();
            world.apply(GameCommand::KeptOf {
                vid,
                playtime,
                reply,
            });
            answer
                .blocking_recv()
                .expect("the world answers")
                .and_then(|kept| kept.points)
                .map(|points| points.real_point(common::point_slot::POINT_PLAYTIME))
        };
        left(&mut world, 180);
        assert_eq!(
            bank(&mut world, Some(302)),
            Some(302),
            "the save reads the bank"
        );
        left(&mut world, 178);
        assert_eq!(
            bank(&mut world, None),
            Some(302),
            "no bank leaves the minutes"
        );
        left(&mut world, 178);
    }

    fn proto(vnum: u32, mob_type: u8) -> gamedata::mob_proto::MobProto {
        gamedata::mob_proto::MobProto {
            line: 2,
            table: gamedata::records::MobTableRecord {
                vnum,
                mob_type,
                ..gamedata::records::MobTableRecord::default()
            },
        }
    }

    fn npc(vid: u32, vnum: u32, char_type: u8) -> world::npc::Npc {
        world::npc::Npc {
            vid,
            vnum,
            race: u16::try_from(vnum & 0xffff).expect("the low 16 bits"),
            char_type,
            x: 600,
            y: 600,
            hp: 100,
            max_hp: 100,
            ..world::npc::Npc::default()
        }
    }

    /// `GetCharactersByRaceNum` holds only registered races: an NPC's, a warp's or a goto's
    /// proto vnum, or a Christmas character's whatever its proto (`G/char_manager.cpp:47-49`).
    /// Of a registered race the first spawned answers; a monster's race answers 0 though it
    /// stands on the map, and so does a registered race that is not there. The race is the
    /// character's low 16 bits (`GetRaceNum`, a `WORD`).
    #[test]
    fn find_npc_by_vnum_answers_only_registered_races() {
        use gamedata::mob_proto::CHAR_TYPE_MONSTER;
        let protos = MobProtos::from_rows(vec![
            proto(60_003, CHAR_TYPE_NPC),
            proto(101, CHAR_TYPE_MONSTER),
            proto(20_018, CHAR_TYPE_WARP),
            proto(20_031, CHAR_TYPE_MONSTER),
            proto(9_010, CHAR_TYPE_GOTO),
        ]);
        let map = MapNpcs {
            npcs: vec![
                npc(0x8000_0001, 101, CHAR_TYPE_MONSTER),
                npc(0x8000_0002, 60_003 + 65_536, CHAR_TYPE_MONSTER),
                npc(0x8000_0003, 60_003, CHAR_TYPE_NPC),
                npc(0x8000_0004, 20_031, CHAR_TYPE_MONSTER),
                npc(0x8000_0005, 9_010, CHAR_TYPE_GOTO),
            ],
            positions: Vec::new(),
            regens: Vec::new(),
        };
        let find = |race: u32| find_by_race(Some(&map), &protos, race);
        assert_eq!(find(60_003), 0x8000_0002, "the first spawned of the race");
        assert_eq!(find(101), 0, "a monster's race is not registered");
        assert_eq!(find(20_031), 0x8000_0004, "the Christmas races are");
        assert_eq!(find(9_010), 0x8000_0005, "a goto's is");
        assert_eq!(find(20_018), 0, "a warp's race, but none on the map");
        assert_eq!(find(60_003 + 65_536), 0, "no proto registers it");
        assert_eq!(find(7), 0);
        assert_eq!(find_by_race(None, &protos, 60_003), 0);
    }

    const THEOWAHDAN: u32 = world::npc::FIRST_NPC_VID + 3;

    /// A world of the owner's quests with Theowahdan at (600, 600), and character 1, level
    /// 40, entered at (500, 500) with no quest run and the inbox drained; then the level-up
    /// that moves the sash quest on, whose letter marks him, at Pulse 100. Answers the world,
    /// the inbox, the level-up's records and the sash quest's number.
    fn marked_theowahdan(
        clock: &crate::game_state::fixtures::TestClock,
    ) -> (
        GameState,
        tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
        Vec<Vec<u8>>,
        u32,
    ) {
        use crate::game_state::fixtures::{enter, open_cells, points_at, stand_field};

        // Only a test moves Theowahdan: `AIFLAG_NOMOVE` keeps his idle wander
        // (`__StateIdle_NPC`, one draw in 7, on the clock) from walking him between the checks.
        let theowahdan = world::npc::Npc {
            ai_flags: gamedata::mob_proto::AIFLAG_NOMOVE,
            ..npc(THEOWAHDAN, 60_003, CHAR_TYPE_NPC)
        };
        let npcs = vec![npc(world::npc::FIRST_NPC_VID + 1, 101, 0), theowahdan];
        let mut world = stand_field(clock, npcs, open_cells(), true);
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager = Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        let sash = manager.host().quest_index(b"sash_mission").expect("sash");
        let protos = MobProtos::from_rows(vec![proto(60_003, CHAR_TYPE_NPC), proto(101, 0)]);
        world.quests = Some(Quests::new(
            manager,
            Arc::new(protos),
            MobNamesByLanguage::default(),
        ));
        let mut inbox = enter(&mut world, 1, (500, 500), 0);
        while inbox.try_recv().is_ok() {}
        let vid = Vid::new(1);
        world
            .characters
            .find_by_vid_mut(vid)
            .expect("entered")
            .set_points(Some(points_at(40, 0, 0)));
        if let Some(quests) = world.quests.as_mut() {
            quests.manager_mut().load_flags(1, Vec::new());
        }
        world.last_pulse = 100;
        let mover = world.own_mover(vid).expect("in the world");
        let records = world.quest_levelup(vid, beside(), mover);
        (world, inbox, records, sash)
    }

    /// Where character 1 stands in [`marked_theowahdan`]'s world.
    fn beside() -> GroundPlace {
        GroundPlace {
            channel: crate::game_state::fixtures::PLACE.0,
            map: crate::game_state::fixtures::PLACE.1,
            ..test_place::AT
        }
    }

    /// The sash quest's state change at level 40 runs its letter (`G/questpc.cpp:352-371`),
    /// which finds Theowahdan on the map and marks him: the client hears `GC_TARGET_CREATE`
    /// (125) before the button's `GC_SCRIPT` and the window's `GC_QUEST_INFO`. The mark is
    /// the character's until it leaves the world, which drops it with no record
    /// (`CTargetManager::Logout`, `G/char.cpp:1815`).
    #[test]
    fn a_letter_marks_the_npc_it_finds_until_the_character_leaves() {
        let clock = crate::game_state::fixtures::TestClock::default();
        let (mut world, mut inbox, records, sash) = marked_theowahdan(&clock);
        // `TPacketGCTargetCreate` in source field order: header, `lID`, the 33-byte name,
        // `dwVID`, `bType`.
        let mut golden = vec![125, 1, 0, 0, 0];
        let mut name = [0; 33];
        name[..10].copy_from_slice(b"Theowahdan");
        golden.extend_from_slice(&name);
        golden.extend_from_slice(&THEOWAHDAN.to_le_bytes());
        golden.push(2);
        assert_eq!(records.first(), Some(&golden), "{records:?}");
        let button = format!("[QUESTBUTTON idx;{sash}|name;A special cloth][DONE]");
        let script = GcScript::decode(&records[1]).expect("the button");
        assert_eq!((script.skin, script.script), (0, button.into_bytes()));
        assert_eq!(records[2].first(), Some(&81), "the window");
        assert_eq!(records.len(), 3);
        assert_eq!(world.quest_targets.vid(1, sash, b"theowahdan"), THEOWAHDAN);

        let _kept = world.leave_world(Vid::new(1));
        assert_eq!(world.quest_targets.vid(1, sash, b"theowahdan"), 0);
        while let Ok(line) = inbox.try_recv() {
            assert_ne!(line.first(), Some(&124), "the logout sends no delete");
        }
    }

    /// `target_event` (`G/target.cpp:57-132`), first a second after the mark: the client
    /// hears where Theowahdan stands (`GC_TARGET_UPDATE`, 123) whenever that changed. Within
    /// 500, 500 itself included, the sash quest has no `arrive` script, so the next check is a
    /// second later; past it the next is the distance over 60 Pulses, at most 125. Once no
    /// character holds his VID, the `die` check ends the mark and the client hears its end.
    #[test]
    fn the_marks_event_follows_the_npc_and_ends_with_it() {
        use crate::game_loop::PulseProcessor;

        fn theowahdan(world: &mut GameState) -> &mut world::npc::Npc {
            let list = world
                .npcs
                .get_mut(&crate::game_state::fixtures::PLACE)
                .expect("the map's NPCs");
            Arc::make_mut(list)
                .npcs
                .iter_mut()
                .find(|npc| npc.vid == THEOWAHDAN)
                .expect("Theowahdan")
        }

        let clock = crate::game_state::fixtures::TestClock::default();
        let (mut world, mut inbox, _records, sash) = marked_theowahdan(&clock);
        let mut heard = |world: &mut GameState, pulse: u64| -> Vec<Vec<u8>> {
            world.process_pulse(pulse);
            std::iter::from_fn(|| inbox.try_recv().ok())
                .filter(|record| matches!(record.first(), Some(123 | 124)))
                .collect()
        };
        let update = |x: i32, y: i32| {
            let mut out = vec![123, 1, 0, 0, 0];
            out.extend_from_slice(&x.to_le_bytes());
            out.extend_from_slice(&y.to_le_bytes());
            out
        };
        let move_to = |world: &mut GameState, x: i32, y: i32| {
            let npc = theowahdan(world);
            (npc.x, npc.y) = (x, y);
        };
        assert!(heard(&mut world, 124).is_empty(), "a second after the mark");
        assert_eq!(heard(&mut world, 125), [update(600, 600)]);
        move_to(&mut world, 600, 3600);
        assert!(heard(&mut world, 149).is_empty());
        assert_eq!(
            heard(&mut world, 150),
            [update(600, 3600)],
            "135 away, no `arrive` script: a second"
        );
        move_to(&mut world, 600, 9000);
        assert!(heard(&mut world, 199).is_empty());
        assert_eq!(
            heard(&mut world, 200),
            [update(600, 9000)],
            "3018 away: 50 Pulses"
        );
        assert!(heard(&mut world, 324).is_empty());
        assert!(
            heard(&mut world, 325).is_empty(),
            "8208 away: 125 Pulses, and the spot did not change"
        );
        move_to(&mut world, 600, 600);
        assert!(heard(&mut world, 449).is_empty());
        assert_eq!(heard(&mut world, 450), [update(600, 600)]);
        move_to(&mut world, 500, 1021);
        assert!(heard(&mut world, 474).is_empty());
        assert_eq!(heard(&mut world, 475), [update(500, 1021)]);
        move_to(&mut world, 500, 1022);
        assert!(heard(&mut world, 499).is_empty());
        assert_eq!(
            heard(&mut world, 500),
            [update(500, 1022)],
            "500 away is within `arrive`'s reach: a second"
        );
        move_to(&mut world, 600, 600);
        assert!(heard(&mut world, 511).is_empty());
        assert_eq!(
            heard(&mut world, 512),
            [update(600, 600)],
            "501 away: 12 Pulses"
        );

        world.destroy_npc(THEOWAHDAN);
        assert!(heard(&mut world, 536).is_empty());
        assert_eq!(
            heard(&mut world, 537),
            [vec![124, 1, 0, 0, 0]],
            "the mark ends"
        );
        assert_eq!(world.quest_targets.vid(1, sash, b"theowahdan"), 0);
        assert!(heard(&mut world, 1000).is_empty());
    }

    /// `target_event` while the character stands on another map than its mark's
    /// (`G/target.cpp:77-78`), as a party summon's `Show` leaves it without a Warp
    /// (`summon_show`): the client hears nothing, and the next check comes
    /// `MINMAX(12, 5000 / 60, 125)` = 83 Pulses later.
    #[test]
    fn a_mark_waits_while_its_character_stands_on_another_map() {
        use super::super::view::EntityKey;
        use crate::game_loop::PulseProcessor;
        use crate::game_state::fixtures::{region, TestClock, PLACE};
        use gamedata::map_atlas::MapRegion;
        use gamedata::server_attr::SectreeGrid;

        let clock = TestClock::default();
        let (mut world, mut inbox, _records, _sash) = marked_theowahdan(&clock);
        let other = MapRegion {
            index: PLACE.1 + 1,
            ..region()
        };
        world.host_map(
            PLACE.0,
            other.index,
            SectreeGrid::of(&other).expect("a grid"),
        );
        let mut heard = |world: &mut GameState, pulse: u64| -> Vec<Vec<u8>> {
            world.process_pulse(pulse);
            std::iter::from_fn(|| inbox.try_recv().ok())
                .filter(|record| matches!(record.first(), Some(123 | 124)))
                .collect()
        };
        // `summon_show`'s steps to another map: out of the old map's index, into the new one's.
        let stand_on = |world: &mut GameState, map: i32| {
            let from = world.place_of(1).expect("a place");
            let mut effects = Vec::new();
            let index = world.maps.get_mut(&from).expect("its map");
            index.remove(EntityKey::Character(1), &mut effects);
            world.bodies.get_mut(&Vid::new(1)).expect("a body").map = map;
            assert!(world.show_body(1, (500, 500, 0), None));
        };
        stand_on(&mut world, other.index);
        assert!(heard(&mut world, 125).is_empty(), "another map");
        stand_on(&mut world, PLACE.1);
        assert!(heard(&mut world, 207).is_empty());
        let mut update = vec![123, 1, 0, 0, 0];
        update.extend_from_slice(&600_i32.to_le_bytes());
        update.extend_from_slice(&600_i32.to_le_bytes());
        assert_eq!(heard(&mut world, 208), [update], "83 Pulses after the miss");
    }

    /// `CQuestManager::Click`'s target arm (`G/questmanager.cpp:934-945`): a click on the
    /// marked Theowahdan runs the sash quest's `theowahdan.target.click`, which ends the mark
    /// (`GC_TARGET_DELETE`) and talks until its first `wait`.
    #[test]
    fn a_click_on_the_marked_npc_runs_the_quests_target_script() {
        let clock = crate::game_state::fixtures::TestClock::default();
        let (mut world, _inbox, _records, sash) = marked_theowahdan(&clock);
        let vid = Vid::new(1);
        let mover = world.own_mover(vid).expect("in the world");
        let (taken, (records, _climbs, _stores, _items)) =
            world.quest_click(vid, (THEOWAHDAN, 60_003), beside(), mover);
        assert!(taken);
        assert_eq!(records[0], [124, 1, 0, 0, 0], "{records:?}");
        let page = GcScript::decode(&records[1]).expect("the first page");
        assert!(
            page.script
                .windows(26)
                .any(|text| text == b"Ah, here you are! I have d"),
            "{page:?}"
        );
        assert_eq!(records.len(), 2);
        assert_eq!(world.quest_targets.vid(1, sash, b"theowahdan"), 0);
        assert!(world.quest_running(vid), "the page waits");
    }
}
