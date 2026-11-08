//! Empire rate bonuses: `/priv_empire` (`G/cmd_gm.cpp:3107-3167`, `G/priv_manager.cpp`).
//!
//! [`GameState::run_priv_empire`] is `ACMD(do_priv_empire)`: four words (empire 0-3, type
//! 1-4, value, duration in hours), the usage lines when one is missing or out of range,
//! and the grant, clamped to 0..200 and a week. [`GameState::priv_of`] is
//! `CPrivManager::GetPriv` whole: the character row first (a negative one wins unless
//! the 70052 unique is worn), else the max of the all-empires row, the character's own
//! row, and its guild's row. The guild and character grants arrive only through the
//! quest calls (`_give_guild_privilege`, `_give_char_privilege`), which no quest makes,
//! and the GM `priv_guild` dead-ends in an empty Lua buffer; the arms exist so the read
//! is whole, and they hold nothing.
//!
//! Legacy stores the grants in the DB process (`D/PrivManager.cpp`) and fans them to
//! every game process; this build holds them in the world and the one process is every
//! Channel (ADR-0002). The expiry is counted in Pulses: the grant's end Pulse is stored
//! beside its value, and the Pulse step clears what ran out, as the DB process's
//! `Update` clears what expired (`D/PrivManager.cpp:40-79`). A grant with value 0
//! still notices, as legacy's does.
//!
//! What the client hears: the grant's `[LS;724;empire;priv;value]` notice to every
//! client (`SendPrivNotice`, `G/cmd_gm.cpp:1338-1361`), and the enter-game
//! `[LS;871-874]` notices for the live bonuses (`_send_bonus_info`,
//! `G/input_login.cpp:39-66`).

use common::vid::Vid;

use protocol::gc_chat::CHAT_TYPE_INFO;

use crate::chat_line::{chat_packet, Arg};
use crate::game_state::GameState;
use crate::item_move::Mover;
use crate::save::PASSES_PER_SEC;
use world::character::Character;

/// `PRIV_ITEM_DROP` (`common/length.h:812`): the item-drop rate bonus.
pub const PRIV_ITEM_DROP: u8 = 1;
/// `PRIV_GOLD_DROP` (`:813`): the gold-drop rate bonus.
pub const PRIV_GOLD_DROP: u8 = 2;
/// `PRIV_GOLD10_DROP` (`:814`): the tenfold-gold rate bonus.
pub const PRIV_GOLD10_DROP: u8 = 3;
/// `PRIV_EXP_PCT` (`:815`): the experience rate bonus.
pub const PRIV_EXP_PCT: u8 = 4;
/// `MAX_PRIV_NUM` (`:816`): the count of priv types, `PRIV_NONE` included.
pub const MAX_PRIV_NUM: usize = 5;
/// `EMPIRE_MAX_NUM` (`common/length.h:38`): all-empires plus the three empires.
pub const EMPIRE_MAX_NUM: usize = 4;

/// The longest grant: a week (`60*60*24*7`), in Pulses.
const MAX_GRANT_PULSES: u64 = 7 * 24 * 60 * 60 * PASSES_PER_SEC as u64;

/// `c_apszEmpireNames` (`G/constants.cpp:1072-1078`): the notice's empire word, by empire.
const EMPIRE_NAMES: [&[u8]; EMPIRE_MAX_NUM] = [b"[LS;1037]", b"[LS;793]", b"[LS;794]", b"[LS;795]"];
/// `c_apszPrivNames` (`:1080-1087`): the notice's priv word, by type.
const PRIV_NAMES: [&[u8]; MAX_PRIV_NUM] =
    [b"", b"[LS;1038]", b"[LS;1039]", b"[LS;1031]", b"[LS;1040]"];

/// The priv word `PRIV_NAMES` carries for `priv_type`, empty past the table.
pub(super) fn priv_word(priv_type: usize) -> &'static [u8] {
    PRIV_NAMES.get(priv_type).copied().unwrap_or(b"")
}

