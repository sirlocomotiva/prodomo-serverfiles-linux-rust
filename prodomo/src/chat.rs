//! Pure policy for the legacy `CInputMain::Chat` handler.
//!
//! `server/server/game/input_main.cpp:781-991`. The protocol crate owns the frame
//! (`protocol::cg_chat::CgChat` and `protocol::gc_chat::GcChat`); this module starts
//! after decoding and reproduces the handler's **order of checks and its state
//! effects** without owning a socket, a world, or a descriptor. The descriptor task
//! in `prodomo/src/main.rs` supplies the context, applies the returned effects, and
//! owns the lifetime.
//!
//! # The order is the contract
//!
//! Every step below is a place where legacy returns early, so moving a check
//! changes what a client can observe. The order is:
//!
//! 1. `size` below 4 closes the descriptor (`:790-795`). This is the only close in
//!    the whole handler.
//! 2. A text that starts with `/` and is longer than one byte goes to
//!    `interpret_command` and **returns** (`:802-806`). It runs before the rate
//!    limit, so a command costs no chat counter.
//! 3. The chat counter (`:808-813`), after the reset of a new window (see below).
//! 4. `AFFECT_BLOCK_CHAT` (`:828-834`).
//! 5. `SpamBlockCheck` (`:836-839`).
//! 6. The banword filter (`:841-842`), **commented out in legacy** — see
//!    [`convert_banwords`].
//! 7. `ProcessTextTag` (`:844-864`), dead in legacy — see the note below.
//! 8. For a shout, the level limit, then the cooldown, then the line to every Channel
//!    (`:882-913`).
//! 9. The type switch (`:926-988`).
//!
//! # The counter is reset every five seconds of Pulses
//!
//! `CHARACTER_MANAGER::Update` resets every character's counter on each Pulse that is a
//! multiple of `PASSES_PER_SEC(5)` (`char_manager.cpp:700-704`, with
//! `FuncUpdateAndResetChatCounter` at `:669-676`), before that Pulse's input is read.
//! The Rewrite has no per-character update on the game thread yet, so [`ChatState`] keeps the
//! window of its last line and resets the counter when a line arrives in a later one. The
//! counter a line reads is the same either way, because nothing else reads it.
//!
//! # Five legacy defects this module does not reproduce
//!
//! * **`bEmpire` is never assigned on the player-originated path.** `input_main.cpp:916-924`
//!   sets `header`, `size`, `type`, `id`, and `bCanFormat`, and nothing else, so the byte
//!   at offset 8 of every chat line from `Chat` is uninitialised stack.
//!   `CHARACTER::ChatPacket` does set it (`char.cpp:5179`). The Rewrite writes the
//!   speaker's empire; that is a recorded Divergence, not parity.
//! * **The banword filter is disabled.** `// CBanwordManager::instance().ConvertString(buf, buflen);`
//!   at `input_main.cpp:842` is a comment, so a chat line with a banned word goes out
//!   unchanged. The Rewrite applies the filter. See [`convert_banwords`].
//! * **`ProcessTextTag` is dead.** `input_main.cpp:302-305` opens with `return 0;`, so the
//!   prism block at `:844-864` can never run and no line is ever refused for markup.
//!   The Rewrite has no prism accounting, so the same outcome holds, but it is reached
//!   because the check does not exist rather than because it returns zero.
//! * **`SendBlockChatInfo` reads uninitialised stack.** `input_main.cpp:140` declares
//!   `char buf[128+1]` and never writes it, then `:153` sends it as a chat line whenever
//!   `sec > 0`. The Rewrite sends only the locale string.
//! * **No shout in the first fifteen seconds of uptime.** `dwLastShoutPulse` starts at 0
//!   (`char.cpp:252` zeroes `m_pointsInstant`), and `:893` compares the heart's Pulse with
//!   it, so every shout before Pulse 375 is dropped as if the character had just shouted. The
//!   Rewrite starts with no last shout, so a character's first shout is never refused by the
//!   cooldown.
//!
//! # `ENABLE_CHAT_SPAMLIMIT` is defined
//!
//! `input_main.cpp:53` defines it, so the live limit is `>= 4` with
//! `DelayedDisconnect(0)` at counter 10, not the dead `#else` arm's `>= 10` with
//! `DelayedDisconnect(5)`. The counter is a `BYTE` (`char.h:922`), so it wraps at 256;
//! legacy never notices because a wrapped counter is below every threshold. This module
//! uses `u8` and wrapping arithmetic for the same reason.

#![warn(missing_docs)]

use gamedata::locale_string::{country_code, LocaleStrings};
use protocol::cg_chat::{CgChat, CG_CHAT_WIRE_SIZE};
use protocol::gc_chat::{GcChat, CHAT_MAX_LEN, CHAT_TYPE_INFO, CHAT_TYPE_TALKING};

use crate::chat_line::{chat_packet, Arg, Recipient};
use crate::client_registry::ClientEntry;
use crate::save::PASSES_PER_SEC;

/// `CHAT_TYPE_TALKING` (`EChatType`, `server/server/common/length.h:405`).
pub const CHAT_TALKING: u8 = CHAT_TYPE_TALKING;
/// `CHAT_TYPE_INFO` (`length.h:406`).
pub const CHAT_INFO: u8 = CHAT_TYPE_INFO;
/// `CHAT_TYPE_NOTICE` (`length.h:407`).
pub const CHAT_NOTICE: u8 = 1 + 1;
/// `CHAT_TYPE_PARTY` (`length.h:408`).
pub const CHAT_PARTY: u8 = 3;
/// `CHAT_TYPE_GUILD` (`length.h:409`).
pub const CHAT_GUILD: u8 = 4;
/// `CHAT_TYPE_COMMAND` (`length.h:410`).
pub const CHAT_COMMAND: u8 = 5;
/// `CHAT_TYPE_SHOUT` (`length.h:411`).
pub const CHAT_SHOUT: u8 = 6;
/// `CHAT_TYPE_WHISPER` (`length.h:412`).
pub const CHAT_WHISPER: u8 = 7;
/// `CHAT_TYPE_BIG_NOTICE` (`length.h:413`).
pub const CHAT_BIG_NOTICE: u8 = 8;
/// `CHAT_TYPE_MONARCH_NOTICE` (`length.h:414`).
pub const CHAT_MONARCH_NOTICE: u8 = 9;
/// `CHAT_TYPE_MAX_NUM` (`length.h:418`) with `ENABLE_DICE_SYSTEM` undefined.
pub const CHAT_MAX_NUM: u8 = 10;

/// `g_iShoutLimitLevel` (`server/server/game/config.cpp:52`), the compiled-in default of
/// `[game] shout_limit_level`.
pub const SHOUT_LIMIT_LEVEL: i32 = 15;

