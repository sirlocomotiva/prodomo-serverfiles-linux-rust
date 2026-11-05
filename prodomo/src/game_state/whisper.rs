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
use crate::item_move::Mover;

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
/// What a `/dc` line did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DcOutcome {
    /// The line was empty: the caller heard the usage.
    Usage,
    /// The name named nobody online: the caller heard the no-such-player line.
    NoSuchPlayer,
    /// The caller named itself: the caller heard the self refusal.
    Itself,
    /// The named client's descriptor was ordered to close.
    Closed(Vid),
}

/// What a `/transfer` line did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TransferOutcome {
    /// The line was empty: the caller heard the usage.
    Usage,
    /// The name named nobody online: the caller heard the no-character line.
    NoSuchCharacter,
    /// The target is on another Channel: the caller heard which one.
    OtherChannel(u8),
    /// The caller named itself: the caller heard the self refusal.
    Itself,
    /// The target warped to the caller's side.
    Warped(Vid),
}

/// What a `/kill` or `/poison` line did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum KillOutcome {
    /// The line was empty: the caller heard the usage.
    Usage,
    /// The name named nobody online: the caller heard the no-such-player line.
    NoSuchPlayer,
    /// The line landed on the returned character.
    Landed(Vid),
}

/// What a `/getqf`, `/setqf` or `/delqf` line did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QfOutcome {
    /// The line missed an argument: the caller heard the syntax.
    Usage,
    /// The name named nobody online: the caller heard the no-character line.
    NoSuchCharacter,
    /// The line landed on the returned character.
    Landed(Vid),
}

/// What a `/stun` line did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StunOutcome {
    /// The line was empty: the caller heard the usage.
    Usage,
    /// The name named nobody online: the caller heard the miss.
    NotInSameMap,
    /// The line landed on the returned character, stunned or already so:
    /// the confirmation went out either way.
    Landed(Vid),
}

/// The 30 ticks of `/stun`'s affect (`Command_ApplyAffect`'s `time`,
/// `G/cmd_gm.cpp:73`).
const STUN_TICKS: i32 = 30;

/// What a `/goto` line did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GotoOutcome {
    /// The line was empty: the caller heard the usage.
    Usage,
    /// The metres landed: the caller shows at the returned pixels.
    Shown((i32, i32)),
    /// The name matched a goto entry: the caller warped to the returned pixels.
    Warped((i32, i32)),
    /// The name matched nothing: silence.
    NoSuchMap,
}

/// What a `/warp` or `/rewarp` line did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WarpOutcome {
    /// The line was empty: the caller heard the usage.
    Usage,
    /// The name named nobody online: the caller heard the miss.
    NoSuchCharacter,
    /// The target is on another Channel: the caller heard which one.
    OtherChannel(u8),
    /// The caller warped to the returned pixels.
    Warped((i32, i32)),
}

/// What a `/private` line did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PrivateOutcome {
    /// The line was empty: the caller heard the usage.
    Usage,
    /// No copy could be made: the caller heard the miss with the index parsed.
    NoSuchMap(i32),
    /// The caller warped to the copy's spawn: the copy's index and the pixels.
    Warped(i32, (i32, i32)),
}

/// `str_to_number(vid)` (`G/cmd_general.cpp:2404-2405`): `strtoul` base 10
/// cast to `u32`. Leading spaces skip, an optional `+`/`-` takes (a `-`
/// negates into the unsigned wrap), digits read until the first non-digit,
/// garbage answers 0, and the running answer wraps as the C multiply does.
fn parse_view_vid(word: &[u8]) -> u32 {
    let Some(offset) = word.iter().position(|byte| !byte.is_ascii_whitespace()) else {
        return 0;
    };
    let rest = &word[offset..];
    let (digits, negative) = match rest.first() {
        Some(b'-') => (&rest[1..], true),
        Some(b'+') => (&rest[1..], false),
        _ => (rest, false),
    };
    let mut value: u32 = 0;
    for byte in digits {
        if !byte.is_ascii_digit() {
            break;
        }
        value = value.wrapping_mul(10).wrapping_add(u32::from(byte - b'0'));
    }
    if negative {
        value.wrapping_neg()
    } else {
        value
    }
}

/// `str_to_number(int)` (`C/utils.h:44-50`): `strtol` base 10, which stops at
/// the first non-digit, clamps past the range, and skips leading spaces and
/// one sign.
pub(super) fn str_to_int(text: &[u8]) -> i32 {
    let mut bytes = text.iter().peekable();
    while bytes.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
        bytes.next();
    }
    let mut negative = false;
    match bytes.peek() {
        Some(b'+') => {
            bytes.next();
        }
        Some(b'-') => {
            negative = true;
            bytes.next();
        }
        _ => {}
    }
    let mut value: i64 = 0;
    for byte in bytes {
        let Some(digit) = byte.checked_sub(b'0').filter(|digit| *digit < 10) else {
            break;
        };
        value = value.saturating_mul(10).saturating_add(i64::from(digit));
    }
    let value = if negative { -value } else { value };
    i32::try_from(value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)))
        .expect("the clamp holds the value inside i32")
}

/// `FindInString(find, in)` (`G/cmd_gm.cpp:205-234`): whether the typed `find`
/// is a case-insensitive prefix of one `|` segment of the table's `name`. An
/// empty `find` matches (`strncasecmp` over 0 bytes).
fn find_in_string(find: &[u8], name: &[u8]) -> bool {
    name.split(|byte| *byte == b'|').any(|segment| {
        segment.len() >= find.len()
            && segment
                .iter()
                .zip(find.iter())
                .all(|(cell, want)| cell.eq_ignore_ascii_case(want))
    })
}

/// `CHARACTER_GoToName`'s scan (`G/cmd_gm.cpp:236-262`): the first entry of
/// `table` whose map takes `map_index` (`#n`, else the typed name is a
/// [`find_in_string`] prefix of its name) and whose empire is 0 or `empire`,
/// as pixels. `CHARACTER_AddGotoInfo` narrows the empire to its low byte
/// (`BYTE`, `:191`) and the metres wrap into pixels (`x * 100`, `:253-254`).
fn goto_hit(
    table: &[quest::host::GotoInfo],
    map_index: i32,
    empire: u8,
    name: &[u8],
) -> Option<(i32, i32)> {
    table.iter().find_map(|info| {
        if map_index != 0 {
            if map_index != info.map_index {
                return None;
            }
        } else if !find_in_string(name, &info.name) {
            return None;
        }
        let entry_empire =
            u8::try_from(info.empire & 0xFF).expect("the mask holds the value inside u8");
        (entry_empire == 0 || entry_empire == empire)
            .then(|| (info.x.wrapping_mul(100), info.y.wrapping_mul(100)))
    })
}

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
/// What `do_hide_costume` did.
pub(super) enum HideCostumeOutcome {
    /// The part hid or showed: the `SetXCostumeHidden` line, the look update, and the
    /// `costume_option.hide_*` flag write for the store.
    Hid {
        /// The `SetXCostumeHidden` command line for the client's own queue.
        line: Vec<u8>,
        /// The look update for the character and everyone that sees it.
        update: Vec<u8>,
        /// The flag write for the store.
        flag: (Vec<u8>, i32),
    },
    /// The character moved, fought, or is mid-move: the `[LS;1971]` refusal line.
    Refused(Vec<u8>),
}

/// `[LS;1971]`: the line `do_hide_costume` chats when the character is mid-move or
/// fought or moved within 1.5 seconds (`G/cmd_general.cpp:3956-3961`).
const HIDE_WHILE_MOVING: &[u8] = b"[LS;1971]";

/// The `SetXCostumeHidden %d` format of part 1-5 (`G/char.cpp:11382-11413`).
pub(super) fn hide_chat_format(part: u8) -> &'static [u8] {
    match part {
        1 => b"SetBodyCostumeHidden %d",
        2 => b"SetHairCostumeHidden %d",
        3 => b"SetSashCostumeHidden %d",
        4 => b"SetAuraCostumeHidden %d",
        _ => b"SetWeaponCostumeHidden %d",
    }
}

/// The `costume_option.hide_*` quest flag of part 1-5 (`G/char.cpp:11382-11413`).
fn hide_flag_name(part: u8) -> &'static [u8] {
    match part {
        1 => b"costume_option.hide_body",
        2 => b"costume_option.hide_hair",
        3 => b"costume_option.hide_sash",
        4 => b"costume_option.hide_aura",
        _ => b"costume_option.hide_weapon",
    }
}

