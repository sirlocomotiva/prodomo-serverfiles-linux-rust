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

use crate::chat_line::{chat_packet, Arg};
use common::vid::Vid;
use protocol::gc_chat::{CHAT_TYPE_COMMAND, CHAT_TYPE_INFO};
use protocol::gc_whisper::{
    GcWhisper, WHISPER_TYPE_OFFLINE, WHISPER_TYPE_SENDER_BLOCKED, WHISPER_TYPE_SYSTEM,
    WHISPER_TYPE_TARGET_BLOCKED,
};
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
/// What a `/block_chat` line did to a live character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BlockTarget {
    /// The name named nobody online: the caller heard the requested line.
    Offline,
    /// The caller named itself: the quiet `ch != tch` arm.
    Itself,
    /// A row moved on the silence of the returned character.
    Moved(Vid),
}

/// `EBlockAction` (`G/char.h:371-379`): the block-mode bits. Only the whisper and
/// the messenger-invite gates read them here; the exchange, party, guild, request
/// and equipment views are unported. Each bit sits on the character's own mode
/// byte (`m_pointsInstant.bBlockMode`, `G/char.h:471`), which `SetBlockMode`
/// writes and `SetBlockModeForce` pushes without the quest flags
/// (`G/char.cpp:7643-7661`).
pub(super) const BLOCK_EXCHANGE: u8 = 1 << 0;
/// `BLOCK_PARTY_INVITE` (`:373`): 1 << 1.
pub(super) const BLOCK_PARTY_INVITE: u8 = 1 << 1;
/// `BLOCK_GUILD_INVITE` (`:374`): 1 << 2.
pub(super) const BLOCK_GUILD_INVITE: u8 = 1 << 2;
pub(super) const BLOCK_WHISPER: u8 = 1 << 3;
/// `BLOCK_MESSENGER_INVITE` (`G/char.h:376`).
pub(super) const BLOCK_MESSENGER_INVITE: u8 = 1 << 4;
/// `BLOCK_PARTY_REQUEST` (`G/char.h:377`): 1 << 5.
pub(super) const BLOCK_PARTY_REQUEST: u8 = 1 << 5;
/// `BLOCK_VIEW_EQUIPMENT` (`G/char.h:378`): 1 << 6.
pub(super) const BLOCK_VIEW_EQUIPMENT: u8 = 1 << 6;

/// The seven `game_option.block_*` quest-flag writes `SetBlockMode` makes beside
/// the byte, each the flag name and its 0/1 value.
pub(super) type BlockFlagWrites = Vec<(Vec<u8>, i32)>;

/// The `EBlockAction` bits in the order `SetBlockMode` writes their flags
/// (`G/char.cpp:7649-7655`).
const BLOCK_BITS: [u8; 7] = [
    BLOCK_EXCHANGE,
    BLOCK_PARTY_INVITE,
    BLOCK_GUILD_INVITE,
    BLOCK_WHISPER,
    BLOCK_MESSENGER_INVITE,
    BLOCK_PARTY_REQUEST,
    BLOCK_VIEW_EQUIPMENT,
];
/// `str_to_number(flag, arg1)` (`G/cmd_general.cpp:2306-2307`): the byte the
/// `/setblockmode` line names. A non-numeric word reads as zero, and only the
/// low byte survives.
pub(super) fn parse_block_mode(word: &[u8]) -> u8 {
    let mut value: u32 = 0;
    for &byte in word {
        let Some(digit) = byte.checked_sub(b'0').filter(|digit| *digit < 10) else {
            return 0;
        };
        value = value.saturating_mul(10).saturating_add(u32::from(digit));
    }
    value.to_le_bytes()[0]
}