/// The Pulses between two shouts, `passes_per_sec * 15` (`input_main.cpp:893`).
pub const SHOUT_COOLDOWN_PULSES: u32 = PASSES_PER_SEC * 15;

/// The Pulses between two resets of the chat counter, `PASSES_PER_SEC(5)`
/// (`char_manager.cpp:700`).
pub const CHAT_COUNTER_WINDOW_PULSES: u32 = PASSES_PER_SEC * 5;

/// The chat counter's refusal threshold, from `IncreaseChatCounter() >= 4` at
/// `input_main.cpp:808` with `ENABLE_CHAT_SPAMLIMIT` defined.
pub const CHAT_COUNTER_LIMIT: u8 = 4;

/// The counter value at which legacy disconnects, from
/// `ch->GetChatCounter() == 10` at `input_main.cpp:810`.
pub const CHAT_COUNTER_DISCONNECT: u8 = 10;

/// The `[LS;652]` locale key `SendBlockChatInfo` sends for a zero or negative
/// remaining duration (`input_main.cpp:128-132`).
pub const BLOCK_CHAT_NO_DURATION: &str = "[LS;652]";

/// The shout level refusal's format, `LC_TEXT("Shout can only be used at level %d or
/// higher.")` with `g_iShoutLimitLevel` (`input_main.cpp:888`).
pub const SHOUT_BELOW_LEVEL: &[u8] = b"Shout can only be used at level %d or higher.";

/// The per-descriptor chat state legacy keeps in `CHARACTER`: the `BYTE`
/// `m_bChatCounter` behind `IncreaseChatCounter` (`char.cpp:8757-8765`), and the
/// `dwLastShoutPulse` behind `GetLastShoutPulse`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatState {
    counter: u8,
    /// The counter window of the last line that reached the counter.
    window: u64,
    /// The Pulse of the last shout that was sent, or `None` before the first.
    last_shout: Option<u64>,
}

impl ChatState {
    /// A fresh session, whose counter is zero and which has never shouted.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            counter: 0,
            window: 0,
            last_shout: None,
        }
    }

    /// The current `m_bChatCounter`.
    #[must_use]
    pub const fn counter(&self) -> u8 {
        self.counter
    }

    /// `ResetChatCounter` (`char.cpp`) as `CHARACTER_MANAGER::Update` runs it, for a line at
    /// `pulse`: the counter goes back to zero when a reset Pulse has passed since the last
    /// line, which is when `pulse` is in a later window of [`CHAT_COUNTER_WINDOW_PULSES`].
    pub fn reset_chat_counter_at(&mut self, pulse: u64) {
        let window = pulse / u64::from(CHAT_COUNTER_WINDOW_PULSES);
        if window != self.window {
            self.window = window;
            self.counter = 0;
        }
    }

    /// The Pulse of the last shout that was sent, or `None` before the first.
    #[must_use]
    pub const fn last_shout(&self) -> Option<u64> {
        self.last_shout
    }

    /// `return ++m_bChatCounter;` (`char.cpp:8759`), including the wrap at 256.
    pub fn increase_chat_counter(&mut self) -> u8 {
        self.counter = self.counter.wrapping_add(1);
        self.counter
    }

    /// `return m_bChatCounter;` (`char.cpp:8764`).
    #[must_use]
    pub const fn get_chat_counter(&self) -> u8 {
        self.counter
    }
}

/// What the sender needs to be known by before a line can be judged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatContext {
    /// `ch->GetName()`, the `snprintf` prefix of the outgoing text.
    pub name: String,
    /// `ch->GetVID()`, the `id` field of the outgoing record.
    pub vid: u32,
    /// `ch->GetEmpire()`, the `bEmpire` of the outgoing record.
    pub empire: u8,
    /// `ch->GetLevel()`, which gates a shout and the spam-block check.
    pub level: u8,
    /// `ch->GetDesc()->GetLanguage()`, whose country code opens a shout.
    pub language: u8,
    /// `thecore_heart->pulse`, the game thread's Pulse when the line is judged.
    pub pulse: u64,
    /// `g_iShoutLimitLevel`, from `[game] shout_limit_level`.
    pub shout_limit_level: i32,
    /// The remaining `AFFECT_BLOCK_CHAT` duration, or `None` when the affect is
    /// absent. Legacy has no way to set it yet, so live it is always `None`.
    pub block_chat_seconds: Option<i32>,
}

/// What a judged line asks the descriptor to do.
///
/// The variants are named after the legacy arm that produces them, and each one
/// says whether it reaches the sender, the map, or only the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatEffect {
    /// `SetPhase(PHASE_CLOSE)` from the short-size refusal (`input_main.cpp:790-795`).
    Close,
    /// `interpret_command(ch, buf + 1, buflen - 1)` (`:804`), carrying the text after
    /// the `/`. Reaches the sender only, through the command interpreter.
    Command {
        /// The command word and its arguments, exactly as the client sent them after
        /// the `/`.
        argument: Vec<u8>,
    },
    /// `DelayedDisconnect(0)` (`:811`). The frame is consumed and the descriptor is
    /// closed on the descriptor's own delayed-disconnect schedule.
    DelayedDisconnect,
    /// A `GC_CHAT` line to the sender alone, built the way `CHARACTER::ChatPacket`
    /// builds one (`char.cpp:5140-5189`): `id` is 0, the empire is the descriptor's,
    /// and `bCanFormat` is left at its constructor value of `true`.
    InfoToSender {
        /// The locale key or literal text, byte for byte.
        text: &'static str,
    },
    /// A `GC_CHAT` talking line to every character on the sender's map, built the way
    /// `CInputMain::Chat` builds one (`:916-953`): `id` is the speaker's VID,
    /// `bCanFormat` is `false`, and the text is `"<name> : <text>"`.
    ///
    /// The recipient filter is `GetMapIndex() == sender's` over every descriptor in
    /// the process (`input_main.cpp:691-692`). The sender is included, because the
    /// sender's own descriptor is in that set.
    TalkingToMap {
        /// The `id` field, the speaker's VID.
        vid: u32,
        /// The `bEmpire` field, the speaker's empire. Legacy leaves this byte
        /// uninitialised on this path; see the module note.
        empire: u8,
        /// The text, `"<name> : <text>"`, with no terminator.
        text: Vec<u8>,
    },
    /// `sys_err("Unknown chat type %d", pinfo->type)` (`:986`). Nothing is sent.
    UnknownType {
        /// The `type` byte the client sent, which legacy never range-checks.
        chat_type: u8,
    },
    /// The shout is below `g_iShoutLimitLevel` (`:886-890`): an info line to the sender that
    /// names the limit.
    ShoutBelowLevel {
        /// The configured limit, which the message names.
        limit: i32,
    },
    /// The shout is inside its cooldown of [`SHOUT_COOLDOWN_PULSES`] (`:893-894`). Nothing is
    /// sent.
    ShoutOnCooldown,
    /// A shout to every Channel (`:897-911`): `GG_SHOUT` to every other core and `SendShout`
    /// on this one, and on each core `FuncShout` sends it to every client with a character
    /// that [`hears_shout`] lets through, the sender included. Each client gets
    /// [`shout_line`] in its own language and empire.
    Shout {
        /// `ch->GetEmpire()`, the `bEmpire` of `GG_SHOUT`, which [`hears_shout`] compares.
        empire: u8,
        /// The text, `"|L<country code>|l <name> : <text>"`, from [`shout_text`].
        text: Vec<u8>,
    },
}

