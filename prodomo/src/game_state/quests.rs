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
use crate::client_registry::ClientOrder;
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
    /// The `/goto` destinations the libraries recorded at load (`add_goto_info`,
    /// `questlua_global.cpp:433`), in order. Read once at construction: the
    /// table never changes after load, so a `/goto` line borrows it instead
    /// of cloning the whole `Settings`.
    goto_info: Vec<quest::host::GotoInfo>,
}

impl Quests {
    /// The quests of `manager`, whose `mob_name` names the monsters of `protos` from `names`.
    #[must_use]
    pub fn new(manager: Manager, protos: Arc<MobProtos>, names: MobNamesByLanguage) -> Self {
        let goto_info = manager.host().settings().goto_info;
        Self {
            manager,
            protos,
            names,
            item_names: ItemNamesByLanguage::default(),
            goto_info,
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

    /// The `/goto` destinations, in load order.
    #[must_use]
    pub fn goto_info(&self) -> &[quest::host::GotoInfo] {
        &self.goto_info
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
    /// `CG_SCRIPT_SELECT_ITEM`: the cell the client picked, or 0 for a closed window.
    SelectItem(u32),
    /// `CG_REQUEST_EVENT_QUEST`: the event panel names a quest, whose button runs.
    EventQuest(Vec<u8>),
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

    /// The name `CInputMain::RequestEventQuest` passes on: legacy builds a
    /// `std::string` from the whole 65-byte field, which reads past its end
    /// when no NUL is stored (a Defect); the Rewrite stops at the first NUL
    /// and then at the field's end.
    #[must_use]
    pub fn event_quest(record: &protocol::cg_quest_text::CgRequestEventQuest) -> Self {
        let end = record
            .sz_name
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(CG_QUEST_TEXT_FIELD_SIZE);
        Self::EventQuest(record.sz_name[..end].to_vec())
    }
}

/// What a quest step reports: the records its scripts sent for the client, the GRAND
/// climbs its reads landed (each for the store's `store_skill`), the `guild.*`
/// writers the scripts ran (each for the descriptor to drain after the step, in order),
/// the item rows its gives changed (for the descriptor to store before it sends), and
/// the gold its `pc.change_money` moves and its melts left the character holding.
pub type QuestStepOutcome = (
    Vec<Vec<u8>>,
    Vec<(u8, u8, i32)>,
    Vec<super::guild::GuildQuestStore>,
    Vec<db::items::RowChange>,
    Option<u64>,
);

/// What a quest run reports: the records, the GRAND climbs, the `guild.*`
/// calls the scripts made (each for [`GameState::answer_guild_quest`] after the step),
/// the last `pc.send_block_mode` byte the scripts set, for the block byte, the
/// `pc.change_alignment` amounts, the `pc.change_money` amounts, and the
/// `pc.hide_costume` hides.
type QuestRun = (
    Vec<Vec<u8>>,
    Vec<(u8, u8, i32)>,
    Vec<super::guild::GuildQuestOp>,
    Option<u8>,
    Vec<i32>,
    Vec<i64>,
    Vec<(u8, bool)>,
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

/// The warp gates a quest step judges, read at the step's start: whether `IsHack`
/// refuses the reader (so `pc.warp` answers false), and whether `CanWarp` passes
/// (what `pc.can_warp` answers). One byte keeps [`Session`] under Clippy's bool cap.
#[derive(Clone, Copy, Default)]
pub(super) struct WarpGates(u8);

impl WarpGates {
    /// `IsHack` refuses the reader.
    const HACK: u8 = 1;
    /// `CanWarp` passes for the reader.
    const ABLE: u8 = 2;

    /// Both gates armed: no hack, and the warp allowed.
    #[must_use]
    pub(super) fn armed() -> Self {
        Self(Self::ABLE)
    }

    /// Whether `IsHack` refuses the reader.
    #[must_use]
    pub(super) fn hack(self) -> bool {
        self.0 & Self::HACK != 0
    }

    /// Whether `CanWarp` passes for the reader.
    #[must_use]
    pub(super) fn able(self) -> bool {
        self.0 & Self::ABLE != 0
    }

    /// Arm or disarm the `IsHack` refusal.
    pub(super) fn set_hack(&mut self, hack: bool) {
        if hack {
            self.0 |= Self::HACK;
        } else {
            self.0 &= !Self::HACK;
        }
    }

    /// Arm or disarm the `CanWarp` pass.
    pub(super) fn set_able(&mut self, able: bool) {
        if able {
            self.0 |= Self::ABLE;
        } else {
            self.0 &= !Self::ABLE;
        }
    }
}

/// The party flags a quest step judges, read at the step's start: whether the
/// reader stands in a party (what `party.is_party` answers), and whether it
/// leads it (what `party.is_leader` answers). One byte keeps [`Session`] under
/// Clippy's bool cap.
#[derive(Clone, Copy, Default)]
pub(super) struct PartyFlags(u8);

impl PartyFlags {
    /// The reader stands in a party.
    const IN_PARTY: u8 = 1;
    /// The reader leads its party.
    const LEADER: u8 = 2;

    /// Build from the two answers.
    #[must_use]
    pub(super) fn new(is_party: bool, is_leader: bool) -> Self {
        let mut flags = Self(0);
        flags.set_party(is_party);
        flags.set_leader(is_leader);
        flags
    }

    /// Whether the reader stands in a party.
    #[must_use]
    pub(super) fn party(self) -> bool {
        self.0 & Self::IN_PARTY != 0
    }

    /// Whether the reader leads its party.
    #[must_use]
    pub(super) fn leader(self) -> bool {
        self.0 & Self::LEADER != 0
    }

    /// Arm or disarm the party flag.
    pub(super) fn set_party(&mut self, party: bool) {
        if party {
            self.0 |= Self::IN_PARTY;
        } else {
            self.0 &= !Self::IN_PARTY;
        }
    }

    /// Arm or disarm the leader flag.
    pub(super) fn set_leader(&mut self, leader: bool) {
        if leader {
            self.0 |= Self::LEADER;
        } else {
            self.0 &= !Self::LEADER;
        }
    }
}

/// A [`Session`] borrowed from the world with its quest manager and reader, ready for
/// the `op` [`GameState::run_quests_giving_raw`] runs.
struct ArmedSession<'s> {
    manager: &'s mut Manager,
    session: Session<'s>,
    reader: Vid,
}

/// The world-visible queues a script step filled, taken from the [`Session`] in one
/// move so [`GameState::run_quests_giving_raw`] stays under the line lint.
struct WorldMoves {
    laid: Vec<world::item::Item>,
    warps: Vec<(i32, i32, i32)>,
    drops: Vec<(u32, i32, Option<i32>)>,
    villages: u32,
    conquerors: u32,
    sash_opens: Vec<bool>,
    changelooks: u32,
    minings: Vec<u32>,
    horse_ops: Vec<super::horse_summon::HorseOp>,
    ds_refines: u32,
    ds_attrs: u32,
    party_sets: Vec<(Vec<u8>, i32)>,
    confirm_records: Vec<(u32, Vec<u8>)>,
    confirm_timeouts: Vec<(u32, i32)>,
}

impl WorldMoves {
    fn take(session: &mut Session<'_>) -> Self {
        Self {
            laid: std::mem::take(&mut session.laid),
            warps: std::mem::take(&mut session.warps),
            drops: std::mem::take(&mut session.drops),
            villages: session.villages,
            conquerors: session.conquerors,
            sash_opens: std::mem::take(&mut session.sash_opens),
            changelooks: session.changelooks,
            minings: std::mem::take(&mut session.minings),
            horse_ops: std::mem::take(&mut session.horse_ops),
            ds_refines: session.ds_refines,
            ds_attrs: session.ds_attrs,
            party_sets: std::mem::take(&mut session.party_sets),
            confirm_records: std::mem::take(&mut session.confirm_records),
            confirm_timeouts: std::mem::take(&mut session.confirm_timeouts),
        }
    }
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
    /// `GetGold()` of the character the script runs for.
    pub(super) gold: u64,
    /// `GetEmpire()` of the character the script runs for.
    pub(super) empire: u8,
    /// `GetName()` of the character the script runs for.
    pub(super) char_name: Vec<u8>,
    /// `GetX()/100`, `GetY()/100` and the local pair against the map's base, of the
    /// character the script runs for.
    pub(super) position: (i32, i32, i32, i32),
    /// `GetLeadershipSkillLevel()` of the character the script runs for.
    pub(super) leadership: u8,
    /// `GetGuild()->GetID()` of the character the script runs for, and 0 with none.
    pub(super) guild_id: u32,
    /// Whether the character the script runs for leads its guild.
    pub(super) is_guild_master: bool,
    /// Its skill slots, which the GRAND read climbs and stamps.
    pub(super) slots: &'s mut super::pc_skill::PcSkillState,
    /// Its timed affects, which the GRAND read's book bonus consumes and the alignment bands
    /// and `pc.remove_skill_book_no_delay` change.
    pub(super) affects: Option<&'s mut super::affect::AffectEvent>,
    /// `GetRealAlignment()` of the character the script runs for, as `pc.change_alignment`
    /// leaves it: the value the script reads back, applied to the world after the step.
    pub(super) alignment: i32,
    /// Every `pc.change_alignment` amount, in order, for [`GameState::update_alignment`] after
    /// the step (legacy runs it in the script).
    pub(super) alignment_changes: Vec<i32>,
    /// Every `pc.change_money` amount, in order, for [`GameState::apply_gold_changes`]
    /// after the step (legacy runs it in the script).
    pub(super) gold_changes: Vec<i64>,
    /// Every `pc.hide_costume` hide, in order, for [`GameState::apply_hidden_changes`]
    /// after the step (legacy sends `UpdatePacket` in the setter).
    pub(super) hides: Vec<(u8, bool)>,
    /// Its points, which the GRAND read's 141 recompute rewrites.
    pub(super) points: Option<&'s mut world::character::Points>,
    /// What the GRAND read climbed and `pc.set_skill_level` set, for the store's
    /// `store_skill`: the slot, level and stamp of each, in order.
    pub(super) climbs: Vec<(u8, u8, i32)>,
    /// The `guild.*` calls and the `pc.destroy_guild` / `pc.remove_from_guild` calls the
    /// scripts made, in order, each for [`GameState::answer_guild_quest`] after the step.
    pub(super) guild_ops: Vec<super::guild::GuildQuestOp>,
    /// `test_server` (`G/config.cpp:72`): whether the deployment runs in test-server mode.
    pub(super) test_server: bool,
    /// Every online PC's shown name and VID, for `find_pc_by_name`: the characters are
    /// mutably borrowed during the step, so the lookup runs on this snapshot.
    pub(super) pcs: Vec<(Vec<u8>, u32)>,
    /// Every online PC's snapshot, by PID, for `q.begin_other_pc_block`: the
    /// characters are mutably borrowed during the step, so a block clones from
    /// this snapshot.
    pub(super) online: std::collections::HashMap<u32, BlockedPc>,
    /// The shadow the `guild.*` calls read: the guilds as the world holds them at the
    /// step's start, so a call answers the same rows the world would judge.
    pub(super) guilds: super::guild::GuildShadow,
    /// The reader's party leader PID, read at the step's start (`None` with no
    /// party): what `party.get_leader_pid` answers, -1 with none.
    pub(super) party_leader: Option<u32>,
    /// The PIDs of the party members on the reader's map at the step's start, in
    /// ascending PID order: what `party.get_member_pids` pushes.
    pub(super) party_member_pids: Vec<u32>,
    /// The reader's party flags, read at the step's start: what `party.getf`
    /// answers, and what `party.setf` changes before the world applies them.
    pub(super) party_flags: std::collections::HashMap<Vec<u8>, i32>,
    /// Every `party.setf` call's name and value, in order, for the world's
    /// flags after the step (legacy writes in the call).
    pub(super) party_sets: Vec<(Vec<u8>, i32)>,
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
    /// The warp gates, read at the step's start, so `pc.warp` and `pc.can_warp`
    /// answer what the world would judge.
    pub(super) gates: WarpGates,
    /// The reader's conqueror level, read at the step's start.
    pub(super) conqueror_level: u8,
    /// The party flags, read at the step's start, so `party.is_party` and
    /// `party.is_leader` answer what the world would judge.
    pub(super) party: PartyFlags,
    /// The Channel the step runs on.
    pub(super) channel: u8,
    /// Every `pc.warp` call's (`x`, `y`, private index), in order, for the warp orders
    /// after the step (legacy's `IsHack` already answered in the script).
    pub(super) warps: Vec<(i32, i32, i32)>,
    /// Every `game.drop_item_with_ownership` call's vnum, count and seconds (`None`
    /// with no number), in order, for the ground after the step.
    pub(super) drops: Vec<(u32, i32, Option<i32>)>,
    /// The `warp_to_village` calls, for the empire starts after the step.
    pub(super) villages: u32,
    /// The `pc.set_conqueror` calls, for the conqueror change after the step.
    pub(super) conquerors: u32,
    /// Every `pc.open_sash` call's combination flag, in order, for the sash opens
    /// after the step (legacy sends `HEADER_GC_SASH` in the call).
    pub(super) sash_opens: Vec<bool>,
    /// The `pc.open_changelook` calls, for the changelook opens after the step.
    pub(super) changelooks: u32,
    /// Every `pc.mining` call's NPC VID, in order, for the digs after the step (legacy runs
    /// `CHARACTER::mining` in the call).
    pub(super) minings: Vec<u32>,
    /// The `horse.summon`, `horse.unsummon`, `horse.revive` and `pc.revive_horse` calls, in
    /// order, which the world runs after the step: the summoned horse is the world's.
    pub(super) horse_ops: Vec<super::horse_summon::HorseOp>,
    /// Whether `ds.give_qualification` ran this step: the grant itself already landed
    /// in the call (an `AddAffect` of kind 540, legacy's `DragonSoul_GiveQualification`
    /// at the call, `G/char_dragonsoul.cpp:75-85`), so a later `ds.is_qualified` in the
    /// same step answers true. The world needs no apply pass.
    pub(super) ds_gives: u32,
    /// The `ds.open_refine_window` calls, for the refine opens after the step (legacy
    /// sends `HEADER_GC_DRAGON_SOUL_REFINE` in the call, with the qualification).
    pub(super) ds_refines: u32,
    /// The `ds.open_attr_change_window` calls, for the attr-change opens after the step
    /// (legacy sends `HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN` in the call).
    pub(super) ds_attrs: u32,
    /// The blocked PCs' snapshots, oldest first: what a `q.begin_other_pc_block`
    /// runs for until its end. `None` is an unswitched frame, for a PID no online
    /// character holds: the stack still grew, but the script kept running for the
    /// old one. Empty outside a block, when the reader's own fields stay live.
    pub(super) blocked: Vec<Option<BlockedPc>>,
    /// Whether the last `begin_other_pc` switched: false when the named
    /// character is not online.
    pub(super) switched: bool,
    /// Every `GC_QUEST_CONFIRM` a script asked for another character, with its PID, in
    /// order: the world writes each after the step, as `ConfirmWithMsg` sends it.
    pub(super) confirm_records: Vec<(u32, Vec<u8>)>,
    /// Every confirm timeout a script armed, with its waiter's PID, in order: the world
    /// counts each down in Pulses after the step.
    pub(super) confirm_timeouts: Vec<(u32, i32)>,
}

/// A blocked PC's snapshot: the fields a script in its block reads, cloned at the
/// block's start from the world the step borrowed.
#[derive(Clone)]
pub(super) struct BlockedPc {
    /// The PID the block pushed: the reader's, or the outer blocked PC's.
    pub(super) pushed: u32,
    /// The blocked PC's PID.
    pub(super) pid: u32,
    /// Its VID.
    pub(super) vid: Vid,
    /// Its level.
    pub(super) level: u8,
    /// Its name.
    pub(super) name: Vec<u8>,
    /// Its points, which the hp/sp/exp readers answer.
    pub(super) points: Option<world::character::Points>,
    /// Its items, which `pc.count_item` reads and `pc.remove_item` takes from.
    pub(super) items: world::character::CharacterItems,
    /// Its usable cells, as the step's rules read them.
    pub(super) usable_cells: u16,
    /// Its removals, in order, for the world after the step.
    pub(super) takes: Vec<(u32, u32)>,
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

    /// The `horse_state %d %d %d` lines the horse queued (`SendHorseInfo`,
    /// `G/char_horse.cpp:369-376`), as this step's records.
    fn send_horse_lines(&mut self) {
        let Some(points) = self.points.as_deref_mut() else {
            return;
        };
        let lines = points.horse_mut().take_info_lines();
        let to = self.mover.recipient(self.locale);
        for line in lines {
            let record = match line {
                world::character::HorseLine::State([level, health, stamina]) => chat_packet(
                    to,
                    CHAT_TYPE_COMMAND,
                    b"horse_state %d %d %d",
                    &[
                        Arg::Int(i64::from(level)),
                        Arg::Int(i64::from(health)),
                        Arg::Int(i64::from(stamina)),
                    ],
                ),
                world::character::HorseLine::Hide => {
                    chat_packet(to, CHAT_TYPE_COMMAND, b"hide_horse_state", &[])
                }
            };
            self.records.push(record);
        }
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

/// `SkillCanUp` (`G/char_skill.cpp:907-1110`, `__7AND8TH_SKILLS__`): a ward skill (221-228)
/// rises only while every other ward skill is at 0, and a boost skill (236-243) only while every
/// other boost skill is (the `__WOLFMAN_CHARACTER__` term of 239 is not compiled). Any other vnum
/// never rises.
fn skill_can_up(vnum: u32, level: impl Fn(u32) -> u8) -> bool {
    let family = match vnum {
        221..=228 => 221..=228,
        236..=243 => 236..=243,
        _ => return false,
    };
    family
        .filter(|other| *other != vnum)
        .all(|other| level(other) == 0)
}

impl Session<'_> {
    /// The innermost switched block's PC, if any: `None` outside a block or when
    /// every open frame is unswitched, when the script still runs for the reader.
    fn blocked_pc(&self) -> Option<&BlockedPc> {
        self.blocked.iter().rev().find_map(|frame| frame.as_ref())
    }

    /// The points of the character the script runs for: the blocked PC's in a
    /// switched block, the reader's own otherwise.
    fn run_points(&self) -> Option<&world::character::Points> {
        if let Some(blocked) = self.blocked_pc() {
            return blocked.points.as_ref();
        }
        self.points.as_deref()
    }

    /// `mining::OreRefine` (`G/mining.cpp:421-464`) for the quest item of ID `ore`: the
    /// melt the `pc.ore_refine` / `pc.diamond_refine` bridge asked for. Legacy spends
    /// the 100 before it gates the gold, so a broke melt loses its ore; the Rewrite
    /// gates first, so the ore, the fee and the stone move or none does (recorded in
    /// STATUS).
    ///
    /// The order is otherwise legacy's: the owner's presence, its count of 100, the
    /// refined vnum's answer, the stone's presence, the gold gate against the computed
    /// fee, then the spend of 100, the fee, the stone, and last the draw (`number(1,
    /// 100) <= pct`), whose win lands the refined item through `give_item` (the stacks,
    /// a free cell or the ground). The client hears them in that order: the ore's
    /// cell, the gold, then the stone's cell (`G/mining.cpp:441-455`). A count that lands
    /// at 0 clears the cell and chains the slots like a used-up stack (`SetCount(0)`,
    /// `G/item.cpp:303-330`). A step whose rows no descriptor stores melts nothing.
    fn melt_ore(&mut self, ore: u32, cost: i32, pct: i32, stone: Option<u32>) -> bool {
        let Some(owner_id) = self
            .giver
            .as_ref()
            .filter(|giver| giver.stores)
            .map(|giver| giver.owner_id)
        else {
            warn!(
                ore,
                cost, "a quest melted ore where no descriptor stores it; none melted"
            );
            return false;
        };
        let held = self
            .giver
            .as_ref()
            .and_then(|giver| giver.items.item(ore).cloned());
        let Some(held) = held else {
            tracing::error!("wrong owner");
            return false;
        };
        if held.count < 100 {
            tracing::error!("not enough count");
            return false;
        }
        let Some(refined) = super::mining::refined_ore_of(held.vnum) else {
            return false;
        };
        let dust = stone.map(|stone| {
            self.giver
                .as_ref()
                .and_then(|giver| giver.items.item(stone).cloned())
        });
        if matches!(dust, Some(None)) {
            return false;
        }
        let fee = Self::compute_refine_fee(cost);
        if u64::try_from(fee.max(0)).is_ok_and(|fee| self.gold < fee) {
            return false;
        }
        let mut spent = Self::removed_moves();
        let mut dusted = Self::removed_moves();
        {
            let Some(giver) = self.giver.as_mut() else {
                return false;
            };
            if !Self::spend_melt_ore(giver.items, giver.slots, self.items, &mut spent, &held) {
                return false;
            }
            if let Some(dust) = dust.flatten() {
                if let Ok(pos) = giver.items.release(dust.id) {
                    let chains = self
                        .items
                        .get(dust.vnum)
                        .is_some_and(world::character::chains_when_used_up);
                    Self::chain_cleared(giver.slots, giver.items, &mut dusted, &dust, pos, chains);
                    dusted
                        .changes
                        .push(world::character::ItemChange::Destroyed { id: dust.id });
                }
            }
        }
        self.land_moves(owner_id, spent);
        self.pay_refine_fee(fee);
        self.land_moves(owner_id, dusted);
        if number(self.dice, 1, 100) > pct {
            return false;
        }
        self.give_item(refined, 1);
        true
    }

    /// `RealRefineRod` or `RealRefinePick` on the tool in inventory `cell`
    /// (`G/fishing.cpp:861-928`, `G/mining.cpp:234-303`): the cell's tool swaps for its
    /// refined vnum or its fallback, with the slots naming the cell deleted, and the code
    /// legacy answers. A step whose descriptor stores nothing refines nothing.
    fn refine_held_tool(&mut self, cell: u8, rod: bool) -> u8 {
        use world::character::{ToolOutcome, ToolRefine};
        let Some(owner_id) = self
            .giver
            .as_ref()
            .filter(|giver| giver.stores)
            .map(|giver| giver.owner_id)
        else {
            warn!(
                cell,
                rod, "a quest refined a tool where no descriptor stores it; none refined"
            );
            return ToolOutcome::Refused.code();
        };
        let line = if rod {
            ToolRefine::Rod
        } else {
            ToolRefine::PickAlone
        };
        let (done, outcome) = {
            let Some(giver) = self.giver.as_mut() else {
                return ToolOutcome::Refused.code();
            };
            let (mut done, outcome) = world::character::refine_tool(
                giver.items,
                giver.ids.as_deref_mut(),
                self.items,
                self.dice,
                cell,
                line,
            );
            world::character::sync_quickslots(&mut done, giver.slots, giver.items);
            (done, outcome)
        };
        self.land_moves(owner_id, done);
        outcome.code()
    }

    /// An empty removal, which a melt's spends fill.
    fn removed_moves() -> world::character::MoveDone {
        world::character::MoveDone {
            kind: world::character::MoveKind::Removed,
            records: Vec::new(),
            changes: Vec::new(),
        }
    }

    /// The records and rows of `done` for the reader, owned by `owner_id`, joined to the
    /// step's own.
    fn land_moves(&mut self, owner_id: u32, done: world::character::MoveDone) {
        let moved = crate::item_move::MovedItems::new(
            owner_id,
            self.reader.raw(),
            done,
            self.mover,
            self.items,
            self.locale,
        );
        self.records.extend(moved.records);
        self.item_rows.extend(moved.changes);
    }

    /// 100 of the `held` stack spent: a stack of exactly 100 clears its cell with the
    /// slots chained as `SetCount(0)` chains them (`G/item.cpp:303-330`), a larger one
    /// shrinks with its update.
    fn spend_melt_ore(
        items: &mut world::character::CharacterItems,
        slots: &mut world::character::Quickslots,
        protos: &super::ItemProtos,
        done: &mut world::character::MoveDone,
        held: &world::item::Item,
    ) -> bool {
        use world::character::{ItemChange, ItemRecord, MoveRecord};
        if held.count == 100 {
            let Ok(pos) = items.release(held.id) else {
                return false;
            };
            let chains = protos
                .get(held.vnum)
                .is_some_and(world::character::chains_when_used_up);
            Self::chain_cleared(slots, items, done, held, pos, chains);
            done.changes.push(ItemChange::Destroyed { id: held.id });
            return true;
        }
        let Ok(count) = items.set_count(held.id, u32::from(held.count - 100)) else {
            return false;
        };
        if let Some(stack) = items.item(held.id) {
            done.records
                .push(MoveRecord::Item(ItemRecord::Update(stack.gc_item_update())));
        }
        done.changes.push(ItemChange::Count { id: held.id, count });
        true
    }

    /// The slots of a cleared `held` cell synced in legacy's order (`SetCount(0)`):
    /// chained cells clear first and chain after, plain ones delete first and clear after.
    fn chain_cleared(
        slots: &mut world::character::Quickslots,
        items: &world::character::CharacterItems,
        done: &mut world::character::MoveDone,
        held: &world::item::Item,
        pos: protocol::item_pos::ItemPos,
        chains: bool,
    ) {
        use world::character::{ItemRecord, MoveRecord, QuickslotSync, SyncTo};
        use world::item::gc_item_clear;
        let clear = MoveRecord::Item(ItemRecord::Set(gc_item_clear(pos)));
        if chains {
            done.records.push(clear);
            done.records.push(MoveRecord::QuickslotSync(QuickslotSync {
                from: held.pos.cell,
                to: SyncTo::Chain(held.vnum),
            }));
        } else {
            done.records.push(MoveRecord::QuickslotSync(QuickslotSync {
                from: held.pos.cell,
                to: SyncTo::Delete,
            }));
            done.records.push(clear);
        }
        world::character::sync_quickslots(done, slots, items);
    }
    /// `ComputeRefineFee(cost, 1)` (`G/char.cpp:8181-8198`): the melt's fee. No refine NPC
    /// holds a guild, so the fee is the script's cost: the script itself prices the guild
    /// and empire rates (`GetOreRefineCost`) and gates the gold on them, as the smith's
    /// is (see `smith_refine`).
    fn compute_refine_fee(cost: i32) -> i32 {
        cost
    }

    /// `PayRefineFee` (`G/char.cpp:8200-8222`) with no guild: the whole fee leaves the
    /// gold, as `pc.change_money` moves it (below-0 and the cap refuse), and no deposit
    /// lands anywhere.
    fn pay_refine_fee(&mut self, fee: i32) {
        self.change_gold(-i64::from(fee.max(0)));
    }
}

impl Game for Session<'_> {
    fn level(&self) -> u8 {
        self.blocked_pc()
            .map_or(self.level, |blocked| blocked.level)
    }

    fn playtime(&self) -> i32 {
        self.points.as_deref().map_or(0, |points| {
            points.real_point(common::point_slot::POINT_PLAYTIME)
        })
    }

    /// `GetRealAlignment()` (`G/char_battle.cpp:3452-3455`), as this run has left it: the
    /// amounts `pc.change_alignment` records are applied to the world after the step, but a
    /// later read in the same script sees the value the earlier change made.
    fn real_alignment(&self) -> i32 {
        self.alignment
    }

    /// `UpdateAlignment(amount)` (`G/char_battle.cpp:3477-3565`): the script's change joins the
    /// step, which applies the amounts in order after it, so the clamp, the crossing update and
    /// the band affects run once per call as legacy runs them.
    fn change_alignment(&mut self, amount: i32) {
        self.alignment = super::alignment::clamped_alignment(self.alignment, amount);
        self.alignment_changes.push(amount);
    }

    /// `FindAffect(AFFECT_SKILL_NO_BOOK_DELAY)` (`G/questlua_pc.cpp:113-119`): whether the
    /// character holds the no-delay book affect.
    fn has_skill_book_no_delay(&self) -> bool {
        self.has_affect(super::pc_skill::AFFECT_SKILL_NO_BOOK_DELAY)
    }

    /// `RemoveAffect(AFFECT_SKILL_NO_BOOK_DELAY)` (`G/questlua_pc.cpp:106-111`): every row of
    /// the affect goes, its point and its remove packet with it.
    fn remove_skill_book_no_delay(&mut self) {
        self.take_affect(super::pc_skill::AFFECT_SKILL_NO_BOOK_DELAY);
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

    fn warp(&mut self, x: i32, y: i32, private: i32) -> bool {
        if self.gates.hack() {
            return false;
        }
        self.warps.push((x, y, private));
        true
    }

    fn drop_with_ownership(&mut self, vnum: u32, count: i32, secs: Option<i32>) {
        self.drops.push((vnum, count, secs));
    }

    fn conqueror_level(&self) -> u8 {
        self.conqueror_level
    }

    fn conqueror_exp(&self) -> u32 {
        self.run_points().map_or(0, |points| {
            u32::try_from(points.conqueror_exp()).unwrap_or(u32::MAX)
        })
    }

    fn warp_to_village(&mut self) {
        self.villages += 1;
    }

    fn can_warp(&self) -> bool {
        self.gates.able()
    }

    fn set_conqueror(&mut self) {
        self.conquerors += 1;
    }

    fn is_mount(&self) -> bool {
        self.run_points()
            .is_some_and(|points| points.mount_vnum() != 0)
    }

    fn is_party(&self) -> bool {
        self.party.party()
    }

    fn is_party_leader(&self) -> bool {
        self.party.leader()
    }

    fn channel_id(&self) -> u8 {
        self.channel
    }

    fn party_flag(&self, name: &[u8]) -> i32 {
        self.party_flags.get(name).copied().unwrap_or(0)
    }

    fn set_party_flag(&mut self, name: Vec<u8>, value: i32) {
        if self.party_leader.is_none() {
            return;
        }
        self.party_flags.insert(name.clone(), value);
        self.party_sets.push((name, value));
    }

    fn party_leader_pid(&self) -> i32 {
        // Legacy pushes the DWORD PID as a Lua number; PIDs never reach the
        // wrap.
        #[allow(clippy::cast_possible_wrap)]
        self.party_leader.map_or(-1, |pid| pid as i32)
    }

    fn party_member_pids(&self) -> Vec<u32> {
        self.party_member_pids.clone()
    }

    fn begin_other_pc(&mut self, pid: u32) {
        let pushed = self
            .blocked
            .iter()
            .rev()
            .find_map(|frame| frame.as_ref().map(|blocked| blocked.pid))
            .unwrap_or(self.pid);
        let mut frame = self.online.get(&pid).cloned();
        if let Some(snap) = frame.as_mut() {
            snap.pushed = pushed;
        }
        self.switched = frame.is_some();
        self.blocked.push(frame);
    }

    fn switched_pc(&self) -> bool {
        self.switched
    }

    fn end_other_pc(&mut self) {
        self.blocked.pop();
    }
    fn dungeon_lives(&self, _index: u32) -> bool {
        false
    }

    fn is_ds_qualified(&self) -> bool {
        self.has_affect(crate::loading_phase::AFFECT_DRAGON_SOUL_QUALIFIED)
    }

    fn give_ds_qualification(&mut self) {
        self.ds_gives += 1;
        let Some(event) = self.affects.as_deref_mut() else {
            return;
        };
        // `DragonSoul_GiveQualification` (`G/char_dragonsoul.cpp:75-85`): `AddAffect(540,
        // APPLY_NONE, 0, AFF_NONE, INFINITE_AFFECT_DURATION, 0, false, false)`
        // (`G/char_affect.cpp:668-760`), in the call: the infinite row appends (no
        // `bOverride`, no dedupe), the zero apply computes nothing, no flag sets and
        // neither speed moves (so no update), and the add packet joins the step's
        // records beside the script's own lines, in call order.
        let grant = super::affect::Affect {
            kind: crate::loading_phase::AFFECT_DRAGON_SOUL_QUALIFIED,
            apply_on: 0,
            value: 0,
            flag: 0,
            remaining: 60 * 365 * 24 * 60 * 60,
            sp_cost: 0,
        };
        event.affects.push(grant);
        self.records.push(super::affect::affect_add_frame(&grant));
    }

    fn open_ds_refine(&mut self) {
        self.ds_refines += 1;
    }

    fn open_ds_attr_change(&mut self) {
        self.ds_attrs += 1;
    }
    fn open_sash(&mut self, combination: bool) {
        self.sash_opens.push(combination);
    }

    fn open_changelook(&mut self) {
        self.changelooks += 1;
    }

    fn mining(&mut self, npc_vid: u32) {
        self.minings.push(npc_vid);
    }

    fn refine_tool(&mut self, cell: u8, rod: bool) -> u8 {
        self.refine_held_tool(cell, rod)
    }

    fn held_vnum(&self, id: u32) -> Option<u32> {
        let giver = self.giver.as_ref()?;
        giver.items.item(id).map(|item| item.vnum)
    }
    fn held_cell(&self, id: u32) -> Option<u16> {
        let giver = self.giver.as_ref()?;
        giver.items.cell_of(id).map(|pos| pos.cell)
    }

    fn inventory_at(&self, cell: u32) -> Option<u32> {
        let giver = self.giver.as_ref()?;
        let cell = u16::try_from(cell).ok()?;
        let at =
            protocol::item_pos::ItemPos::new(common::item_slots::EWindows::Inventory as u8, cell);
        giver.items.item_at(at).map(|item| item.id)
    }

    fn job(&self) -> u8 {
        self.card.0
    }

    fn set_socket(&mut self, id: u32, index: usize, value: i32) {
        if let Some(item) = self.laid.iter_mut().find(|item| item.id == id) {
            // On the ground the item has no owner, so `UpdatePacket` sends nothing.
            let _written = item.set_socket(index, value);
            return;
        }
        let Some(giver) = self.giver.as_mut().filter(|giver| giver.stores) else {
            warn!(
                id,
                index, "a quest set a socket where no descriptor stores it; none set"
            );
            return;
        };
        let done = world::character::set_item_socket(giver.items, id, (index, value));
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

    fn count_item(&self, vnum: u32) -> i32 {
        if let Some(blocked) = self.blocked_pc() {
            return world::character::count_specify_item(
                &blocked.items,
                blocked.usable_cells,
                vnum,
            );
        }
        self.giver.as_ref().map_or(0, |giver| {
            world::character::count_specify_item(giver.items, giver.rules.usable_cells, vnum)
        })
    }

    /// `RemoveSpecifyItem(vnum, count)` for the character the script runs for. In a block
    /// it takes from the blocked PC's snapshot and records the take, which runs against
    /// the world after the step. Otherwise, as before: the reader's records reach the
    /// reader's client at once and its rows are answered for the store. A step whose rows
    /// no descriptor stores takes nothing.
    fn remove_item(&mut self, vnum: u32, count: u32) {
        if let Some(index) = self.blocked.iter().rposition(Option::is_some) {
            let Some(frame) = self.blocked.get_mut(index).and_then(Option::as_mut) else {
                return;
            };
            let mut slots = world::character::Quickslots::default();
            let done = world::character::remove_specify_item(
                &mut frame.items,
                &mut slots,
                self.items,
                frame.usable_cells,
                (vnum, count),
            );
            if !done.records.is_empty() {
                frame.takes.push((vnum, count));
            }
            let _ = done;
            return;
        }
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

    fn horse(&self) -> quest::manager::HorseView {
        self.run_points()
            .map(world::character::Points::horse)
            .map_or_else(quest::manager::HorseView::default, |horse| {
                quest::manager::HorseView {
                    level: horse.level(),
                    health: horse.health(),
                    stamina: horse.stamina(),
                    max_health: horse.max_health(),
                    max_stamina: horse.max_stamina(),
                    riding: horse.riding(),
                    summoned: horse.summoned_vid().unwrap_or(0),
                }
            })
    }

    fn set_horse_level(&mut self, level: i32) {
        // `SetHorseLevel` (`G/char_horse.cpp:396-400`): the horse, then the horse skill at
        // its level; `ComputePoints` and `SkillLevelPacket` follow (`G/questlua_horse.cpp`).
        let now = self.global_time();
        let Some(points) = self.points.as_deref_mut() else {
            return;
        };
        points.horse_mut().set_level(level, now);
        let landed = points.horse().level();
        let records = points.compute_points();
        self.send_horse_lines();
        let skill = super::horse::SKILL_HORSE;
        self.slots.levels[usize::from(skill)] = landed;
        let stamp = self.slots.next_read[usize::from(skill)];
        self.climbs
            .push((skill, landed, i32::from_ne_bytes(stamp.to_ne_bytes())));
        self.records.extend(crate::loading_phase::point_changes(
            &records,
            self.reader.raw(),
        ));
        self.send_skill_levels();
    }

    fn feed_horse(&mut self) {
        let now = self.global_time();
        if let Some(points) = self.points.as_deref_mut() {
            let _fed = points.horse_mut().feed(now);
        }
        self.send_horse_lines();
    }

    fn summon_horse(&mut self, from_far: bool, vnum: u32) {
        self.horse_ops
            .push(super::horse_summon::HorseOp::Summon { from_far, vnum });
    }

    fn unsummon_horse(&mut self) {
        self.horse_ops.push(super::horse_summon::HorseOp::Unsummon);
    }

    fn revive_horse(&mut self) {
        self.horse_ops.push(super::horse_summon::HorseOp::Revive);
    }

    fn ride_horse(&mut self) {
        self.horse_ops.push(super::horse_summon::HorseOp::Ride);
    }

    fn unride_horse(&mut self) {
        self.horse_ops.push(super::horse_summon::HorseOp::Unride);
    }

    fn number(&mut self, from: i32, to: i32) -> i32 {
        number(self.dice, from, to)
    }
    fn ore_refine(&mut self, ore: u32, cost: i32, pct: i32, stone: Option<u32>) -> bool {
        self.melt_ore(ore, cost, pct, stone)
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

    fn set_skill_level(&mut self, vnum: u32, level: u8) {
        // `SetSkillLevel` (`G/char_skill.cpp:183-258`): a vnum past the 255 slots logs and sets
        // nothing, and so does a 7/8th skill `SkillCanUp` refuses a level above 0. The slot takes
        // the level up to 40; the master type is the ladder's (`master_of`). No point is
        // recomputed, as legacy recomputes none here. The `ENABLE_REWARD_SYSTEM` tail waits for
        // the reward missions (`sys.custom.reward`).
        let slot = u8::try_from(vnum)
            .ok()
            .filter(|slot| usize::from(*slot) < self.slots.levels.len());
        match slot {
            None => tracing::warn!(vnum, "vnum overflow"),
            Some(_)
                if (221..=243).contains(&vnum)
                    && level != 0
                    && !skill_can_up(vnum, |other| self.slot(other)) => {}
            Some(slot) => {
                let level = level.min(40);
                let index = usize::from(slot);
                self.slots.levels[index] = level;
                let stamp = self.slots.next_read[index];
                self.climbs
                    .push((slot, level, i32::from_ne_bytes(stamp.to_ne_bytes())));
            }
        }
        self.send_skill_levels();
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
    /// `SetXCostumeHidden` (`G/char.cpp:11382-11413`): the mask bit lands on the borrowed
    /// points now, the client hears `SetXCostumeHidden %d` in this step's records, and the
    /// hide queues for the step's end, where the parts recompute and the update goes out
    /// (`UpdatePacket`). The bridge already wrote the `costume_option.hide_*` quest flag.
    fn hide_costume(&mut self, part: u8, hide: bool) {
        if let Some(points) = self.points.as_deref_mut() {
            points.set_hidden(part, hide);
        }
        let format = super::whisper::hide_chat_format(part);
        let to = self.mover.recipient(self.locale);
        self.records.push(chat_packet(
            to,
            CHAT_TYPE_COMMAND,
            format,
            &[Arg::Int(i64::from(hide))],
        ));
        self.hides.push((part, hide));
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
    fn gold(&self) -> u64 {
        self.gold
    }
    /// `pc_change_money` (`G/questlua_pc.cpp:1102-1121`): the gold moves at the call, as
    /// `ChangeGold` runs it (`G/char.cpp:3823-3850`): a move that would drop the gold
    /// below 0 is logged and moves nothing, a later read in the same script sees the
    /// value the earlier move made, and the client hears `GC_CHARACTER_GOLD_CHANGE`
    /// (the amount 0 for a loss) where the call ran. The row itself moves after the
    /// step ([`GameState::apply_gold_changes`]), since the character is borrowed out.
    fn change_gold(&mut self, amount: i64) {
        let total = i128::from(self.gold) + i128::from(amount);
        if total < 0 {
            tracing::error!("QUEST wrong ChangeGold {amount} (now {})", self.gold);
            return;
        }
        if total >= i128::from(world::character::GOLD_MAX_MAX) {
            tracing::error!("[OVERFLOW_GOLD] OriGold {} AddedGold {amount}", self.gold);
            return;
        }
        if let Ok(gold) = u64::try_from(total) {
            self.gold = gold;
            self.gold_changes.push(amount);
            let mut record = Vec::new();
            protocol::gc_actors::GcCharacterGoldChange::new(
                self.reader.raw(),
                if amount <= 0 { 0 } else { amount },
                gold,
            )
            .encode_into(&mut record);
            self.records.push(record);
        }
    }

    fn empire(&self) -> u8 {
        self.empire
    }

    fn hp(&self) -> i32 {
        self.run_points().map_or(0, world::character::Points::hp)
    }

    fn max_hp(&self) -> i32 {
        self.run_points()
            .map_or(0, world::character::Points::max_hp)
    }

    fn sp(&self) -> i32 {
        self.run_points().map_or(0, world::character::Points::sp)
    }

    fn max_sp(&self) -> i32 {
        self.run_points()
            .map_or(0, world::character::Points::max_sp)
    }

    fn exp(&self) -> u32 {
        self.run_points()
            .map_or(0, |points| u32::try_from(points.exp()).unwrap_or(u32::MAX))
    }

    fn next_exp(&self) -> u32 {
        self.run_points().map_or(0, |points| {
            u32::try_from(common::levels::next_exp(points.level())).unwrap_or(u32::MAX)
        })
    }

    fn race(&self) -> u16 {
        u16::from(self.card.0)
    }

    fn char_name(&self) -> Vec<u8> {
        self.blocked_pc()
            .map_or_else(|| self.char_name.clone(), |blocked| blocked.name.clone())
    }

    fn char_vid(&self) -> u32 {
        self.blocked_pc()
            .map_or(self.reader.raw(), |blocked| blocked.vid.raw())
    }

    fn leadership(&self) -> u8 {
        self.leadership
    }

    fn position(&self) -> (i32, i32, i32, i32) {
        self.position
    }

    fn worn_vnum(&self, cell: u8) -> Option<u32> {
        let bound = u16::from(cell);
        if bound >= common::item_slots::WEAR_MAX_NUM {
            return None;
        }
        let giver = self.giver.as_ref()?;
        world::character::Equipment::of(giver.items, self.items)
            .wear(bound)
            .map(|worn| worn.item.vnum)
    }

    fn empire_of(&self, vid: u32) -> Option<u8> {
        self.npcs?
            .npcs
            .iter()
            .find(|npc| npc.vid == vid)
            .map(|npc| npc.empire)
    }
    fn npc_guild(&self, _vid: u32) -> u32 {
        0
    }
    fn gm_level(&self) -> u8 {
        // `m_pointsInstant.gm_level` (`G/char.cpp:6358`), which rode the admission: the
        // test-server silence (`:6355-6356`) is the audit's (V26), so the production arm
        // answers here, as the death penalty's does.
        self.points.as_deref().map_or(
            crate::command::GM_PLAYER,
            world::character::Points::gm_level,
        )
    }

    fn has_empty_inventory(&self, vnum: u32) -> bool {
        let Some(proto) = self.items.get(vnum) else {
            return false;
        };
        let size = u8::try_from(proto.size).unwrap_or(u8::MAX);
        let Some(giver) = self.giver.as_ref() else {
            return false;
        };
        giver
            .items
            .find_free_inventory_cell(giver.rules.usable_cells, size)
            .is_some()
    }

    fn held_socket(&self, id: u32, index: usize) -> Option<i32> {
        let giver = self.giver.as_ref()?;
        giver.items.item(id)?.socket(index)
    }

    fn held_value(&self, id: u32, index: usize) -> Option<i32> {
        let giver = self.giver.as_ref()?;
        let item = giver.items.item(id)?;
        self.items.get(item.vnum)?.values.get(index).copied()
    }

    fn held_shape(&self, id: u32) -> Option<(u16, u8, u8)> {
        let giver = self.giver.as_ref()?;
        let item = giver.items.item(id)?;
        let (kind, sub) = self
            .items
            .get(item.vnum)
            .map_or((0, 0), |proto| (proto.item_type, proto.sub_type));
        let kind = u8::try_from(kind).unwrap_or(0);
        let sub = u8::try_from(sub).unwrap_or(0);
        Some((item.count, kind, sub))
    }
    fn guild_id(&self) -> u32 {
        self.guild_id
    }

    fn is_guild_master(&self) -> bool {
        self.is_guild_master
    }

    fn test_server(&self) -> bool {
        self.test_server
    }

    fn find_pc_vid(&self, name: &[u8]) -> u32 {
        // `FindPC`'s lower-cased map (`G/char_manager.cpp:244-259`): names are ASCII and
        // unique regardless of case, so a case-insensitive match is the map's answer.
        self.pcs
            .iter()
            .find(|(shown, _)| shown.eq_ignore_ascii_case(name))
            .map_or(0, |(_, vid)| *vid)
    }

    fn pid_of(&self, vid: u32) -> Option<u32> {
        self.online
            .values()
            .find(|blocked| blocked.vid.raw() == vid)
            .map(|blocked| blocked.pid)
    }

    /// `ConfirmWithMsg` (`G/char.cpp:8076-8088`) at the `confirm` call: a confirm for
    /// the reader itself rides its own records, ahead of the `[CONFIRM_WAIT]` the
    /// yield sends; one for another character goes to its client after the step.
    fn confirm_to(&mut self, to_pid: u32, msg: &[u8], timeout: i32, request_pid: u32) {
        // `ConfirmWithMsg`'s `strlcpy` into 65 bytes keeps at most 64.
        let mut message = [0u8; protocol::gc_fields::GC_QUEST_MESSAGE_FIELD_SIZE];
        let end = msg.iter().position(|byte| *byte == 0).unwrap_or(msg.len());
        let kept = end.min(message.len() - 1);
        message[..kept].copy_from_slice(&msg[..kept]);
        let record = protocol::gc_fields::GcQuestConfirm::new(message, timeout, request_pid);
        let mut out = Vec::new();
        record.encode_into(&mut out);
        if self.caller == Some(to_pid) {
            self.records.push(out);
        } else {
            self.confirm_records.push((to_pid, out));
        }
    }

    fn arm_confirm_timeout(&mut self, waiter: u32, timeout: i32) {
        self.confirm_timeouts.push((waiter, timeout));
    }

    fn destroy_guild(&mut self) {
        // Both calls push nothing, so no shadow answers: the world judges after the step.
        self.guild_ops.push(super::guild::GuildQuestOp {
            name: b"destroy_guild".to_vec(),
            args: Vec::new(),
        });
    }

    fn remove_from_guild(&mut self) {
        self.guild_ops.push(super::guild::GuildQuestOp {
            name: b"remove_from_guild".to_vec(),
            args: Vec::new(),
        });
    }
    fn request_make_guild(&mut self) {
        self.records
            .push(vec![protocol::gc_small::HEADER_GC_REQUEST_MAKE_GUILD]);
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
                    QuestStep::SelectItem(selection) => {
                        manager.select_item(pid, *selection, game);
                    }
                    QuestStep::EventQuest(name) => {
                        manager.event_quest(pid, name, game);
                    }
                },
            );
        let Some(((), (mut records, climbs, ops, block_mode, _, gold_moves, _), items)) = answered
        else {
            return Ok(QuestStepOutcome::default());
        };
        let gold = (!gold_moves.is_empty()).then(|| {
            self.characters
                .find_by_vid(vid)
                .map_or(0, world::character::Character::gold)
        });
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
        Ok((records, climbs, stores, items, gold))
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
        let Some(((), (records, _, _, block_mode, _, _, _))) =
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
        let Some((taken, (mut records, climbs, ops, block_mode, _, gold_moves, _), items)) = self
            .run_quests_giving(place, (mover, level, pid), true, |manager, game| {
                manager.click(pid, target, u32::from(race), game)
            })
        else {
            return (false, QuestStepOutcome::default());
        };
        let gold = (!gold_moves.is_empty()).then(|| {
            self.characters
                .find_by_vid(vid)
                .map_or(0, world::character::Character::gold)
        });
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
        (taken, (records, climbs, stores, items, gold))
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
        // `GetArena() != NULL || IsObserverMode()` on vnums 50051-50053
        // (`G/char_item.cpp:2843-2850`): no arenas, so the observer arm alone tells
        // `[LS;1205]` and keeps the item.
        if matches!(vnum, 50051..=50053) && self.is_observing(vid) {
            if let Some(mover) = self.own_mover(vid) {
                self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;1205]", &[]);
            }
            return;
        }
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
        let Some(((), (records, climbs, ops, block_mode, _, _, _), items)) = self
            .run_quests_giving(place, (mover, level, pid), true, |manager, game| {
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

    /// Run the `logout` scripts of the character under `vid`, which is leaving the world
    /// (`CQuestManager::LogoutPC`, `G/questmanager.cpp:516-535`), and drop its quest
    /// targets (`CTargetManager::Logout`, `G/char.cpp:1815`). The scripts' records go
    /// nowhere: the client is leaving. Their flag writes land before the caller takes
    /// the flags, and the record stays for that taking.
    pub(super) fn end_quests(&mut self, vid: Vid) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        self.run_quest_event(vid, |manager, game| {
            manager.logout(pid, game);
        });
        self.quest_targets.logout(pid);
        self.confirm_timeouts.remove(&pid);
    }

    /// Arms the waiter's `confirm` timeout: its script hears `CONFIRM_TIMEOUT` after
    /// `timeout` seconds (`confirm_timeout_event`, `G/questlua.cpp:879-904`).
    /// A non-positive timeout fires on the step's next Pulse.
    fn arm_confirm_timeout(&mut self, waiter: u32, timeout: i32) {
        let wait = u64::try_from(timeout.max(0)).unwrap_or(u64::MAX);
        let due = self
            .last_pulse
            .saturating_add(wait.saturating_mul(u64::from(crate::save::PASSES_PER_SEC)));
        self.confirm_timeouts.insert(waiter, due);
    }

    /// `CInputMain::QuestConfirm` (`G/input_main.cpp:2236-2248`) for the client of the
    /// character under `vid`: `FindByPID(requestPID)` names the waiter, and
    /// `CQuestManager::Confirm` runs the waiter's script as its own (`GetPC` makes the
    /// waiter the current character), so the answer's records go to the waiter's client
    /// and the replier hears nothing. A waiter that is not online answers nothing.
    ///
    /// An answer the script heard disarms its timeout, so a later `confirm` of the same
    /// script never hears the earlier one's; a refused answer leaves the wait and its
    /// timeout standing.
    pub(super) fn answer_confirm(&mut self, vid: Vid, answer: u8, request_pid: u32) {
        let Ok(replier) = self
            .characters
            .find_by_vid(vid)
            .map(world::character::Character::player_id)
        else {
            return;
        };
        let Ok(waiter) = self
            .characters
            .find_by_pid(request_pid)
            .map(world::character::Character::vid)
        else {
            return;
        };
        let heard = self.run_quest_event(waiter, |manager, game| {
            manager.confirm(request_pid, answer, Some(replier), game)
        });
        if heard {
            self.confirm_timeouts.remove(&request_pid);
        }
    }

    /// `confirm_timeout_event` of every armed waiter due at `pulse`: the waiter's script
    /// hears `CONFIRM_TIMEOUT` (2) from no replier, and the answer's records go to its
    /// client. A waiter that left lost its arm; one whose script stopped waiting hears
    /// nothing but its arm goes.
    pub(super) fn run_confirm_timeouts(&mut self, pulse: u64) {
        let due: Vec<u32> = self
            .confirm_timeouts
            .iter()
            .filter_map(|(waiter, due)| (pulse >= *due).then_some(*waiter))
            .collect();
        for waiter in due {
            self.confirm_timeouts.remove(&waiter);
            let Ok(vid) = self
                .characters
                .find_by_pid(waiter)
                .map(world::character::Character::vid)
            else {
                continue;
            };
            self.run_quest_event(vid, |manager, game| {
                manager.confirm(waiter, quest::manager::CONFIRM_TIMEOUT, None, game);
            });
        }
    }

    /// Drop the quest record of `pid` after its save took the flags
    /// (`CQuestManager::DisconnectPC`, `G/questmanager.cpp:1068-1071`).
    pub(super) fn forget_quests(&mut self, pid: u32) {
        if let Some(quests) = self.quests.as_mut() {
            quests.manager.disconnect(pid);
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
        let landed = self.run_quests(place, mover, level, pid, |manager, game| {
            manager.levelup(pid, game);
        });
        let Some(((), (records, _, _, block_mode, _, _, _))) = landed else {
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
        let Some(((), (records, _, _, block_mode, _, _, _))) = landed else {
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
    ///
    /// The `pc.change_alignment` amounts the script recorded are applied after the step, in
    /// order, as legacy's `UpdateAlignment` runs at each call (`G/char_battle.cpp:3477-3565`).
    pub(super) fn run_quests_giving<R>(
        &mut self,
        place: GroundPlace,
        (mover, level, pid): (Mover, u8, u32),
        stores_items: bool,
        op: impl FnOnce(&mut Manager, &mut dyn Game) -> R,
    ) -> Option<(R, QuestRun, Vec<db::items::RowChange>)> {
        let (answer, mut run, rows) =
            self.run_quests_giving_raw(place, (mover, level, pid), stores_items, op)?;
        let vid = self
            .characters
            .find_by_pid(pid)
            .map_or(Vid::new(0), world::character::Character::vid);
        self.apply_alignment_changes(vid, &run.4);
        self.apply_gold_changes(vid, &run.5);
        if let Some(frame) = self.apply_hidden_changes(vid, &run.6) {
            use super::view::EntityKey;
            let me = EntityKey::Character(vid.raw());
            self.packet_around(me, &frame, Some(me));
            run.0.push(frame);
        }
        Some((answer, run, rows))
    }

    /// The step behind [`Self::run_quests_giving`], which then applies the alignment amounts.
    fn run_quests_giving_raw<R>(
        &mut self,
        place: GroundPlace,
        (mover, level, pid): (Mover, u8, u32),
        stores_items: bool,
        op: impl FnOnce(&mut Manager, &mut dyn Game) -> R,
    ) -> Option<(R, QuestRun, Vec<db::items::RowChange>)> {
        let (answer, run, item_rows, queued, reader, takes) = {
            let mut armed = self.arm_quest_session(place, (mover, level, pid), stores_items)?;
            let answer = op(armed.manager, &mut armed.session);
            let queued = WorldMoves::take(&mut armed.session);
            let Session {
                records,
                climbs,
                guild_ops,
                block_mode,
                item_rows,
                alignment_changes,
                gold_changes,
                hides,
                blocked,
                ..
            } = armed.session;
            let mut takes: Vec<(u32, u32, u32)> = Vec::new();
            for snap in blocked.iter().flatten() {
                for (vnum, count) in &snap.takes {
                    takes.push((snap.pid, *vnum, *count));
                }
            }
            let run = (
                records,
                climbs,
                guild_ops,
                block_mode,
                alignment_changes,
                gold_changes,
                hides,
            );
            (answer, run, item_rows, queued, armed.reader, takes)
        };
        self.apply_world_moves(reader, place, mover, pid, queued);
        let mut blocked_rows = Vec::new();
        for (take_pid, vnum, count) in takes {
            self.apply_blocked_take(&mut blocked_rows, take_pid, (vnum, count));
        }
        let mut item_rows = item_rows;
        item_rows.extend(blocked_rows);
        Some((answer, run, item_rows))
    }

    /// A [`Session`] borrowed from the world, with its manager and reader: the split-out
    /// head of [`Self::run_quests_giving_raw`] arms one so neither function trips the line
    /// lint.
    #[allow(clippy::type_complexity)]
    fn arm_quest_session(
        &mut self,
        place: GroundPlace,
        (mover, level, pid): (Mover, u8, u32),
        stores_items: bool,
    ) -> Option<ArmedSession<'_>> {
        let reader = self
            .characters
            .find_by_pid(pid)
            .map_or(Vid::new(0), world::character::Character::vid);
        let hacked = self.is_hack(reader, mover).is_some();
        let GameState {
            characters,
            npcs,
            quests,
            protos: items,
            locale,
            dice,
            bodies,
            war_maps,
            maps,
            pc_skills,
            affect_events,
            pc_skill_tables,
            guilds,
            parties,
            test_server,
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
        let mut context = reader_context(
            characters, bodies, war_maps, maps, pc_skills, guilds, parties, reader,
        );
        if hacked {
            context.gates.set_hack(true);
            context.gates.set_able(false);
        }
        let Quests {
            manager,
            protos,
            names,
            item_names,
            goto_info: _,
        } = &mut *quests;
        let session = build_session(session_inputs(
            place,
            mover,
            level,
            pid,
            reader,
            context,
            npcs,
            protos,
            names,
            item_names,
            items,
            locale,
            pc_skill_tables,
            dice,
            bodies,
            characters,
            pc_skills,
            affect_events,
            *test_server,
            *last_pulse,
            guilds,
            item_count_limit,
            item_ids,
            gauss,
            item_attrs,
            item_masks,
            quest_targets,
            stores_items,
        ));
        Some(ArmedSession {
            manager,
            session,
            reader,
        })
    }
    /// Apply the `pc.change_alignment` amounts a step recorded, in order, as legacy's
    /// `UpdateAlignment` runs at each call (`G/char_battle.cpp:3477-3565`): each amount is
    /// clamped on the value the one before it left, and each crossing sends its update and
    /// moves the band affects before the next amount runs.
    fn apply_alignment_changes(&mut self, vid: Vid, amounts: &[i32]) {
        for amount in amounts {
            self.update_alignment(vid, *amount);
        }
    }
    /// Apply the `pc.change_money` amounts a step recorded, in order, as legacy's
    /// `ChangeGold` moves the row at each call (`G/char.cpp:3823-3850`): the session
    /// already refused the moves that would drop below 0 or reach `GOLD_MAX_MAX`, so
    /// each applies; the guards re-check on the live row. The client's
    /// `GC_CHARACTER_GOLD_CHANGE` rides the step's own records, where the fee lands
    /// before the melt's page.
    fn apply_gold_changes(&mut self, vid: Vid, amounts: &[i64]) {
        for amount in amounts {
            let Ok(character) = self.characters.find_by_vid(vid) else {
                continue;
            };
            let total = i128::from(character.gold()) + i128::from(*amount);
            let Ok(gold) = u64::try_from(total) else {
                continue;
            };
            if total >= i128::from(world::character::GOLD_MAX_MAX) {
                continue;
            }
            if let Ok(character) = self.characters.find_by_vid_mut(vid) {
                character.set_gold(gold);
            }
        }
    }
    /// Apply the `pc.hide_costume` hides a step recorded. Each `pc_hide_costume` call sends
    /// `UpdatePacket` unconditionally (`G/questlua_pc.cpp:3598`), so a step that hid
    /// anything answers one frame, even when no bit moved: the costume script hides all
    /// five parts at every login, and its update follows its lines. The frame batches
    /// legacy's one-per-call updates (the end parts are the same either way); the caller
    /// queues it behind the step's own records for the character and sends it to everyone
    /// that sees it, since a direct broadcast would jump ahead of the chats the step
    /// queued (the login scripts hide before `game_option` speaks).
    fn apply_hidden_changes(&mut self, vid: Vid, hides: &[(u8, bool)]) -> Option<Vec<u8>> {
        if hides.is_empty() {
            return None;
        }
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return None;
        };
        let (items, points) = character.items_and_points_mut();
        let points = points?;
        for (part, hide) in hides {
            points.set_hidden(*part, *hide);
        }
        let equipment = world::character::Equipment::of(items, &self.protos);
        let parts = points.hidden_parts(&equipment);
        points.set_parts(parts);
        let flags = self.affect_flags_of(vid);
        self.update_frame(vid, flags)
    }

    /// One `pc.remove_item` take a block recorded for `take_pid`: `RemoveSpecifyItem`
    /// runs against the world after the step, its records go to its owner's client,
    /// and its rows join `rows` for the descriptor to store. Quiet when the
    /// character is gone or its descriptor stores nothing.
    fn apply_blocked_take(
        &mut self,
        rows: &mut Vec<db::items::RowChange>,
        take_pid: u32,
        (vnum, count): (u32, u32),
    ) {
        let Ok(vid) = self
            .characters
            .find_by_pid(take_pid)
            .map(world::character::Character::vid)
        else {
            return;
        };
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let usable = common::item_slots::usable_inventory_cells(character.inven_point());
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        let done = world::character::remove_specify_item(
            items,
            slots,
            &self.protos,
            usable,
            (vnum, count),
        );
        let taken = crate::item_move::MovedItems::new(
            take_pid,
            vid.raw(),
            done,
            mover,
            &self.protos,
            &self.locale,
        );
        for record in taken.records {
            let _sent = self.write_to_client(vid, record);
        }
        rows.extend(taken.changes);
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

    /// One `game.drop_item_with_ownership` call: `CreateItem(vnum, count)` lies at the
    /// reader's feet (`number(-200, 200)` each way, `G/questlua_game.cpp:205-206`),
    /// owned by the reader (`SetOwnership(ch[, secs])`, clamped to 30 at 10 and below).
    /// Nothing happens with no proto, as legacy's `NULL` return does.
    fn lay_drop_owned(
        &mut self,
        reader: Vid,
        place: GroundPlace,
        vnum: u32,
        count: i32,
        secs: Option<i32>,
        _mover: Mover,
    ) {
        let Some(item) = self.make_drop_hit(vnum, count, -1) else {
            return;
        };
        let z = self.spot_of(reader.raw()).map_or(0, |spot| spot.z);
        let dx = number(&mut self.dice, -200, 200);
        let dy = number(&mut self.dice, -200, 200);
        let Some(ground) = self.lay_drop_hit(
            place.channel,
            place.map,
            (place.x + dx, place.y + dy, z),
            item,
        ) else {
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
        let secs = secs.map_or(30, |secs| if secs <= 10 { 30 } else { secs });
        self.set_ground_ownership(ground, Some((pid, name)), secs);
    }

    /// One `pc.warp` call the step ran past `IsHack`: order the reader's client the
    /// warp, on the Channel that hosts the target map when one does. The descriptor
    /// runs `WarpSet` (the private index, the sort gate, the save, `GC_WARP`); a target
    /// no Channel hosts orders nowhere, as legacy's failed `CMapLocation::Get` warps
    /// nowhere while the call still answers true.
    fn order_quest_warp(&mut self, reader: Vid, x: i32, y: i32, private: i32, _mover: Mover) {
        let target = self.recall.atlas().and_then(|atlas| atlas.index_at(x, y));
        let channel = target.and_then(|index| {
            self.maps
                .keys()
                .find(|(_, map)| *map == index)
                .map(|(channel, _)| *channel)
        });
        if target.is_some() && channel.is_none() {
            warn!(
                vid = reader.raw(),
                x, y, "a quest warp found no channel hosting its map; none ordered"
            );
            return;
        }
        let raw = reader.raw();
        let Some(clients) = self.clients.as_ref() else {
            warn!(vid = raw, "a quest warp has no client registry to reach");
            return;
        };
        let Some((channel_now, map)) = self.place_of(raw) else {
            warn!(vid = raw, "a quest warp found no place the reader stands");
            return;
        };
        let ordered = clients
            .members_on_map(channel_now, map)
            .into_iter()
            .find(|(_, entry)| entry.vid == raw)
            .is_some_and(|(id, _)| {
                clients.order(
                    channel_now,
                    id,
                    ClientOrder::Warp {
                        x,
                        y,
                        private_map_index: private,
                        channel,
                        exit: None,
                        sorted_until: self.sort_lock(reader),
                    },
                )
            });
        if !ordered {
            warn!(vid = raw, x, y, "a quest warp found no client to order");
        }
    }

    /// One `warp_to_village` call: `WarpSet` to the reader's empire start
    /// (`g_start_position`, `G/questlua_global.cpp:1066-1067`), through the quest
    /// warp's order.
    fn order_village_warp(&mut self, reader: Vid, mover: Mover) {
        let empire = self.bodies.get(&reader).map_or(0, |body| body.card.empire);
        let at = crate::channel_login::EMPIRE_START
            .get(usize::from(empire))
            .copied()
            .unwrap_or((0, 0));
        self.order_quest_warp(reader, at.0, at.1, 0, mover);
    }

    /// One `pc.set_conqueror` call: `SetConqueror(true)` (`G/char.cpp:11659-11700`).
    /// A conqueror already, or below the level-120 filled marble, chats its line and
    /// changes nothing; else the level becomes 1, the step, point and sungma reset,
    /// the exp clears, and the points recompute and go out with the trailing
    /// conqueror-experience record.
    fn make_conqueror(&mut self, reader: Vid, mover: Mover) {
        let max_level = self.exp_limits.max_level;
        let (level, exp, conqueror) = {
            let Ok(character) = self.characters.find_by_vid(reader) else {
                return;
            };
            let Some(points) = character.points() else {
                return;
            };
            (points.level(), points.exp(), points.conqueror_level())
        };
        if conqueror > 0 {
            let line = chat_packet(
                mover.recipient(&self.locale),
                CHAT_TYPE_INFO,
                b"You have already reached the Champion Level.",
                &[],
            );
            let _sent = self.write_to_client(reader, line);
            return;
        }
        let need = common::levels::next_exp(level) / 4;
        if i32::from(level) < max_level || exp < need {
            let line = chat_packet(
                mover.recipient(&self.locale),
                CHAT_TYPE_INFO,
                b"You must fill up at least 1 EXP Marble at level 120 to reach Champion Level.",
                &[],
            );
            let _sent = self.write_to_client(reader, line);
            return;
        }
        // `MINMAX(0, 1, gPlayerMaxLevel)`: the 1 survives any plausible level cap.
        self.reset_pc_conqueror(reader, 1, 0);
    }

    /// One `pc.open_sash` call: `OpenSash(combination)` (`G/char.cpp:9888-9934`).
    /// The already-open window chats `SASH_OPEN`; opening one window while the
    /// other stands chats `SASH_CLOSE` (for combination) or `COMBINE_CLOSE` (for
    /// absorption). `LC_TEXT` of an unknown key is the key itself, so the raw keys
    /// go out. Else the window stands and the client hears the open record.
    fn open_sash(&mut self, reader: Vid, mover: Mover, combination: bool) {
        let line: Option<&[u8]> = match self.sash_windows.get(&reader).copied() {
            Some(open) if open == combination => Some(b"SASH_OPEN"),
            Some(_) if combination => Some(b"SASH_CLOSE"),
            Some(_) => Some(b"COMBINE_CLOSE"),
            None => None,
        };
        if let Some(line) = line {
            let line = chat_packet(mover.recipient(&self.locale), CHAT_TYPE_INFO, line, &[]);
            let _sent = self.write_to_client(reader, line);
            return;
        }
        self.sash_windows.insert(reader, combination);
        let open = protocol::gc_sash::GcSash::open(combination).encode();
        let _sent = self.write_to_client(reader, open);
    }

    /// One `pc.open_changelook` call: `ChangeLookWindow(true, false)`
    /// (`G/char.cpp:10504-10535`). The already-open window chats the opened line.
    /// `LC_TEXT` of an unknown key is the key itself, so the raw line goes out.
    /// Else the window stands and the client hears the open record, with the
    /// 50,000,000 cost.
    fn open_changelook(&mut self, reader: Vid, mover: Mover) {
        if self.changelook_windows.contains(&reader) {
            let line = b"[Transmutation] The window is already opened";
            let line = chat_packet(mover.recipient(&self.locale), CHAT_TYPE_INFO, line, &[]);
            let _sent = self.write_to_client(reader, line);
            return;
        }
        self.changelook_windows.insert(reader);
        let open = protocol::gc_changelook::GcChangeLook::open().encode();
        let _sent = self.write_to_client(reader, open);
    }

    /// A step's [`WorldMoves`] in the queues' old application order: the laid and
    /// dropped items, the village and conqueror counts, the sash and changelook
    /// opens, the `ds.*` calls, the party flags, then the warps.
    fn apply_world_moves(
        &mut self,
        reader: Vid,
        place: GroundPlace,
        mover: Mover,
        pid: u32,
        queued: WorldMoves,
    ) {
        for item in queued.laid {
            self.lay_given(reader, place, item);
        }
        for (vnum, count, secs) in queued.drops {
            self.lay_drop_owned(reader, place, vnum, count, secs, mover);
        }
        for _ in 0..queued.villages {
            self.order_village_warp(reader, mover);
        }
        for _ in 0..queued.conquerors {
            self.make_conqueror(reader, mover);
        }
        for combination in queued.sash_opens {
            self.open_sash(reader, mover, combination);
        }
        for _ in 0..queued.changelooks {
            self.open_changelook(reader, mover);
        }
        for npc in queued.minings {
            self.mining(reader, npc, mover);
        }
        self.apply_horse_ops(reader, queued.horse_ops);
        self.apply_ds_calls(reader, queued.ds_refines, queued.ds_attrs);
        for (flag, value) in queued.party_sets {
            self.parties.set_flag(pid, flag, value);
        }
        for (to_pid, record) in queued.confirm_records {
            if let Ok(target) = self.characters.find_by_pid(to_pid) {
                let _sent = self.write_to_client(target.vid(), record);
            }
        }
        for (waiter, timeout) in queued.confirm_timeouts {
            self.arm_confirm_timeout(waiter, timeout);
        }
        for (x, y, private) in queued.warps {
            self.order_quest_warp(reader, x, y, private, mover);
        }
    }

    /// The `ds.*` window calls a step queued: the refine opens and the attr-change
    /// opens, in call order within each kind. The grants need no pass: each landed in
    /// its call, beside the script's own records.
    fn apply_ds_calls(&mut self, reader: Vid, refines: u32, attrs: u32) {
        for _ in 0..refines {
            self.open_ds_refine(reader);
        }
        for _ in 0..attrs {
            self.open_ds_attr_change(reader);
        }
    }

    /// One `ds.open_refine_window` call: with the qualification the reader's client
    /// hears `HEADER_GC_DRAGON_SOUL_REFINE` with `DS_SUB_HEADER_OPEN` (0) and a
    /// default `TItemPos` (`G/char_dragonsoul.cpp:138-158`).
    fn open_ds_refine(&mut self, reader: Vid) {
        let qualified = self.affect_events.get(&reader).is_some_and(|event| {
            event
                .affects
                .iter()
                .any(|affect| affect.kind == crate::loading_phase::AFFECT_DRAGON_SOUL_QUALIFIED)
        });
        if !qualified {
            return;
        }
        let mut open = Vec::new();
        protocol::gc_vid::GcDragonSoulRefine {
            sub_type: 0,
            // `SItemPos()` (`common/length.h:959-964`): `INVENTORY` (1), `WORD_MAX`.
            pos: protocol::item_pos::ItemPos::new(1, 0xffff),
        }
        .encode_into(&mut open);
        let _sent = self.write_to_client(reader, open);
    }

    /// One `ds.open_attr_change_window` call: the reader's client hears the
    /// one-byte `HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN` (`G/char.cpp:11235-11242`).
    fn open_ds_attr_change(&mut self, reader: Vid) {
        let mut open = Vec::new();
        protocol::gc_small::GcHeaderOnly::new(
            protocol::gc_inventory::HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN.value(),
        )
        .encode_into(&mut open);
        let _sent = self.write_to_client(reader, open);
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
    gold: u64,
    empire: u8,
    char_name: Vec<u8>,
    position: (i32, i32, i32, i32),
    leadership: u8,
    guild_id: u32,
    is_guild_master: bool,
    gates: WarpGates,
    conqueror_level: u8,
    party: PartyFlags,
    party_leader: Option<u32>,
    party_flags: std::collections::HashMap<Vec<u8>, i32>,
    party_member_pids: Vec<u32>,
}

/// Every borrow [`GameState::arm_quest_session`] threads into its [`Session`], as
/// one struct so the builder stays under the argument lint.
struct SessionInputs<'s> {
    place: GroundPlace,
    mover: Mover,
    level: u8,
    pid: u32,
    reader: Vid,
    context: ReaderContext,
    npcs: &'s BTreeMap<(u8, i32), Arc<MapNpcs>>,
    protos: &'s MobProtos,
    names: &'s MobNamesByLanguage,
    item_names: &'s ItemNamesByLanguage,
    items: &'s ItemProtos,
    locale: &'s LocaleStrings,
    tables: &'s gamedata::pc_skill_data::PcSkills,
    dice: &'s mut Pcg32,
    bodies: &'s std::collections::HashMap<Vid, super::motion::Body>,
    characters: &'s mut world::character::CharacterManager,
    skills: &'s mut std::collections::HashMap<u32, super::pc_skill::PcSkillState>,
    affects: &'s mut std::collections::HashMap<Vid, super::affect::AffectEvent>,
    test_server: bool,
    pulse: u64,
    guilds: &'s super::guild::Guilds,
    count_limit: &'s u16,
    ids: &'s mut Option<world::item::ItemIds>,
    gauss: &'s mut world::character::Gauss,
    attrs: &'s [gamedata::records::ItemAttrRecord],
    masks: &'s gamedata::item_mask::ItemMasks,
    targets: &'s mut QuestTargets,
    stores: bool,
}
/// The reader's job and skill group, level and player ID, and `GetMapIndex()` and `GetWarMap()`
/// (`G/input_login.cpp:780`): the map the character stands on, and the war map it stands on,
/// which is the map the world holds a war for.
/// The reader's card and caller, its gold, empire, name, position, leadership, and the maps
/// it reads them from: `GetMapIndex()` and `GetWarMap()` (`G/input_login.cpp:780`), the
/// map it stands on and the war map it stands on.
#[allow(clippy::too_many_arguments)]
fn reader_context(
    characters: &world::character::CharacterManager,
    bodies: &std::collections::HashMap<Vid, super::motion::Body>,
    war_maps: &BTreeMap<i32, super::war_map::WarMap>,
    maps: &BTreeMap<(u8, i32), super::view::MapIndex>,
    pc_skills: &std::collections::HashMap<u32, super::pc_skill::PcSkillState>,
    guilds: &super::guild::Guilds,
    parties: &super::party::Parties,
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
    let body = bodies.get(&reader);
    let map_index = body.map_or(0, |body| body.map);
    let war_map = war_maps.get(&map_index).map_or(0, |war| war.map_index);
    let gold = character.map_or(0, world::character::Character::gold);
    let empire = body.map_or(0, |body| body.card.empire);
    let char_name = character.map_or(Vec::new(), |character| character.name().as_bytes().to_vec());
    let position = body.map_or((0, 0, 0, 0), |body| {
        let key = super::view::EntityKey::Character(reader.raw());
        let (x, y) = maps
            .get(&(body.channel, body.map))
            .and_then(|index| index.spot(key))
            .map_or((0, 0), |spot| (spot.x, spot.y));
        let (base_x, base_y) = maps
            .get(&(body.channel, body.map))
            .map_or((0, 0), super::view::MapIndex::base);
        (x / 100, y / 100, (x - base_x) / 100, (y - base_y) / 100)
    });
    let leadership = pc_skills.get(&reader.raw()).map_or(0, |state| {
        state.levels[super::pc_skill::SKILL_LEADERSHIP as usize]
    });
    let (guild_id, is_guild_master) =
        caller.map_or((0, false), |pid| GameState::guild_pair(guilds, pid));
    let conqueror_level = character
        .and_then(|character| character.points())
        .map_or(0, world::character::Points::conqueror_level);
    let party_leader = caller.and_then(|pid| parties.leader_of(pid));
    let (is_party, is_party_leader) = match (caller, party_leader) {
        (Some(pid), Some(leader)) => (true, leader == pid),
        _ => (false, false),
    };
    let party_flags = party_leader
        .and_then(|leader| parties.flags_of(leader))
        .unwrap_or_default();
    let party_member_pids = party_leader
        .and_then(|leader| parties.on_map_members(leader, map_index, bodies))
        .unwrap_or_default();
    ReaderContext {
        card,
        pc_level,
        caller,
        map_index,
        war_map,
        gold,
        empire,
        char_name,
        position,
        leadership,
        guild_id,
        is_guild_master,
        gates: WarpGates::armed(),
        conqueror_level,
        party: PartyFlags::new(is_party, is_party_leader),
        party_leader,
        party_flags,
        party_member_pids,
    }
}

/// A blocked PC's snapshot of the character online under `pid`: its VID, level,
/// name, points and items, behind the PID the block pushed. `None` with no such
/// online character (`GetPC` returned `NULL`, `G/questmanager.cpp:1087-1099`).
fn block_snapshot(
    characters: &world::character::CharacterManager,
    pushed: u32,
    pid: u32,
) -> Option<BlockedPc> {
    let character = characters.find_by_pid(pid).ok()?;
    Some(BlockedPc {
        pushed,
        pid,
        vid: character.vid(),
        level: character
            .points()
            .map_or(0, world::character::Points::level),
        name: character.name().as_bytes().to_vec(),
        points: character.points().cloned(),
        usable_cells: common::item_slots::usable_inventory_cells(character.inven_point()),
        items: character.items().clone(),
        takes: Vec::new(),
    })
}

/// Every online PC's shown name and VID, for a step's `find_pc_by_name`.
fn quest_step_pcs(characters: &world::character::CharacterManager) -> Vec<(Vec<u8>, u32)> {
    characters
        .player_names()
        .into_iter()
        .map(|(name, vid)| (name.as_bytes().to_vec(), vid.raw()))
        .collect()
}

/// Every online PC's snapshot, by PID, for a step's `q.begin_other_pc_block`.
fn quest_step_online(
    characters: &world::character::CharacterManager,
) -> std::collections::HashMap<u32, BlockedPc> {
    characters
        .player_names()
        .into_iter()
        .filter_map(|(_, vid)| characters.find_by_vid(vid).ok())
        .map(world::character::Character::player_id)
        .filter_map(|pid| block_snapshot(characters, 0, pid).map(|snap| (pid, snap)))
        .collect()
}

/// The NPCs of the map at `place`.
fn map_npcs(npcs: &BTreeMap<(u8, i32), Arc<MapNpcs>>, place: GroundPlace) -> Option<&MapNpcs> {
    npcs.get(&(place.channel, place.map)).map(Arc::as_ref)
}

/// Arm the session's item giver for `character`: the stack limit, the cells its unlock
/// stat opens, and the quest-step rules. Answers the borrowed points beside it, or
/// nothing when the reader is not in the world.
#[allow(clippy::too_many_arguments)]
fn arm_item_giver<'s>(
    character: Option<&'s mut world::character::Character>,
    item_count_limit: &'s u16,
    item_ids: &'s mut Option<world::item::ItemIds>,
    gauss: &'s mut world::character::Gauss,
    item_attrs: &'s [gamedata::records::ItemAttrRecord],
    item_masks: &'s gamedata::item_mask::ItemMasks,
    stores_items: bool,
    pid: u32,
) -> (Option<&'s mut world::character::Points>, Option<Giver<'s>>) {
    let Some(character) = character else {
        return (None, None);
    };
    let usable_cells = common::item_slots::usable_inventory_cells(character.inven_point());
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

/// Pack the arming borrows into one [`SessionInputs`], so [`GameState::arm_quest_session`]
/// stays under the line lint.
#[allow(clippy::too_many_arguments)]
fn session_inputs<'s>(
    place: GroundPlace,
    mover: Mover,
    level: u8,
    pid: u32,
    reader: Vid,
    context: ReaderContext,
    npcs: &'s BTreeMap<(u8, i32), Arc<MapNpcs>>,
    protos: &'s MobProtos,
    names: &'s MobNamesByLanguage,
    item_names: &'s ItemNamesByLanguage,
    items: &'s ItemProtos,
    locale: &'s LocaleStrings,
    tables: &'s gamedata::pc_skill_data::PcSkills,
    dice: &'s mut Pcg32,
    bodies: &'s std::collections::HashMap<Vid, super::motion::Body>,
    characters: &'s mut world::character::CharacterManager,
    skills: &'s mut std::collections::HashMap<u32, super::pc_skill::PcSkillState>,
    affects: &'s mut std::collections::HashMap<Vid, super::affect::AffectEvent>,
    test_server: bool,
    pulse: u64,
    guilds: &'s super::guild::Guilds,
    count_limit: &'s u16,
    ids: &'s mut Option<world::item::ItemIds>,
    gauss: &'s mut world::character::Gauss,
    attrs: &'s [gamedata::records::ItemAttrRecord],
    masks: &'s gamedata::item_mask::ItemMasks,
    targets: &'s mut QuestTargets,
    stores: bool,
) -> SessionInputs<'s> {
    SessionInputs {
        place,
        mover,
        level,
        pid,
        reader,
        context,
        npcs,
        protos,
        names,
        item_names,
        items,
        locale,
        tables,
        dice,
        bodies,
        characters,
        skills,
        affects,
        test_server,
        pulse,
        guilds,
        count_limit,
        ids,
        gauss,
        attrs,
        masks,
        targets,
        stores,
    }
}

/// A [`Session`] from its packed borrows: the maps, the reader's snapshot, and the
/// emptied per-step vectors.
fn build_session(inputs: SessionInputs<'_>) -> Session<'_> {
    let SessionInputs {
        place,
        mover,
        level,
        pid,
        reader,
        context,
        npcs,
        protos,
        names,
        item_names,
        items,
        locale,
        tables,
        dice,
        bodies,
        characters,
        skills,
        affects,
        test_server,
        pulse,
        guilds,
        count_limit,
        ids,
        gauss,
        attrs,
        masks,
        targets,
        stores,
    } = inputs;
    session_head(
        place,
        mover,
        level,
        pid,
        reader,
        context,
        npcs,
        protos,
        names,
        item_names,
        items,
        locale,
        tables,
        dice,
        bodies,
        characters,
        skills,
        affects,
        test_server,
        pulse,
        guilds,
        count_limit,
        ids,
        gauss,
        attrs,
        masks,
        targets,
        stores,
    )
}

/// The head half of [`build_session`]: the maps, the reader's snapshot, and the armed
/// item giver, before the literal fills the per-step vectors.
#[allow(clippy::too_many_arguments)]
fn session_head<'s>(
    place: GroundPlace,
    mover: Mover,
    level: u8,
    pid: u32,
    reader: Vid,
    context: ReaderContext,
    npcs: &'s BTreeMap<(u8, i32), Arc<MapNpcs>>,
    protos: &'s MobProtos,
    names: &'s MobNamesByLanguage,
    item_names: &'s ItemNamesByLanguage,
    items: &'s ItemProtos,
    locale: &'s LocaleStrings,
    tables: &'s gamedata::pc_skill_data::PcSkills,
    dice: &'s mut Pcg32,
    bodies: &'s std::collections::HashMap<Vid, super::motion::Body>,
    characters: &'s mut world::character::CharacterManager,
    skills: &'s mut std::collections::HashMap<u32, super::pc_skill::PcSkillState>,
    affects: &'s mut std::collections::HashMap<Vid, super::affect::AffectEvent>,
    test_server: bool,
    pulse: u64,
    guilds: &'s super::guild::Guilds,
    count_limit: &'s u16,
    ids: &'s mut Option<world::item::ItemIds>,
    gauss: &'s mut world::character::Gauss,
    attrs: &'s [gamedata::records::ItemAttrRecord],
    masks: &'s gamedata::item_mask::ItemMasks,
    targets: &'s mut QuestTargets,
    stores: bool,
) -> Session<'s> {
    let alignment = bodies.get(&reader).map_or(0, |body| body.card.alignment);
    let pcs = quest_step_pcs(characters);
    let online = quest_step_online(characters);
    let (points, item_giver) = arm_item_giver(
        characters.find_by_vid_mut(reader).ok(),
        count_limit,
        ids,
        gauss,
        attrs,
        masks,
        stores,
        pid,
    );
    Session {
        level,
        npcs: map_npcs(npcs, place),
        protos,
        names,
        items,
        item_names,
        locale,
        mover,
        map_index: context.map_index,
        war_map: context.war_map,
        tables,
        dice,
        reader,
        card: context.card,
        pc_level: context.pc_level,
        caller: context.caller,
        gold: context.gold,
        empire: context.empire,
        char_name: context.char_name,
        position: context.position,
        leadership: context.leadership,
        guild_id: context.guild_id,
        is_guild_master: context.is_guild_master,
        slots: skills.entry(reader.raw()).or_default(),
        test_server,
        pcs,
        online,
        affects: affects.get_mut(&reader),
        alignment,
        alignment_changes: Vec::new(),
        gold_changes: Vec::new(),
        hides: Vec::new(),
        points,
        climbs: Vec::new(),
        guild_ops: Vec::new(),
        block_mode: None,
        guilds: guilds.clone(),
        party_leader: context.party_leader,
        party_member_pids: context.party_member_pids,
        party_flags: context.party_flags,
        party_sets: Vec::new(),
        records: Vec::new(),
        pid,
        targets,
        pulse,
        giver: item_giver,
        item_rows: Vec::new(),
        laid: Vec::new(),
        gates: context.gates,
        conqueror_level: context.conqueror_level,
        party: context.party,
        channel: place.channel,
        warps: Vec::new(),
        drops: Vec::new(),
        villages: 0,
        conquerors: 0,
        sash_opens: Vec::new(),
        changelooks: 0,
        minings: Vec::new(),
        horse_ops: Vec::new(),
        ds_gives: 0,
        ds_refines: 0,
        ds_attrs: 0,
        blocked: Vec::new(),
        switched: false,
        confirm_records: Vec::new(),
        confirm_timeouts: Vec::new(),
    }
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
            card: (3, 1),
            pc_level: 10,
            caller: None,
            gold: 0,
            empire: 0,
            char_name: Vec::new(),
            position: (0, 0, 0, 0),
            leadership: 0,
            guild_id: 0,
            is_guild_master: false,
            slots: &mut slots,
            affects: None,
            alignment: 0,
            alignment_changes: Vec::new(),
            gold_changes: Vec::new(),
            hides: Vec::new(),
            points: None,
            climbs: Vec::new(),
            guild_ops: Vec::new(),
            test_server: true,
            pcs: Vec::new(),
            online: std::collections::HashMap::new(),
            guilds: super::super::guild::GuildShadow::default(),
            records: Vec::new(),
            block_mode: None,
            pid: 1,
            targets: &mut targets,
            pulse: 0,
            giver: None,
            item_rows: Vec::new(),
            laid: Vec::new(),
            gates: WarpGates::armed(),
            conqueror_level: 0,
            party: PartyFlags::default(),
            party_leader: None,
            party_member_pids: Vec::new(),
            blocked: Vec::new(),
            switched: false,
            confirm_records: Vec::new(),
            confirm_timeouts: Vec::new(),
            party_flags: std::collections::HashMap::new(),
            party_sets: Vec::new(),
            channel: 1,
            warps: Vec::new(),
            drops: Vec::new(),
            villages: 0,
            conquerors: 0,
            sash_opens: Vec::new(),
            changelooks: 0,
            minings: Vec::new(),
            horse_ops: Vec::new(),
            ds_gives: 0,
            ds_refines: 0,
            ds_attrs: 0,
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
            assert_eq!(session.job(), 3);
            assert_eq!(session.held_vnum(1000), None);
        });
    }

    /// `pc.warp` records the warp and answers true past `IsHack`, and a hacked reader
    /// records nothing and answers false; `game.drop_item_with_ownership` records the
    /// drop; `pc.get_conqueror_level` reads the armed level.
    #[test]
    fn the_session_warps_past_its_hack_and_records_its_drops() {
        let (protos, names) = (MobProtos::default(), MobNamesByLanguage::default());
        let (items, item_names) = (ItemProtos::default(), ItemNamesByLanguage::default());
        with_session((&protos, &names), (&items, &item_names), |session| {
            session.conqueror_level = 33;
            assert_eq!(session.conqueror_level(), 33);
            assert!(session.warp(853_700, 1_416_400, 0));
            session.gates.set_hack(true);
            assert!(!session.warp(100, 200, 0));
            assert_eq!(session.warps, [(853_700, 1_416_400, 0)]);
            session.drop_with_ownership(2145, 1, Some(300));
            session.drop_with_ownership(2145, 1, None);
            assert_eq!(session.drops, [(2145, 1, Some(300)), (2145, 1, None)]);
        });
    }

    /// `warp_to_village` and `pc.set_conqueror` record their calls, and `pc.can_warp`
    /// reads the armed gate.
    #[test]
    fn the_session_records_its_villages_and_its_conquerors() {
        let (protos, names) = (MobProtos::default(), MobNamesByLanguage::default());
        let (items, item_names) = (ItemProtos::default(), ItemNamesByLanguage::default());
        with_session((&protos, &names), (&items, &item_names), |session| {
            assert!(session.can_warp());
            session.gates.set_able(false);
            assert!(!session.can_warp());
            session.warp_to_village();
            session.warp_to_village();
            assert_eq!(session.villages, 2);
            session.set_conqueror();
            assert_eq!(session.conquerors, 1);
        });
    }

    /// `pc.open_sash` records each combination flag in order, and
    /// `pc.open_changelook` counts its calls.
    #[test]
    fn the_session_records_its_window_opens() {
        let (protos, names) = (MobProtos::default(), MobNamesByLanguage::default());
        let (items, item_names) = (ItemProtos::default(), ItemNamesByLanguage::default());
        with_session((&protos, &names), (&items, &item_names), |session| {
            session.open_sash(true);
            session.open_sash(false);
            assert_eq!(session.sash_opens, [true, false]);
            session.open_changelook();
            assert_eq!(session.changelooks, 1);
        });
    }

    /// The GM level, the room check and the held item's readers: the level is the
    /// session's own; a vnum with no proto has no room, and a sword has room while a
    /// base cell below the unlocked count is empty; the socket, value, count and type
    /// read the held item and its proto, and answer `None` for an item the character
    /// does not hold.
    #[test]
    fn the_session_answers_the_level_the_room_and_the_held_item() {
        use gamedata::item_proto::ItemProto;

        let mut spiral = ItemProto::for_category_rule(72_347, 3, 0);
        spiral.flags = world::item::ITEM_FLAG_STACKABLE;
        spiral.values[3] = 10;
        let mut stronger = ItemProto::for_category_rule(72_348, 3, 0);
        stronger.flags = world::item::ITEM_FLAG_STACKABLE;
        with_giving_session(vec![spiral, stronger], |session| {
            assert_eq!(session.gm_level(), crate::command::GM_GOD);
            assert!(!session.has_empty_inventory(85_000));
            assert!(session.has_empty_inventory(72_347));
            assert_eq!(session.held_socket(1000, 0), None);
            assert_eq!(session.held_value(1000, 3), None);
            assert_eq!(session.held_shape(1000), None);

            assert_eq!(session.give_item(72_347, 3), 1000);
            assert!(
                session.has_empty_inventory(72_347),
                "one cell is still free"
            );
            assert_eq!(session.held_socket(1000, 0), Some(0));
            assert_eq!(session.held_socket(1000, 6), None);
            assert_eq!(session.held_value(1000, 3), Some(10));
            assert_eq!(session.held_value(1000, 6), None);
            assert_eq!(session.held_shape(1000), Some((3, 3, 0)));

            assert_eq!(session.give_item(72_348, 1), 1001);
            assert!(!session.has_empty_inventory(72_347), "no base cell is free");
        });
    }
    /// The test-server flag, the PC lookup and the guild pair: the flag is the session's
    /// own; the lookup matches case-insensitively and answers 0 for an unknown name; the
    /// pair record their ops for the judge after the step.
    #[test]
    fn the_session_answers_the_flag_the_lookup_and_the_guild_pair() {
        let (protos, names) = (MobProtos::default(), MobNamesByLanguage::default());
        let (items, item_names) = (ItemProtos::default(), ItemNamesByLanguage::default());
        with_session((&protos, &names), (&items, &item_names), |session| {
            use quest::manager::Game;
            assert!(session.test_server());
            session.pcs = vec![(b"Yankee".to_vec(), 77)];
            assert_eq!(session.find_pc_vid(b"yankee"), 77);
            assert_eq!(session.find_pc_vid(b"YANKEE"), 77);
            assert_eq!(session.find_pc_vid(b"Nobody"), 0);
            session.destroy_guild();
            session.remove_from_guild();
            let ops: Vec<&[u8]> = session
                .guild_ops
                .iter()
                .map(|op| op.name.as_slice())
                .collect();
            assert_eq!(
                ops,
                [b"destroy_guild".as_slice(), b"remove_from_guild".as_slice()]
            );
        });
    }

    /// `pc.set_skill_level`'s `SetSkillLevel` then `SkillLevelPacket`: the slot takes the level
    /// up to 40 with its read stamp for the store; a boost or ward skill rises only while the
    /// rest of its family is at 0, its own level aside, though 0 always lands and 244 has no
    /// family; slot 254 is the last, a vnum past it sets nothing; and every call sends byte 76.

    #[test]
    fn a_set_skill_level_lands_within_the_slots_and_the_exclusions() {
        let (protos, names) = (MobProtos::default(), MobNamesByLanguage::default());
        let (items, item_names) = (ItemProtos::default(), ItemNamesByLanguage::default());
        with_session((&protos, &names), (&items, &item_names), |session| {
            session.slots.next_read[1] = 0x8102_0304;
            let calls = [
                (239, 1),
                (243, 1),
                (236, 3),
                (221, 2),
                (222, 1),
                (221, 3),
                (244, 5),
                (1, 60),
                (254, 7),
                (255, 1),
                (239, 0),
                (243, 1),
            ];
            for (vnum, level) in calls {
                session.set_skill_level(vnum, level);
            }
            let stamp = i32::from_ne_bytes(0x8102_0304_u32.to_ne_bytes());
            assert_eq!(
                session.climbs,
                [
                    (239, 1, 0),
                    (221, 2, 0),
                    (221, 3, 0),
                    (244, 5, 0),
                    (1, 40, stamp),
                    (254, 7, 0),
                    (239, 0, 0),
                    (243, 1, 0),
                ]
            );
            let levels =
                [239, 243, 236, 221, 222, 244, 1, 254].map(|vnum| session.skill_level(vnum));
            assert_eq!(levels, [0, 1, 0, 3, 0, 5, 40, 7]);
            assert_eq!(session.records.len(), calls.len());
            assert!(session.records.iter().all(|record| record[0] == 76));
        });
    }
    /// The reader names read the session's own fields: gold, empire, the borrowed points'
    /// hp/sp and maxima, exp and the table's next exp, the card's race, the name, the VID,
    /// leadership and the position pairs; with no points the numbers are 0.
    #[test]
    fn the_session_reads_its_character_back() {
        let (protos, names) = (MobProtos::default(), MobNamesByLanguage::default());
        let (items, item_names) = (ItemProtos::default(), ItemNamesByLanguage::default());
        with_session((&protos, &names), (&items, &item_names), |session| {
            session.gold = 200_001;
            session.empire = 2;
            session.char_name = b"Zulu".to_vec();
            session.position = (4701, 9500, 101, 300);
            session.leadership = 7;
            assert_eq!(session.gold(), 200_001);
            assert_eq!(session.empire(), 2);
            assert_eq!(session.char_name(), b"Zulu");
            assert_eq!(session.char_vid(), 1);
            assert_eq!(session.leadership(), 7);
            assert_eq!(session.position(), (4701, 9500, 101, 300));
            assert_eq!(session.race(), 3);
            assert_eq!(session.hp(), 0);
            assert_eq!(session.max_hp(), 0);
            assert_eq!(session.sp(), 0);
            assert_eq!(session.max_sp(), 0);
            assert_eq!(session.exp(), 0);
            assert_eq!(session.next_exp(), 0);
            assert_eq!(session.worn_vnum(4), None);
            assert_eq!(session.empire_of(1234), None);
            assert_eq!(session.guild_id(), 0);
            assert!(!session.is_guild_master());
            session.guild_id = 42;
            session.is_guild_master = true;
            assert_eq!(session.guild_id(), 42);
            assert!(session.is_guild_master());
            assert!(!session.is_mount(), "no ported path mounts");
            assert!(!session.is_party());
            assert!(!session.is_party_leader());
            session.party.set_party(true);
            session.party.set_leader(true);
            assert!(session.is_party_leader());
            assert_eq!(session.channel_id(), 1);
            assert_eq!(session.party_leader_pid(), -1, "no party is -1");
            assert_eq!(session.party_flag(b"frt_dungeon_index"), 0);
            session.set_party_flag(b"frt_dungeon_index".to_vec(), 210);
            assert_eq!(session.party_sets.len(), 0, "no party sets nothing");
            session.party_leader = Some(4321);
            session
                .party_flags
                .insert(b"frt_dungeon_index".to_vec(), 200);
            assert_eq!(session.party_leader_pid(), 4321);
            assert_eq!(session.party_flag(b"frt_dungeon_index"), 200);
            session.set_party_flag(b"frt_dungeon_index".to_vec(), 210);
            assert_eq!(session.party_flag(b"frt_dungeon_index"), 210);
            assert_eq!(session.party_sets, [(b"frt_dungeon_index".to_vec(), 210)]);
        });
    }

    /// In a block the level, name, VID and count readers answer the blocked PC's, a removal
    /// takes from its snapshot and records the take, and the end restores the reader. An
    /// unswitched frame answers the reader.
    #[test]
    fn a_block_runs_for_the_blocked_pc_and_its_end_restores_the_reader() {
        let (protos, names) = (MobProtos::default(), MobNamesByLanguage::default());
        let (items, item_names) = (ItemProtos::default(), ItemNamesByLanguage::default());
        with_session((&protos, &names), (&items, &item_names), |session| {
            session.level = 30;
            session.char_name = b"Leader".to_vec();
            let mut blocked_items = world::character::CharacterItems::new();
            let mut ticket = world::item::Item::new(7, 30613);
            ticket.set_size(1).expect("a positive footprint");
            ticket.set_count(2).expect("a positive count");
            world::character::grant(&mut blocked_items, &mut ticket, &[], 0).expect("a free cell");
            session.online.insert(
                99,
                BlockedPc {
                    pushed: 1,
                    pid: 99,
                    vid: Vid::new(777),
                    level: 90,
                    name: b"Member".to_vec(),
                    points: None,
                    items: blocked_items,
                    usable_cells: common::item_slots::usable_inventory_cells(0),
                    takes: Vec::new(),
                },
            );
            session.begin_other_pc(99);
            assert!(session.switched_pc());
            assert_eq!(session.level(), 90);
            assert_eq!(session.char_name(), b"Member");
            assert_eq!(session.char_vid(), 777);
            assert_eq!(session.count_item(30613), 2);
            session.remove_item(30613, 1);
            assert_eq!(session.count_item(30613), 1);
            assert_eq!(session.blocked.len(), 1);
            session.end_other_pc();
            assert_eq!(session.level(), 30);
            assert_eq!(session.char_name(), b"Leader");
            assert_eq!(session.char_vid(), 1);
            assert_eq!(session.count_item(30613), 0);
            // Nobody online under 100: the stack grows but the script keeps the reader.
            session.begin_other_pc(100);
            assert!(!session.switched_pc());
            assert_eq!(session.level(), 30);
            assert_eq!(session.blocked.len(), 1);
            session.end_other_pc();
            assert!(session.blocked.is_empty());
        });
    }

    /// `ds.is_qualified` reads the 540 affect: the grant lands in its call (the infinite
    /// row and its add packet), and the two window opens queue for the apply.
    #[test]
    fn the_session_grants_the_ds_qualification_in_its_call_and_queues_the_opens() {
        let (protos, names) = (MobProtos::default(), MobNamesByLanguage::default());
        let (items, item_names) = (ItemProtos::default(), ItemNamesByLanguage::default());
        let locale = LocaleStrings::default();
        let mut dice = Pcg32::new(1, 1);
        let tables = gamedata::pc_skill_data::PcSkills::default();
        let mut slots = super::super::pc_skill::PcSkillState::default();
        let mut targets = QuestTargets::default();
        let mut event = super::super::affect::AffectEvent {
            due: 0,
            sequence: 0,
            affects: Vec::new(),
        };
        let mut session = Session {
            map_index: 0,
            war_map: 0,
            level: 1,
            npcs: None,
            protos: &protos,
            names: &names,
            items: &items,
            item_names: &item_names,
            locale: &locale,
            mover: test_place::MOVER,
            tables: &tables,
            dice: &mut dice,
            reader: Vid::new(1),
            card: (3, 1),
            pc_level: 10,
            caller: None,
            gold: 0,
            empire: 0,
            char_name: Vec::new(),
            position: (0, 0, 0, 0),
            leadership: 0,
            guild_id: 0,
            is_guild_master: false,
            slots: &mut slots,
            affects: Some(&mut event),
            alignment: 0,
            alignment_changes: Vec::new(),
            gold_changes: Vec::new(),
            hides: Vec::new(),
            points: None,
            climbs: Vec::new(),
            guild_ops: Vec::new(),
            test_server: true,
            pcs: Vec::new(),
            online: std::collections::HashMap::new(),
            guilds: super::super::guild::GuildShadow::default(),
            records: Vec::new(),
            block_mode: None,
            pid: 1,
            targets: &mut targets,
            pulse: 0,
            giver: None,
            item_rows: Vec::new(),
            laid: Vec::new(),
            gates: WarpGates::armed(),
            conqueror_level: 0,
            party: PartyFlags::default(),
            party_leader: None,
            party_member_pids: Vec::new(),
            blocked: Vec::new(),
            switched: false,
            confirm_records: Vec::new(),
            confirm_timeouts: Vec::new(),
            party_flags: std::collections::HashMap::new(),
            party_sets: Vec::new(),
            channel: 1,
            warps: Vec::new(),
            drops: Vec::new(),
            villages: 0,
            conquerors: 0,
            sash_opens: Vec::new(),
            changelooks: 0,
            minings: Vec::new(),
            horse_ops: Vec::new(),
            ds_gives: 0,
            ds_refines: 0,
            ds_attrs: 0,
        };
        assert!(!Game::is_ds_qualified(&session));
        Game::give_ds_qualification(&mut session);
        Game::open_ds_refine(&mut session);
        Game::open_ds_attr_change(&mut session);
        assert_eq!(session.ds_gives, 1);
        assert_eq!(session.ds_refines, 1);
        assert_eq!(session.ds_attrs, 1);
        // The grant landed in the call: the infinite row sits in the event, and its add
        // packet joined the step's records, so a later read in the same step answers
        // true.
        assert!(Game::is_ds_qualified(&session));
        assert_ds_grant(&session);
    }

    /// The infinite 540 row the grant pushed, and its add packet in the step's records.
    fn assert_ds_grant(session: &Session<'_>) {
        let held = session
            .affects
            .as_deref()
            .expect("armed")
            .affects
            .iter()
            .find(|affect| affect.kind == crate::loading_phase::AFFECT_DRAGON_SOUL_QUALIFIED)
            .expect("granted");
        assert_eq!(held.remaining, 60 * 365 * 24 * 60 * 60);
        assert_eq!(held.apply_on, 0);
        assert_eq!(held.value, 0);
        assert_eq!(held.flag, 0);
        assert_eq!(held.sp_cost, 0);
        assert_eq!(
            session.records,
            vec![super::super::affect::affect_add_frame(held)],
            "the add joins the step's records"
        );
    }

    /// `pc.change_money` joins the step: a later read sees the moved gold, and the amounts
    /// queue for the apply. A move below 0 or onto `GOLD_MAX_MAX` is refused with the gold
    /// untouched.
    #[test]
    fn a_money_change_moves_the_read_and_queues() {
        let (protos, names) = (MobProtos::default(), MobNamesByLanguage::default());
        let (items, item_names) = (ItemProtos::default(), ItemNamesByLanguage::default());
        with_session((&protos, &names), (&items, &item_names), |session| {
            use quest::manager::Game;
            session.gold = 200_000;
            session.change_gold(-50_000);
            assert_eq!(session.gold(), 150_000);
            session.change_gold(25);
            assert_eq!(session.gold(), 150_025);
            assert_eq!(session.gold_changes, [-50_000, 25]);
            session.change_gold(-150_026);
            assert_eq!(session.gold(), 150_025, "below zero refuses");
            let cap = i64::try_from(world::character::GOLD_MAX_MAX).expect("the cap fits");
            session.change_gold(cap - 150_025);
            assert_eq!(session.gold(), 150_025, "onto the cap refuses");
            assert_eq!(session.gold_changes.len(), 2);
        });
    }
    /// `pc.hide_costume` queues its chat and its hide: with no points the mask has nowhere
    /// to land, but the client still hears `SetXCostumeHidden` and the step still applies.
    #[test]
    fn a_costume_hide_queues_its_chat_and_hide() {
        let (protos, names) = (MobProtos::default(), MobNamesByLanguage::default());
        let (items, item_names) = (ItemProtos::default(), ItemNamesByLanguage::default());
        with_session((&protos, &names), (&items, &item_names), |session| {
            use quest::manager::Game;
            session.hide_costume(1, true);
            session.hide_costume(3, false);
            assert_eq!(session.hides, [(1, true), (3, false)]);
            assert_eq!(session.records.len(), 2);
            let locale = LocaleStrings::default();
            let to = test_place::MOVER.recipient(&locale);
            assert_eq!(
                session.records[0],
                chat_packet(
                    to,
                    CHAT_TYPE_COMMAND,
                    b"SetBodyCostumeHidden %d",
                    &[Arg::Int(1)]
                ),
            );
            assert_eq!(
                session.records[1],
                chat_packet(
                    to,
                    CHAT_TYPE_COMMAND,
                    b"SetSashCostumeHidden %d",
                    &[Arg::Int(0)]
                ),
            );
        });
    }

    /// The test sword's vnum.
    const SWORD: u32 = 19;

    /// A session that may give and remove, for a character whose two unlocked cells are
    /// empty, with the rows `protos` as its item protos and item IDs from 1000. The
    /// reader is a god, so the GM level answers without further setup.
    fn with_giving_session<R>(
        protos: Vec<gamedata::item_proto::ItemProto>,
        f: impl FnOnce(&mut Session<'_>) -> R,
    ) -> R {
        use world::character::{CharacterItems, Gauss, MoveRules, Quickslots};
        use world::item::{ItemIdRange, ItemIds};

        let items_protos = ItemProtos::from_rows(protos);
        let (mobs, names) = (MobProtos::default(), MobNamesByLanguage::default());
        let item_names = ItemNamesByLanguage::default();
        let (locale, mut dice) = (LocaleStrings::default(), Pcg32::new(1, 1));
        let tables = gamedata::pc_skill_data::PcSkills::default();
        let mut slots = super::super::pc_skill::PcSkillState::default();
        let mut targets = QuestTargets::default();
        let mut held = CharacterItems::new();
        let mut quickslots = Quickslots::default();
        let mut ids = ItemIds::new(ItemIdRange::new(1000, 2000, 1000).unwrap());
        let mut gauss = Gauss::new();
        let mut points = crate::game_state::fixtures::points_at(10, 10_000, 0);
        points.set_gm_level(crate::command::GM_GOD);
        let mut session = Session {
            map_index: 0,
            war_map: 0,
            level: 1,
            npcs: None,
            protos: &mobs,
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
            gold: 0,
            empire: 0,
            char_name: Vec::new(),
            position: (0, 0, 0, 0),
            leadership: 0,
            guild_id: 0,
            is_guild_master: false,
            slots: &mut slots,
            affects: None,
            alignment: 0,
            alignment_changes: Vec::new(),
            gold_changes: Vec::new(),
            hides: Vec::new(),
            points: Some(&mut points),
            climbs: Vec::new(),
            guild_ops: Vec::new(),
            test_server: true,
            pcs: Vec::new(),
            online: std::collections::HashMap::new(),
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
            gates: WarpGates::armed(),
            conqueror_level: 0,
            party: PartyFlags::default(),
            party_leader: None,
            party_member_pids: Vec::new(),
            blocked: Vec::new(),
            switched: false,
            confirm_records: Vec::new(),
            confirm_timeouts: Vec::new(),
            party_flags: std::collections::HashMap::new(),
            party_sets: Vec::new(),
            channel: 1,
            warps: Vec::new(),
            drops: Vec::new(),
            villages: 0,
            conquerors: 0,
            sash_opens: Vec::new(),
            changelooks: 0,
            minings: Vec::new(),
            horse_ops: Vec::new(),
            ds_gives: 0,
            ds_refines: 0,
            ds_attrs: 0,
        };
        f(&mut session)
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
        with_giving_session(
            vec![gamedata::item_proto::ItemProto::for_category_rule(
                SWORD, 1, 0,
            )],
            |session| {
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
            },
        );
    }

    /// `item.set_socket`'s `SetSocket` in a session that may give: a held item's socket is
    /// written, its cell updated (`GC_ITEM_UPDATE`, 0x19) and its sockets answered for the
    /// store; a laid one takes the value and sends nothing; an ID the character has neither
    /// held nor laid writes nothing. A step whose rows no descriptor stores writes nothing.
    #[test]
    fn a_socket_lands_on_the_held_or_laid_item_and_is_stored() {
        with_giving_session(
            vec![gamedata::item_proto::ItemProto::for_category_rule(
                SWORD, 1, 0,
            )],
            |session| {
                let given = [0; 3].map(|_| session.give_item(SWORD, 1));
                assert_eq!(given, [1000, 1001, 1002]);
                let (records, rows) = (session.records.len(), session.item_rows.len());

                session.set_socket(1000, 1, 0x0102_0304);
                assert_eq!(session.records.len(), records + 1);
                assert_eq!(session.records[records][0], 0x19);
                assert!(matches!(
                    session.item_rows.last(),
                    Some(db::items::RowChange::Sockets { id: 1000, sockets })
                        if sockets[..3] == [0, 0x0102_0304, 0]
                ));
                assert_eq!(session.item_rows.len(), rows + 1);

                session.set_socket(1002, 2, 7);
                session.set_socket(4242, 0, 1);
                assert_eq!(session.laid[0].sockets[2], 7);
                assert_eq!(
                    (session.records.len(), session.item_rows.len()),
                    (records + 1, rows + 1)
                );

                if let Some(giver) = session.giver.as_mut() {
                    giver.stores = false;
                }
                session.set_socket(1001, 0, 9);
                assert_eq!(session.item_rows.len(), rows + 1);
                let unchanged = session
                    .giver
                    .as_ref()
                    .and_then(|giver| giver.items.item(1001));
                assert_eq!(unchanged.map(|item| item.sockets[0]), Some(0));
            },
        );
    }
    /// `pc.diamond_refine`'s melt of a stack of 100 diamond ore: the count gate of 19
    /// fails first (and a count of 19 of a non-ore answers no melt at all), then a stack
    /// of 200 shrinks to 100 with its update and its row, the 10000 fee leaves the gold,
    /// the draw at 100 lands the refined 50621 with its ID, and its answer is true. A
    /// broke melt gates the gold before it spends, so the ore stays (legacy spends the
    /// 100 first, a Defect not reproduced). An unheld ID melts nothing.
    #[test]
    fn a_diamond_melt_spends_a_hundred_and_the_fee_then_lands_the_refined() {
        use world::item::ITEM_FLAG_STACKABLE;
        const DIAMOND: u32 = 50_601;
        const REFINED: u32 = 50_621;
        let mut diamond = gamedata::item_proto::ItemProto::for_category_rule(DIAMOND, 1, 0);
        diamond.flags |= ITEM_FLAG_STACKABLE;
        let refined = gamedata::item_proto::ItemProto::for_category_rule(REFINED, 1, 0);
        with_giving_session(vec![diamond, refined], |session| {
            let ore = session.give_item(DIAMOND, 19);
            assert!(!session.melt_ore(ore, 10_000, 100, None));
            assert_eq!(session.count_item(DIAMOND), 19);
            assert!(!session.melt_ore(4242, 10_000, 100, None));
            assert!(session.gold_changes.is_empty());

            let more = session.give_item(DIAMOND, 181);
            assert_eq!(more, ore, "the second give merged into the first stack");
            assert_eq!(session.count_item(DIAMOND), 200);
            session.gold = 10_000;
            let (records, rows) = (session.records.len(), session.item_rows.len());
            assert!(session.melt_ore(ore, 10_000, 100, None));
            assert_eq!(session.count_item(DIAMOND), 100);
            assert_eq!(session.held_vnum(1001), Some(REFINED));
            assert_eq!(session.gold, 0);
            assert_eq!(session.gold_changes, [-10_000]);
            assert_eq!(
                session.records.len(),
                records + 4,
                "the update, the fee, then the name and set"
            );
            assert_eq!(session.records[records][0], 0x19);
            assert_eq!(session.records[records + 1][0], 225);
            assert!(matches!(
                session.item_rows[rows],
                db::items::RowChange::Count { id, count: 100 } if id == ore
            ));
            assert!(matches!(
                session.item_rows.last(),
                Some(db::items::RowChange::Created(row)) if row.id == 1001
            ));

            assert!(!session.melt_ore(ore, 10_000, 100, None));
            assert_eq!(
                session.count_item(DIAMOND),
                100,
                "the broke melt spends nothing"
            );
            session.gold = 10_000;
            assert!(!session.melt_ore(ore, 10_000, 0, None));
            assert_eq!(session.count_item(DIAMOND), 0);
            assert!(
                session
                    .giver
                    .as_ref()
                    .is_some_and(|giver| giver.items.item(ore).is_none()),
                "the exact 100 cleared the cell"
            );
        });
    }
    /// `pc.ore_refine`'s melt of 100 gold ore with its +0 metin stone: a held stone ID
    /// spends the whole stone stack with its row, the fee leaves the gold, and the draw
    /// at 100 lands the refined 50622. An unheld stone ID melts nothing and spends
    /// nothing.
    #[test]
    fn an_ore_melt_spends_the_stone_too_and_lands_the_refined() {
        use world::item::ITEM_FLAG_STACKABLE;
        const GOLD_ORE: u32 = 50_602;
        const REFINED: u32 = 50_622;
        const STONE: u32 = 28_030;
        let mut ore_proto = gamedata::item_proto::ItemProto::for_category_rule(GOLD_ORE, 1, 0);
        ore_proto.flags |= ITEM_FLAG_STACKABLE;
        let refined = gamedata::item_proto::ItemProto::for_category_rule(REFINED, 1, 0);
        let stone = gamedata::item_proto::ItemProto::for_category_rule(STONE, 10, 0);
        with_giving_session(vec![ore_proto, refined, stone], |session| {
            let ore = session.give_item(GOLD_ORE, 100);
            let dust = session.give_item(STONE, 1);
            session.gold = 3_000;
            assert!(!session.melt_ore(ore, 3_000, 100, Some(4242)));
            assert_eq!(session.count_item(GOLD_ORE), 100);
            assert_eq!(session.count_item(STONE), 1);
            assert_eq!(session.gold, 3_000);

            let rows = session.item_rows.len();
            assert!(session.melt_ore(ore, 3_000, 100, Some(dust)));
            assert_eq!(session.gold, 0);
            assert_eq!(session.held_vnum(1002), Some(REFINED));
            assert!(session
                .giver
                .as_ref()
                .is_some_and(|giver| giver.items.item(ore).is_none()));
            assert!(session
                .giver
                .as_ref()
                .is_some_and(|giver| giver.items.item(dust).is_none()));
            assert_eq!(session.item_rows.len(), rows + 3);
            assert!(matches!(
                session.item_rows[rows + 1],
                db::items::RowChange::Destroyed { id } if id == dust
            ));
        });
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
            gold: 0,
            empire: 0,
            char_name: Vec::new(),
            position: (0, 0, 0, 0),
            leadership: 0,
            guild_id: 0,
            is_guild_master: false,
            slots: &mut slots,
            affects: None,
            alignment: 0,
            alignment_changes: Vec::new(),
            gold_changes: Vec::new(),
            hides: Vec::new(),
            points: None,
            climbs: Vec::new(),
            guild_ops: Vec::new(),
            test_server: true,
            pcs: Vec::new(),
            online: std::collections::HashMap::new(),
            guilds: super::super::guild::GuildShadow::default(),
            records: Vec::new(),
            block_mode: None,
            pid: 1,
            targets: &mut QuestTargets::default(),
            pulse: 0,
            giver: None,
            item_rows: Vec::new(),
            laid: Vec::new(),
            gates: WarpGates::armed(),
            conqueror_level: 0,
            party: PartyFlags::default(),
            party_leader: None,
            party_member_pids: Vec::new(),
            blocked: Vec::new(),
            switched: false,
            confirm_records: Vec::new(),
            confirm_timeouts: Vec::new(),
            party_flags: std::collections::HashMap::new(),
            party_sets: Vec::new(),
            channel: 1,
            warps: Vec::new(),
            drops: Vec::new(),
            villages: 0,
            conquerors: 0,
            sash_opens: Vec::new(),
            changelooks: 0,
            minings: Vec::new(),
            horse_ops: Vec::new(),
            ds_gives: 0,
            ds_refines: 0,
            ds_attrs: 0,
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
            gold: 0,
            empire: 0,
            char_name: Vec::new(),
            position: (0, 0, 0, 0),
            leadership: 0,
            guild_id: 0,
            is_guild_master: false,
            slots: &mut slots,
            affects: None,
            alignment: 0,
            alignment_changes: Vec::new(),
            gold_changes: Vec::new(),
            hides: Vec::new(),
            points: None,
            climbs: Vec::new(),
            guild_ops: Vec::new(),
            test_server: true,
            pcs: Vec::new(),
            online: std::collections::HashMap::new(),
            guilds: super::super::guild::GuildShadow::default(),
            records: Vec::new(),
            block_mode: None,
            pid: 1,
            targets: &mut QuestTargets::default(),
            pulse: 0,
            giver: None,
            item_rows: Vec::new(),
            laid: Vec::new(),
            gates: WarpGates::armed(),
            conqueror_level: 0,
            party: PartyFlags::default(),
            party_leader: None,
            party_member_pids: Vec::new(),
            blocked: Vec::new(),
            switched: false,
            confirm_records: Vec::new(),
            confirm_timeouts: Vec::new(),
            party_flags: std::collections::HashMap::new(),
            party_sets: Vec::new(),
            channel: 1,
            warps: Vec::new(),
            drops: Vec::new(),
            villages: 0,
            conquerors: 0,
            sash_opens: Vec::new(),
            changelooks: 0,
            minings: Vec::new(),
            horse_ops: Vec::new(),
            ds_gives: 0,
            ds_refines: 0,
            ds_attrs: 0,
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
        let (records, _climbs, _stores, _items, _gold) = world
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

        let (records, _climbs, _stores, _items, _gold) = world
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
        let (records, _climbs, _stores, _items, _gold) = world
            .quest(
                vid,
                &QuestStep::Button(quest | QUEST_INFO_BIT),
                place,
                mover,
            )
            .expect("the world holds the character");
        assert!(records.is_empty(), "{records:?}");
        let (records, _climbs, _stores, _items, _gold) = world
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
            let (records, _climbs, _stores, _items, _gold) = world
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
        let (taken, (records, _climbs, _stores, _items, _gold)) =
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
    /// The Time Spiral 72347's `use` script end-to-end through the live locale: a
    /// level-30 reader's login letters the quest, its button moves it to `talk`, and with
    /// a `wait_time` an hour out the spiral's use reads the proto value (`item.get_value`
    /// answers the `VALUE3` 10), shortens the wait by a tenth, banks the new stamp, and
    /// takes the spiral. No dialog waits.
    #[test]
    fn the_time_spiral_shortens_the_attr67_wait_through_the_live_script() {
        use crate::game_state::fixtures::{a_world, enter_seeing_with, TestClock, PLACE};
        use common::item_slots::EWindows;
        use gamedata::mob_locale_names::MobNamesByLanguage;
        use protocol::item_pos::ItemPos;
        use quest::manager::Manager;
        use std::sync::Arc;
        use world::item::Item;

        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let vid = Vid::new(1);
        let cell = ItemPos::new(EWindows::Inventory as u8, 0);
        let mut spiral = Item::new(11, 72_347);
        spiral.set_size(1).expect("a positive footprint");
        spiral.set_count(1).expect("a positive count");
        let (mut inbox, _shown) =
            enter_seeing_with(&mut world, PLACE, 1, (5, 5), 10_000, &[(cell, spiral)]);
        while inbox.try_recv().is_ok() {}

        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager = Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        world.quests = Some(Quests::new(
            manager,
            Arc::default(),
            MobNamesByLanguage::default(),
        ));
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
        let seeded = i32::try_from(now + 3_600).expect("a 32-bit time");
        world
            .quests
            .as_mut()
            .expect("the quests are loaded")
            .manager_mut()
            .load_flags(pid, [(b"attr67add_item.wait_time".to_vec(), seeded)]);
        // Thirty levels open the login's letter gate.
        world
            .characters
            .find_by_vid_mut(vid)
            .expect("the reader is in the world")
            .set_points(Some(crate::game_state::fixtures::points_at(30, 10_000, 0)));
        world.quest_login(vid);
        let quest = world
            .quests
            .as_ref()
            .expect("the quests are loaded")
            .manager()
            .host()
            .quest_index(b"item_attr67add")
            .expect("the quest is loaded");
        let place = crate::game_state::GroundPlace {
            channel: PLACE.0,
            map: PLACE.1,
            x: 5,
            y: 5,
        };
        let mover = world.own_mover(vid).expect("in the world");
        world
            .quest(vid, &QuestStep::Button(quest), place, mover)
            .expect("the button runs");
        assert!(
            !world.quest_running(vid),
            "the button's page ends without a wait"
        );
        let used = world.use_item(vid, cell, mover).expect("the use runs");
        assert_eq!(used.kind, world::character::MoveKind::QuestUsed);
        assert!(!world.quest_running(vid), "no dialog waits");
        let left = world
            .quests
            .as_ref()
            .expect("the quests are loaded")
            .manager()
            .quest_flag(pid, b"attr67add_item.wait_time")
            .expect("the stamp was banked");
        let shortened = seeded - left;
        assert!(
            (359..=361).contains(&shortened),
            "a tenth of the hour, give a tick: {shortened}"
        );
        assert!(
            world
                .characters()
                .find_by_vid(vid)
                .expect("the reader is in the world")
                .items()
                .item_at(cell)
                .is_none(),
            "the spiral left"
        );
        while inbox.try_recv().is_ok() {}
    }

    /// A step's `game.drop_item_with_ownership` lays the made item at the reader's
    /// feet after the step, owned by the reader for the stamped seconds; a missing
    /// number and 10 or below own it for 30. A vnum with no proto lays nothing.
    #[test]
    fn the_steps_drop_lies_owned_at_the_readers_feet() {
        use crate::game_state::fixtures::{a_world, enter, TestClock};
        use world::item::ItemIdRange;
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
        world
            .install_item_ids(ItemIdRange::new(1, 1_000_000, 1).expect("a usable range"))
            .expect("the first install");
        let place = GroundPlace {
            channel: 1,
            map: 41,
            x: 500,
            y: 500,
        };
        let mover = world.own_mover(vid).expect("the entrant has a body");
        while inbox.try_recv().is_ok() {}
        world
            .run_quests(place, mover, 10, pid, |_manager, game| {
                game.drop_with_ownership(2145, 1, Some(300));
                game.drop_with_ownership(2145, 1, None);
                game.drop_with_ownership(2145, 1, Some(10));
                game.drop_with_ownership(85_000, 1, Some(300));
            })
            .expect("the quests are loaded");
        let mut ground: Vec<_> = world.ground.values().collect();
        ground.sort_by_key(|lying| lying.ground.vid);
        assert_eq!(ground.len(), 3, "the vnum with no proto laid nothing");
        for lying in &ground {
            assert_eq!(lying.ground.item.vnum, 2145);
            assert_eq!(
                lying.owner.as_ref().map(|owner| owner.pid),
                Some(pid),
                "the reader owns every drop"
            );
            let (x, y) = (lying.ground.x, lying.ground.y);
            assert!(
                (300..=700).contains(&x) && (300..=700).contains(&y),
                "at the feet, +-200: ({x}, {y})"
            );
        }
        let secs: Vec<i32> = ground
            .iter()
            .map(|lying| {
                lying.owner.as_ref().map_or(-1, |owner| {
                    i32::try_from(owner.due.saturating_sub(world.last_pulse) / 25).unwrap_or(-1)
                })
            })
            .collect();
        assert_eq!(secs, [300, 30, 30], "the stamps clamp at 10 and below");
        while inbox.try_recv().is_ok() {}
    }

    /// A step's `pc.set_conqueror` chats the already line for a conqueror, the marble
    /// line for a reader below its filled marble, and makes a conqueror of a reader at
    /// 120 with the marble filled: the level 1, the step, point, exp and sungma reset.
    #[test]
    fn the_steps_conqueror_chats_its_lines_or_makes_the_reader() {
        use crate::game_state::fixtures::{a_world, enter, points_at, TestClock};
        use common::point_slot as point;
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
            map: 41,
            x: 500,
            y: 500,
        };
        let mover = world.own_mover(vid).expect("the entrant has a body");
        while inbox.try_recv().is_ok() {}
        let run = |world: &mut GameState, points: world::character::Points| {
            world
                .characters
                .find_by_vid_mut(vid)
                .expect("the reader stands")
                .set_points(Some(points));
            world
                .run_quests(place, mover, 10, pid, |_manager, game| {
                    game.set_conqueror();
                })
                .expect("the quests are loaded");
        };
        // A conqueror already: the first line, and nothing moves.
        let mut already = points_at(120, 10_000, 0);
        let _records = already.become_conqueror(30);
        run(&mut world, already);
        let line = inbox.try_recv().expect("the already line");
        assert!(
            line.windows(b"Champion Level.".len())
                .any(|window| window == b"Champion Level."),
            "the already line chats"
        );
        while inbox.try_recv().is_ok() {}
        // Below the marble: the second line, and nothing moves.
        run(&mut world, points_at(10, 10_000, 0));
        let line = inbox.try_recv().expect("the marble line");
        assert!(
            line.windows(b"EXP Marble".len())
                .any(|window| window == b"EXP Marble"),
            "the marble line chats"
        );
        while inbox.try_recv().is_ok() {}
        // At 120 with the marble filled: the reader becomes a conqueror.
        run(
            &mut world,
            points_at(120, 10_000, common::levels::next_exp(120) / 4),
        );
        let frames: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        assert!(
            frames.iter().all(|frame| frame[0] != 4),
            "no line chats on success"
        );
        let trailing = world::character::PointRecord {
            kind: u8::try_from(point::POINT_CONQUEROR_EXP).unwrap(),
            amount: 0,
            value: 0,
            broadcast: false,
        };
        assert_eq!(
            frames.last(),
            Some(&crate::loading_phase::point_changes(&[trailing], vid.raw()).concat()),
            "the points close on the conqueror experience"
        );
        let points = world
            .characters()
            .find_by_vid(vid)
            .expect("the reader stands")
            .points()
            .expect("the reader has points");
        assert_eq!(points.conqueror_level(), 1);
        for slot in [
            point::POINT_CONQUEROR_LEVEL_STEP,
            point::POINT_CONQUEROR_POINT,
            point::POINT_CONQUEROR_EXP,
            point::POINT_SUNGMA_STR,
            point::POINT_SUNGMA_HP,
            point::POINT_SUNGMA_MOVE,
            point::POINT_SUNGMA_IMMUNE,
        ] {
            assert_eq!(points.get_point(slot), 0);
            assert_eq!(points.real_point(slot), 0);
        }
    }

    /// A step's `pc.open_sash` opens the combination window for `true` and the
    /// absorption window for a fresh reader, sends the open record, and chats
    /// the legacy keys when the window already stands: `SASH_OPEN` for the
    /// same one, `SASH_CLOSE` for combination over absorption, `COMBINE_CLOSE`
    /// for absorption over combination.
    #[test]
    fn the_steps_sash_opens_or_chats_its_lines() {
        use crate::game_state::fixtures::{a_world, enter, TestClock};
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
            map: 41,
            x: 500,
            y: 500,
        };
        let mover = world.own_mover(vid).expect("the entrant has a body");
        let to = mover.recipient(&world.locale);
        let already = chat_packet(to, CHAT_TYPE_INFO, b"SASH_OPEN", &[]);
        let close = chat_packet(to, CHAT_TYPE_INFO, b"COMBINE_CLOSE", &[]);
        while inbox.try_recv().is_ok() {}
        // Fresh: the combination window opens.
        world
            .run_quests(place, mover, 10, pid, |_manager, game| {
                game.open_sash(true);
            })
            .expect("the quests are loaded");
        let open = protocol::gc_sash::GcSash::open(true).encode();
        assert_eq!(inbox.try_recv().expect("the open record"), open);
        while inbox.try_recv().is_ok() {}
        // Same window again: `SASH_OPEN`, and nothing moves.
        world
            .run_quests(place, mover, 10, pid, |_manager, game| {
                game.open_sash(true);
            })
            .expect("the quests are loaded");
        assert_eq!(inbox.try_recv().expect("the already line"), already);
        while inbox.try_recv().is_ok() {}
        // Absorption over combination: `COMBINE_CLOSE`.
        world
            .run_quests(place, mover, 10, pid, |_manager, game| {
                game.open_sash(false);
            })
            .expect("the quests are loaded");
        assert_eq!(inbox.try_recv().expect("the close line"), close);
        while inbox.try_recv().is_ok() {}
    }

    /// One `pc.open_changelook` call opens the window; the second chats the opened
    /// line.
    #[test]
    fn the_steps_changelook_opens_or_chats_its_line() {
        use crate::game_state::fixtures::{a_world, enter, TestClock};
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
            map: 41,
            x: 500,
            y: 500,
        };
        let mover = world.own_mover(vid).expect("the entrant has a body");
        let to = mover.recipient(&world.locale);
        let already = chat_packet(
            to,
            CHAT_TYPE_INFO,
            b"[Transmutation] The window is already opened",
            &[],
        );
        while inbox.try_recv().is_ok() {}
        world
            .run_quests(place, mover, 10, pid, |_manager, game| {
                game.open_changelook();
            })
            .expect("the quests are loaded");
        let open = protocol::gc_changelook::GcChangeLook::open().encode();
        assert_eq!(inbox.try_recv().expect("the open record"), open);
        while inbox.try_recv().is_ok() {}
        world
            .run_quests(place, mover, 10, pid, |_manager, game| {
                game.open_changelook();
            })
            .expect("the quests are loaded");
        assert_eq!(inbox.try_recv().expect("the already line"), already);
        while inbox.try_recv().is_ok() {}
    }

    /// `party.setf` lands on the world's party and `party.getf` reads it back,
    /// and `party.get_leader_pid` answers the leader; with no party the set is
    /// quiet and the reads answer 0 and -1.
    #[test]
    fn the_steps_party_flags_set_and_read() {
        use crate::game_state::fixtures::{a_world, enter, TestClock};
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
            map: 41,
            x: 500,
            y: 500,
        };
        let mover = world.own_mover(vid).expect("the entrant has a body");
        while inbox.try_recv().is_ok() {}
        let flag = b"frt_dungeon_index".to_vec();
        // No party: the set is quiet, the reads answer 0 and -1.
        let (read, leader) = world
            .run_quests(place, mover, 10, pid, |_manager, game| {
                game.set_party_flag(flag.clone(), 210);
                (game.party_flag(&flag), game.party_leader_pid())
            })
            .expect("the quests are loaded")
            .0;
        assert_eq!((read, leader), (0, -1));
        // In a party: the set lands and the next step reads it back.
        world.create_party(pid);
        world
            .run_quests(place, mover, 10, pid, |_manager, game| {
                game.set_party_flag(flag.clone(), 210);
            })
            .expect("the quests are loaded");
        assert_eq!(world.parties.flag_of(pid, &flag), 210);
        let (read, leader) = world
            .run_quests(place, mover, 10, pid, |_manager, game| {
                (game.party_flag(&flag), game.party_leader_pid())
            })
            .expect("the quests are loaded")
            .0;
        assert_eq!(
            (read, leader),
            (210, i32::try_from(pid).expect("a small PID"))
        );
        while inbox.try_recv().is_ok() {}
    }
}
