//! The whisper: a private line from one character to another.
//!
//! # What the legacy does
//!
//! `CInputMain::Whisper` (`server/server/game/input_main.cpp:340-649`) is the whole
//! system. It reads the target's name and the text out of the record, finds the
//! target, and hands the line to the target's descriptor. The sender is told
//! nothing when the line lands.
//!
//! # Where the target is found
//!
//! The legacy looks in two places. The first is its own character manager:
//! `CHARACTER_MANAGER::instance().FindPC(pinfo->szNameTo)`
//! (`G/char_manager.cpp:244-260`), which finds the online character of the
//! lower-cased name, so the lookup ignores the case. The second is the P2P
//! manager, which knows the characters of the other cores
//! (`P2P_MANAGER::instance().Find(pinfo->szNameTo)`, `G/input_main.cpp:403`).
//!
//! The Rewrite runs every Channel in one process (ADR-0002), so the second lookup
//! is the process-wide client registry rather than a P2P relay. The first lookup
//! is this module's: the character manager of the Channel this world owns.
//!
//! # What the line says
//!
//! The record the target receives is the sender's name and the text. The type is
//! `WHISPER_TYPE_NORMAL`, or `WHISPER_TYPE_GM` when the sender is a game master
//! (`G/input_main.cpp:619-620`). The descriptor builds that record, because the
//! sender's game-master level and the test-server mode are its own
//! (`CHARACTER::IsGM`, `G/char.cpp:6373-6380`).
//!
//! The type's high nibble carries an empire in the legacy's cross-Channel path
//! (`G/input_main.cpp:566-568`), and the Channel that receives the line strips it
//! again (`G/input_p2p.cpp:80-83`). The text conversion that high nibble is for
//! is `ConvertEmpireText`, whose body is empty in this tree
//! (`G/empire_text_convert.cpp:47-55`), so the nibble never reaches the client
//! and the Rewrite does not send it.
//!
//! # A target that is not there
//!
//! The legacy leaves the line for the target when no character answers to the
//! name (`G/input_main.cpp:404-505`). The line waits in the DB server's memory
//! (`CClientManager::m_OfflineMessage`, `server/server/db/ClientManager.cpp:4631`)
//! and is read back when the target next enters the game
//! (`CHARACTER::ReadOfflineMessages`, `G/char.cpp:11438-11446`). The DB server is
//! retired (ADR-0001), so the waiting lines live in this world's memory, which is
//! the same store in one process.
//!
//! The sender is told what happened with a `WHISPER_TYPE_SYSTEM` record: either
//! that the line was left for the target, or that the sender has to wait before
//! it may leave another one. The wait is ten seconds per sender
//! (`G/input_main.cpp:426-428`).

use common::vid::Vid;
use protocol::gc_whisper::{GcWhisper, WHISPER_TYPE_OFFLINE, WHISPER_TYPE_SYSTEM};
use tracing::{debug, warn};

use crate::game_loop_messages::WhisperOutcome;
use crate::game_state::GameState;

/// A line left for a character that was not online when it was sent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OfflineMessage {
    /// The name of the character that sent it.
    pub from: String,
    /// The line itself: the bytes the sender sent, unchanged.
    pub text: Vec<u8>,
    /// `get_dword_time()` when the line was left, which is when it starts to age.
    pub left_at: u32,
}

/// The ten seconds one character waits between two lines left for an absent
/// target (`bDelay`, `G/input_main.cpp:426`).
const OFFLINE_MESSAGE_DELAY_MS: u32 = 10 * 1000;

/// The five hours a waiting line lives (`iHour`,
/// `CClientManager::OfflineMessageGarbage`, `server/server/db/ClientManager.cpp:4642`).
const OFFLINE_MESSAGE_LIFETIME_MS: u32 = 5 * 60 * 60 * 1000;

/// What the legacy says when the line was left for the target
/// (`G/input_main.cpp:472`).
const OFFLINE_MESSAGE_SENT: &[u8] = b"An offline message has been sent.";

/// What the legacy says when the sender must wait
/// (`G/input_main.cpp:479`).
const OFFLINE_MESSAGE_WAIT: &[u8] = b"You have to wait 10 seconds for send offline message.";

impl GameState {
    /// The character online under `name` in this Channel, if any.
    ///
    /// `FindPC` (`G/char_manager.cpp:244-260`) finds the online character of the
    /// lower-cased name, so the lookup ignores the case, and the Rewrite's
    /// character manager indexes the names the same way
    /// (`world/src/character/manager.rs:209-217`).
    pub(super) fn whisper_target(&self, name: &str) -> Option<Vid> {
        self.characters
            .find_player_by_name(name)
            .ok()
            .map(world::character::Character::vid)
    }