impl ChatEffect {
    /// Whether this effect reaches the client at all.
    ///
    /// A line that only logs is not an observable, so a parity scenario cannot pin it
    /// on the wire; a scenario pins [`ChatEffect::Close`] and
    /// [`ChatEffect::DelayedDisconnect`] by the connection ending instead.
    #[must_use]
    pub const fn reaches_client(&self) -> bool {
        matches!(
            self,
            Self::DelayedDisconnect
                | Self::InfoToSender { .. }
                | Self::TalkingToMap { .. }
                | Self::ShoutBelowLevel { .. }
                | Self::Shout { .. }
        )
    }
}

/// What one `CG_CHAT` frame asks for, after decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatOutcome {
    /// The declared `size` is under [`CG_CHAT_WIRE_SIZE`]; the descriptor closes.
    Close,
    /// The frame was judged. The effects are in legacy order.
    Judged {
        /// What to do, in order.
        effects: Vec<ChatEffect>,
    },
}

/// Judge one `CG_CHAT` frame, in legacy order.
///
/// The returned effects are produced in exactly the sequence the legacy handler
/// would perform its side effects, so a caller can apply them in order without
/// re-deriving the sequence.
pub fn judge_chat(record: &CgChat, state: &mut ChatState, context: &ChatContext) -> ChatOutcome {
    if usize::from(record.declared_size) < CG_CHAT_WIRE_SIZE {
        return ChatOutcome::Close;
    }

    // `if (buflen > 1 && *buf == '/')` (`input_main.cpp:802`). The bound is
    // deliberately `> 1`, so a lone `/` is not a command.
    if record.text.len() > 1 && record.text[0] == b'/' {
        return ChatOutcome::Judged {
            effects: vec![ChatEffect::Command {
                argument: record.text[1..].to_vec(),
            }],
        };
    }

    // `#ifdef ENABLE_CHAT_SPAMLIMIT` (`input_main.cpp:807-813`), on the counter the last
    // reset left.
    state.reset_chat_counter_at(context.pulse);
    let counter = state.increase_chat_counter();
    if counter >= CHAT_COUNTER_LIMIT {
        let effects = if counter == CHAT_COUNTER_DISCONNECT {
            vec![ChatEffect::DelayedDisconnect]
        } else {
            Vec::new()
        };
        return ChatOutcome::Judged { effects };
    }

    // `ch->FindAffect(AFFECT_BLOCK_CHAT)` (`input_main.cpp:828-834`).
    if let Some(seconds) = context.block_chat_seconds {
        let text = if seconds <= 0 {
            BLOCK_CHAT_NO_DURATION
        } else {
            // Legacy would also send its uninitialised `buf` here. Only the locale
            // string is reproduced; see the module note.
            "[LS;1042]"
        };
        return ChatOutcome::Judged {
            effects: vec![ChatEffect::InfoToSender { text }],
        };
    }

    // `switch (pinfo->type)` (`input_main.cpp:926-988`). `SpamBlockCheck` and the
    // prism check sit between the block-chat affect and this switch; the first needs
    // a per-IP score the Rewrite has no store for and the second is dead in legacy.
    match record.chat_type {
        CHAT_TALKING => ChatOutcome::Judged {
            effects: vec![ChatEffect::TalkingToMap {
                vid: context.vid,
                empire: context.empire,
                text: talking_text(&context.name, &record.text),
            }],
        },
        CHAT_PARTY => ChatOutcome::Judged {
            // `if (!ch->GetParty()) ch->ChatPacket(CHAT_TYPE_INFO, "[LS;655]");`
            // The Rewrite has no party, so the no-party arm is the only reachable
            // one and it is the one taken.
            effects: vec![ChatEffect::InfoToSender { text: "[LS;655]" }],
        },
        CHAT_GUILD => ChatOutcome::Judged {
            effects: vec![ChatEffect::InfoToSender { text: "[LS;656]" }],
        },
        CHAT_SHOUT => ChatOutcome::Judged {
            effects: vec![judge_shout(record, state, context)],
        },
        other => ChatOutcome::Judged {
            effects: vec![ChatEffect::UnknownType { chat_type: other }],
        },
    }
}

/// The shout block (`input_main.cpp:882-913`): the level limit, then the cooldown, then the
/// line. `ShoutLog` and the battle pass mission run before the level limit in legacy
/// (`:871-877`) and are not ported.
fn judge_shout(record: &CgChat, state: &mut ChatState, context: &ChatContext) -> ChatEffect {
    if i32::from(context.level) < context.shout_limit_level {
        return ChatEffect::ShoutBelowLevel {
            limit: context.shout_limit_level,
        };
    }
    let cooldown = u64::from(SHOUT_COOLDOWN_PULSES);
    if state
        .last_shout
        .is_some_and(|last| context.pulse.saturating_sub(last) < cooldown)
    {
        return ChatEffect::ShoutOnCooldown;
    }
    state.last_shout = Some(context.pulse);
    ChatEffect::Shout {
        empire: context.empire,
        text: shout_text(context.language, &context.name, &record.text),
    }
}

/// Build `"|L<country code>|l <name> : <text>"`, the `snprintf` at `input_main.cpp:897`, with
/// `LC_LOCALE` of the speaker's language.
///
/// It is written into `char chatbuf[CHAT_MAX_LEN + 1]`, so the text is cut to `CHAT_MAX_LEN`
/// bytes, and `GG_SHOUT`'s `szText` has the same size.
#[must_use]
pub fn shout_text(language: u8, name: &str, text: &[u8]) -> Vec<u8> {
    let code = country_code(language);
    let mut out = Vec::with_capacity(code.len() + name.len() + text.len() + 8);
    out.extend_from_slice(b"|L");
    out.extend_from_slice(code.as_bytes());
    out.extend_from_slice(b"|l ");
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(b" : ");
    out.extend_from_slice(text);
    out.truncate(CHAT_MAX_LEN);
    out
}