/// `parse_time_str` (`G/utils.cpp:235-285`): the seconds a `do_block_chat` back
/// names. A number just adds to the seconds when the back ends with no unit; an
/// empty back is `0`; a byte that is neither a digit nor `s`, `m`, `h` or `d`
/// refuses the whole line. `None` is that refusal, `Some(0)` a 0 lay: legacy
/// lays it (`G/cmd_gm.cpp:3429-3430`), where it stands on the next tick.
pub(super) fn parse_block_seconds(back: &[u8]) -> Option<i32> {
    let mut tmp: i64 = 0;
    let mut secs: i64 = 0;
    for &byte in back {
        match byte {
            b'm' | b'M' => {
                secs = secs.saturating_add(tmp.saturating_mul(60));
                tmp = 0;
            }
            b'h' | b'H' => {
                secs = secs.saturating_add(tmp.saturating_mul(3_600));
                tmp = 0;
            }
            b'd' | b'D' => {
                secs = secs.saturating_add(tmp.saturating_mul(86_400));
                tmp = 0;
            }
            b's' | b'S' => {
                secs = secs.saturating_add(tmp);
                tmp = 0;
            }
            b'0'..=b'9' => {
                tmp = tmp
                    .saturating_mul(10)
                    .saturating_add(i64::from(byte - b'0'));
            }
            _ => return None,
        }
    }
    Some(i32::try_from(secs.saturating_add(tmp)).unwrap_or(i32::MAX))
}

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
        // `if (ch->FindAffect(AFFECT_BLOCK_CHAT))` (`G/input_main.cpp:364-368`): a
        // silenced sender hears `[LS;652]` and the line goes nowhere, ahead of the
        // self-name drop, the blocks and the target lookup.
        if self.block_chat_seconds_of(vid).is_some() {
            self.answer_silenced_whisper(vid);
            return WhisperOutcome::Answered;
        }
        // `if (pkChr == ch) return (iExtraLen);` (`G/input_main.cpp:372`): the
        // sender's own name is dropped without a word, before any block check.
        if target == Some(vid) {
            return WhisperOutcome::Dropped;
        }
        // `if (ch->IsBlockMode(BLOCK_WHISPER))` (`G/input_main.cpp:387-400`,
        // repeated at `:508-517` once the target's descriptor is known): a
        // sender who blocks whispers hears the sender-blocked record with the
        // target's name in it, and the line goes nowhere.
        if self.is_block_mode(vid, BLOCK_WHISPER) {
            self.answer_blocked(vid, name, WHISPER_TYPE_SENDER_BLOCKED);
            return WhisperOutcome::Answered;
        }
        if let Some(target) = target {
            if self.is_block_mode(target, BLOCK_WHISPER) {
                // `else if (pkChr && pkChr->IsBlockMode(BLOCK_WHISPER))`
                // (`G/input_main.cpp:518-529`): the target blocks whispers, so the
                // sender hears the target-blocked record with the target's name.
                self.answer_blocked(vid, name, WHISPER_TYPE_TARGET_BLOCKED);
                return WhisperOutcome::Answered;
            }
        }
        match target {
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

    /// Tell the sender its line was refused for a block (`G/input_main.cpp:387-400`
    /// and `:518-529`): a record of `whisper_type` whose name is the *target's*
    /// name, with no text. The legacy writes the name twice (`strlcpy` into
    /// `szNameFrom` both times, `:392-393`) and sends the fixed record with no
    /// payload, so the name the client shows is the target's.
    fn answer_blocked(&mut self, vid: common::vid::Vid, name: &[u8], whisper_type: u8) {
        let record = GcWhisper {
            whisper_type,
            name_from: crate::loading_phase::name_bytes_field(name),
            can_format: true,
            text: Vec::new(),
        };
        let Ok(bytes) = record.encode() else {
            warn!(?vid, "a block notice did not fit its record");
            return;
        };
        let _sent = self.write_to_client(vid, bytes);
    }

    /// Tell a silenced sender its line went nowhere (`G/input_main.cpp:364-368`):
    /// `ChatPacket(CHAT_TYPE_INFO, "[LS;652]")`, the same locale key a silenced
    /// talker hears for a zero remaining duration.
    fn answer_silenced_whisper(&mut self, vid: Vid) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            crate::chat::BLOCK_CHAT_NO_DURATION.as_bytes(),
            &[],
        );
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
    /// `CInputDB::ReadOfflineMessages` (`G/input_db.cpp:2455-2476`) refuses a character
    /// that blocks whispers (`:2460`) and otherwise writes one record per line: the
    /// name is the sender's, the text is the line with its NUL, and the type is
    /// `WHISPER_TYPE_OFFLINE`.
    pub(super) fn deliver_offline_messages(&mut self, vid: common::vid::Vid, name: &str) {
        if self.is_block_mode(vid, BLOCK_WHISPER) {
            return;
        }
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

    /// The block mode of the character under `vid` (`m_pointsInstant.bBlockMode`,
    /// `G/char.h:471`), or zero when nobody is there. The whisper gates, the
    /// messenger-invite gates and the exchange, party, guild and party-request gates
    /// read it; the equipment bit rides along unwatched (`do_view_equip` is unported).
    #[must_use]
    pub(super) fn block_mode(&self, vid: Vid) -> u8 {
        self.block_modes.get(&vid).copied().unwrap_or(0)
    }

    /// Whether the character under `vid` blocks `bit` (`CHARACTER::IsBlockMode`,
    /// `G/char.h:899`).
    #[must_use]
    pub(super) fn is_block_mode(&self, vid: Vid, bit: u8) -> bool {
        self.block_mode(vid) & bit != 0
    }

    /// The `game_option.block_*` quest flag for one block bit, which `SetBlockMode`
    /// writes beside the byte (`G/char.cpp:7649-7655`).
    const fn block_flag(bit: u8) -> &'static [u8] {
        match bit {
            BLOCK_EXCHANGE => b"game_option.block_exchange",
            BLOCK_PARTY_INVITE => b"game_option.block_party_invite",
            BLOCK_GUILD_INVITE => b"game_option.block_guild_invite",
            BLOCK_WHISPER => b"game_option.block_whisper",
            BLOCK_MESSENGER_INVITE => b"game_option.block_messenger_invite",
            BLOCK_PARTY_REQUEST => b"game_option.block_party_request",
            _ => b"game_option.block_view_equipment",
        }
    }

    /// `CHARACTER::SetBlockMode` (`G/char.cpp:7643-7656`): write `mode`, chat
    /// `setblockmode %d` to the character's own client, and write the seven
    /// `game_option.block_*` quest flags. The caller answers the character's own
    /// record and the store from the returned byte and flags.
    pub(super) fn set_block_mode(
        &mut self,
        vid: Vid,
        mode: u8,
    ) -> Option<(Vec<u8>, BlockFlagWrites)> {
        let character = self.characters.find_by_vid(vid).ok()?;
        let pid = character.player_id();
        self.block_modes.insert(vid, mode);
        let mut flags = BlockFlagWrites::new();
        if let Some(quests) = self.quests.as_mut() {
            let manager = quests.manager_mut();
            for bit in BLOCK_BITS {
                let value = i32::from(u8::from(mode & bit != 0));
                manager.set_quest_flag(pid, Self::block_flag(bit), value);
                flags.push((Self::block_flag(bit).to_vec(), value));
            }
        } else {
            for bit in BLOCK_BITS {
                flags.push((
                    Self::block_flag(bit).to_vec(),
                    i32::from(u8::from(mode & bit != 0)),
                ));
            }
        }
        let line = self.own_mover(vid).map(|mover| {
            chat_packet(
                mover.recipient(&self.locale),
                CHAT_TYPE_COMMAND,
                b"setblockmode %d",
                &[Arg::Int(i64::from(mode))],
            )
        });
        line.map(|line| (line, flags))
    }

    /// The silence privilege: `GetQuestFlag("chat_privilege.block")` (`G/cmd_gm.cpp:3367`).
    const BLOCK_CHAT_PRIVILEGE: &'static [u8] = b"chat_privilege.block";

    /// The gate of `do_block_chat` (`G/cmd_gm.cpp:3366-3368`): below `GM_HIGH_WIZARD`
    /// the privilege flag opens it; without either the caller hears the
    /// no-such-command line (`G/cmd.cpp:693-698`). Runs before the usage and
    /// format checks, as the legacy runs it before it splits the line.
    pub(super) fn block_chat_allowed(&mut self, vid: Vid) -> bool {
        let gm = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(world::character::Character::points)
            .map_or(
                crate::command::GM_PLAYER,
                world::character::Points::gm_level,
            );
        let privileged = gm < crate::command::GM_HIGH_WIZARD && {
            let pid = self
                .characters
                .find_by_vid(vid)
                .ok()
                .map(world::character::Character::player_id);
            pid.is_some_and(|pid| {
                self.quests
                    .as_ref()
                    .and_then(|quests| quests.manager().quest_flag(pid, Self::BLOCK_CHAT_PRIVILEGE))
                    .is_some_and(|value| value > 0)
            })
        };
        if gm < crate::command::GM_HIGH_WIZARD && !privileged {
            if let Some(mover) = self.own_mover(vid) {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    crate::command::NO_SUCH_COMMAND.as_bytes(),
                    &[],
                );
            }
            return false;
        }
        true
    }

    /// `do_block_chat` past the gate (`G/cmd_gm.cpp:3370-3431`): lay the silence
    /// `name` names for `seconds` ticks. `parse_time_str` (`G/utils.cpp:236-287`)
    /// never yields less than 0, and the parse turns a bare name into a 0 the
    /// same way the 0-spelling lays it: `do_block_chat` has no separate
    /// unsilence arm (`:3429-3430` are its only live-character lines).
    ///
    /// An absent name is the offline arm (`G/cmd_gm.cpp:3401-3418`, whose
    /// game-to-game half is retired, ADR-0001, and whose no-such-name half is a
    /// store write the caller makes): the caller hears the requested line and
    /// nothing moves. The caller's own name lays nothing.
    ///
    /// A lay of more than one tick chats `[LS;852]` to the target first
    /// (`AddAffect`, `G/char_affect.cpp:670-674`), then lays
    /// `AddAffect(223, POINT_NONE, 0, 0, seconds, 0, true)`; a 0 lay stands on
    /// the next tick (`ProcessAffect` ends it on its first run,
    /// `G/char_affect.cpp:424-442`), and is lifted by its own 1-to-0 tick
    /// (a Divergence: legacy's silence never ticks).
    pub(super) fn block_chat(&mut self, vid: Vid, name: &[u8], seconds: i32) -> BlockTarget {
        let target = std::str::from_utf8(name)
            .ok()
            .and_then(|name| self.whisper_target(name));
        let Some(target) = target else {
            // `FindPC` found nobody and the retired game-to-game relay knew
            // nobody either: the caller hears the requested line, as both
            // offline arms end it (`G/cmd_gm.cpp:3414-3418`).
            if let Some(mover) = self.own_mover(vid) {
                self.tell(vid, mover, CHAT_TYPE_INFO, b"Chat block requested", &[]);
            }
            return BlockTarget::Offline;
        };
        if target == vid {
            return BlockTarget::Itself;
        }
        if seconds > 1 {
            if let Some(mover) = self.own_mover(target) {
                self.tell(target, mover, CHAT_TYPE_INFO, b"[LS;852]", &[]);
            }
        }
        self.add_pc_affect(
            target,
            super::affect::Affect {
                kind: crate::loading_phase::AFFECT_BLOCK_CHAT,
                apply_on: 0,
                value: 0,
                flag: 0,
                remaining: seconds,
                sp_cost: 0,
            },
            true,
        );
        BlockTarget::Moved(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_state::fixtures::{a_world, enter, TestClock};
    use protocol::gc_inventory::{HEADER_GC_AFFECT_ADD, HEADER_GC_AFFECT_REMOVE};
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

    /// A character that blocks whispers hears no waiting line when it enters
    /// (`G/input_db.cpp:2460`): the lines stay untaken.
    #[test]
    fn a_character_who_blocks_whispers_reads_no_waiting_line() {
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
        world.block_modes.insert(Vid::new(OTHER), BLOCK_WHISPER);
        world.deliver_offline_messages(Vid::new(OTHER), "Nobody");
        assert!(inbox.try_recv().is_err(), "the waiting line was not read");
        assert_eq!(
            world.take_offline_messages(b"Nobody").len(),
            1,
            "the line stays waiting"
        );
    }

    #[test]
    fn the_byte_is_digits_only_and_only_the_low_byte_survives() {
        assert_eq!(parse_block_mode(b"8"), 8);
        assert_eq!(parse_block_mode(b"264"), 8, "only the low byte survives");
        assert_eq!(parse_block_mode(b""), 0, "checked before parsing");
        assert_eq!(
            parse_block_mode(b"8x"),
            0,
            "a non-numeric word reads as zero"
        );
        assert_eq!(parse_block_mode(b"x"), 0);
    }

    /// Enter the second player and return its inbox, past the enter queue. The
    /// `enter` fixture shows the body, which the chat lines need for their mover.
    fn enter_second_player(world: &mut GameState) -> tokio::sync::mpsc::UnboundedReceiver<Vec<u8>> {
        let mut inbox = enter(world, OTHER, (3200, 3200), 0);
        let _burst = heard(&mut inbox);
        inbox
    }

    /// Drain what one inbox holds.
    fn heard(inbox: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// Raise the caller to a GM that may silence (`GM_HIGH_WIZARD`).
    fn make_gm(world: &mut GameState, vid: u32) {
        let mut caller_points = world
            .characters
            .find_by_vid(Vid::new(vid))
            .expect("the caller is online")
            .points()
            .expect("the caller has points")
            .clone();
        caller_points.set_gm_level(crate::command::GM_HIGH_WIZARD);
        world
            .characters
            .find_by_vid_mut(Vid::new(vid))
            .expect("the caller is online")
            .set_points(Some(caller_points));
    }

    #[test]
    fn a_blocked_sender_hears_the_sender_blocked_record() {
        let (mut world, _clock, mut sender) = a_world_with_a_player();
        let mut target = enter_second_player(&mut world);
        let _second_burst = heard(&mut sender);
        world.block_modes.insert(Vid::new(VID), BLOCK_WHISPER);

        let outcome =
            world.deliver_whisper(Vid::new(VID), b"P101", b"hello", b"the record".to_vec());
        assert_eq!(outcome, WhisperOutcome::Answered);
        let notice = sender.try_recv().expect("the sender was told");
        assert_eq!(notice[0], protocol::gc_inventory::HEADER_GC_WHISPER.value());
        assert_eq!(notice[3], WHISPER_TYPE_SENDER_BLOCKED);
        let mut named = b"P101\0".to_vec();
        named.extend_from_slice(&[0; 20]);
        assert_eq!(
            &notice[4..29],
            named.as_slice(),
            "the record names the target"
        );
        assert!(target.try_recv().is_err(), "the target heard nothing");
    }

    #[test]
    fn a_blocked_target_hears_the_target_blocked_record() {
        let (mut world, _clock, mut sender) = a_world_with_a_player();
        let mut target = enter_second_player(&mut world);
        let _second_burst = heard(&mut sender);
        world.block_modes.insert(Vid::new(OTHER), BLOCK_WHISPER);

        let outcome =
            world.deliver_whisper(Vid::new(VID), b"P101", b"hello", b"the record".to_vec());
        assert_eq!(outcome, WhisperOutcome::Answered);
        let notice = sender.try_recv().expect("the sender was told");
        assert_eq!(notice[3], WHISPER_TYPE_TARGET_BLOCKED);
        assert!(target.try_recv().is_err(), "the target heard nothing");
    }

    #[test]
    fn self_whisper_stays_quiet_even_when_blocked() {
        let (mut world, _clock, mut inbox) = a_world_with_a_player();
        world.block_modes.insert(Vid::new(VID), BLOCK_WHISPER);

        let outcome =
            world.deliver_whisper(Vid::new(VID), b"P100", b"hello", b"the record".to_vec());
        assert_eq!(outcome, WhisperOutcome::Dropped);
        assert!(inbox.try_recv().is_err(), "the sender heard nothing");
    }

    #[test]
    fn set_block_mode_writes_the_byte_the_line_and_the_seven_flags() {
        let (mut world, _clock, _inbox) = a_world_with_a_player();
        let (line, flags) = world
            .set_block_mode(Vid::new(VID), BLOCK_WHISPER | BLOCK_MESSENGER_INVITE)
            .expect("the character is online");
        assert_eq!(
            world.block_mode(Vid::new(VID)),
            BLOCK_WHISPER | BLOCK_MESSENGER_INVITE
        );
        assert!(world.is_block_mode(Vid::new(VID), BLOCK_WHISPER));
        assert!(!world.is_block_mode(Vid::new(VID), BLOCK_EXCHANGE));
        assert_eq!(line[0], protocol::gc_inventory::HEADER_GC_CHAT.value());
        assert!(line
            .windows(b"setblockmode 24".len())
            .any(|window| window == b"setblockmode 24"));
        let names: Vec<&[u8]> = flags.iter().map(|(name, _)| name.as_slice()).collect();
        assert_eq!(
            names,
            [
                b"game_option.block_exchange".as_slice(),
                b"game_option.block_party_invite".as_slice(),
                b"game_option.block_guild_invite".as_slice(),
                b"game_option.block_whisper".as_slice(),
                b"game_option.block_messenger_invite".as_slice(),
                b"game_option.block_party_request".as_slice(),
                b"game_option.block_view_equipment".as_slice(),
            ]
        );
        let values: Vec<i32> = flags.iter().map(|(_, value)| *value).collect();
        assert_eq!(values, [0, 0, 0, 1, 1, 0, 0]);
        assert!(world
            .set_block_mode(Vid::new(9999), BLOCK_WHISPER)
            .is_none());
        assert_eq!(world.block_mode(Vid::new(9999)), 0);
    }

    #[test]
    fn the_back_names_seconds_units_or_refusal() {
        assert_eq!(parse_block_seconds(b""), Some(0), "an empty back is 0");
        assert_eq!(parse_block_seconds(b"0"), Some(0), "the 0 lay");
        assert_eq!(
            parse_block_seconds(b"90"),
            Some(90),
            "a bare number is seconds"
        );
        assert_eq!(parse_block_seconds(b"10s"), Some(10));
        assert_eq!(parse_block_seconds(b"10m"), Some(600));
        assert_eq!(parse_block_seconds(b"2h"), Some(7_200));
        assert_eq!(parse_block_seconds(b"1d"), Some(86_400));
        assert_eq!(parse_block_seconds(b"1m30s"), Some(90));
        assert_eq!(
            parse_block_seconds(b"1M 30S"),
            None,
            "a space refuses the line"
        );
        assert_eq!(
            parse_block_seconds(b"10x"),
            None,
            "an unknown byte refuses the line"
        );
        assert_eq!(parse_block_seconds(b"10m,"), None);
    }

    #[test]
    fn a_player_without_the_privilege_hears_the_no_such_command_line() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let mut target = enter_second_player(&mut world);
        let _enter = heard(&mut caller);

        assert!(!world.block_chat_allowed(Vid::new(VID)));
        let lines = heard(&mut caller);
        assert_eq!(lines.len(), 1, "the refusal is the only line");
        assert!(
            lines[0]
                .windows(crate::command::NO_SUCH_COMMAND.len())
                .any(|window| window == crate::command::NO_SUCH_COMMAND.as_bytes()),
            "the caller hears the no-such-command line"
        );
        assert!(world.block_chat_seconds_of(Vid::new(OTHER)).is_none());
        assert!(target.try_recv().is_err(), "the target heard nothing");
    }

    #[test]
    fn the_gate_runs_before_the_word_is_split() {
        // The legacy gate runs before `one_argument` (`G/cmd_gm.cpp:3366-3370`),
        // so an empty line from a player answers the refusal, never the usage.
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let _enter = heard(&mut caller);
        let (reply, answer) = tokio::sync::oneshot::channel();
        world.answer_block_chat(Vid::new(VID), b"", reply);
        assert_eq!(
            answer.blocking_recv().expect("the answer"),
            crate::game_loop_messages::BlockChatAnswer::Shown
        );
        let lines = heard(&mut caller);
        assert_eq!(lines.len(), 1, "the refusal is the only line");
        assert!(
            lines[0]
                .windows(crate::command::NO_SUCH_COMMAND.len())
                .any(|window| window == crate::command::NO_SUCH_COMMAND.as_bytes()),
            "the gate answers before the split"
        );
    }

    #[test]
    fn a_gm_silences_and_the_target_hears_the_notice_and_the_add() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let mut target = enter_second_player(&mut world);
        make_gm(&mut world, VID);
        let _enter = heard(&mut caller);

        assert_eq!(
            world.block_chat(Vid::new(VID), b"P101", 65),
            BlockTarget::Moved(Vid::new(OTHER))
        );
        assert_eq!(world.block_chat_seconds_of(Vid::new(OTHER)), Some(65));
        assert!(
            caller.try_recv().is_err(),
            "the caller hears nothing on a lay"
        );
        let fanned = heard(&mut target);
        assert_eq!(fanned.len(), 2, "the notice and the add");
        assert!(
            fanned[0].windows(8).any(|window| window == b"[LS;852]"),
            "the notice chats first"
        );
        assert_eq!(
            fanned[1][0],
            HEADER_GC_AFFECT_ADD.value(),
            "the add packet follows"
        );
    }

    #[test]
    fn a_silence_of_one_tick_lays_no_notice() {
        let (mut world, _clock, _caller) = a_world_with_a_player();
        let mut target = enter_second_player(&mut world);
        make_gm(&mut world, VID);

        assert_eq!(
            world.block_chat(Vid::new(VID), b"P101", 1),
            BlockTarget::Moved(Vid::new(OTHER))
        );
        assert_eq!(world.block_chat_seconds_of(Vid::new(OTHER)), Some(1));
        let fanned = heard(&mut target);
        assert_eq!(fanned.len(), 1, "only the add, no `[LS;852]`");
        assert_eq!(fanned[0][0], HEADER_GC_AFFECT_ADD.value());
    }

    #[test]
    fn the_caller_cannot_silence_itself() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        make_gm(&mut world, VID);
        let _enter = heard(&mut caller);

        assert_eq!(
            world.block_chat(Vid::new(VID), b"P100", 60),
            BlockTarget::Itself
        );
        assert!(world.block_chat_seconds_of(Vid::new(VID)).is_none());
        assert!(caller.try_recv().is_err(), "a self-name is quiet");
    }

    #[test]
    fn an_absent_name_is_the_requested_line() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        make_gm(&mut world, VID);
        let _enter = heard(&mut caller);

        assert_eq!(
            world.block_chat(Vid::new(VID), b"Nobody", 60),
            BlockTarget::Offline
        );
        let lines = heard(&mut caller);
        assert_eq!(lines.len(), 1, "the requested line is the only line");
        assert!(
            lines[0]
                .windows(b"Chat block requested".len())
                .any(|window| window == b"Chat block requested"),
            "the caller hears the requested line"
        );
    }

    #[test]
    fn a_zero_lay_overwrites_the_row_and_adds_quietly() {
        // `do_block_chat` has no unsilence arm: a 0 (or a bare name) is the same
        // `AddAffect` every lay is (`G/cmd_gm.cpp:3429-3430`). The override
        // retires the old row first, then the 0-add lands.
        let (mut world, _clock, _caller) = a_world_with_a_player();
        let mut target = enter_second_player(&mut world);
        make_gm(&mut world, VID);
        assert_eq!(
            world.block_chat(Vid::new(VID), b"P101", 65),
            BlockTarget::Moved(Vid::new(OTHER))
        );
        let _lay = heard(&mut target);

        assert_eq!(
            world.block_chat(Vid::new(VID), b"P101", 0),
            BlockTarget::Moved(Vid::new(OTHER))
        );
        assert_eq!(world.block_chat_seconds_of(Vid::new(OTHER)), Some(1));
        let fanned = heard(&mut target);
        assert_eq!(fanned.len(), 2, "the retirement and the add");
        assert_eq!(fanned[0][0], HEADER_GC_AFFECT_REMOVE.value());
        assert_eq!(fanned[1][0], HEADER_GC_AFFECT_ADD.value());
    }

    #[test]
    fn a_zero_lay_with_no_row_adds_quietly() {
        let (mut world, _clock, _caller) = a_world_with_a_player();
        let mut target = enter_second_player(&mut world);
        make_gm(&mut world, VID);

        assert_eq!(
            world.block_chat(Vid::new(VID), b"P101", 0),
            BlockTarget::Moved(Vid::new(OTHER))
        );
        assert_eq!(world.block_chat_seconds_of(Vid::new(OTHER)), Some(1));
        let fanned = heard(&mut target);
        assert_eq!(
            fanned.len(),
            1,
            "only the add, no `[LS;852]` and no `[LS;988]`"
        );
        assert_eq!(fanned[0][0], HEADER_GC_AFFECT_ADD.value());
    }

    #[test]
    fn a_silenced_whisper_is_answered_with_the_locale_key() {
        let (mut world, _clock, mut sender) = a_world_with_a_player();
        let mut target = enter_second_player(&mut world);
        make_gm(&mut world, VID);
        assert_eq!(
            world.block_chat(Vid::new(VID), b"P101", 65),
            BlockTarget::Moved(Vid::new(OTHER))
        );
        let _lay = heard(&mut target);
        let _lay_notice = heard(&mut sender);

        // The silence gates the sender: silence the sender too and whisper.
        world.add_pc_affect(
            Vid::new(VID),
            super::super::affect::Affect {
                kind: crate::loading_phase::AFFECT_BLOCK_CHAT,
                apply_on: 0,
                value: 0,
                flag: 0,
                remaining: 30,
                sp_cost: 0,
            },
            true,
        );
        let _add = heard(&mut sender);
        let outcome =
            world.deliver_whisper(Vid::new(VID), b"P101", b"hello", b"the record".to_vec());
        assert_eq!(outcome, WhisperOutcome::Answered);
        let lines = heard(&mut sender);
        assert_eq!(lines.len(), 1, "the refusal is the only line");
        assert!(
            lines[0].windows(8).any(|window| window == b"[LS;652]"),
            "a silenced sender hears `[LS;652]`"
        );
        assert!(target.try_recv().is_err(), "the target heard nothing");
    }
}