    /// Run one whisper from the character online under `vid`.
    ///
    /// `record` is the record the target receives, which the caller built: the
    /// sender's name, the sender's game-master type and the text. `name` is the
    /// target's name as the client sent it and `text` is the line itself, which
    /// the offline path needs on its own.
    pub(super) fn answer_whisper(
        &mut self,
        vid: common::vid::Vid,
        name: &[u8],
        text: &[u8],
        record: Vec<u8>,
        reply: tokio::sync::oneshot::Sender<WhisperOutcome>,
    ) {
        let outcome = self.deliver_whisper(vid, name, text, record);
        if reply.send(outcome).is_err() {
            debug!(?vid, "nobody was left to hear a whisper's answer");
        }
    }

    /// The line's way to its target, or to the sender's notice.
    fn deliver_whisper(
        &mut self,
        vid: common::vid::Vid,
        name: &[u8],
        text: &[u8],
        record: Vec<u8>,
    ) -> WhisperOutcome {
        // `FindPC` compares bytes, and a character's Name is ASCII, so a name that
        // is not text cannot name one. The line then waits for the name, as it
        // does in the legacy, where the name is a C string of bytes.
        let target = std::str::from_utf8(name)
            .ok()
            .and_then(|name| self.whisper_target(name));
        match target {
            Some(target) if target == vid => {
                // `if (pkChr == ch) return (iExtraLen);` (`G/input_main.cpp:372`):
                // the sender's own name is dropped without a word.
                WhisperOutcome::Dropped
            }
            Some(target) => {
                if text.is_empty() {
                    // `if (buflen > 0)` (`G/input_main.cpp:622`): an empty line
                    // sends nothing.
                    return WhisperOutcome::Dropped;
                }
                // The target is on this Channel: its descriptor gets the record.
                let _sent = self.write_to_client(target, record);
                WhisperOutcome::Delivered
            }
            None => self.deliver_whisper_elsewhere(vid, name, text, record),
        }
    }

    /// The target is not on this Channel.
    ///
    /// The legacy asked its P2P manager for the name and relayed the line to the
    /// core that had it (`G/input_main.cpp:403-415`). Every Channel is in this
    /// process's registry, so the lookup is the same walk
    /// (`ChannelClients::outbox_of`).
    fn deliver_whisper_elsewhere(
        &mut self,
        vid: common::vid::Vid,
        name: &[u8],
        text: &[u8],
        record: Vec<u8>,
    ) -> WhisperOutcome {
        let elsewhere = std::str::from_utf8(name)
            .ok()
            .and_then(|name| self.clients.as_ref()?.outbox_of(name));
        if let Some(outbox) = elsewhere {
            if text.is_empty() {
                // `if (buflen > 0)` (`G/input_main.cpp:622`): an empty line sends
                // nothing.
                return WhisperOutcome::Dropped;
            }
            let _sent = outbox.send(record);
            return WhisperOutcome::Delivered;
        }
        self.leave_offline_message(vid, name, text)
    }

    /// Leave the line for a target that is not here, and tell the sender.
    ///
    /// `G/input_main.cpp:404-505`. The legacy leaves the line only when the
    /// sender may leave one, which is ten seconds after the last, and it answers
    /// the sender either way.
    fn leave_offline_message(
        &mut self,
        vid: common::vid::Vid,
        name: &[u8],
        text: &[u8],
    ) -> WhisperOutcome {
        let now = self.clock.now();
        let last = self.last_offline_pm.get(&vid).copied().unwrap_or(0);
        if now.wrapping_sub(last) > OFFLINE_MESSAGE_DELAY_MS {
            if text.is_empty() {
                // `else return (iExtraLen);` (`G/input_main.cpp:474`): an empty
                // line leaves nothing and says nothing.
                return WhisperOutcome::Dropped;
            }
            // `CHARACTER::SendOfflineMessage` (`G/char.cpp:11421-11436`) stores
            // the line and stamps the sender's clock. It declines an empty name.
            if !name.is_empty() {
                let from = self
                    .characters
                    .find_by_vid(vid)
                    .map(|character| character.name().to_owned())
                    .unwrap_or_default();
                self.offline_messages
                    .entry(name.to_vec())
                    .or_default()
                    .push(OfflineMessage {
                        from,
                        text: text.to_vec(),
                        left_at: now,
                    });
                self.last_offline_pm.insert(vid, now);
            }
            self.answer_offline(vid, name, OFFLINE_MESSAGE_SENT);
        } else {
            self.answer_offline(vid, name, OFFLINE_MESSAGE_WAIT);
        }
        WhisperOutcome::Answered
    }