/// `FuncShout`'s filter (`input_p2p.cpp:227`): a client hears a shout when shouts are
/// global or when its empire is the shouter's.
///
/// Legacy also lets a GM of any empire hear it. The Rewrite gives no character a GM level yet,
/// so every client is `GM_PLAYER` and that clause is not ported.
#[must_use]
pub const fn hears_shout(global: bool, shouter: u8, listener: u8) -> bool {
    global || shouter == listener
}

/// The `GC_CHAT` line one client gets for a shout: `ChatPacket(CHAT_TYPE_SHOUT, "%s", m_str)`
/// (`input_p2p.cpp:233`), so the format `"%s"` is looked up in the client's language, and the
/// empire is the client's.
#[must_use]
pub fn shout_line(text: &[u8], to: Recipient<'_>) -> Vec<u8> {
    chat_packet(to, CHAT_SHOUT, b"%s", &[Arg::Text(text)])
}

/// `FuncShout` for one client (`input_p2p.cpp:224-234`): the line `listener` gets for a shout
/// from the `shouter` empire, or `None` when it does not hear it.
///
/// The line is built from the listener's descriptor, as `ChatPacket` builds it, so it is looked
/// up in the listener's language and carries the listener's empire.
#[must_use]
pub fn shout_for(
    listener: &ClientEntry,
    strings: &LocaleStrings,
    global: bool,
    shouter: u8,
    text: &[u8],
) -> Option<Vec<u8>> {
    hears_shout(global, shouter, listener.empire).then(|| {
        let to = Recipient {
            strings,
            language: listener.language,
            empire: listener.empire,
        };
        shout_line(text, to)
    })
}

/// Build `"<name> : <text>"`, the `snprintf` at `input_main.cpp:868`.
///
/// Legacy writes into `char chatbuf[CHAT_MAX_LEN + 1]` (513 bytes) and then, at
/// `:879-880`, forces `len` to 512 when `snprintf` reported a truncation. That
/// sends up to 512 bytes of which the tail is uninitialised stack, so the Rewrite
/// sends the real bytes and records the defect instead of the silent truncation.
#[must_use]
pub fn talking_text(name: &str, text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(name.len() + 3 + text.len());
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(b" : ");
    out.extend_from_slice(text);
    out
}

/// Build the `GC_CHAT` record for one map broadcast, as `CInputMain::Chat` does.
///
/// `bCanFormat` is `false` on this path (`input_main.cpp:923`), which differs from
/// `CHARACTER::ChatPacket`, where the field keeps its constructor value of `true`.
#[must_use]
pub fn talking_record(effect: &ChatEffect) -> Option<GcChat> {
    let ChatEffect::TalkingToMap { vid, empire, text } = effect else {
        return None;
    };
    Some(GcChat {
        chat_type: CHAT_TALKING,
        id: *vid,
        empire: *empire,
        can_format: false,
        text: text.clone(),
    })
}

/// The encoded `GC_CHAT` line for an effect that `CHARACTER::ChatPacket` answers, or `None`
/// for every other effect.
///
/// The line is built by [`chat_packet`]: the text is looked up in the recipient's language and
/// then formatted. The one info text with an argument is the shout level refusal
/// (`input_main.cpp:888`, `LC_TEXT("Shout can only be used at level %d or higher.")`), and no
/// other info text holds a `%`.
#[must_use]
pub fn info_line(effect: &ChatEffect, to: Recipient<'_>) -> Option<Vec<u8>> {
    match effect {
        ChatEffect::InfoToSender { text } => Some(chat_packet(to, CHAT_INFO, text.as_bytes(), &[])),
        ChatEffect::ShoutBelowLevel { limit } => Some(chat_packet(
            to,
            CHAT_INFO,
            SHOUT_BELOW_LEVEL,
            &[Arg::Int(i64::from(*limit))],
        )),
        _ => None,
    }
}

