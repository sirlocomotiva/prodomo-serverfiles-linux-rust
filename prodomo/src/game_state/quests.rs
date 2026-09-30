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
use world::character::{number, Character, Pcg32};
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
}

/// One quest request of a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestStep {
    /// `CG_SCRIPT_ANSWER`: above 250 continues a `wait`, otherwise picks a menu entry.
    Answer(u8),
    /// `CG_QUEST_INPUT_STRING`: the text an `input()` waits for.
    Input(Vec<u8>),
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

/// What a running script reaches: the character's map, its client and the world's dice.
struct Session<'s> {
    /// The NPCs of the character's map.
    npcs: Option<&'s MapNpcs>,
    /// Which monsters exist.
    protos: &'s MobProtos,
    /// Their names.
    names: &'s MobNamesByLanguage,
    /// The chat lines' formats.
    locale: &'s LocaleStrings,
    /// The character's language and empire.
    mover: Mover,
    /// `number()`'s draw.
    dice: &'s mut Pcg32,
    /// What the client is sent, in order.
    records: Vec<Vec<u8>>,
}

impl Game for Session<'_> {
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
    /// the records its script sent.
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
    ) -> Result<Vec<Vec<u8>>, MoveItemRefused> {
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?
            .player_id();
        let answered = self.run_quests(place, mover, |manager, game| match step {
            QuestStep::Answer(answer) => manager.answer(pid, *answer, game),
            QuestStep::Input(text) => manager.input(pid, text, game),
        });
        Ok(answered.map(|((), records)| records).unwrap_or_default())
    }

    /// `CQuestManager::Click` for the character under `vid` clicking the NPC `target` of race
    /// `race`: whether a quest took the click, and the records its scripts sent either way.
    pub(super) fn quest_click(
        &mut self,
        vid: Vid,
        (target, race): (u32, u16),
        place: GroundPlace,
        mover: Mover,
    ) -> (bool, Vec<Vec<u8>>) {
        let Ok(pid) = self.characters.find_by_vid(vid).map(Character::player_id) else {
            return (false, Vec::new());
        };
        self.run_quests(place, mover, |manager, game| {
            manager.click(pid, target, u32::from(race), game)
        })
        .unwrap_or_default()
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

    /// Run `op` on the manager with a [`Game`] for a character standing at `place`.
    fn run_quests<R>(
        &mut self,
        place: GroundPlace,
        mover: Mover,
        op: impl FnOnce(&mut Manager, &mut dyn Game) -> R,
    ) -> Option<(R, Vec<Vec<u8>>)> {
        let quests = self.quests.as_mut()?;
        let mut session = Session {
            npcs: map_npcs(&self.npcs, place),
            protos: &quests.protos,
            names: &quests.names,
            locale: &self.locale,
            mover,
            dice: &mut self.dice,
            records: Vec::new(),
        };
        let answer = op(&mut quests.manager, &mut session);
        Some((answer, session.records))
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
        let mut session = Session {
            npcs: None,
            protos: &protos,
            names: &names,
            locale: &locale,
            mover: test_place::MOVER,
            dice: &mut dice,
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