/// The hours a priv string prints for a grant ending at `ends`, read on `pulse`:
/// legacy's `(end_time_sec - now) / 3600.0f` (`G/questlua_global.cpp:565`). The
/// seconds saturate to `u32` (past 136 years) so the float conversion is exact;
/// the `f64` prints shortest, where the stream prints six significant digits.
pub(super) fn priv_hours(ends: u64, pulse: u64) -> f64 {
    let secs = ends.saturating_sub(pulse) / u64::from(PASSES_PER_SEC);
    let secs = u32::try_from(secs).unwrap_or(u32::MAX);
    f64::from(secs) / 3600.0
}
/// The zero-value removal line's bytes around the two `%s` words
/// (`G/priv_manager.cpp:171`), read byte-exact: `"%s" + middle + "%s" + tail`.
const REMOVAL_MIDDLE: &[u8] = b"A\xc7 ";
const REMOVAL_TAIL: &[u8] = b"AI \xc1\xa4\xbb\xf3A\xb8\xb7\xce \xb513A?\xd41A\xb4I\xb4U.";
/// The guild grant's notice format (`:108`), read byte-exact.
const GUILD_GRANT_FORMAT: &[u8] = b"%s \xb1a\xb5aA\xc7 %sAI %d%% \xc1o\xb0!\xc7\xdf1A\xb4I\xb4U!";
/// The guild removal's notice format (`:114`), read byte-exact.
const GUILD_REMOVAL_FORMAT: &[u8] =
    b"%s \xb1a\xb5aA\xc7 %sAI \xc1\xa4\xbb\xf3A\xb8\xb7\xce \xb513A?\xd41A\xb4I\xb4U.";

/// One grant: its clamped value and the Pulse it ends on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct EmpirePriv {
    /// The clamped value.
    pub(super) value: i32,
    /// The Pulse the grant ends on.
    pub(super) ends: u64,
}

/// The empire grants, `[type][empire]` (`m_aakPrivEmpireData`).
pub(super) type EmpirePrivs = [[EmpirePriv; EMPIRE_MAX_NUM]; MAX_PRIV_NUM];

/// The grants a quest step's priv readers read: snapshots of the world's empire and
/// guild stores and the Pulse they are read on. The character store is not shadowed:
/// legacy has no quest getter for it (`GetPrivByCharacter` serves only `GetPriv`).
#[derive(Debug, Clone, Default)]
pub(super) struct PrivShadow {
    /// The empire grants.
    pub(super) empire: EmpirePrivs,
    /// The guild grants, by type.
    pub(super) guild: [std::collections::HashMap<u32, EmpirePriv>; MAX_PRIV_NUM],
    /// The Pulse the step reads on.
    pub(super) pulse: u64,
}

/// One character grant: its clamped value and the Pulse it ends on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct CharacterPriv {
    /// The clamped value, negative for a bad grant.
    pub(super) value: i32,
    /// The Pulse the grant ends on.
    pub(super) ends: u64,
}