/// Replace every banned word with `'*'`, as `CBanwordManager::ConvertString` does.
///
/// This is the filter legacy **intends**: `banword.cpp:73-125` walks the buffer
/// twice by character width, `memset`s each match with `'*'`, and leaves the length
/// alone, so the substituted line is the same number of bytes. The `'*'` skip at
/// `banword.cpp:102-107` stops the scan from re-examining an already-substituted
/// region.
///
/// # The deliberate Divergence
///
/// The only call on the chat path is commented out at `input_main.cpp:842` (the two
/// other call sites, `:434` and `:554`, are in `Whisper` and are also comments), so
/// the legacy server **sends a banned word unchanged**. Applying the filter is a
/// Divergence, recorded in the ledger, and not a Defect reproduction: a bad word
/// reaching every player on the map is a defect an honest client can cause.
///
/// The Rewrite has no banword list loaded yet, so live this is a no-op with an empty
/// list. The function is the real algorithm so that loading the list later needs no
/// rewrite of the chat path.
///
/// `is_twobyte` is per-locale (`locale_service.cpp:30`) and the Rewrite's only
/// Locale is `europe`, where the legacy pointer is the Latin-1 test, so a two-byte
/// character does not occur and every match is one byte wide.
#[must_use]
pub fn convert_banwords(text: &[u8], words: &[Vec<u8>]) -> Vec<u8> {
    let mut out = text.to_vec();
    for word in words {
        if word.is_empty() {
            continue;
        }
        let mut index = 0;
        while index < out.len() {
            if out[index] == b'*' {
                index += 1;
                continue;
            }
            let end = index + word.len();
            if end <= out.len() && out[index..end] == word[..] {
                for slot in &mut out[index..end] {
                    *slot = b'*';
                }
                index = end;
            } else {
                index += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamedata::locale_string::LanguageTable;
    use protocol::cg_wire::ClientFrame;

    fn context() -> ChatContext {
        ChatContext {
            name: "Ayla".to_string(),
            vid: 4242,
            empire: 0,
            level: 1,
            language: 1,
            pulse: 0,
            shout_limit_level: SHOUT_LIMIT_LEVEL,
            block_chat_seconds: None,
        }
    }

    fn say(text: &[u8]) -> CgChat {
        CgChat {
            declared_size: u16::try_from(text.len() + CG_CHAT_WIRE_SIZE).unwrap(),
            chat_type: CHAT_TALKING,
            text: text.to_vec(),
        }
    }

    fn judged(record: &CgChat, state: &mut ChatState) -> Vec<ChatEffect> {
        match judge_chat(record, state, &context()) {
            ChatOutcome::Close => vec![ChatEffect::Close],
            ChatOutcome::Judged { effects } => effects,
        }
    }

    #[test]
    fn a_size_under_four_closes_the_descriptor() {
        let mut state = ChatState::new();
        for declared in 0..4_u16 {
            let record = CgChat {
                declared_size: declared,
                chat_type: CHAT_TALKING,
                text: b"hi".to_vec(),
            };
            assert_eq!(
                judge_chat(&record, &mut state, &context()),
                ChatOutcome::Close,
                "size {declared} must close",
            );
        }
        assert_eq!(
            state.counter(),
            0,
            "a refused frame must not spend the counter"
        );
    }

    #[test]
    fn a_talking_line_becomes_one_map_broadcast() {
        let mut state = ChatState::new();
        let effects = judged(&say(b"hello"), &mut state);
        assert_eq!(
            effects,
            vec![ChatEffect::TalkingToMap {
                vid: 4242,
                empire: 0,
                text: b"Ayla : hello".to_vec(),
            }],
        );
        assert_eq!(state.counter(), 1);
    }

    #[test]
    fn the_outgoing_record_matches_the_legacy_fields() {
        let mut state = ChatState::new();
        let effects = judged(&say(b"hi"), &mut state);
        let record = talking_record(&effects[0]).expect("a record");
        // `input_main.cpp:918-924`: header, size, type, id, and bCanFormat = false.
        assert_eq!(record.chat_type, CHAT_TALKING);
        assert_eq!(record.id, 4242);
        assert_eq!(record.empire, 0);
        assert!(!record.can_format, "this path sets bCanFormat false");
        assert_eq!(record.text, b"Ayla : hi");
        assert_eq!(
            record.text.len() + 10,
            10 + "Ayla : hi".len(),
            "size counts the fixed part plus the text with no terminator",
        );
    }

    #[test]
    fn talking_record_refuses_every_other_effect() {
        assert!(talking_record(&ChatEffect::Close).is_none());
        assert!(talking_record(&ChatEffect::DelayedDisconnect).is_none());
        assert!(talking_record(&ChatEffect::ShoutOnCooldown).is_none());
    }

    #[test]
    fn a_slash_line_is_a_command_and_costs_no_counter() {
        let mut state = ChatState::new();
        let effects = judged(&say(b"/wave"), &mut state);
        assert_eq!(
            effects,
            vec![ChatEffect::Command {
                argument: b"wave".to_vec(),
            }],
        );
        assert_eq!(state.counter(), 0, "a command returns before the counter");
    }

    #[test]
    fn a_lone_slash_is_not_a_command() {
        let mut state = ChatState::new();
        // `buflen > 1` at `input_main.cpp:802`, so a single `/` falls through.
        let effects = judged(&say(b"/"), &mut state);
        assert_eq!(
            effects,
            vec![ChatEffect::TalkingToMap {
                vid: 4242,
                empire: 0,
                text: b"Ayla : /".to_vec(),
            }],
        );
        assert_eq!(state.counter(), 1);
    }

    #[test]
    fn the_counter_drops_the_fourth_line_and_disconnects_at_ten() {
        let mut state = ChatState::new();
        for index in 1..=3_u8 {
            let effects = judged(&say(b"hi"), &mut state);
            assert_eq!(effects.len(), 1, "line {index} must reach the map",);
            assert!(matches!(effects[0], ChatEffect::TalkingToMap { .. }));
        }
        // The fourth line is consumed with nothing sent.
        assert_eq!(judged(&say(b"hi"), &mut state), Vec::<ChatEffect>::new());
        for expected in 5..=9_u8 {
            assert_eq!(
                judged(&say(b"hi"), &mut state),
                Vec::<ChatEffect>::new(),
                "line {expected} must be dropped",
            );
        }
        // `ch->GetChatCounter() == 10` at `input_main.cpp:810`.
        assert_eq!(
            judged(&say(b"hi"), &mut state),
            vec![ChatEffect::DelayedDisconnect]
        );
        assert_eq!(state.counter(), 10);
    }

    #[test]
    fn the_counter_wraps_like_a_byte_and_then_works_again() {
        let mut state = ChatState::new();
        for _ in 0..9 {
            assert!(
                !judged(&say(b"hi"), &mut state)
                    .iter()
                    .any(|effect| matches!(effect, ChatEffect::DelayedDisconnect)),
                "only the tenth line disconnects",
            );
        }
        assert_eq!(
            judged(&say(b"hi"), &mut state),
            vec![ChatEffect::DelayedDisconnect]
        );
        assert_eq!(state.get_chat_counter(), 10);
        // An eleventh line is still dropped, because the arm needs exactly ten.
        assert_eq!(judged(&say(b"hi"), &mut state), Vec::<ChatEffect>::new());
        // The `BYTE` wraps at 256 and a wrapped counter is below every threshold, so
        // the session starts being served again.
        for _ in 0..245 {
            let _ = judged(&say(b"hi"), &mut state);
        }
        assert_eq!(state.get_chat_counter(), 0, "the counter wraps like a BYTE");
        assert_eq!(
            judged(&say(b"hi"), &mut state),
            vec![ChatEffect::TalkingToMap {
                vid: 4242,
                empire: 0,
                text: b"Ayla : hi".to_vec(),
            }],
        );
    }

    #[test]
    fn a_block_chat_affect_answers_with_the_locale_key() {
        let mut state = ChatState::new();
        let mut ctx = context();
        ctx.block_chat_seconds = Some(0);
        let record = say(b"hi");
        let ChatOutcome::Judged { effects } = judge_chat(&record, &mut state, &ctx) else {
            panic!("a block-chat affect must not close");
        };
        assert_eq!(
            effects,
            vec![ChatEffect::InfoToSender {
                text: BLOCK_CHAT_NO_DURATION
            }]
        );
        assert_eq!(
            state.counter(),
            1,
            "the affect is checked after the counter"
        );
    }

    #[test]
    fn a_positive_block_duration_sends_only_the_locale_string() {
        let mut state = ChatState::new();
        let mut ctx = context();
        ctx.block_chat_seconds = Some(65);
        let record = say(b"hi");
        let ChatOutcome::Judged { effects } = judge_chat(&record, &mut state, &ctx) else {
            panic!("a block-chat affect must not close");
        };
        assert_eq!(
            effects,
            vec![ChatEffect::InfoToSender { text: "[LS;1042]" }]
        );
    }

    #[test]
    fn party_and_guild_without_the_group_answer_with_their_locale_key() {
        let mut state = ChatState::new();
        for (chat_type, text) in [(CHAT_PARTY, "[LS;655]"), (CHAT_GUILD, "[LS;656]")] {
            let record = CgChat {
                declared_size: 6,
                chat_type,
                text: b"hi".to_vec(),
            };
            assert_eq!(
                judged(&record, &mut state),
                vec![ChatEffect::InfoToSender { text }],
                "type {chat_type} must answer with {text}",
            );
        }
    }

    #[test]
    fn a_shout_below_level_fifteen_is_refused_with_the_limit() {
        let mut state = ChatState::new();
        let record = CgChat {
            declared_size: 6,
            chat_type: CHAT_SHOUT,
            text: b"hi".to_vec(),
        };
        assert_eq!(
            judged(&record, &mut state),
            vec![ChatEffect::ShoutBelowLevel { limit: 15 }],
        );
    }

    fn shout_at(state: &mut ChatState, pulse: u64, level: u8) -> Vec<ChatEffect> {
        let record = CgChat {
            declared_size: 6,
            chat_type: CHAT_SHOUT,
            text: b"hi".to_vec(),
        };
        let ctx = ChatContext {
            level,
            pulse,
            ..context()
        };
        match judge_chat(&record, state, &ctx) {
            ChatOutcome::Close => vec![ChatEffect::Close],
            ChatOutcome::Judged { effects } => effects,
        }
    }

    fn sent() -> Vec<ChatEffect> {
        vec![ChatEffect::Shout {
            empire: 0,
            text: b"|Len|l Ayla : hi".to_vec(),
        }]
    }

    #[test]
    fn a_shout_at_level_fifteen_goes_out_with_the_speakers_country_code() {
        let mut state = ChatState::new();
        assert_eq!(shout_at(&mut state, 0, 15), sent());
        assert_eq!(state.last_shout(), Some(0));
        let mut state = ChatState::new();
        let ctx = ChatContext {
            level: 15,
            language: 5,
            empire: 3,
            ..context()
        };
        let record = CgChat {
            declared_size: 9,
            chat_type: CHAT_SHOUT,
            text: b"hallo".to_vec(),
        };
        assert_eq!(
            judge_chat(&record, &mut state, &ctx),
            ChatOutcome::Judged {
                effects: vec![ChatEffect::Shout {
                    empire: 3,
                    text: b"|Lde|l Ayla : hallo".to_vec(),
                }],
            },
        );
    }

    #[test]
    fn the_shout_cooldown_is_fifteen_seconds_of_pulses() {
        let mut state = ChatState::new();
        assert_eq!(shout_at(&mut state, 1000, 15), sent());
        // `pulse - last < passes_per_sec * 15` (`input_main.cpp:893`): 374 Pulses later is
        // still inside, 375 is not.
        assert_eq!(
            shout_at(&mut state, 1374, 15),
            vec![ChatEffect::ShoutOnCooldown]
        );
        assert_eq!(
            state.last_shout(),
            Some(1000),
            "a shout refused by the cooldown does not restart it",
        );
        assert_eq!(shout_at(&mut state, 1375, 15), sent());
        assert_eq!(state.last_shout(), Some(1375));
        assert_eq!(SHOUT_COOLDOWN_PULSES, 375);
    }

    #[test]
    fn the_first_shout_after_boot_is_not_refused() {
        // Legacy refuses every shout before Pulse 375, because the last shout starts at
        // Pulse 0. That Defect is not reproduced.
        let mut state = ChatState::new();
        assert_eq!(state.last_shout(), None);
        assert_eq!(shout_at(&mut state, 1, 15), sent());
        assert_eq!(
            shout_at(&mut state, 2, 15),
            vec![ChatEffect::ShoutOnCooldown]
        );
    }

    #[test]
    fn a_shout_below_the_level_does_not_start_the_cooldown() {
        let mut state = ChatState::new();
        assert_eq!(
            shout_at(&mut state, 1000, 14),
            vec![ChatEffect::ShoutBelowLevel { limit: 15 }]
        );
        assert_eq!(state.last_shout(), None);
        assert_eq!(shout_at(&mut state, 1001, 15), sent());
    }

    #[test]
    fn the_counter_runs_before_the_shout_block() {
        let mut state = ChatState::new();
        for _ in 0..3 {
            assert_eq!(shout_at(&mut state, 1000, 14).len(), 1);
        }
        // The fourth line in the window is dropped by the counter before the shout block.
        assert_eq!(shout_at(&mut state, 1000, 15), Vec::<ChatEffect>::new());
        assert_eq!(state.last_shout(), None);
    }

    #[test]
    fn the_shout_level_limit_comes_from_the_configuration() {
        let judge = |level: u8, limit: i32| {
            let ctx = ChatContext {
                level,
                shout_limit_level: limit,
                ..context()
            };
            let record = CgChat {
                declared_size: 6,
                chat_type: CHAT_SHOUT,
                text: b"hi".to_vec(),
            };
            match judge_chat(&record, &mut ChatState::new(), &ctx) {
                ChatOutcome::Judged { effects } => effects,
                ChatOutcome::Close => panic!("a shout must not close"),
            }
        };
        assert_eq!(
            judge(29, 30),
            vec![ChatEffect::ShoutBelowLevel { limit: 30 }]
        );
        assert_eq!(judge(30, 30), sent());
        // The configuration refuses a limit below 1, so 1 lets every character shout.
        assert_eq!(judge(1, 1), sent());
        // The level is a byte and the limit an `int`, so a limit above every level refuses
        // every level instead of wrapping.
        assert_eq!(
            judge(u8::MAX, 256),
            vec![ChatEffect::ShoutBelowLevel { limit: 256 }]
        );
    }

    #[test]
    fn the_counter_resets_when_a_line_arrives_in_a_later_window() {
        let mut state = ChatState::new();
        let mut talk = |pulse: u64| {
            let ctx = ChatContext { pulse, ..context() };
            match judge_chat(&say(b"hi"), &mut state, &ctx) {
                ChatOutcome::Judged { effects } => effects.len(),
                ChatOutcome::Close => panic!("a line must not close"),
            }
        };
        for _ in 0..3 {
            assert_eq!(talk(124), 1);
        }
        assert_eq!(
            talk(124),
            0,
            "the fourth line in Pulses 0 to 124 is dropped"
        );
        // Pulse 125 is a reset Pulse, and legacy runs the reset before that Pulse's input.
        assert_eq!(talk(125), 1);
        assert_eq!(talk(249), 1, "no reset inside the window");
        assert_eq!(talk(249), 1);
        assert_eq!(talk(249), 0);
        // A gap of several windows is one reset.
        assert_eq!(talk(1000), 1);
        assert_eq!(state.counter(), 1);
        assert_eq!(CHAT_COUNTER_WINDOW_PULSES, 125);
    }

    #[test]
    fn shout_text_is_the_snprintf_form_cut_to_the_chat_limit() {
        assert_eq!(shout_text(1, "Ayla", b"hi"), b"|Len|l Ayla : hi".to_vec());
        assert_eq!(shout_text(5, "Ayla", b"hi"), b"|Lde|l Ayla : hi".to_vec());
        assert_eq!(
            shout_text(0, "Ayla", b"hi"),
            b"|Len|l Ayla : hi".to_vec(),
            "an unknown language is English",
        );
        // The longest Name and the longest text overrun `chatbuf`.
        let name = "N".repeat(24);
        let text = vec![b'x'; 485];
        let long = shout_text(1, &name, &text);
        assert_eq!(long.len(), CHAT_MAX_LEN);
        assert_eq!(&long[..34], format!("|Len|l {name} : ").as_bytes());
        let fits = vec![b'y'; CHAT_MAX_LEN - b"|Len|l Ayla : ".len()];
        assert_eq!(shout_text(1, "Ayla", &fits).len(), CHAT_MAX_LEN);
        assert!(shout_text(1, "Ayla", &fits).ends_with(b"yy"));
    }

    #[test]
    fn a_shout_is_heard_by_its_empire_or_by_everyone_when_global() {
        assert!(hears_shout(false, 1, 1));
        assert!(!hears_shout(false, 1, 2));
        assert!(!hears_shout(false, 3, 2));
        assert!(hears_shout(true, 1, 2));
        assert!(hears_shout(true, 3, 3));
    }

    #[test]
    fn every_other_type_only_logs() {
        let mut checked = 0_u16;
        for chat_type in 0..=u8::MAX {
            // A fresh counter per value: the rate limit would otherwise swallow
            // every value after the third.
            let mut state = ChatState::new();
            let record = CgChat {
                declared_size: 6,
                chat_type,
                text: b"hi".to_vec(),
            };
            let expected = match chat_type {
                CHAT_TALKING | CHAT_PARTY | CHAT_GUILD | CHAT_SHOUT => None,
                other => Some(other),
            };
            let effects = judged(&record, &mut state);
            match expected {
                None => assert!(
                    !effects
                        .iter()
                        .any(|effect| matches!(effect, ChatEffect::UnknownType { .. })),
                    "type {chat_type} has a real arm",
                ),
                Some(other) => {
                    assert_eq!(effects, vec![ChatEffect::UnknownType { chat_type: other }]);
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 252, "256 values less the four live arms");
    }

    #[test]
    fn an_unknown_type_sends_nothing_to_the_client() {
        let mut state = ChatState::new();
        let record = CgChat {
            declared_size: 6,
            chat_type: 200,
            text: b"hi".to_vec(),
        };
        let effects = judged(&record, &mut state);
        assert!(!effects[0].reaches_client(), "the default arm only logs");
    }

    #[test]
    fn the_order_is_size_then_command_then_counter() {
        // A short size closes even when the text is a command.
        let mut state = ChatState::new();
        let record = CgChat {
            declared_size: 3,
            chat_type: CHAT_TALKING,
            text: b"/wave".to_vec(),
        };
        assert_eq!(
            judge_chat(&record, &mut state, &context()),
            ChatOutcome::Close,
        );
        // A command is judged before the counter, even at counter 3.
        state.increase_chat_counter();
        state.increase_chat_counter();
        state.increase_chat_counter();
        assert_eq!(state.counter(), 3);
        let effects = judged(&say(b"/wave"), &mut state);
        assert!(matches!(effects[0], ChatEffect::Command { .. }));
        assert_eq!(state.counter(), 3, "the counter is untouched by a command");
    }

    #[test]
    fn the_chat_type_values_are_the_compiler_values() {
        // `EChatType` at `server/server/common/length.h:403-419`, measured with
        // `ENABLE_DICE_SYSTEM` undefined.
        assert_eq!(CHAT_TALKING, 0);
        assert_eq!(CHAT_INFO, 1);
        assert_eq!(CHAT_NOTICE, 2);
        assert_eq!(CHAT_PARTY, 3);
        assert_eq!(CHAT_GUILD, 4);
        assert_eq!(CHAT_COMMAND, 5);
        assert_eq!(CHAT_SHOUT, 6);
        assert_eq!(CHAT_WHISPER, 7);
        assert_eq!(CHAT_BIG_NOTICE, 8);
        assert_eq!(CHAT_MONARCH_NOTICE, 9);
        assert_eq!(CHAT_MAX_NUM, 10);
    }

    #[test]
    fn talking_text_is_the_snprintf_form() {
        assert_eq!(talking_text("Ayla", b"hello"), b"Ayla : hello".to_vec());
        assert_eq!(talking_text("", b"x"), b" : x".to_vec());
        assert_eq!(talking_text("Ayla", b""), b"Ayla : ".to_vec());
    }

    #[test]
    fn convert_banwords_replaces_in_place_and_keeps_the_length() {
        let words = vec![b"bad".to_vec(), b"ugly".to_vec()];
        assert_eq!(
            convert_banwords(b"a bad word", &words),
            b"a *** word".to_vec()
        );
        assert_eq!(convert_banwords(b"uglybad", &words), b"*******".to_vec());
        assert_eq!(convert_banwords(b"clean", &words), b"clean".to_vec());
    }

    #[test]
    fn convert_banwords_ignores_an_empty_list_and_an_empty_word() {
        assert_eq!(convert_banwords(b"text", &[]), b"text".to_vec());
        let words = vec![Vec::new(), b"e".to_vec()];
        // An empty word would otherwise match everywhere and never advance.
        assert_eq!(convert_banwords(b"text", &words), b"t*xt".to_vec());
    }

    #[test]
    fn convert_banwords_does_not_rescan_a_substituted_region() {
        // `banword.cpp:102-107` skips over `'*'`, so a word that is a suffix of a
        // longer banned word is not matched inside the stars.
        let words = vec![b"abc".to_vec(), b"bc".to_vec()];
        assert_eq!(convert_banwords(b"abc", &words), b"***".to_vec());
    }

    #[test]
    fn convert_banwords_matches_every_occurrence() {
        let words = vec![b"ab".to_vec()];
        assert_eq!(convert_banwords(b"ab-ab-ab", &words), b"**-**-**".to_vec());
    }

    #[test]
    fn convert_banwords_is_a_no_op_with_an_empty_list_which_is_live_state() {
        // The Rewrite has no banword list loaded yet, so the live chat path is this.
        let mut state = ChatState::new();
        let effects = judged(&say(b"anything at all"), &mut state);
        assert_eq!(
            effects,
            vec![ChatEffect::TalkingToMap {
                vid: 4242,
                empire: 0,
                text: b"Ayla : anything at all".to_vec(),
            }],
        );
    }

    fn to(strings: &LocaleStrings, empire: u8) -> Recipient<'_> {
        Recipient {
            strings,
            language: 1,
            empire,
        }
    }

    #[test]
    fn an_info_line_is_the_legacy_chat_packet_with_the_speakers_empire() {
        // `GC_CHAT` (byte 4), `size` 18 as a little-endian `WORD`, `CHAT_TYPE_INFO`, `id` 0,
        // `bEmpire` 3, `bCanFormat` 1, then the text with no terminator.
        let strings = LocaleStrings::default();
        let effect = ChatEffect::InfoToSender { text: "[LS;655]" };
        let mut expected = vec![4, 18, 0, 1, 0, 0, 0, 0, 3, 1];
        expected.extend_from_slice(b"[LS;655]");
        assert_eq!(info_line(&effect, to(&strings, 3)), Some(expected));
        let other = info_line(&effect, to(&strings, 1)).expect("an info line");
        assert_eq!(other[8], 1, "the empire byte follows the descriptor");
    }

    #[test]
    fn the_shout_level_refusal_is_an_info_line_that_names_the_limit() {
        let strings = LocaleStrings::default();
        let effect = ChatEffect::ShoutBelowLevel { limit: 15 };
        let line = info_line(&effect, to(&strings, 2)).expect("a line");
        let text = b"Shout can only be used at level 15 or higher.";
        let mut expected = vec![4, 10 + 45, 0, 1, 0, 0, 0, 0, 2, 1];
        expected.extend_from_slice(text);
        assert_eq!(text.len(), 45);
        assert_eq!(line, expected);
        assert!(effect.reaches_client());
    }

    #[test]
    fn an_info_line_is_in_the_recipients_language() {
        let table = b"\"Shout can only be used at level %d or higher.\";\"Ab Stufe %d.\";\n\
                      \"[LS;655]\";\"keine Gruppe\";\n";
        let strings = LocaleStrings::default().with_table(5, LanguageTable::parse(table));
        let german = Recipient {
            language: 5,
            ..to(&strings, 1)
        };
        let shout = info_line(&ChatEffect::ShoutBelowLevel { limit: 15 }, german);
        assert_eq!(&shout.expect("a line")[10..], b"Ab Stufe 15.");
        let party = info_line(&ChatEffect::InfoToSender { text: "[LS;655]" }, german);
        assert_eq!(&party.expect("a line")[10..], b"keine Gruppe");
        let english = info_line(
            &ChatEffect::InfoToSender { text: "[LS;655]" },
            to(&strings, 1),
        );
        assert_eq!(&english.expect("a line")[10..], b"[LS;655]");
    }

    #[test]
    fn only_an_info_effect_builds_an_info_line() {
        let strings = LocaleStrings::default();
        assert!(info_line(&ChatEffect::Close, to(&strings, 1)).is_none());
        assert!(info_line(&ChatEffect::DelayedDisconnect, to(&strings, 1)).is_none());
        assert!(info_line(&ChatEffect::ShoutOnCooldown, to(&strings, 1)).is_none());
        assert!(!ChatEffect::ShoutOnCooldown.reaches_client());
        let talking = ChatEffect::TalkingToMap {
            vid: 1,
            empire: 1,
            text: b"Ayla : hi".to_vec(),
        };
        assert!(info_line(&talking, to(&strings, 1)).is_none());
    }

    #[test]
    fn a_shout_line_is_a_chat_packet_in_the_listeners_language_and_empire() {
        // `GC_CHAT`, `size`, `CHAT_TYPE_SHOUT`, `id` 0, the listener's `bEmpire`,
        // `bCanFormat` 1, then the text.
        let strings = LocaleStrings::default();
        let text = b"|Len|l Ayla : hi";
        let mut expected = vec![4, 26, 0, 6, 0, 0, 0, 0, 2, 1];
        expected.extend_from_slice(text);
        assert_eq!(shout_line(text, to(&strings, 2)), expected);
        assert_eq!(shout_line(text, to(&strings, 3))[8], 3);
        // `"%s"` is itself looked up in the listener's language.
        let table = b"\"%s\";\"> %s\";\n";
        let strings = LocaleStrings::default().with_table(5, LanguageTable::parse(table));
        let german = Recipient {
            language: 5,
            ..to(&strings, 1)
        };
        assert_eq!(&shout_line(text, german)[10..], b"> |Len|l Ayla : hi");
        assert_eq!(&shout_line(text, to(&strings, 1))[10..], text);
        // `%s` stops at a NUL.
        assert_eq!(&shout_line(b"a\0b", to(&strings, 1))[10..], b"a");
    }

    #[test]
    fn each_listener_gets_the_shout_in_its_own_language_and_empire() {
        // Given: a German table that translates the shout's `"%s"`, a German listener of empire
        // 2 and an English listener of empire 1.
        let table = b"\"%s\";\"> %s\";\n";
        let strings = LocaleStrings::default().with_table(5, LanguageTable::parse(table));
        let listener = |empire, language| ClientEntry {
            channel: 1,
            map: 1,
            name: "Listener".to_string(),
            vid: 1,
            empire,
            language,
        };
        let german = listener(2, 5);
        let english = listener(1, 1);
        let text = b"|Len|l Ayla : hi";

        // When: an empire-1 shout is built for each, global and not.
        let global_german = shout_for(&german, &strings, true, 1, text).expect("a line");
        let global_english = shout_for(&english, &strings, true, 1, text).expect("a line");

        // Then: each line is in the listener's language with the listener's empire, and only
        // a global shout reaches another empire.
        assert_eq!(&global_german[8..], b"\x02\x01> |Len|l Ayla : hi");
        assert_eq!(&global_english[8..], b"\x01\x01|Len|l Ayla : hi");
        assert_eq!(shout_for(&german, &strings, false, 1, text), None);
        assert_eq!(
            shout_for(&english, &strings, false, 1, text),
            Some(global_english)
        );
        assert_eq!(
            shout_for(&german, &strings, false, 2, text),
            Some(global_german)
        );
    }

    #[test]
    fn a_shout_reaches_the_client_and_builds_no_other_line() {
        let strings = LocaleStrings::default();
        let effect = sent().remove(0);
        assert!(effect.reaches_client());
        assert!(info_line(&effect, to(&strings, 1)).is_none());
        assert!(talking_record(&effect).is_none());
    }

    #[test]
    fn a_record_decoded_from_a_frame_behaves_the_same() {
        let payload = [0x08, 0x00, CHAT_TALKING, b'h', b'i'];
        let frame = ClientFrame::new(CgChat::header(), payload);
        let record = CgChat::decode(&frame).expect("a valid line");
        let mut state = ChatState::new();
        let effects = judged(&record, &mut state);
        assert_eq!(
            effects,
            vec![ChatEffect::TalkingToMap {
                vid: 4242,
                empire: 0,
                text: b"Ayla : hi".to_vec(),
            }],
        );
    }
}