/// `(BYTE)strtoul(word)` (`common/utils.h:20-26`): the part or hide byte a
/// `/hide_costume` word names. Leading spaces are already gone, an optional sign leads,
/// digits run to the first non-digit, and only the low byte survives; a word with no
/// digits reads as zero, and a magnitude past `u64::MAX` clamps there first.
pub(super) fn parse_hide_number(word: &[u8]) -> u8 {
    let (negative, digits) = match word.split_first() {
        Some((b'-', rest)) => (true, rest),
        Some((b'+', rest)) => (false, rest),
        _ => (false, word),
    };
    let mut value: u64 = 0;
    let mut any = false;
    for &byte in digits {
        let digit = byte.wrapping_sub(b'0');
        if digit >= 10 {
            break;
        }
        any = true;
        value = value.saturating_mul(10).saturating_add(u64::from(digit));
    }
    if !any {
        return 0;
    }
    let low = u8::try_from(value & 0xff).expect("a masked byte fits");
    if negative {
        low.wrapping_neg()
    } else {
        low
    }
}

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
    /// The store's answer takes the lines away as it sends them
    /// (`CClientManager::RequestReadOfflineMessages`,
    /// `server/server/db/ClientManager.cpp:4613-4629`, whose erase is at `:4628`), and
    /// only then does `CInputDB::ReadOfflineMessages` (`G/input_db.cpp:2455-2476`)
    /// refuse a character that blocks whispers (`:2460`): the refused lines are gone.
    /// Otherwise it writes one record per line: the name is the sender's, the text is
    /// the line with its NUL, and the type is `WHISPER_TYPE_OFFLINE`.
    pub(super) fn deliver_offline_messages(&mut self, vid: common::vid::Vid, name: &str) {
        let messages = self.take_offline_messages(name.as_bytes());
        if self.is_block_mode(vid, BLOCK_WHISPER) {
            return;
        }
        for message in messages {
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
    /// (`server/server/db/ClientManager.cpp:4637-4654`) drops them hourly. Its test is
    /// `duration_cast<hours>(age).count() >= iHour` (`:4648`), and the cast truncates, so
    /// a line goes at five hours to the millisecond.
    pub(super) fn expire_offline_messages(&mut self) {
        let now = self.clock.now();
        self.offline_messages.retain(|_, messages| {
            messages
                .retain(|message| now.wrapping_sub(message.left_at) < OFFLINE_MESSAGE_LIFETIME_MS);
            !messages.is_empty()
        });
    }

    /// Forget the whisper state of the character under `vid`, as its departure does.
    ///
    /// Every enter is a new `CHARACTER`, and `CHARACTER::Initialize` zeroes both values
    /// it holds here: the block mode with the rest of `m_pointsInstant`
    /// (`G/char.cpp:252`), and the offline-line clock (`dwLastOfflinePMTime = 0`,
    /// `:270-271`). The VID is the player id, so the next session would find either
    /// under the same key. Only `/setblockmode` and the quest login's
    /// `pc.send_block_mode` write the byte again.
    pub(super) fn forget_whisper_session(&mut self, vid: Vid) {
        let _clock = self.last_offline_pm.remove(&vid);
        let _mode = self.block_modes.remove(&vid);
        self.forget_logs(vid);
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
    /// `do_hide_costume` (`G/cmd_general.cpp:3934-3976`) for the character under `vid`:
    /// part 1-5 hides or shows its costume. The mask bit lands, the parts recompute, the
    /// `costume_option.hide_*` quest flag follows, and the caller sends the answered chat
    /// line and look update. Answers `None` when the character is not online, is mid-move
    /// or fought or moved within 1.5 seconds (legacy chats `[LS;1971]` then — the caller
    /// sends the answered refusal instead), or names no part 1-5.
    pub(super) fn hide_costume(
        &mut self,
        vid: Vid,
        part: u8,
        hide: bool,
    ) -> Option<HideCostumeOutcome> {
        let character = self.characters.find_by_vid(vid).ok()?;
        let pid = character.player_id();
        let now = self.clock.now();
        let body = self.bodies.get(&vid)?;
        let attacking = body
            .last_attack
            .is_some_and(|at| now.wrapping_sub(at) < 1_500);
        let moving = body.motion.moving || now.wrapping_sub(body.last_move) < 1_500;
        let mover = self.own_mover(vid)?;
        if attacking || moving {
            let line = chat_packet(
                mover.recipient(&self.locale),
                CHAT_TYPE_INFO,
                HIDE_WHILE_MOVING,
                &[],
            );
            return Some(HideCostumeOutcome::Refused(line));
        }
        if !(1..=5).contains(&part) {
            return None;
        }
        if let Ok(character) = self.characters.find_by_vid_mut(vid) {
            let (items, points) = character.items_and_points_mut();
            if let Some(points) = points {
                points.set_hidden(part, hide);
                let equipment = world::character::Equipment::of(items, &self.protos);
                let parts = points.hidden_parts(&equipment);
                points.set_parts(parts);
            }
        }
        let name = hide_flag_name(part);
        let value = i32::from(hide);
        if let Some(quests) = self.quests.as_mut() {
            quests.manager_mut().set_quest_flag(pid, name, value);
        }
        let line = chat_packet(
            mover.recipient(&self.locale),
            CHAT_TYPE_COMMAND,
            hide_chat_format(part),
            &[Arg::Int(i64::from(hide))],
        );
        let flags = self.affect_flags_of(vid);
        let update = self.update_frame(vid, flags)?;
        Some(HideCostumeOutcome::Hid {
            line,
            update,
            flag: (name.to_vec(), value),
        })
    }

    /// The silence privilege: `GetQuestFlag("chat_privilege.block")` (`G/cmd_gm.cpp:3367`).
    const BLOCK_CHAT_PRIVILEGE: &'static [u8] = b"chat_privilege.block";

    /// The gate of `do_block_chat` (`G/cmd_gm.cpp:3366-3369`): below `GM_HIGH_WIZARD`
    /// the privilege flag opens it; without either the caller hears `there is no
    /// such command`, which no country table carries, so it goes out as written
    /// (`ChatPacket`'s locale lookup, `G/char.cpp:5147-5154`). Runs before the
    /// usage and format checks, as the legacy runs it before it splits the line.
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
                self.tell(vid, mover, CHAT_TYPE_INFO, b"there is no such command", &[]);
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

    /// `ACMD(do_block_chat_list)` (`G/cmd_gm.cpp:3298-3310`, answered in
    /// `G/db.cpp:572-592`): the caller hears one `%s %s sec` line per silenced
    /// character, or `No one currently blocked`. The walk is the character
    /// manager's name index, which stores names lower-cased (`HashMap`, not
    /// legacy's `player` row order); neither line is in any country table, so
    /// both go out as written (`ChatPacket`'s locale lookup,
    /// `G/char.cpp:5147-5154`). The gate is `block_chat_allowed`'s: the same
    /// `HIGH_WIZARD` level and the same privilege flag (`:3301-3305` reuse
    /// `:3366-3368`'s line).
    pub(super) fn block_chat_list(&mut self, vid: Vid) {
        if !self.block_chat_allowed(vid) {
            return;
        }
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let mut rows = Vec::new();
        for (name, target) in self.characters.player_names() {
            if let Some(seconds) = self.block_chat_seconds_of(target) {
                rows.push((name.as_bytes().to_vec(), seconds));
            }
        }
        if rows.is_empty() {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"No one currently blocked", &[]);
            return;
        }
        for (name, seconds) in &rows {
            let duration = seconds.to_string();
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"%s %s sec",
                &[Arg::Text(name), Arg::Text(duration.as_bytes())],
            );
        }
    }

    /// `do_disconnect` (`G/cmd_gm.cpp:1755-1782`): close the named character's
    /// descriptor. An empty line answers the usage; an unknown name answers
    /// `%s: no such a player.` with the name as typed; the caller's own name
    /// answers `cannot disconnect myself`. Else the target's descriptor is
    /// ordered to close (`DESC_MANAGER::DestroyDesc`, `:1781`), which
    /// disconnects the character and saves its row on the way out. The lookup
    /// is the descriptor set's exact match (`name_with_desc_func`,
    /// `G/desc_manager.cpp:315-329`), not `FindPC`'s lowered one: a name in
    /// another case names nobody. The GM gate (`GM_LOW_WIZARD`) is the
    /// dispatcher's, not this reducer's.
    pub(super) fn disconnect(&mut self, vid: Vid, rest: &[u8]) -> DcOutcome {
        let (name, _) = crate::command::one_argument(rest);
        let Some(mover) = self.own_mover(vid) else {
            return DcOutcome::Usage;
        };
        if name.is_empty() {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"ex) /dc <player name>", &[]);
            return DcOutcome::Usage;
        }
        let target = std::str::from_utf8(&name)
            .ok()
            .and_then(|name| self.player_exactly_named(name));
        let Some(target) = target else {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"%s: no such a player.",
                &[Arg::Text(&name)],
            );
            return DcOutcome::NoSuchPlayer;
        };
        if target == vid {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"cannot disconnect myself", &[]);
            return DcOutcome::Itself;
        }
        self.order_close(target);
        DcOutcome::Closed(target)
    }

    /// The VID of the online PC whose shown name is exactly `name`, for
    /// `do_disconnect`'s descriptor-set lookup (`G/desc_manager.cpp:325`).
    fn player_exactly_named(&self, name: &str) -> Option<Vid> {
        self.characters
            .player_names()
            .into_iter()
            .find(|(shown, _)| *shown == name)
            .map(|(_, vid)| vid)
    }

    /// Order the descriptor holding `target` to close, through the client
    /// registry, as `DestroyDesc` does from the game thread's side.
    pub(super) fn order_close(&self, target: Vid) {
        let raw = target.raw();
        let Some(place) = self.place_of(raw) else {
            warn!(vid = raw, "a disconnect found no place the target stands");
            return;
        };
        let Some(clients) = self.clients.as_ref() else {
            warn!(vid = raw, "a disconnect has no client registry to reach");
            return;
        };
        let ordered = clients
            .members_on_map(place.0, place.1)
            .into_iter()
            .find(|(_, entry)| entry.vid == raw)
            .is_some_and(|(id, _)| {
                clients.order(place.0, id, crate::client_registry::ClientOrder::Close)
            });
        if !ordered {
            warn!(vid = raw, "a disconnect found no client to order");
        }
    }

    /// `do_transfer` (`G/cmd_gm.cpp:102-151`): warp the named character to the
    /// caller's side. An empty line answers the usage; an unknown name answers
    /// `There is no character(%s) by that name` with the name as typed; a
    /// target on another Channel answers `Target(%s) is in %d channel (my
    /// channel %d)`; the caller's own name answers `Transfer me?!?`. Else the
    /// target warps to the caller's feet (`tch->WarpSet(ch->GetX(),
    /// ch->GetY(), ch->GetMapIndex())`, `:150`). The lookup is `FindPC`'s
    /// lowered one (`G/char_manager.cpp:244-260`); the cross-Channel arm is
    /// the P2P one (`P2P_MANAGER::Find`, `:116`), which the client registry
    /// answers here (ADR-0001 retires the GG relay, so `Transfer requested`
    /// has no arm: a same-Channel target the world missed is a caller-side
    /// miss too, and a missed same-Channel name answers the no-character
    /// line). The GM gate (`GM_LOW_WIZARD`) is the dispatcher's.
    pub(super) fn transfer(&mut self, vid: Vid, rest: &[u8]) -> TransferOutcome {
        let (name, _) = crate::command::one_argument(rest);
        let Some(mover) = self.own_mover(vid) else {
            return TransferOutcome::Usage;
        };
        if name.is_empty() {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Usage: transfer <name>", &[]);
            return TransferOutcome::Usage;
        }
        if let Some(target) = std::str::from_utf8(&name)
            .ok()
            .and_then(|name| self.whisper_target(name))
        {
            if target == vid {
                self.tell(vid, mover, CHAT_TYPE_INFO, b"Transfer me?!?", &[]);
                return TransferOutcome::Itself;
            }
            let mine = self.bodies.get(&vid).map_or(0, |body| body.channel);
            let theirs = self.bodies.get(&target).map_or(mine, |body| body.channel);
            if theirs != mine {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"Target(%s) is in %d channel (my channel %d)",
                    &[
                        Arg::Text(&name),
                        Arg::Int(i64::from(theirs)),
                        Arg::Int(i64::from(mine)),
                    ],
                );
                return TransferOutcome::OtherChannel(theirs);
            }
            let feet = self
                .bodies
                .get(&vid)
                .map_or((0, 0), |body| body.motion.dest);
            self.order_transfer(target, feet);
            return TransferOutcome::Warped(target);
        }
        if let Some(channel) = std::str::from_utf8(&name).ok().and_then(|name| {
            self.clients
                .as_ref()
                .and_then(|clients| clients.entry_of(name))
                .map(|(_, entry, _)| entry.channel)
        }) {
            let mine = self.bodies.get(&vid).map_or(0, |body| body.channel);
            if channel != mine {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"Target(%s) is in %d channel (my channel %d)",
                    &[
                        Arg::Text(&name),
                        Arg::Int(i64::from(channel)),
                        Arg::Int(i64::from(mine)),
                    ],
                );
                return TransferOutcome::OtherChannel(channel);
            }
        }
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"There is no character(%s) by that name",
            &[Arg::Text(&name)],
        );
        TransferOutcome::NoSuchCharacter
    }

    /// Warp `target` to `feet` through the client registry, as `WarpSet` does
    /// from the game thread's side.
    fn order_transfer(&self, target: Vid, feet: (i32, i32)) {
        let raw = target.raw();
        let Some(place) = self.place_of(raw) else {
            warn!(vid = raw, "a transfer found no place the target stands");
            return;
        };
        let Some(clients) = self.clients.as_ref() else {
            warn!(vid = raw, "a transfer has no client registry to reach");
            return;
        };
        let ordered = clients
            .members_on_map(place.0, place.1)
            .into_iter()
            .find(|(_, entry)| entry.vid == raw)
            .is_some_and(|(id, _)| {
                clients.order(
                    place.0,
                    id,
                    crate::client_registry::ClientOrder::Warp {
                        x: feet.0,
                        y: feet.1,
                        private_map_index: 0,
                        channel: None,
                        exit: None,
                        sorted_until: self.sort_lock(target),
                    },
                )
            });
        if !ordered {
            warn!(vid = raw, "a transfer found no client to order");
        }
    }

    /// `do_kill` (`G/cmd_gm.cpp:1784-1805`): the named character dies. An
    /// empty line answers `ex) /kill <player name>`; an unknown name answers
    /// `%s: no such a player` (no trailing stop, unlike `/dc`'s line) with
    /// the name as typed. Else the target dies by `Dead()` (`:1804`), with
    /// no killer but the PID it kept. The lookup is the descriptor set's
    /// exact match (`name_with_desc_func`, `G/desc_manager.cpp:315-329`),
    /// like `/dc`'s. The GM gate (`GM_HIGH_WIZARD`) is the dispatcher's.
    pub(super) fn kill(&mut self, vid: Vid, rest: &[u8]) -> KillOutcome {
        let target = self.kill_target(vid, rest, b"ex) /kill <player name>");
        let Some(target) = target else {
            return if crate::command::one_argument(rest).0.is_empty() {
                KillOutcome::Usage
            } else {
                KillOutcome::NoSuchPlayer
            };
        };
        self.dead_pc(target, None);
        KillOutcome::Landed(target)
    }

    /// `do_getqf` (`G/cmd_gm.cpp:2863-2888`): the target's quest flags, one
    /// line each, as `SendFlagList` writes them (`G/questpc.cpp:711-726`): a
    /// `.__status` flag prints `quest state (value)` with the state's name,
    /// any other prints `name value`. An empty line names the caller; an
    /// unknown name answers `There is no such character`. The lookup is
    /// `FindPC`'s lowered one (`G/char_manager.cpp:244-260`). With no quest
    /// record the target holds no flag, so nothing answers. The GM gate
    /// (`GM_LOW_WIZARD`) is the dispatcher's.
    pub(super) fn getqf(&mut self, vid: Vid, rest: &[u8]) -> QfOutcome {
        let (name, _) = crate::command::one_argument(rest);
        let Some(mover) = self.own_mover(vid) else {
            return QfOutcome::Usage;
        };
        let target = if name.is_empty() {
            vid
        } else {
            let found = std::str::from_utf8(&name)
                .ok()
                .and_then(|name| self.whisper_target(name));
            let Some(found) = found else {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"There is no such character",
                    &[],
                );
                return QfOutcome::NoSuchCharacter;
            };
            found
        };
        let Some(quests) = self.quests.as_ref() else {
            return QfOutcome::Landed(target);
        };
        let pid = self
            .characters
            .find_by_vid(target)
            .map_or(0, world::character::Character::player_id);
        let mut flags = quests.manager().flags_of(pid);
        flags.sort_by(|left, right| left.0.cmp(&right.0));
        for (name, value) in &flags {
            if let Some(quest) = name.strip_suffix(b".__status") {
                let state = quests.manager().state_name(quest, *value);
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"%s %s (%d)",
                    &[
                        Arg::Text(quest),
                        Arg::Text(&state),
                        Arg::Int(i64::from(*value)),
                    ],
                );
            } else {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"%s %d",
                    &[Arg::Text(name), Arg::Int(i64::from(*value))],
                );
            }
        }
        QfOutcome::Landed(target)
    }

    /// `do_setqf` (`G/cmd_gm.cpp:2958-2992`): sets the target's quest flag to
    /// the line's value and chats `Quest flag set: %s %d`. A missing flag
    /// name answers the syntax; an unknown target name answers `There is no
    /// such character`. The value is a bare `str_to_number`, 0 when the word
    /// holds no digits. The lookup is `FindPC`'s lowered one. The GM gate
    /// (`GM_LOW_WIZARD`) is the dispatcher's.
    pub(super) fn setqf(&mut self, vid: Vid, rest: &[u8]) -> QfOutcome {
        let (flag, tail) = crate::command::one_argument(rest);
        let (word, tail) = crate::command::one_argument(tail);
        let (name, _) = crate::command::one_argument(tail);
        let Some(mover) = self.own_mover(vid) else {
            return QfOutcome::Usage;
        };
        if flag.is_empty() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Syntax: setqf <flagname> <value> [<character name>]",
                &[],
            );
            return QfOutcome::Usage;
        }
        let target = if name.is_empty() {
            vid
        } else {
            let found = std::str::from_utf8(&name)
                .ok()
                .and_then(|name| self.whisper_target(name));
            let Some(found) = found else {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"There is no such character",
                    &[],
                );
                return QfOutcome::NoSuchCharacter;
            };
            found
        };
        let pid = self
            .characters
            .find_by_vid(target)
            .map_or(0, world::character::Character::player_id);
        let value = str_to_int(&word);
        if let Some(quests) = self.quests.as_mut() {
            quests.manager_mut().set_quest_flag(pid, &flag, value);
        }
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"Quest flag set: %s %d",
            &[Arg::Text(&flag), Arg::Int(i64::from(value))],
        );
        QfOutcome::Landed(target)
    }

    /// `do_delqf` (`G/cmd_gm.cpp:2994-3027`): deletes the target's quest flag.
    /// A missing flag name answers the syntax; an unknown target name answers
    /// `There is no such character`. A flag that was there chats `Success`,
    /// one that was not chats `Failed, quest flag does not exist`. The
    /// lookup is `FindPC`'s lowered one. The GM gate (`GM_LOW_WIZARD`) is the
    /// dispatcher's.
    pub(super) fn delqf(&mut self, vid: Vid, rest: &[u8]) -> QfOutcome {
        let (flag, tail) = crate::command::one_argument(rest);
        let (name, _) = crate::command::one_argument(tail);
        let Some(mover) = self.own_mover(vid) else {
            return QfOutcome::Usage;
        };
        if flag.is_empty() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Syntax: delqf <flagname> [<character name>]",
                &[],
            );
            return QfOutcome::Usage;
        }
        let target = if name.is_empty() {
            vid
        } else {
            let found = std::str::from_utf8(&name)
                .ok()
                .and_then(|name| self.whisper_target(name));
            let Some(found) = found else {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"There is no such character",
                    &[],
                );
                return QfOutcome::NoSuchCharacter;
            };
            found
        };
        let pid = self
            .characters
            .find_by_vid(target)
            .map_or(0, world::character::Character::player_id);
        let deleted = self
            .quests
            .as_mut()
            .is_some_and(|quests| quests.manager_mut().delete_quest_flag(pid, &flag));
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            if deleted {
                b"Success"
            } else {
                b"Failed, quest flag does not exist"
            },
            &[],
        );
        QfOutcome::Landed(target)
    }

    /// `do_set_state` (`G/cmd_gm.cpp:2891-2956`): the target's quest moves to
    /// the named state and the caller hears `setting quest state flag %s %s
    /// %d` with the quest name twice and the state's number. A missing name
    /// answers the syntax; an unknown target name answers `There is no such
    /// character`. An unknown quest or state is number 0, as legacy's
    /// `GetQuestStateIndex` answers. The lookup is `FindPC`'s lowered one.
    /// The GM gate (`GM_LOW_WIZARD`) is the dispatcher's.
    pub(super) fn set_state(&mut self, vid: Vid, rest: &[u8]) -> QfOutcome {
        let (quest, tail) = crate::command::one_argument(rest);
        let (state, tail) = crate::command::one_argument(tail);
        let (name, _) = crate::command::one_argument(tail);
        let Some(mover) = self.own_mover(vid) else {
            return QfOutcome::Usage;
        };
        if quest.is_empty() || state.is_empty() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Syntax: set_state <questname> <statename> [<character name>]",
                &[],
            );
            return QfOutcome::Usage;
        }
        let target = if name.is_empty() {
            vid
        } else {
            let found = std::str::from_utf8(&name)
                .ok()
                .and_then(|name| self.whisper_target(name));
            let Some(found) = found else {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"There is no such character",
                    &[],
                );
                return QfOutcome::NoSuchCharacter;
            };
            found
        };
        let pid = self
            .characters
            .find_by_vid(target)
            .map_or(0, world::character::Character::player_id);
        let value = self
            .quests
            .as_ref()
            .map_or(0, |quests| quests.manager().state_index(&quest, &state));
        if let Some(quests) = self.quests.as_mut() {
            quests.manager_mut().set_quest_state(pid, &quest, value);
        }
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"setting quest state flag %s %s %d",
            &[
                Arg::Text(&quest),
                Arg::Text(&quest),
                Arg::Int(i64::from(value)),
            ],
        );
        QfOutcome::Landed(target)
    }

    /// `do_qf` (`G/cmd_gm.cpp:2326-2364`): the caller's current quest moves
    /// to the named state and the caller hears `setting quest state flag %s
    /// %s %d` with the quest's name, the state's name, and the state's
    /// number. An empty line is silent. A caller that ran no quest hears
    /// `setting quest state flag failed`. An unknown state is number 0, as
    /// legacy's `GetQuestStateIndex` answers. The GM gate
    /// (`GM_IMPLEMENTOR`) is the dispatcher's.
    pub(super) fn qf(&mut self, vid: Vid, rest: &[u8]) -> QfOutcome {
        let (state, _) = crate::command::one_argument(rest);
        let Some(mover) = self.own_mover(vid) else {
            return QfOutcome::Usage;
        };
        if state.is_empty() {
            return QfOutcome::Usage;
        }
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        let quest = self
            .quests
            .as_ref()
            .map_or_else(Vec::new, |quests| quests.manager().current_quest_name(pid));
        if quest.is_empty() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"setting quest state flag failed",
                &[],
            );
            return QfOutcome::Landed(vid);
        }
        let value = self
            .quests
            .as_ref()
            .map_or(0, |quests| quests.manager().state_index(&quest, &state));
        if let Some(quests) = self.quests.as_mut() {
            quests.manager_mut().set_quest_state(pid, &quest, value);
        }
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"setting quest state flag %s %s %d",
            &[
                Arg::Text(&quest),
                Arg::Text(&state),
                Arg::Int(i64::from(value)),
            ],
        );
        QfOutcome::Landed(vid)
    }

    /// `do_clear_quest` (`G/cmd_gm.cpp:3732-3743`): the caller's quest
    /// forgets every flag and rests at 0 (`PC::ClearQuest`,
    /// `G/questpc.cpp:681-701`), silent. An empty line is silent. The GM gate
    /// (`GM_HIGH_WIZARD`) is the dispatcher's.
    pub(super) fn clear_quest(&mut self, vid: Vid, rest: &[u8]) -> QfOutcome {
        let (quest, _) = crate::command::one_argument(rest);
        if quest.is_empty() {
            return QfOutcome::Usage;
        }
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        if let Some(quests) = self.quests.as_mut() {
            quests.manager_mut().clear_quest(pid, &quest);
        }
        QfOutcome::Landed(vid)
    }

    /// The 22 wear positions `SendEquipment` walks, in its order
    /// (`G/char.cpp:7819-7821`).
    const VIEW_EQUIP_ORDER: [u16; protocol::gc_view_equip::VIEW_EQUIP_SLOTS] = [
        common::enums::EWearPositions::Body as u16,
        common::enums::EWearPositions::Head as u16,
        common::enums::EWearPositions::Foots as u16,
        common::enums::EWearPositions::Wrist as u16,
        common::enums::EWearPositions::Weapon as u16,
        common::enums::EWearPositions::Neck as u16,
        common::enums::EWearPositions::Ear as u16,
        common::enums::EWearPositions::Unique1 as u16,
        common::enums::EWearPositions::Unique2 as u16,
        common::enums::EWearPositions::Arrow as u16,
        common::enums::EWearPositions::Shield as u16,
        common::enums::EWearPositions::CostumeBody as u16,
        common::enums::EWearPositions::CostumeHair as u16,
        common::enums::EWearPositions::CostumeMount as u16,
        common::enums::EWearPositions::CostumeWeapon as u16,
        common::enums::EWearPositions::CostumeSash as u16,
        common::enums::EWearPositions::CostumeAura as u16,
        common::enums::EWearPositions::Ring1 as u16,
        common::enums::EWearPositions::Ring2 as u16,
        common::enums::EWearPositions::Belt as u16,
        common::enums::EWearPositions::CostumePet as u16,
        common::enums::EWearPositions::CostumeSashSkin as u16,
    ];

    /// `do_view_equip` (`G/cmd_general.cpp:2395-2426`): the caller views the
    /// VID's worn equipment. An empty line, an unknown VID, or a VIC that is
    /// not a player is silent. A target that blocks the equipment view hears
    /// `[i] Player %s blocked the equipment view system`; a game master —
    /// any non-player level, or every character in the test-server mode
    /// (`CHARACTER::IsGM`, `G/char.cpp:6373-6380`) — hears `[i] Player %s
    /// has this function blocked`. Else the caller's descriptor gets the
    /// `HEADER_GC_VIEW_EQUIP` record (`SendEquipment`, `G/char.cpp:7815-7846`).
    pub(super) fn view_equip(&mut self, vid: Vid, rest: &[u8]) -> QfOutcome {
        let (word, _) = crate::command::one_argument(rest);
        let Some(mover) = self.own_mover(vid) else {
            return QfOutcome::Usage;
        };
        if word.is_empty() {
            return QfOutcome::Usage;
        }
        let target = Vid::new(parse_view_vid(&word));
        let Ok(character) = self.characters.find_by_vid(target) else {
            return QfOutcome::Landed(vid);
        };
        if character.kind() != world::character::CharacterKind::Player {
            return QfOutcome::Landed(vid);
        }
        let name = character.name().as_bytes().to_vec();
        if self.is_block_mode(target, BLOCK_VIEW_EQUIPMENT) {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"[i] Player %s blocked the equipment view system",
                &[Arg::Text(&name)],
            );
            return QfOutcome::Landed(vid);
        }
        let target_gm = self
            .characters
            .find_by_vid(target)
            .ok()
            .and_then(|character| character.points())
            .is_some_and(|points| points.gm_level() != crate::command::GM_PLAYER)
            || self.test_server;
        if target_gm {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"[i] Player %s has this function blocked",
                &[Arg::Text(&name)],
            );
            return QfOutcome::Landed(vid);
        }
        let character = self
            .characters
            .find_by_vid(target)
            .expect("the target was just there");
        let items = character.items();
        let mut equips = [protocol::gc_view_equip::ViewEquipCell::default();
            protocol::gc_view_equip::VIEW_EQUIP_SLOTS];
        for (slot, wear) in Self::VIEW_EQUIP_ORDER.iter().enumerate() {
            let Some(item) = items.item_at(world::character::wear_pos(*wear)) else {
                continue;
            };
            equips[slot] = protocol::gc_view_equip::ViewEquipCell {
                vnum: item.vnum,
                count: item.count,
                transmutation: item.transmutation,
                refine_element: item.refine_element,
                sockets: item.sockets,
                attributes: item.attributes,
            };
        }
        let record = protocol::gc_view_equip::GcViewEquip {
            vid: target.raw(),
            equips,
        }
        .encode();
        let _sent = self.write_to_client(vid, record);
        QfOutcome::Landed(vid)
    }

    /// `do_stun` (`G/cmd_gm.cpp:87-90`, through `Command_ApplyAffect`,
    /// `:50-83`): the named character is stunned. An empty line answers
    /// `Usage: stun <name>`; an unknown name answers `%s is not in same map`
    /// with the name as typed. Else the target takes the stun affect
    /// (`SkillAttackAffect` at 1000/1000 past the affect-flag and
    /// `IMMUNE_STUN` gates, `G/battle.h:64-91`) and the caller hears the
    /// confirmation `%s stun`. The lookup is `FindPC`'s lowered one
    /// (`G/char_manager.cpp:244-260`), like `/transfer`'s and unlike
    /// `/kill`'s exact one. The GM gate (`GM_LOW_WIZARD`) is the
    /// dispatcher's.
    pub(super) fn stun(&mut self, vid: Vid, rest: &[u8]) -> StunOutcome {
        let (name, _) = crate::command::one_argument(rest);
        let Some(mover) = self.own_mover(vid) else {
            return StunOutcome::Usage;
        };
        if name.is_empty() {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Usage: stun <name>", &[]);
            return StunOutcome::Usage;
        }
        let target = std::str::from_utf8(&name)
            .ok()
            .and_then(|name| self.whisper_target(name));
        let Some(target) = target else {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"%s is not in same map",
                &[Arg::Text(&name)],
            );
            return StunOutcome::NotInSameMap;
        };
        // `SkillAttackAffect`'s three gates: a set affect flag skips, the
        // 1000/1000 draw always lands, and a set immune flag skips
        // (`IsImmune` is a bare bit test under `__FIX_BUG_IMMUNE__`,
        // `G/char_resist.cpp:250-256`). The test-server party chats have no
        // arm: the mode is V26.
        let stunned = self.affect_flags_of(target)[0] & super::fight::AFF_STUN_WORD != 0;
        let immune = self
            .characters
            .find_by_vid(target)
            .ok()
            .and_then(|character| character.points())
            .is_some_and(|points| points.immune_flag() & super::pc_skill::IMMUNE_STUN != 0);
        if !stunned && !immune {
            self.add_pc_affect(
                target,
                super::affect::Affect {
                    kind: crate::loading_phase::AFFECT_STUN,
                    apply_on: 0,
                    value: 0,
                    flag: crate::loading_phase::AFF_STUN,
                    remaining: STUN_TICKS,
                    sp_cost: 0,
                },
                true,
            );
        }
        self.tell(vid, mover, CHAT_TYPE_INFO, b"%s stun", &[Arg::Text(&name)]);
        StunOutcome::Landed(target)
    }

    /// `do_warp` (`G/cmd_gm.cpp:399-457`): the caller warps to the named
    /// character or the metres. An empty line answers `Usage: warp <character
    /// name> | <x meter> <y meter>`. When both arguments start with a digit
    /// they are metres (`strtol`, `C/utils.h:44-50`); else the first names a
    /// character through `FindPC`'s lowered match (`G/char_manager.cpp:244-260`,
    /// this process's PCs, hence every Channel), an unknown name answers
    /// `There is no one(%s) by that name`, and a target on another Channel
    /// answers `Target(%s) is in %d channel (my channel %d)` from the bodies'
    /// Channel comparison (`WarpToPID`'s same-Channel arm, `G/char.cpp:8099-8121`,
    /// never runs, and the P2P lookup answers the cross-Channel line, as
    /// `/transfer`'s does). Else the caller hears `You warp to ( %d, %d )`
    /// with the pixels and warps there (`WarpSet` + `Stop`). A name's feet
    /// pass through `tch->GetX() / 100` and back (`:445-451`), so the warp
    /// lands on the metre at or below the target's feet. The GM gate
    /// (`GM_LOW_WIZARD`) is the dispatcher's.
    pub(super) fn warp_command(&mut self, vid: Vid, rest: &[u8]) -> WarpOutcome {
        let (first, second) = crate::command::two_arguments(rest);
        let Some(mover) = self.own_mover(vid) else {
            return WarpOutcome::Usage;
        };
        if first.is_empty() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Usage: warp <character name> | <x meter> <y meter>",
                &[],
            );
            return WarpOutcome::Usage;
        }
        let metres = first.first().is_some_and(u8::is_ascii_digit)
            && second.first().is_some_and(u8::is_ascii_digit);
        let (x, y) = if metres {
            // `x *= 100` (`:450-451`): a 32-bit wrap on the legacy target, so
            // `wrapping_mul` (a plain `*` would panic in debug).
            (
                str_to_int(&first).wrapping_mul(100),
                str_to_int(&second).wrapping_mul(100),
            )
        } else {
            let target = std::str::from_utf8(&first)
                .ok()
                .and_then(|name| self.whisper_target(name));
            let Some(target) = target else {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"There is no one(%s) by that name",
                    &[Arg::Text(&first)],
                );
                return WarpOutcome::NoSuchCharacter;
            };
            let mine = self.place_of(vid.raw()).map(|place| place.0);
            let theirs = self.place_of(target.raw()).map(|place| place.0);
            if mine != theirs {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"Target(%s) is in %d channel (my channel %d)",
                    &[
                        Arg::Text(&first),
                        Arg::Int(i64::from(theirs.unwrap_or(0))),
                        Arg::Int(i64::from(mine.unwrap_or(0))),
                    ],
                );
                return WarpOutcome::OtherChannel(theirs.unwrap_or(0));
            }
            let Some(spot) = self.spot_of(target.raw()) else {
                return WarpOutcome::NoSuchCharacter;
            };
            (spot.x / 100 * 100, spot.y / 100 * 100)
        };
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"You warp to ( %d, %d )",
            &[Arg::Int(i64::from(x)), Arg::Int(i64::from(y))],
        );
        self.order_self_warp(vid, (x, y));
        WarpOutcome::Warped((x, y))
    }

    /// `do_private` (`G/cmd_gm.cpp:2301-2325`): the caller warps to a private copy
    /// of the named map's spawn. An empty line answers `Usage: private <map
    /// index>`; the word is `str_to_number(long)` (`map_index`, dead `lMapIndex`
    /// beside it). `CreatePrivateMap` (`G/sectree_manager.cpp:943-1026`) refuses
    /// an index at or past 10000 and a map the index does not list, and the
    /// caller hears `Can't find map by index %d` with the parsed index. Else
    /// the copy stands on the base map's grid (`SectreeGrid::of`, like a war
    /// map's), the caller remembers where it stood (`SaveExitLocation`,
    /// `G/char.cpp:6679-6683`) and warps to the copy's spawn (`WarpSet` +
    /// `GC_WARP`) through the registry, with the exit riding the order so the
    /// save writes it. The GM gate (`GM_IMPLEMENTOR`) is the dispatcher's.
    pub(super) fn private_command(&mut self, vid: Vid, rest: &[u8]) -> PrivateOutcome {
        let (first, _) = crate::command::one_argument(rest);
        let Some(mover) = self.own_mover(vid) else {
            return PrivateOutcome::Usage;
        };
        if first.is_empty() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Usage: private <map index>",
                &[],
            );
            return PrivateOutcome::Usage;
        }
        let map_index = str_to_int(&first);
        let taken: Vec<i32> = self.maps.keys().map(|&(_, map)| map).collect();
        let copy = super::war_map::private_index(map_index, &taken)
            .filter(|_| self.recall.in_atlas(map_index));
        let Some(copy) = copy else {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Can't find map by index %d",
                &[Arg::Int(i64::from(map_index))],
            );
            return PrivateOutcome::NoSuchMap(map_index);
        };
        let (channel, here, feet) = match (self.place_of(vid.raw()), self.spot_of(vid.raw())) {
            (Some((channel, here)), Some(spot)) => (channel, here, (spot.x, spot.y)),
            _ => return PrivateOutcome::NoSuchMap(map_index),
        };
        let Some((Some(grid), spawn)) = self
            .recall
            .atlas()
            .and_then(|atlas| atlas.region(map_index))
            .map(|region| (gamedata::server_attr::SectreeGrid::of(region), region.spawn))
        else {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Can't find map by index %d",
                &[Arg::Int(i64::from(map_index))],
            );
            return PrivateOutcome::NoSuchMap(map_index);
        };
        self.host_map(channel, copy, grid);
        self.order_private_warp(vid, channel, here, feet, copy, spawn);
        PrivateOutcome::Warped(copy, spawn)
    }

    /// Warp the caller to a private copy's spawn through the client registry,
    /// with the place it came from riding the order (`SaveExitLocation`,
    /// `G/char.cpp:6679-6683`), which the save writes beside the position.
    fn order_private_warp(
        &self,
        vid: Vid,
        channel: u8,
        here: i32,
        feet: (i32, i32),
        copy: i32,
        spawn: (i32, i32),
    ) {
        let raw = vid.raw();
        let Some(clients) = self.clients.as_ref() else {
            warn!(vid = raw, "a private warp has no client registry to reach");
            return;
        };
        let exit = crate::save::ExitLocation {
            x: feet.0,
            y: feet.1,
            map_index: here,
        };
        let ordered = clients
            .members_on_map(channel, here)
            .into_iter()
            .find(|(_, entry)| entry.vid == raw)
            .is_some_and(|(id, _)| {
                clients.order(
                    channel,
                    id,
                    crate::client_registry::ClientOrder::Warp {
                        x: spawn.0,
                        y: spawn.1,
                        private_map_index: copy,
                        channel: None,
                        exit: Some(exit),
                        sorted_until: self.sort_lock(vid),
                    },
                )
            });
        if !ordered {
            warn!(vid = raw, "a private warp found no client to order");
        }
    }

    /// `do_rewarp` (`G/cmd_gm.cpp:459-465`, behind `ENABLE_NEWSTUFF`, which the
    /// owner's build defines): the caller hears `You warp to ( %d, %d )` with
    /// its own pixels and rewarps in place. The GM gate (`GM_LOW_WIZARD`) is
    /// the dispatcher's.
    pub(super) fn rewarp_command(&mut self, vid: Vid) -> WarpOutcome {
        let Some(mover) = self.own_mover(vid) else {
            return WarpOutcome::Usage;
        };
        let Some(spot) = self.spot_of(vid.raw()) else {
            return WarpOutcome::Usage;
        };
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"You warp to ( %d, %d )",
            &[Arg::Int(i64::from(spot.x)), Arg::Int(i64::from(spot.y))],
        );
        self.order_self_warp(vid, (spot.x, spot.y));
        WarpOutcome::Warped((spot.x, spot.y))
    }

    /// Warp the caller itself to `feet` through the client registry, as
    /// `order_transfer` does for a target.
    fn order_self_warp(&self, vid: Vid, feet: (i32, i32)) {
        self.order_transfer(vid, feet);
    }

    /// `do_goto` (`G/cmd_gm.cpp:313-397`): the caller shows at the metres or
    /// warps to the named map. An empty line answers `Usage: goto <x meter>
    /// <y meter>`. When both arguments start with a digit they are metres
    /// from the caller's map base (`GetMapBasePosition`, no offset when the
    /// caller stands nowhere indexed); the caller hears `You goto ( %d, %d )`
    /// with the metres and shows there (`Show` + `Stop`, no `GC_WARP`). Else
    /// the first names a goto entry: `#n` takes the map index, a digit second
    /// word takes the empire clamped to 1..=3, otherwise the caller's own. A
    /// hit hears `You warp to ( %d, %d )` with the pixels and warps there
    /// (`WarpSet` + `Stop`); a miss is silent. The lookup is
    /// `CHARACTER_GoToName`'s (`:236-262`): the table in load order, the
    /// typed name a case-insensitive prefix of one `|` segment
    /// (`FindInString`, `:205-234`). Legacy then answers `Cannot find map`
    /// on a hit — the `if` tests the found flag the wrong way (`:357-361`),
    /// a Defect the receipt records — and the port does not. The GM gate
    /// (`GM_LOW_WIZARD`) is the dispatcher's.
    pub(super) fn goto_command(&mut self, vid: Vid, rest: &[u8]) -> GotoOutcome {
        let (first, second) = crate::command::two_arguments(rest);
        let Some(mover) = self.own_mover(vid) else {
            return GotoOutcome::Usage;
        };
        if first.is_empty() && second.is_empty() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Usage: goto <x meter> <y meter>",
                &[],
            );
            return GotoOutcome::Usage;
        }
        if first.first().is_some_and(u8::is_ascii_digit)
            && second.first().is_some_and(u8::is_ascii_digit)
        {
            return self.goto_metres(vid, mover, &first, &second);
        }
        self.goto_name(vid, mover, &first, &second)
    }

    /// `do_goto`'s metres arm: `Show` at the metres from the caller's map
    /// base, then `Stop`.
    fn goto_metres(&mut self, vid: Vid, mover: Mover, first: &[u8], second: &[u8]) -> GotoOutcome {
        let mut x = str_to_int(first);
        let mut y = str_to_int(second);
        // `GetMapBasePosition(ch->GetX(), ch->GetY())`: the base of the region
        // holding the caller, in metres onto the line's (`:330-335`).
        if let Some(spot) = self.spot_of(vid.raw()) {
            if let Some(base) = self
                .recall
                .atlas()
                .and_then(|atlas| atlas.index_at(spot.x, spot.y))
                .and_then(|index| self.recall.atlas()?.region(index))
            {
                x = x.wrapping_add(base.sx / 100);
                y = y.wrapping_add(base.sy / 100);
            }
        }
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"You goto ( %d, %d )",
            &[Arg::Int(i64::from(x)), Arg::Int(i64::from(y))],
        );
        let at = (x.wrapping_mul(100), y.wrapping_mul(100));
        // `Show(ch->GetMapIndex(), x, y, z)` (`:395`): the caller's own map,
        // whose `false` the line ignores.
        let _shown = self.show_body(vid.raw(), (at.0, at.1, 0), None);
        self.stop(vid.raw());
        GotoOutcome::Shown(at)
    }

    /// `do_goto`'s name arm through `CHARACTER_GoToName`: the warp on a hit,
    /// silence on a miss.
    fn goto_name(&mut self, vid: Vid, mover: Mover, first: &[u8], second: &[u8]) -> GotoOutcome {
        let map_index = if first.first() == Some(&b'#') {
            str_to_int(first.get(1..).unwrap_or_default())
        } else {
            0
        };
        let empire = if !second.is_empty() && second.first().is_some_and(u8::is_ascii_digit) {
            super::pk_mode::parse_mode(second).clamp(1, 3)
        } else {
            self.bodies.get(&vid).map_or(0, |body| body.card.empire)
        };
        let table = self
            .quests
            .as_ref()
            .map(super::quests::Quests::goto_info)
            .unwrap_or_default();
        let Some(at) = goto_hit(table, map_index, empire, first) else {
            return GotoOutcome::NoSuchMap;
        };
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"You warp to ( %d, %d )",
            &[Arg::Int(i64::from(at.0)), Arg::Int(i64::from(at.1))],
        );
        self.order_self_warp(vid, at);
        self.stop(vid.raw());
        GotoOutcome::Warped(at)
    }

    /// `do_poison` (`G/cmd_gm.cpp:1808-1829`): the named character is poisoned.
    /// An empty line answers `ex) /poison <player name>`; an unknown name
    /// answers `%s: no such a player` with the name as typed. Else the target
    /// is poisoned by nobody (`AttackedByPoison(NULL)`, `:1828`). The lookup
    /// is the descriptor set's exact match, like `/dc`'s. The GM gate
    /// (`GM_IMPLEMENTOR`) is the dispatcher's.
    pub(super) fn poison(&mut self, vid: Vid, rest: &[u8]) -> KillOutcome {
        let target = self.kill_target(vid, rest, b"ex) /poison <player name>");
        let Some(target) = target else {
            return if crate::command::one_argument(rest).0.is_empty() {
                KillOutcome::Usage
            } else {
                KillOutcome::NoSuchPlayer
            };
        };
        self.poison_pc(target, None);
        KillOutcome::Landed(target)
    }

    /// The exact-named target of a `/kill` or `/poison` line, answering the
    /// `usage` or the no-such-player line on the way.
    fn kill_target(&mut self, vid: Vid, rest: &[u8], usage: &[u8]) -> Option<Vid> {
        let (name, _) = crate::command::one_argument(rest);
        let mover = self.own_mover(vid)?;
        if name.is_empty() {
            self.tell(vid, mover, CHAT_TYPE_INFO, usage, &[]);
            return None;
        }
        let target = std::str::from_utf8(&name)
            .ok()
            .and_then(|name| self.player_exactly_named(name));
        let Some(target) = target else {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"%s: no such a player",
                &[Arg::Text(&name)],
            );
            return None;
        };
        Some(target)
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
    /// (`G/input_db.cpp:2460`), and the lines are gone: the store's answer took them
    /// before the gate ran (`server/server/db/ClientManager.cpp:4628`).
    #[test]
    fn a_character_who_blocks_whispers_loses_the_lines_that_waited() {
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
        assert!(
            world.take_offline_messages(b"Nobody").is_empty(),
            "the line was taken all the same"
        );
    }

    /// A line goes at five hours to the millisecond, and one a millisecond younger
    /// stays (`duration_cast<hours>(age).count() >= iHour`,
    /// `server/server/db/ClientManager.cpp:4648`).
    #[test]
    fn a_line_is_dropped_at_five_hours_and_not_before() {
        let (mut world, clock, _inbox) = a_world_with_a_player();
        let line = |left_at| OfflineMessage {
            from: "P100".to_owned(),
            text: b"hello".to_vec(),
            left_at,
        };
        world
            .offline_messages
            .insert(b"Nobody".to_vec(), vec![line(1_000), line(1_001)]);
        world
            .offline_messages
            .insert(b"Somebody".to_vec(), vec![line(1_000)]);
        clock.set(1_000 + OFFLINE_MESSAGE_LIFETIME_MS);

        world.expire_offline_messages();
        assert_eq!(
            world.take_offline_messages(b"Nobody"),
            vec![line(1_001)],
            "the five-hour line went and the younger one stayed"
        );
        assert!(
            !world.offline_messages.contains_key(b"Somebody".as_slice()),
            "a name with no line left is forgotten"
        );
    }

    /// A character that leaves and comes back may leave a line at once: the new
    /// session's clock starts at zero (`dwLastOfflinePMTime = 0`, `G/char.cpp:270-271`),
    /// while the same session waits its ten seconds.
    #[test]
    fn a_new_session_may_leave_a_line_at_once() {
        let (mut world, clock, mut inbox) = a_world_with_a_player();
        clock.set(20_000);
        world.deliver_whisper(Vid::new(VID), b"Nobody", b"first", b"record".to_vec());
        let _sent = inbox.try_recv().expect("the sender was told");
        clock.set(21_000);
        world.deliver_whisper(Vid::new(VID), b"Nobody", b"second", b"record".to_vec());
        let wait = inbox.try_recv().expect("the sender was told");
        assert_eq!(
            &wait[GC_WHISPER_WIRE_SIZE..],
            b"You have to wait 10 seconds for send offline message.\0"
        );

        world.leave_world(Vid::new(VID)).expect("the sender leaves");
        let mut inbox = enter(&mut world, VID, (3200, 3200), 0);
        let _burst = heard(&mut inbox);
        clock.set(22_000);
        let outcome = world.deliver_whisper(Vid::new(VID), b"Nobody", b"third", b"record".to_vec());
        assert_eq!(outcome, WhisperOutcome::Answered);
        let notice = inbox.try_recv().expect("the sender was told");
        assert_eq!(
            &notice[GC_WHISPER_WIRE_SIZE..],
            b"An offline message has been sent.\0",
            "the new session's clock starts at zero"
        );
        let texts: Vec<Vec<u8>> = world
            .take_offline_messages(b"Nobody")
            .into_iter()
            .map(|message| message.text)
            .collect();
        assert_eq!(texts, vec![b"first".to_vec(), b"third".to_vec()]);
    }

    /// A character that leaves and comes back starts with no block bit
    /// (`memset(&m_pointsInstant, 0, ...)`, `G/char.cpp:252`): its whisper reaches the
    /// waiting store, and a line left for it is read at its enter.
    #[test]
    fn a_new_session_starts_with_no_block_byte() {
        let (mut world, clock, _inbox) = a_world_with_a_player();
        clock.set(20_000);
        let _first_session = enter_second_player(&mut world);
        world
            .set_block_mode(Vid::new(OTHER), BLOCK_WHISPER | BLOCK_MESSENGER_INVITE)
            .expect("the character is online");
        assert!(world.is_block_mode(Vid::new(OTHER), BLOCK_WHISPER));

        world
            .leave_world(Vid::new(OTHER))
            .expect("the character leaves");
        assert_eq!(
            world.block_mode(Vid::new(OTHER)),
            0,
            "the leave forgets the byte"
        );
        world.offline_messages.insert(
            b"P101".to_vec(),
            vec![OfflineMessage {
                from: "P100".to_owned(),
                text: b"hello".to_vec(),
                left_at: 20_000,
            }],
        );
        let mut other = enter(&mut world, OTHER, (3200, 3200), 0);
        let _burst = heard(&mut other);
        assert_eq!(
            world.block_mode(Vid::new(OTHER)),
            0,
            "the enter starts at zero"
        );
        world.deliver_offline_messages(Vid::new(OTHER), "P101");
        let line = other.try_recv().expect("the waiting line was read");
        assert_eq!(line[3], WHISPER_TYPE_OFFLINE);
        assert_eq!(&line[GC_WHISPER_WIRE_SIZE..], b"hello\0");
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
    fn the_hide_number_reads_strtoul_low_byte() {
        assert_eq!(parse_hide_number(b"1"), 1);
        assert_eq!(parse_hide_number(b"5"), 5);
        assert_eq!(
            parse_hide_number(b"12abc"),
            12,
            "digits run to the first non-digit"
        );
        assert_eq!(parse_hide_number(b"abc"), 0, "no digits is zero");
        assert_eq!(parse_hide_number(b""), 0);
        assert_eq!(parse_hide_number(b"+3"), 3);
        assert_eq!(
            parse_hide_number(b"-1"),
            255,
            "the low byte of the wrapped magnitude"
        );
        assert_eq!(parse_hide_number(b"257"), 1, "only the low byte survives");
        assert_eq!(
            parse_hide_number(b"99999999999999999999"),
            255,
            "past u64::MAX clamps, then the low byte"
        );
    }

    #[test]
    fn a_hide_lands_its_bit_chats_updates_and_writes_its_flag() {
        let (mut world, clock, _inbox) = a_world_with_a_player();
        // The enter stamped the move time; the gate needs 1.5 idle seconds.
        clock.set(1_500);
        let hid = world.hide_costume(Vid::new(VID), 1, true);
        let Some(HideCostumeOutcome::Hid { line, update, flag }) = hid else {
            panic!("part 1 hides");
        };
        assert!(line
            .windows(b"SetBodyCostumeHidden 1".len())
            .any(|window| window == b"SetBodyCostumeHidden 1"));
        assert_eq!(update[0], 0x13, "the look update");
        assert_eq!(flag, (b"costume_option.hide_body".to_vec(), 1));
        let points = world
            .characters
            .find_by_vid(Vid::new(VID))
            .expect("online")
            .points()
            .expect("points");
        assert_eq!(points.hidden(), 0b00001);
        // A show clears the bit and writes 0.
        let Some(HideCostumeOutcome::Hid { flag, .. }) =
            world.hide_costume(Vid::new(VID), 1, false)
        else {
            panic!("part 1 shows");
        };
        assert_eq!(flag, (b"costume_option.hide_body".to_vec(), 0));
        // No part 0 or 6: quiet.
        assert!(world.hide_costume(Vid::new(VID), 0, true).is_none());
        assert!(world.hide_costume(Vid::new(VID), 6, true).is_none());
        // Nobody online: quiet.
        assert!(world.hide_costume(Vid::new(9999), 1, true).is_none());
    }

    #[test]
    fn a_hide_within_the_move_window_is_refused() {
        let (mut world, clock, _inbox) = a_world_with_a_player();
        // The enter stamped the move time at 0; 1,499 ms later the gate still holds.
        clock.set(1_499);
        let Some(HideCostumeOutcome::Refused(line)) = world.hide_costume(Vid::new(VID), 1, true)
        else {
            panic!("the gate refuses");
        };
        assert!(line
            .windows(b"[LS;1971]".len())
            .any(|window| window == b"[LS;1971]"));
        let points = world
            .characters
            .find_by_vid(Vid::new(VID))
            .expect("online")
            .points()
            .expect("points");
        assert_eq!(points.hidden(), 0, "the refusal lands nothing");
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
                .windows(b"there is no such command".len())
                .any(|window| window == b"there is no such command"),
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
                .windows(b"there is no such command".len())
                .any(|window| window == b"there is no such command"),
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
    fn a_silenced_caller_lists_its_name_and_ticks() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let mut target = enter_second_player(&mut world);
        make_gm(&mut world, VID);
        let _enter = heard(&mut caller);

        assert_eq!(
            world.block_chat(Vid::new(VID), b"P101", 65),
            BlockTarget::Moved(Vid::new(OTHER))
        );
        let _fanned = heard(&mut target);
        world.block_chat_list(Vid::new(VID));
        let lines = heard(&mut caller);
        assert_eq!(lines.len(), 1, "the one silence's line");
        assert!(
            lines[0].windows(11).any(|window| window == b"P101 65 sec"),
            "the caller hears the name and the ticks"
        );
        assert!(
            target.try_recv().is_err(),
            "the list reaches only the caller"
        );
    }

    #[test]
    fn an_empty_list_answers_no_one_currently_blocked() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let mut target = enter_second_player(&mut world);
        make_gm(&mut world, VID);
        let _enter = heard(&mut caller);
        let _burst = heard(&mut target);

        world.block_chat_list(Vid::new(VID));
        let lines = heard(&mut caller);
        assert_eq!(lines.len(), 1, "the empty list's line");
        assert!(
            lines[0]
                .windows(b"No one currently blocked".len())
                .any(|window| window == b"No one currently blocked"),
            "the caller hears the empty line"
        );
        assert!(
            target.try_recv().is_err(),
            "the list reaches only the caller"
        );
    }

    #[test]
    fn a_player_without_the_privilege_lists_nothing_but_the_refusal() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let mut target = enter_second_player(&mut world);
        let _enter = heard(&mut caller);

        world.block_chat_list(Vid::new(VID));
        let lines = heard(&mut caller);
        assert_eq!(lines.len(), 1, "the refusal is the only line");
        assert!(
            lines[0]
                .windows(b"there is no such command".len())
                .any(|window| window == b"there is no such command"),
            "the gate refuses the list too"
        );
        assert!(target.try_recv().is_err(), "the target heard nothing");
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

    #[test]
    fn an_empty_dc_answers_the_usage() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let _enter = heard(&mut caller);

        assert_eq!(world.disconnect(Vid::new(VID), b""), DcOutcome::Usage);
        let lines = heard(&mut caller);
        assert_eq!(lines.len(), 1, "the usage is the only line");
        assert!(
            lines[0]
                .windows(b"ex) /dc <player name>".len())
                .any(|window| window == b"ex) /dc <player name>"),
            "the caller hears the usage"
        );
    }

    #[test]
    fn a_dc_names_its_target_exactly() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let mut target = enter_second_player(&mut world);
        let _enter = heard(&mut caller);

        // A name in another case names nobody: the lookup is the descriptor
        // set's exact match, not `FindPC`'s lowered one.
        assert_eq!(
            world.disconnect(Vid::new(VID), b"p101"),
            DcOutcome::NoSuchPlayer
        );
        let lines = heard(&mut caller);
        assert_eq!(lines.len(), 1, "the no-such-player line is the only line");
        assert!(
            lines[0]
                .windows(b"p101: no such a player.".len())
                .any(|window| window == b"p101: no such a player."),
            "the name is echoed as typed"
        );
        assert!(target.try_recv().is_err(), "the target heard nothing");

        assert_eq!(
            world.disconnect(Vid::new(VID), b"Nobody"),
            DcOutcome::NoSuchPlayer
        );
        let lines = heard(&mut caller);
        assert!(
            lines[0]
                .windows(b"Nobody: no such a player.".len())
                .any(|window| window == b"Nobody: no such a player."),
            "an unknown name echoes too"
        );
    }

    #[test]
    fn a_dc_refuses_the_caller_itself() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let _enter = heard(&mut caller);

        assert_eq!(world.disconnect(Vid::new(VID), b"P100"), DcOutcome::Itself);
        let lines = heard(&mut caller);
        assert_eq!(lines.len(), 1, "the refusal is the only line");
        assert!(
            lines[0]
                .windows(b"cannot disconnect myself".len())
                .any(|window| window == b"cannot disconnect myself"),
            "the caller hears the self refusal"
        );
    }

    #[test]
    fn a_dc_on_a_target_reports_closed_and_tells_it_nothing() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let mut target = enter_second_player(&mut world);
        let _enter = heard(&mut caller);

        // No registry is attached in this test, so the close order lands
        // nowhere; the wire proves the close through the real scenario.
        assert_eq!(
            world.disconnect(Vid::new(VID), b"P101"),
            DcOutcome::Closed(Vid::new(OTHER))
        );
        assert!(caller.try_recv().is_err(), "a landed close is quiet");
        assert!(target.try_recv().is_err(), "the target hears nothing");
    }

    #[test]
    fn an_empty_transfer_answers_the_usage() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let _enter = heard(&mut caller);

        assert_eq!(world.transfer(Vid::new(VID), b""), TransferOutcome::Usage);
        let lines = heard(&mut caller);
        assert_eq!(lines.len(), 1, "the usage is the only line");
        assert!(
            lines[0]
                .windows(b"Usage: transfer <name>".len())
                .any(|window| window == b"Usage: transfer <name>"),
            "the caller hears the usage"
        );
    }

    #[test]
    fn a_transfer_names_its_target_without_regard_to_case() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let mut target = enter_second_player(&mut world);
        let _enter = heard(&mut caller);

        // The lookup is `FindPC`'s lowered one, unlike `/dc`'s exact one.
        assert_eq!(
            world.transfer(Vid::new(VID), b"p101"),
            TransferOutcome::Warped(Vid::new(OTHER))
        );
        assert!(caller.try_recv().is_err(), "a landed warp is quiet");
        assert!(
            target.try_recv().is_err(),
            "the warp is an order, not a line"
        );
    }

    #[test]
    fn an_unknown_transfer_answers_the_no_character_line() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let _enter = heard(&mut caller);

        // No registry is attached, so no cross-Channel arm answers.
        assert_eq!(
            world.transfer(Vid::new(VID), b"Nobody"),
            TransferOutcome::NoSuchCharacter
        );
        let lines = heard(&mut caller);
        assert_eq!(lines.len(), 1, "the no-character line is the only line");
        assert!(
            lines[0]
                .windows(b"There is no character(Nobody) by that name".len())
                .any(|window| window == b"There is no character(Nobody) by that name"),
            "the name is echoed as typed"
        );
    }

    #[test]
    fn a_transfer_refuses_the_caller_itself() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let _enter = heard(&mut caller);

        assert_eq!(
            world.transfer(Vid::new(VID), b"P100"),
            TransferOutcome::Itself
        );
        let lines = heard(&mut caller);
        assert_eq!(lines.len(), 1, "the refusal is the only line");
        assert!(
            lines[0]
                .windows(b"Transfer me?!?".len())
                .any(|window| window == b"Transfer me?!?"),
            "the caller hears the self refusal"
        );
    }

    #[test]
    fn a_kill_answers_its_usage_and_its_miss() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let mut target = enter_second_player(&mut world);
        let _enter = heard(&mut caller);

        assert_eq!(world.kill(Vid::new(VID), b""), KillOutcome::Usage);
        let lines = heard(&mut caller);
        assert!(
            lines[0]
                .windows(b"ex) /kill <player name>".len())
                .any(|window| window == b"ex) /kill <player name>"),
            "the caller hears the usage"
        );
        // The exact match, like `/dc`'s: another case names nobody, and the
        // line has no trailing stop.
        assert_eq!(
            world.kill(Vid::new(VID), b"p101"),
            KillOutcome::NoSuchPlayer
        );
        let lines = heard(&mut caller);
        assert!(
            lines[0]
                .windows(b"p101: no such a player".len())
                .any(|window| window == b"p101: no such a player"),
            "the name is echoed as typed"
        );
        assert!(target.try_recv().is_err(), "the target heard nothing");
    }

    #[test]
    fn a_kill_kills_its_target_and_tells_the_caller_nothing() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let mut target = enter_second_player(&mut world);
        let _enter = heard(&mut caller);
        let _burst = heard(&mut target);
        // The fixture enters with no combat row; the live enter starts the
        // recovery, which opens it.
        world.start_pc_recovery(Vid::new(OTHER));

        assert_eq!(
            world.kill(Vid::new(VID), b"P101"),
            KillOutcome::Landed(Vid::new(OTHER))
        );
        // The death fanned to the caller too, as it stands in view; no chat
        // line did.
        for line in heard(&mut caller) {
            assert_ne!(
                line[0],
                protocol::gc_inventory::HEADER_GC_CHAT.value(),
                "a landed kill chats nothing"
            );
        }
        assert!(world.is_dead(OTHER), "the target is dead");
        let fanned = heard(&mut target);
        assert!(!fanned.is_empty(), "the target hears its death");
    }

    #[test]
    fn a_poison_answers_its_usage_and_poisons_its_target() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let mut target = enter_second_player(&mut world);
        let _enter = heard(&mut caller);

        assert_eq!(world.poison(Vid::new(VID), b""), KillOutcome::Usage);
        let lines = heard(&mut caller);
        assert!(
            lines[0]
                .windows(b"ex) /poison <player name>".len())
                .any(|window| window == b"ex) /poison <player name>"),
            "the caller hears the usage"
        );
        assert_eq!(
            world.poison(Vid::new(VID), b"Nobody"),
            KillOutcome::NoSuchPlayer
        );
        let lines = heard(&mut caller);
        assert!(
            lines[0]
                .windows(b"Nobody: no such a player".len())
                .any(|window| window == b"Nobody: no such a player"),
            "the unknown name echoes as typed"
        );
        let _burst = heard(&mut target);
        assert_eq!(
            world.poison(Vid::new(VID), b"P101"),
            KillOutcome::Landed(Vid::new(OTHER))
        );
        // The poison's flag update fans to the caller too, as it stands in
        // view; no chat line does.
        for line in heard(&mut caller) {
            assert_ne!(
                line[0],
                protocol::gc_inventory::HEADER_GC_CHAT.value(),
                "a landed poison chats nothing"
            );
        }
        let fanned = heard(&mut target);
        assert!(
            fanned
                .iter()
                .any(|line| line[0] == HEADER_GC_AFFECT_ADD.value()),
            "the target hears its poison's add"
        );
    }

    #[test]
    fn a_stun_answers_its_usage_and_its_miss() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let mut target = enter_second_player(&mut world);
        let _enter = heard(&mut caller);

        assert_eq!(world.stun(Vid::new(VID), b""), StunOutcome::Usage);
        let lines = heard(&mut caller);
        assert!(
            lines[0]
                .windows(b"Usage: stun <name>".len())
                .any(|window| window == b"Usage: stun <name>"),
            "the caller hears the usage"
        );
        assert_eq!(
            world.stun(Vid::new(VID), b"Nobody"),
            StunOutcome::NotInSameMap
        );
        let lines = heard(&mut caller);
        assert!(
            lines[0]
                .windows(b"Nobody is not in same map".len())
                .any(|window| window == b"Nobody is not in same map"),
            "the unknown name echoes as typed"
        );
        assert!(target.try_recv().is_err(), "the target heard nothing");
    }

    #[test]
    fn a_stun_stuns_its_target_whatever_the_case_and_confirms() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let mut target = enter_second_player(&mut world);
        let _enter = heard(&mut caller);

        // The lookup is `FindPC`'s lowered one, unlike `/kill`'s exact one.
        assert_eq!(
            world.stun(Vid::new(VID), b"p101"),
            StunOutcome::Landed(Vid::new(OTHER))
        );
        let lines = heard(&mut caller);
        assert!(
            lines.iter().any(|line| line
                .windows(b"p101 stun".len())
                .any(|window| window == b"p101 stun")),
            "the caller hears the confirmation as typed"
        );
        let fanned = heard(&mut target);
        assert!(
            fanned
                .iter()
                .any(|line| line[0] == HEADER_GC_AFFECT_ADD.value()),
            "the target hears its stun's add"
        );
        // Already stunned: the confirmation still goes out, and no second
        // add lands.
        assert_eq!(
            world.stun(Vid::new(VID), b"P101"),
            StunOutcome::Landed(Vid::new(OTHER))
        );
        let lines = heard(&mut caller);
        assert!(
            lines.iter().any(|line| line
                .windows(b"P101 stun".len())
                .any(|window| window == b"P101 stun")),
            "the second stun confirms too"
        );
        let fanned = heard(&mut target);
        assert!(
            fanned
                .iter()
                .all(|line| line[0] != HEADER_GC_AFFECT_ADD.value()),
            "the second stun adds nothing"
        );
    }

    #[test]
    fn a_warp_answers_its_usage_and_its_miss() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let mut target = enter_second_player(&mut world);
        let _enter = heard(&mut caller);

        assert_eq!(world.warp_command(Vid::new(VID), b""), WarpOutcome::Usage);
        let lines = heard(&mut caller);
        assert!(
            lines[0]
                .windows(b"Usage: warp <character name> | <x meter> <y meter>".len())
                .any(|window| window == b"Usage: warp <character name> | <x meter> <y meter>"),
            "the caller hears the usage"
        );
        assert_eq!(
            world.warp_command(Vid::new(VID), b"Nobody"),
            WarpOutcome::NoSuchCharacter
        );
        let lines = heard(&mut caller);
        assert!(
            lines[0]
                .windows(b"There is no one(Nobody) by that name".len())
                .any(|window| window == b"There is no one(Nobody) by that name"),
            "the unknown name echoes as typed"
        );
        assert!(target.try_recv().is_err(), "the target heard nothing");
    }

    #[test]
    fn a_warp_to_metres_names_its_pixels_and_a_rewarp_rerides_its_spot() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let _target = enter_second_player(&mut world);
        let _enter = heard(&mut caller);

        // Both arguments start with a digit: metres, times 100. The world has
        // no client registry, so the warp order goes nowhere and only the
        // confirmation is heard.
        assert_eq!(
            world.warp_command(Vid::new(VID), b"47 95"),
            WarpOutcome::Warped((4700, 9500))
        );
        let lines = heard(&mut caller);
        assert!(
            lines[0]
                .windows(b"You warp to ( 4700, 9500 )".len())
                .any(|window| window == b"You warp to ( 4700, 9500 )"),
            "the caller hears the pixels"
        );
        // `strtol` stops at the first non-digit: 47x is 47.
        assert_eq!(
            world.warp_command(Vid::new(VID), b"47x 95y"),
            WarpOutcome::Warped((4700, 9500))
        );
        let _lines = heard(&mut caller);
        // `x *= 100` wraps on the legacy target: `i32::MAX` metres land on
        // -100, without panicking.
        assert_eq!(
            world.warp_command(Vid::new(VID), b"2147483647 1"),
            WarpOutcome::Warped((-100, 100))
        );
        let _lines = heard(&mut caller);
        // The caller stands at the fixture's (3200, 3200).
        assert_eq!(
            world.rewarp_command(Vid::new(VID)),
            WarpOutcome::Warped((3200, 3200))
        );
        let lines = heard(&mut caller);
        assert!(
            lines[0]
                .windows(b"You warp to ( 3200, 3200 )".len())
                .any(|window| window == b"You warp to ( 3200, 3200 )"),
            "the rewarp names the caller's own pixels"
        );
    }

    #[test]
    fn a_warp_to_a_name_lands_whatever_the_case() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let mut target = enter_second_player(&mut world);
        let _enter = heard(&mut caller);

        // The lookup is `FindPC`'s lowered one: another case still lands, on
        // the metre at the target's feet (`tch->GetX() / 100`, off-metre feet
        // truncate down).
        assert_eq!(
            world.warp_command(Vid::new(VID), b"p101"),
            WarpOutcome::Warped((3200, 3200))
        );
        let lines = heard(&mut caller);
        assert!(
            lines[0]
                .windows(b"You warp to ( 3200, 3200 )".len())
                .any(|window| window == b"You warp to ( 3200, 3200 )"),
            "the caller hears the target's feet"
        );
        assert!(target.try_recv().is_err(), "the target heard nothing");
    }

    #[test]
    fn a_warp_to_a_name_truncates_off_metre_feet() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut caller = enter(&mut world, VID, (3200, 3200), 0);
        let mut target = enter(&mut world, OTHER, (3277, 3249), 0);
        let _burst = heard(&mut target);
        let _enter = heard(&mut caller);

        // `x = tch->GetX() / 100; x *= 100`: 3277 lands on 3200.
        assert_eq!(
            world.warp_command(Vid::new(VID), b"P101"),
            WarpOutcome::Warped((3200, 3200))
        );
        let lines = heard(&mut caller);
        assert!(
            lines[0]
                .windows(b"You warp to ( 3200, 3200 )".len())
                .any(|window| window == b"You warp to ( 3200, 3200 )"),
            "the caller hears the truncated metres"
        );
        assert!(target.try_recv().is_err(), "the target heard nothing");
    }

    #[test]
    fn a_goto_answers_its_usage_and_stays_silent_on_a_miss() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let _enter = heard(&mut caller);

        assert_eq!(world.goto_command(Vid::new(VID), b""), GotoOutcome::Usage);
        let lines = heard(&mut caller);
        assert!(
            lines[0]
                .windows(b"Usage: goto <x meter> <y meter>".len())
                .any(|window| window == b"Usage: goto <x meter> <y meter>"),
            "the caller hears the usage"
        );
        // No quests are loaded, so the table is empty and the name arm is
        // silent.
        assert_eq!(
            world.goto_command(Vid::new(VID), b"Flame"),
            GotoOutcome::NoSuchMap
        );
        assert!(caller.try_recv().is_err(), "the miss is silent");
    }

    #[test]
    fn a_goto_to_metres_shows_at_the_pixels() {
        let (mut world, _clock, mut caller) = a_world_with_a_player();
        let _enter = heard(&mut caller);

        // No atlas: no base offset. The caller hears the metres and shows at
        // the pixels.
        assert_eq!(
            world.goto_command(Vid::new(VID), b"40 33"),
            GotoOutcome::Shown((4000, 3300))
        );
        let lines = heard(&mut caller);
        assert!(
            lines[0]
                .windows(b"You goto ( 40, 33 )".len())
                .any(|window| window == b"You goto ( 40, 33 )"),
            "the caller hears the metres"
        );
        let spot = world.spot_of(VID).expect("the caller stands somewhere");
        assert_eq!((spot.x, spot.y), (4000, 3300), "the caller shows there");
    }

    #[test]
    fn a_goto_to_metres_adds_the_map_base() {
        use gamedata::map_atlas::{MapAtlas, MapRegion};
        use std::collections::HashMap;
        use std::sync::Arc;

        let clock = TestClock::default();
        let region = MapRegion {
            index: 1,
            name: b"map1".to_vec(),
            sx: 100_000,
            sy: 200_000,
            ex: 300_000,
            ey: 400_000,
            spawn: (100_000, 200_000),
            empire_spawns: None,
        };
        let mut world = a_world(&clock).with_recall(
            Arc::new(MapAtlas::from_regions(vec![region])),
            &HashMap::new(),
            &[],
        );
        let mut caller = enter(&mut world, VID, (150_000, 250_000), 0);
        let _enter = heard(&mut caller);

        // The caller stands inside the region, so the base (1000, 2000
        // metres) joins the line's (40, 33).
        assert_eq!(
            world.goto_command(Vid::new(VID), b"40 33"),
            GotoOutcome::Shown((104_000, 203_300))
        );
        let lines = heard(&mut caller);
        assert!(
            lines[0]
                .windows(b"You goto ( 1040, 2033 )".len())
                .any(|window| window == b"You goto ( 1040, 2033 )"),
            "the caller hears the offset metres"
        );
    }

    #[test]
    fn find_in_string_matches_a_prefix_of_one_segment() {
        assert!(find_in_string(b"", b"Flame"), "empty finds all");
        assert!(find_in_string(b"Flame", b"Flame"), "exact");
        assert!(find_in_string(b"fla", b"Flame"), "prefix");
        assert!(find_in_string(b"FLA", b"Flame"), "case folds");
        assert!(
            find_in_string(b"fire", b"Flame|fire|flame2"),
            "second segment"
        );
        assert!(!find_in_string(b"Flames", b"Flame"), "longer finds nothing");
        assert!(!find_in_string(b"lame", b"Flame"), "not a prefix");
        assert!(!find_in_string(b"x", b"Flame|fire"), "no segment");
    }

    #[test]
    fn a_goto_hit_takes_the_first_matching_entry() {
        use quest::host::GotoInfo;

        let entry = |name: &[u8], empire: i32, map: i32, x: i32, y: i32| GotoInfo {
            name: name.to_vec(),
            empire,
            map_index: map,
            x,
            y,
        };
        let table = vec![
            entry(b"Snow", 1, 61, 4342, 2906),
            entry(b"Flame|fire", 0, 62, 5994, 7563),
        ];
        assert_eq!(
            goto_hit(&table, 0, 1, b"Snow"),
            Some((434_200, 290_600)),
            "exact, own empire"
        );
        assert_eq!(
            goto_hit(&table, 0, 2, b"sno"),
            None,
            "another empire misses"
        );
        assert_eq!(
            goto_hit(&table, 0, 2, b"fir"),
            Some((599_400, 756_300)),
            "empire 0 takes all"
        );
        assert_eq!(
            goto_hit(&table, 62, 3, b"ignored"),
            Some((599_400, 756_300)),
            "the map index skips the name"
        );
        assert_eq!(goto_hit(&table, 63, 1, b""), None, "no such map");
        assert_eq!(goto_hit(&[], 0, 1, b"Snow"), None, "empty table");
    }

    /// The trio lands and misses: `setqf` sets and chats the confirmation,
    /// `getqf` lists the flag, `delqf` deletes with `Success` and then
    /// `Failed`, and an unknown name answers `There is no such character`.
    #[test]
    fn the_qf_trio_sets_lists_and_deletes() {
        use super::super::fixtures::{a_world, enter, TestClock};
        use super::super::quests::Quests;
        use quest::manager::Manager;
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager = Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        world.quests = Some(Quests::new(
            manager,
            std::sync::Arc::default(),
            gamedata::mob_locale_names::MobNamesByLanguage::default(),
        ));
        let mut inbox = enter(&mut world, VID, (3200, 3200), 0);
        while inbox.try_recv().is_ok() {}
        let vid = Vid::new(VID);
        let drained = |inbox: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>| {
            std::iter::from_fn(|| inbox.try_recv().ok()).collect::<Vec<_>>()
        };
        assert_eq!(world.setqf(vid, b"train.count 7"), QfOutcome::Landed(vid));
        let heard = drained(&mut inbox);
        assert_eq!(heard.len(), 1);
        assert!(
            heard[0].ends_with(b"Quest flag set: train.count 7"),
            "the confirmation: {:?}",
            heard[0]
        );
        assert_eq!(world.getqf(vid, b""), QfOutcome::Landed(vid));
        let heard = drained(&mut inbox);
        assert_eq!(heard.len(), 1);
        assert!(
            heard[0].ends_with(b"train.count 7"),
            "the list: {:?}",
            heard[0]
        );
        assert_eq!(world.delqf(vid, b"train.count"), QfOutcome::Landed(vid));
        let heard = drained(&mut inbox);
        assert!(heard[0].ends_with(b"Success"), "the delete: {:?}", heard[0]);
        assert_eq!(world.delqf(vid, b"train.count"), QfOutcome::Landed(vid));
        let heard = drained(&mut inbox);
        assert!(
            heard[0].ends_with(b"Failed, quest flag does not exist"),
            "the missing delete: {:?}",
            heard[0]
        );
        assert_eq!(world.getqf(vid, b""), QfOutcome::Landed(vid));
        assert!(drained(&mut inbox).is_empty(), "the flag is gone");
        assert_eq!(world.getqf(vid, b"Nobody"), QfOutcome::NoSuchCharacter);
        let heard = drained(&mut inbox);
        assert!(
            heard[0].ends_with(b"There is no such character"),
            "the miss: {:?}",
            heard[0]
        );
    }

    /// `set_state` moves the quest and chats the confirmation with the
    /// quest name twice and the state's number; the status flag follows,
    /// and an unknown state is 0.
    #[test]
    fn set_state_moves_the_quest_and_chats_its_number() {
        use super::super::fixtures::{a_world, enter, TestClock};
        use super::super::quests::Quests;
        use quest::manager::Manager;
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager = Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        world.quests = Some(Quests::new(
            manager,
            std::sync::Arc::default(),
            gamedata::mob_locale_names::MobNamesByLanguage::default(),
        ));
        let mut inbox = enter(&mut world, VID, (3200, 3200), 0);
        while inbox.try_recv().is_ok() {}
        let vid = Vid::new(VID);
        let index = world
            .quests
            .as_ref()
            .expect("quests loaded")
            .manager()
            .state_index(b"sash_mission", b"information");
        assert!(index != 0, "the sash's information state has a number");
        assert_eq!(
            world.set_state(vid, b"sash_mission information"),
            QfOutcome::Landed(vid)
        );
        let heard: Vec<_> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        assert_eq!(heard.len(), 1);
        let line = format!("setting quest state flag sash_mission sash_mission {index}");
        assert!(
            heard[0].ends_with(line.as_bytes()),
            "the confirmation: {:?}",
            heard[0]
        );
        let pid = world
            .characters
            .find_by_vid(vid)
            .map(world::character::Character::player_id)
            .expect("the caller is online");
        assert_eq!(
            world
                .quests
                .as_ref()
                .expect("quests loaded")
                .manager()
                .quest_flag(pid, b"sash_mission.__status"),
            Some(index)
        );
        assert_eq!(
            world.set_state(vid, b"sash_mission nowhere"),
            QfOutcome::Landed(vid)
        );
        let heard: Vec<_> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        assert!(
            heard[0].ends_with(b"setting quest state flag sash_mission sash_mission 0"),
            "the unknown state is 0: {:?}",
            heard[0]
        );
    }
}