/// `(int)strtol(word)` (`common/utils.h:44-50`), saturating past `i32`'s ends.
fn parse_priv_number(word: &[u8]) -> i32 {
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
    /// `ACMD(do_priv_empire)`: the usage lines when a word is missing or out of range,
    /// else the grant and its notice to every client.
    pub(super) fn run_priv_empire(&mut self, vid: Vid, rest: &[u8], mover: Mover) {
        let (empire_word, back) = crate::command::one_argument(rest);
        let (type_word, back) = crate::command::one_argument(back);
        let (value_word, back) = crate::command::one_argument(back);
        let (duration_word, _) = crate::command::one_argument(back);
        if empire_word.is_empty()
            || type_word.is_empty()
            || value_word.is_empty()
            || duration_word.is_empty()
        {
            self.priv_usage(vid, mover);
            return;
        }
        let empire = parse_priv_number(&empire_word);
        let priv_type = parse_priv_number(&type_word);
        let mut value = parse_priv_number(&value_word);
        // `value = MINMAX(0, value, 1000)`: the command clamps before the range checks,
        // so a value past 1000 is not a usage error.
        value = value.clamp(0, 1000);
        let duration = parse_priv_number(&duration_word);
        if !(0..=3).contains(&empire) || !(1..=4).contains(&priv_type) || value < 0 || duration < 0
        {
            self.priv_usage(vid, mover);
            return;
        }
        let hours = u64::try_from(duration).unwrap_or(u64::MAX);
        let pulses = hours
            .saturating_mul(60 * 60 * u64::from(PASSES_PER_SEC))
            .min(MAX_GRANT_PULSES);
        // The range check above holds both in `u8`; the conversion cannot fail.
        let (empire, priv_type) = (
            u8::try_from(empire).expect("empire is 0..3"),
            u8::try_from(priv_type).expect("the type is 1..4"),
        );
        self.give_empire_priv(
            empire,
            priv_type,
            value.clamp(0, 200),
            self.last_pulse.saturating_add(pulses),
        );
    }

    /// The `usage : priv_empire` lines.
    fn priv_usage(&self, vid: Vid, mover: Mover) {
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"usage : priv_empire <empire> <type> <value> <duration>",
            &[],
        );
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"  <empire>    0 - 3 (0==all)",
            &[],
        );
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"  <type>      1:item_drop, 2:gold_drop, 3:gold10_drop, 4:exp",
            &[],
        );
        self.tell(vid, mover, CHAT_TYPE_INFO, b"  <value>     percent", &[]);
        self.tell(vid, mover, CHAT_TYPE_INFO, b"  <duration>  hour", &[]);
    }

    /// `GiveEmpirePriv` (`G/priv_manager.cpp:142-180`): the clamped grant and its notice
    /// to every client (`SendPrivNotice`): `[LS;724;empire;priv;value]` when the value
    /// is nonzero, else the raw removal line with the two words baked in.
    pub(super) fn give_empire_priv(&mut self, empire: u8, priv_type: u8, value: i32, ends: u64) {
        let (empire, priv_type) = (usize::from(empire), usize::from(priv_type));
        if priv_type >= MAX_PRIV_NUM || empire >= EMPIRE_MAX_NUM {
            return;
        }
        self.empire_privs[priv_type][empire] = EmpirePriv { value, ends };
        // The notice names the grant's empire, not the hearer's: the empire word is the
        // grant's, and each hearer translates both words into its own language
        // (`LC_LOCALE_TEXT(word, d->GetLanguage())`, `G/cmd_gm.cpp:1338-1361`) before
        // the format is rendered.
        let empire_word = EMPIRE_NAMES[empire];
        let priv_word = PRIV_NAMES[priv_type];
        if let Some(clients) = self.clients.as_ref() {
            if value != 0 {
                clients.deliver_everywhere(|entry| {
                    let empire_word = self.locale.find(empire_word, entry.language);
                    let priv_word = self.locale.find(priv_word, entry.language);
                    Some(chat_packet(
                        crate::chat_line::Recipient {
                            strings: &self.locale,
                            language: entry.language,
                            empire: entry.empire,
                        },
                        protocol::gc_chat::CHAT_TYPE_NOTICE,
                        b"[LS;724;%s;%s;%d]",
                        &[
                            Arg::Text(empire_word),
                            Arg::Text(priv_word),
                            Arg::Int(i64::from(value)),
                        ],
                    ))
                });
            } else {
                // The removal line is not an LS key (`:170-174`): the two translated
                // words are the `%s` arguments, and the hearer's empire rides the
                // record, as the descriptor's does. The bytes are the legacy file's,
                // read byte-exact.
                let mut format = Vec::with_capacity(64);
                format.extend_from_slice(b"%s");
                format.extend_from_slice(REMOVAL_MIDDLE);
                format.extend_from_slice(b"%s");
                format.extend_from_slice(REMOVAL_TAIL);
                clients.deliver_everywhere(|entry| {
                    let empire_word = self.locale.find(empire_word, entry.language);
                    let priv_word = self.locale.find(priv_word, entry.language);
                    Some(chat_packet(
                        crate::chat_line::Recipient {
                            strings: &self.locale,
                            language: entry.language,
                            empire: entry.empire,
                        },
                        protocol::gc_chat::CHAT_TYPE_NOTICE,
                        &format,
                        &[Arg::Text(empire_word), Arg::Text(priv_word)],
                    ))
                });
            }
        }
    }

    /// `CPrivManager::GetPriv` (`G/priv_manager.cpp:224-244`): the character row first —
    /// a negative one wins unless the 70052 unique is worn — else the max of the
    /// all-empires row, the character's own row, and its guild's row. An expired grant
    /// reads as 0 even before the Pulse step clears it.
    pub(super) fn priv_of(&self, vid: Vid, priv_type: u8) -> i32 {
        let priv_type = usize::from(priv_type);
        if priv_type >= MAX_PRIV_NUM {
            return 0;
        }
        let character = self.characters.find_by_vid(vid).ok();
        let pid = character.map(Character::player_id);
        let by_character = pid.map_or(0, |pid| {
            self.character_privs
                .get(priv_type)
                .and_then(|row| row.get(&pid))
                .map_or(0, |grant| {
                    if self.last_pulse >= grant.ends {
                        0
                    } else {
                        grant.value
                    }
                })
        });
        if by_character < 0 && !self.wears_unique(vid, 70_052) {
            return by_character;
        }
        let empire = self.bodies.get(&vid).map_or(0, |body| body.card.empire);
        let empire = usize::from(empire);
        let live = |grant: EmpirePriv| {
            if self.last_pulse >= grant.ends {
                0
            } else {
                grant.value
            }
        };
        let mut best = by_character.max(live(self.empire_privs[priv_type][0]));
        if empire < EMPIRE_MAX_NUM {
            best = best.max(live(self.empire_privs[priv_type][empire]));
        }
        if let Some(pid) = pid {
            if let Some(guild) = self.guild_of(pid) {
                if let Some(grant) = self
                    .guild_privs
                    .get(priv_type)
                    .and_then(|row| row.get(&guild))
                {
                    best = best.max(live(*grant));
                }
            }
        }
        best
    }

    /// `GiveGuildPriv` (`G/priv_manager.cpp:88-124`): the clamped grant and its notice
    /// to every online character's client (`SendGuildPrivNotice`), silent with no such
    /// guild, as legacy's is. Only the quest call reaches it, and no quest makes it.
    pub(super) fn give_guild_priv(&mut self, guild: u32, priv_type: u8, value: i32, ends: u64) {
        let priv_type = usize::from(priv_type);
        if priv_type >= MAX_PRIV_NUM {
            return;
        }
        let value = value.clamp(0, 50);
        self.guild_privs[priv_type].insert(guild, EmpirePriv { value, ends });
        let Some(name) = self.guild_name(guild) else {
            return;
        };
        let priv_word = PRIV_NAMES[priv_type];
        if let Some(clients) = self.clients.as_ref() {
            if value != 0 {
                clients.deliver_everywhere(|entry| {
                    let priv_word = self.locale.find(priv_word, entry.language);
                    Some(chat_packet(
                        crate::chat_line::Recipient {
                            strings: &self.locale,
                            language: entry.language,
                            empire: entry.empire,
                        },
                        protocol::gc_chat::CHAT_TYPE_NOTICE,
                        GUILD_GRANT_FORMAT,
                        &[
                            Arg::Text(&name),
                            Arg::Text(priv_word),
                            Arg::Int(i64::from(value)),
                        ],
                    ))
                });
            } else {
                clients.deliver_everywhere(|entry| {
                    let priv_word = self.locale.find(priv_word, entry.language);
                    Some(chat_packet(
                        crate::chat_line::Recipient {
                            strings: &self.locale,
                            language: entry.language,
                            empire: entry.empire,
                        },
                        protocol::gc_chat::CHAT_TYPE_NOTICE,
                        GUILD_REMOVAL_FORMAT,
                        &[Arg::Text(&name), Arg::Text(priv_word)],
                    ))
                });
            }
        }
    }

    /// The quest guild grant (`RequestGiveGuildPriv` through the DB process's
    /// `AddGuildPriv`, `G/priv_manager.cpp:27-46`, `D/PrivManager.cpp:114-150`): the value
    /// clamps to 0..50 and the seconds to a week, and the grant ends that many seconds
    /// on, in Pulses.
    pub(super) fn grant_guild_priv(&mut self, guild: u32, priv_type: u32, value: i32, time: i32) {
        let Ok(priv_type) = u8::try_from(priv_type) else {
            return;
        };
        let secs = u64::try_from(time.max(0))
            .unwrap_or(u64::MAX)
            .min(7 * 24 * 60 * 60);
        let ends = self
            .last_pulse
            .saturating_add(secs.saturating_mul(u64::from(PASSES_PER_SEC)));
        self.give_guild_priv(guild, priv_type, value, ends);
    }

    /// The quest character grant (`RequestGiveCharacterPriv` through `AddCharPriv`,
    /// `G/priv_manager.cpp:67-84`, `D/PrivManager.cpp:82-104`): the value clamps to
    /// 0..100 on the game side, and the DB side's refusal and expiry stand.
    pub(super) fn grant_character_priv(&mut self, reader: Vid, priv_type: u32, value: i32) {
        let Ok(priv_type) = u8::try_from(priv_type) else {
            return;
        };
        let Some(pid) = self
            .characters
            .find_by_vid(reader)
            .ok()
            .map(Character::player_id)
        else {
            return;
        };
        self.give_character_priv(pid, priv_type, value.clamp(0, 100));
    }

    /// The quest empire grant (`RequestGiveEmpirePriv` through `AddEmpirePriv`,
    /// `G/priv_manager.cpp:48-65`, `D/PrivManager.cpp:152-186`): the value clamps to
    /// 0..200 and the seconds to a week.
    pub(super) fn grant_empire_priv_quest(
        &mut self,
        empire: u32,
        priv_type: u32,
        value: i32,
        time: i32,
    ) {
        let (Ok(empire), Ok(priv_type)) = (u8::try_from(empire), u8::try_from(priv_type)) else {
            return;
        };
        let secs = u64::try_from(time.max(0))
            .unwrap_or(u64::MAX)
            .min(7 * 24 * 60 * 60);
        let ends = self
            .last_pulse
            .saturating_add(secs.saturating_mul(u64::from(PASSES_PER_SEC)));
        self.give_empire_priv(empire, priv_type, value.clamp(0, 200), ends);
    }

    /// `GiveCharacterPriv` (`G/priv_manager.cpp:126-140`): the clamped grant, silent.
    /// The DB process refuses a second grant while one stands and a zero without one
    /// (`D/PrivManager.cpp:82-104`); the character grants expire after 2 hours good or
    /// 1 hour bad, which the Pulse step clears. Only the quest call reaches it, and no
    /// quest makes it.
    pub(super) fn give_character_priv(&mut self, pid: u32, priv_type: u8, value: i32) {
        let priv_type = usize::from(priv_type);
        if priv_type >= MAX_PRIV_NUM {
            return;
        }
        if self.character_privs[priv_type].contains_key(&pid) || value == 0 {
            return;
        }
        let hours = if value > 0 { 2 } else { 1 };
        let ends = self
            .last_pulse
            .saturating_add(hours * 60 * 60 * u64::from(PASSES_PER_SEC));
        self.character_privs[priv_type].insert(pid, CharacterPriv { value, ends });
    }

    /// Clear the grants whose end Pulse passed. Runs on the Pulse step, as the DB
    /// process's `Update` clears them (`D/PrivManager.cpp:40-79`).
    pub(super) fn expire_empire_privs(&mut self) {
        for row in self.empire_privs.iter_mut().flatten() {
            if self.last_pulse >= row.ends {
                *row = EmpirePriv::default();
            }
        }
        let last = self.last_pulse;
        for row in &mut self.guild_privs {
            row.retain(|_, grant| last < grant.ends);
        }
        for row in &mut self.character_privs {
            row.retain(|_, grant| last < grant.ends);
        }
    }

    /// The enter-game priv notices (`G/input_login.cpp:39-66`): `[LS;871-874]` for each
    /// live bonus, in type order. The burst carries them after the Channel record; the
    /// body's empire rides the record, as the descriptor's does.
    pub(super) fn priv_bonus_lines(&self, vid: Vid) -> Vec<Vec<u8>> {
        let empire = self.bodies.get(&vid).map_or(0, |body| body.card.empire);
        let mut lines = Vec::new();
        for (priv_type, key) in [
            (PRIV_ITEM_DROP, b"[LS;871;%d]"),
            (PRIV_GOLD_DROP, b"[LS;872;%d]"),
            (PRIV_GOLD10_DROP, b"[LS;873;%d]"),
            (PRIV_EXP_PCT, b"[LS;874;%d]"),
        ] {
            let bonus = self.priv_of(vid, priv_type);
            if bonus != 0 {
                lines.push(chat_packet(
                    crate::chat_line::Recipient {
                        strings: &self.locale,
                        language: self.bodies.get(&vid).map_or(0, |body| body.card.language),
                        empire,
                    },
                    protocol::gc_chat::CHAT_TYPE_NOTICE,
                    key,
                    &[Arg::Int(i64::from(bonus))],
                ));
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_priv_number, PRIV_EXP_PCT, PRIV_GOLD10_DROP, PRIV_GOLD_DROP, PRIV_ITEM_DROP,
    };
    use crate::client_registry::{ChannelClients, ClientEntry};
    use crate::game_state::fixtures::{a_world, enter, TestClock, PLACE};
    use common::vid::Vid;
    use std::sync::Arc;

    /// `strtol`'s walk: the sign, the digit run, the saturation, and the no-digit zero.
    #[test]
    fn the_priv_number_reads_strtol() {
        assert_eq!(parse_priv_number(b"123"), 123);
        assert_eq!(parse_priv_number(b"+123abc"), 123);
        assert_eq!(parse_priv_number(b"  123"), 0);
        assert_eq!(parse_priv_number(b"-5"), -5);
        assert_eq!(parse_priv_number(b"99999999999999999999"), i32::MAX);
        assert_eq!(parse_priv_number(b"-99999999999999999999"), i32::MIN);
        assert_eq!(parse_priv_number(b"no digits"), 0);
        assert_eq!(parse_priv_number(b""), 0);
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

    /// The grant lands clamped and noticed; its own row beats the all-row; the
    /// expired grant reads 0; the cleared row is gone.
    #[test]
    fn the_grant_lands_clamped_noticed_and_expiring() {
        let clock = TestClock::default();
        let clients = Arc::new(ChannelClients::new());
        let mut world = a_world(&clock).with_clients(Arc::clone(&clients));
        let _inbox = enter(&mut world, VID, (3200, 3200), 0);
        let mut lease = clients.join(ClientEntry {
            channel: PLACE.0,
            map: PLACE.1,
            name: format!("P{VID}"),
            vid: VID,
            empire: 1,
            language: 0,
        });
        world.run_priv_empire(Vid::new(VID), b"1 4 500 1", mover());
        // The 500 clamps to 200; the notice names empire 1 and exp.
        let notice = lease.try_next().expect("the grant is noticed");
        assert_eq!(text_of(&notice), b"[LS;724;[LS;793];[LS;1040];200]");
        assert_eq!(world.priv_of(Vid::new(VID), PRIV_EXP_PCT), 200);
        // The all-row undercuts the own row; the other empires' do not touch it.
        world.run_priv_empire(Vid::new(VID), b"0 4 50 1", mover());
        let _second = lease.try_next().expect("the all grant is noticed");
        assert_eq!(world.priv_of(Vid::new(VID), PRIV_EXP_PCT), 200);
        world.run_priv_empire(Vid::new(VID), b"2 1 70 1", mover());
        let _third = lease.try_next().expect("the other empire is noticed");
        assert_eq!(world.priv_of(Vid::new(VID), PRIV_ITEM_DROP), 0);
        assert_eq!(world.priv_of(Vid::new(VID), PRIV_GOLD_DROP), 0);
        assert_eq!(world.priv_of(Vid::new(VID), PRIV_GOLD10_DROP), 0);
        // The Pulse past the hour's end clears the grant; it reads 0 even first.
        world.expire_empire_privs();
        assert_eq!(world.priv_of(Vid::new(VID), PRIV_EXP_PCT), 200);
        world.last_pulse = 25 * 3600;
        assert_eq!(world.priv_of(Vid::new(VID), PRIV_EXP_PCT), 0);
        world.expire_empire_privs();
        assert_eq!(world.empire_privs[usize::from(PRIV_EXP_PCT)][1].value, 0);
        drop(lease);
    }

    /// A missing word, an empire past 3, a type past 4, and a negative duration
    /// each show the five usage lines and grant nothing.
    #[test]
    fn the_usage_lines_show_when_a_word_is_missing_or_out_of_range() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = enter(&mut world, VID, (3200, 3200), 0);
        for rest in [b"1 4 50".as_slice(), b"4 4 50 1", b"1 5 50 1", b"1 4 50 -1"] {
            world.run_priv_empire(Vid::new(VID), rest, mover());
            for expected in [
                b"usage : priv_empire <empire> <type> <value> <duration>".as_slice(),
                b"  <empire>    0 - 3 (0==all)",
                b"  <type>      1:item_drop, 2:gold_drop, 3:gold10_drop, 4:exp",
                b"  <value>     percent",
                b"  <duration>  hour",
            ] {
                let line = inbox.try_recv().expect("a usage line is queued");
                assert_eq!(text_of(&line), expected);
            }
        }
        assert_eq!(world.priv_of(Vid::new(VID), PRIV_EXP_PCT), 0);
    }

    /// The character loss wins over every gain, unless the 70052 unique is worn;
    /// the guild row joins the max; the second character grant and the zero refuse.
    #[test]
    fn the_character_loss_wins_unless_the_unique_is_worn() {
        use common::enums::EWearPositions;
        use common::item_slots::INVENTORY_MAX_NUM;
        use protocol::item_pos::ItemPos;
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, VID, (3200, 3200), 0);
        let pid = world
            .characters
            .find_by_vid(Vid::new(VID))
            .unwrap()
            .player_id();
        world.give_character_priv(pid, PRIV_EXP_PCT, -20);
        assert_eq!(world.priv_of(Vid::new(VID), PRIV_EXP_PCT), -20);
        world.give_empire_priv(1, PRIV_EXP_PCT, 200, 25 * 3600);
        assert_eq!(world.priv_of(Vid::new(VID), PRIV_EXP_PCT), -20);
        // The second grant while one stands refuses, and so does a zero.
        world.give_character_priv(pid, PRIV_EXP_PCT, 50);
        assert_eq!(world.priv_of(Vid::new(VID), PRIV_EXP_PCT), -20);
        world.give_character_priv(pid + 1, PRIV_EXP_PCT, 0);
        assert_eq!(world.character_privs[usize::from(PRIV_EXP_PCT)].len(), 1);
        // The unique in the first slot lifts the loss; the empire gain shows.
        let mut ward = world::item::Item::new(77, 70_052);
        ward.set_size(1).unwrap();
        let at = ItemPos::new(
            common::item_slots::EWindows::Inventory as u8,
            INVENTORY_MAX_NUM + EWearPositions::Unique1 as u16,
        );
        world
            .characters
            .find_by_vid_mut(Vid::new(VID))
            .unwrap()
            .items_mut()
            .set(at, &ward)
            .unwrap();
        assert_eq!(world.priv_of(Vid::new(VID), PRIV_EXP_PCT), 200);
        // The 2-hour good and 1-hour bad grants expire on the Pulse step.
        world.last_pulse = 25 * 3600 * 2;
        world.expire_empire_privs();
        assert_eq!(world.character_privs[usize::from(PRIV_EXP_PCT)].len(), 0);
        assert_eq!(world.priv_of(Vid::new(VID), PRIV_EXP_PCT), 0);
    }

    /// The guild grant clamps to 50, notices every client with the guild's name,
    /// and joins the member's max; a grant to no guild stores but stays silent.
    #[test]
    fn the_guild_grant_notices_and_joins_the_max() {
        let clock = TestClock::default();
        let clients = Arc::new(ChannelClients::new());
        let mut world = a_world(&clock).with_clients(Arc::clone(&clients));
        let _inbox = enter(&mut world, VID, (3200, 3200), 0);
        let mut lease = clients.join(ClientEntry {
            channel: PLACE.0,
            map: PLACE.1,
            name: format!("P{VID}"),
            vid: VID,
            empire: 1,
            language: 0,
        });
        let pid = world
            .characters
            .find_by_vid(Vid::new(VID))
            .unwrap()
            .player_id();
        world.install_guilds(
            &db::guilds::StoredGuilds {
                guilds: vec![db::guilds::StoredGuild {
                    id: 7,
                    name: "Knights".to_owned(),
                    master_pid: pid,
                    sp: 400,
                    level: 1,
                    exp: 0,
                    skill_point: 0,
                    skills: vec![0; 12],
                    ladder_point: 0,
                    win: 0,
                    draw: 0,
                    loss: 0,
                    gold: 0,
                }],
                grades: vec![db::guilds::StoredGrade {
                    guild_id: 7,
                    grade: 1,
                    name: "Leader".to_owned(),
                    auth: 15,
                }],
                members: vec![db::guilds::StoredMember {
                    pid,
                    guild_id: 7,
                    grade: 1,
                    is_general: false,
                    offer: 0,
                    level: 10,
                    job: 0,
                    name: format!("P{pid}"),
                }],
            },
            &[],
            &[],
        );
        world.link_guild(Vid::new(VID));
        world.give_guild_priv(7, PRIV_EXP_PCT, 500, 25 * 3600);
        let notice = lease.try_next().expect("the grant is noticed");
        assert!(text_of(&notice).starts_with(b"Knights"), "{notice:02x?}");
        assert_eq!(world.priv_of(Vid::new(VID), PRIV_EXP_PCT), 50);
        // A grant to no guild stores but stays silent.
        world.give_guild_priv(9, PRIV_EXP_PCT, 10, 25 * 3600);
        assert!(lease.try_next().is_none(), "no guild, no notice");
        assert_eq!(world.priv_of(Vid::new(VID), PRIV_EXP_PCT), 50);
        world.last_pulse = 25 * 3600;
        world.expire_empire_privs();
        assert_eq!(world.priv_of(Vid::new(VID), PRIV_EXP_PCT), 0);
        drop(lease);
    }

    /// The login burst carries one `[LS;871-874]` line per live bonus, in type
    /// order; a zero grant writes none.
    #[test]
    fn the_login_notices_name_the_live_bonuses_in_type_order() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, VID, (3200, 3200), 0);
        world.give_empire_priv(1, PRIV_GOLD_DROP, 30, 25 * 3600);
        world.give_empire_priv(1, PRIV_EXP_PCT, 50, 25 * 3600);
        world.give_empire_priv(1, PRIV_ITEM_DROP, 0, 25 * 3600);
        let lines = world.priv_bonus_lines(Vid::new(VID));
        assert_eq!(lines.len(), 2);
        assert_eq!(text_of(&lines[0]), b"[LS;872;30]");
        assert_eq!(text_of(&lines[1]), b"[LS;874;50]");
    }
}