    /// Tell the sender what happened to its line.
    ///
    /// `G/input_main.cpp:482-493`: a `WHISPER_TYPE_SYSTEM` record whose name is
    /// the *target's* name and whose text is the message, NUL included, because
    /// the length the legacy writes is `strlen(msg) + 1`.
    fn answer_offline(&mut self, vid: common::vid::Vid, name: &[u8], message: &[u8]) {
        let mut text = message.to_vec();
        text.push(0);
        let record = GcWhisper {
            whisper_type: WHISPER_TYPE_SYSTEM,
            name_from: crate::loading_phase::name_bytes_field(name),
            can_format: true,
            text,
        };
        let Ok(bytes) = record.encode() else {
            warn!(?vid, "a whisper notice did not fit its record");
            return;
        };
        let _sent = self.write_to_client(vid, bytes);
    }

    /// The lines waiting for the character that just entered, and their removal.
    ///
    /// `CHARACTER::ReadOfflineMessages` (`G/char.cpp:11438-11446`) asks for the
    /// lines when the character enters, and the answer takes them away
    /// (`CClientManager::RequestReadOfflineMessages`,
    /// `server/server/db/ClientManager.cpp:4613-4629`).
    pub(super) fn take_offline_messages(&mut self, name: &[u8]) -> Vec<OfflineMessage> {
        self.offline_messages.remove(name).unwrap_or_default()
    }

    /// The lines that waited for the character that just entered.
    ///
    /// `CInputDB::ReadOfflineMessages` (`G/input_db.cpp:2455-2476`) writes one
    /// record per line: the name is the sender's, the text is the line with its
    /// NUL, and the type is `WHISPER_TYPE_OFFLINE`.
    pub(super) fn deliver_offline_messages(&mut self, vid: common::vid::Vid, name: &str) {
        for message in self.take_offline_messages(name.as_bytes()) {
            let mut text = message.text.clone();
            text.push(0);
            // `int len = MIN(CHAT_MAX_LEN, strlen(p->szMessage) + 1);`
            text.truncate(common::constants::CHAT_MAX_LEN as usize);
            let record = GcWhisper {
                whisper_type: WHISPER_TYPE_OFFLINE,
                name_from: crate::loading_phase::name_field(&message.from),
                can_format: true,
                text,
            };
            let Ok(bytes) = record.encode() else {
                warn!(?vid, "an offline message did not fit its record");
                continue;
            };
            let _sent = self.write_to_client(vid, bytes);
        }
    }

    /// Drop the lines that have waited for five hours.
    ///
    /// `CClientManager::OfflineMessageGarbage`
    /// (`server/server/db/ClientManager.cpp:4637-4654`) drops them hourly.
    pub(super) fn expire_offline_messages(&mut self) {
        let now = self.clock.now();
        self.offline_messages.retain(|_, messages| {
            messages
                .retain(|message| now.wrapping_sub(message.left_at) <= OFFLINE_MESSAGE_LIFETIME_MS);
            !messages.is_empty()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_state::fixtures::{a_world, enter, TestClock};
    use protocol::gc_whisper::GC_WHISPER_WIRE_SIZE;

    /// The fixture names the character it enters `P{vid}`.
    const VID: u32 = 100;
    /// The other character the fixture enters.
    const OTHER: u32 = 101;

    fn a_world_with_a_player() -> (
        GameState,
        TestClock,
        tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    ) {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let inbox = enter(&mut world, VID, (3200, 3200), 0);
        (world, clock, inbox)
    }

    #[test]
    fn an_unknown_name_has_no_target() {
        let (world, _clock, _inbox) = a_world_with_a_player();
        assert!(world.whisper_target("Nobody").is_none());
    }

    #[test]
    fn a_name_finds_its_character_whatever_the_case() {
        let (world, _clock, _inbox) = a_world_with_a_player();
        assert_eq!(world.whisper_target("P100"), Some(Vid::new(VID)));
        assert_eq!(world.whisper_target("p100"), Some(Vid::new(VID)));
    }

    #[test]
    fn the_line_reaches_the_target_and_not_the_sender() {
        let (mut world, _clock, _sender) = a_world_with_a_player();
        let (tx, mut inbox) = tokio::sync::mpsc::unbounded_channel();
        world
            .enter_world_with_items(
                Vid::new(OTHER),
                OTHER,
                "P101",
                &[],
                crate::client_registry::ClientOutbox::new(tx),
            )
            .expect("the second player enters");
        let _list = inbox.try_recv().expect("the enter WAR_LIST is queued");

        let outcome =
            world.deliver_whisper(Vid::new(VID), b"P101", b"hello", b"the record".to_vec());
        assert_eq!(outcome, WhisperOutcome::Delivered);
        assert_eq!(
            inbox.try_recv().expect("the target has the line"),
            b"the record".to_vec()
        );
    }

    #[test]
    fn the_sender_hears_nothing_when_it_names_itself() {
        let (mut world, _clock, _inbox) = a_world_with_a_player();
        let outcome =
            world.deliver_whisper(Vid::new(VID), b"P100", b"hello", b"the record".to_vec());
        assert_eq!(outcome, WhisperOutcome::Dropped);
    }

    #[test]
    fn a_line_for_an_absent_name_waits_and_the_sender_is_told() {
        let (mut world, clock, mut inbox) = a_world_with_a_player();
        clock.set(20_000);

        let outcome =
            world.deliver_whisper(Vid::new(VID), b"Nobody", b"hello", b"the record".to_vec());
        assert_eq!(outcome, WhisperOutcome::Answered);
        let notice = inbox.try_recv().expect("the sender was told");
        assert_eq!(notice[0], protocol::gc_inventory::HEADER_GC_WHISPER.value());
        assert_eq!(notice[3], WHISPER_TYPE_SYSTEM, "the notice is the system's");
        assert_eq!(
            &notice[GC_WHISPER_WIRE_SIZE..],
            b"An offline message has been sent.\0",
            "the notice carries the legacy's words"
        );
        let waiting = world.take_offline_messages(b"Nobody");
        assert_eq!(waiting.len(), 1, "the line waits for the target");
        assert_eq!(waiting[0].from, "P100", "the line names its sender");
        assert_eq!(waiting[0].text, b"hello", "the line keeps its bytes");
    }

    #[test]
    fn a_second_line_within_ten_seconds_is_refused() {
        let (mut world, clock, mut inbox) = a_world_with_a_player();
        clock.set(20_000);

        world.deliver_whisper(Vid::new(VID), b"Nobody", b"hello", b"the record".to_vec());
        let _first = inbox.try_recv().expect("the sender was told");
        clock.set(25_000);
        let outcome =
            world.deliver_whisper(Vid::new(VID), b"Nobody", b"again", b"the record".to_vec());
        assert_eq!(outcome, WhisperOutcome::Answered);
        let notice = inbox.try_recv().expect("the sender was told");
        assert_eq!(
            &notice[GC_WHISPER_WIRE_SIZE..],
            b"You have to wait 10 seconds for send offline message.\0",
            "the notice carries the legacy's words"
        );
        assert_eq!(
            world.take_offline_messages(b"Nobody").len(),
            1,
            "the second line was not left"
        );
    }

    #[test]
    fn an_empty_line_leaves_nothing_and_says_nothing() {
        let (mut world, clock, mut inbox) = a_world_with_a_player();
        clock.set(20_000);

        let outcome = world.deliver_whisper(Vid::new(VID), b"Nobody", b"", b"record".to_vec());
        assert_eq!(outcome, WhisperOutcome::Dropped);
        assert!(inbox.try_recv().is_err(), "the sender heard nothing");
        assert!(world.take_offline_messages(b"Nobody").is_empty());
    }

    #[test]
    fn the_lines_waiting_for_a_character_are_read_when_it_enters() {
        let (mut world, clock, _sender) = a_world_with_a_player();
        clock.set(20_000);
        world.offline_messages.insert(
            b"Nobody".to_vec(),
            vec![OfflineMessage {
                from: "P100".to_owned(),
                text: b"hello".to_vec(),
                left_at: 20_000,
            }],
        );
        let (tx, mut inbox) = tokio::sync::mpsc::unbounded_channel();
        world
            .enter_world_with_items(
                Vid::new(OTHER),
                OTHER,
                "Nobody",
                &[],
                crate::client_registry::ClientOutbox::new(tx),
            )
            .expect("the character enters");
        let _list = inbox.try_recv().expect("the enter WAR_LIST is queued");

        world.deliver_offline_messages(Vid::new(OTHER), "Nobody");
        let line = inbox.try_recv().expect("the waiting line was read");
        assert_eq!(line[0], protocol::gc_inventory::HEADER_GC_WHISPER.value());
        assert_eq!(line[3], WHISPER_TYPE_OFFLINE, "the line is an offline one");
        assert_eq!(
            &line[GC_WHISPER_WIRE_SIZE..],
            b"hello\0",
            "the line keeps its bytes and its NUL"
        );
        assert!(
            world.take_offline_messages(b"Nobody").is_empty(),
            "the line was taken away"
        );
    }
}
