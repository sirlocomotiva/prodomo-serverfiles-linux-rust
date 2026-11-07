//! `CHARACTER::ChatPacket` (`G/char.cpp:5140-5187`): a server line to one character.
//!
//! `__MULTI_LANGUAGE_SYSTEM__` is defined, so legacy first looks the format up in the table of
//! the descriptor's language (`LC_LOCALE_TEXT`), unless the type is `CHAT_TYPE_COMMAND`. A text
//! the table lacks is used as it is. Then `vsnprintf` formats it into `CHAT_MAX_LEN + 1` bytes.
//! The record has `id` 0, the descriptor's empire, and `bCanFormat` at its constructor value of
//! `true`.
//!
//! The descriptor's language is the one the client chose at the auth login
//! (`DESC::m_accountTable.bLanguage`, filled by `QUERY_LOGIN_BY_KEY` at
//! `D/ClientManagerLogin.cpp:136-138`).
//!
//! # Two legacy defects this module does not reproduce
//!
//! * **A line longer than `CHAT_MAX_LEN` over-reads.** `vsnprintf` answers the length the whole
//!   line would have had, and legacy sends that many bytes from its 513-byte buffer. Here the
//!   line is cut to the `CHAT_MAX_LEN` bytes `vsnprintf` wrote.
//! * **A conversion with no argument reads past the arguments.** A translated format may hold
//!   more conversions than its caller passes, and `vsnprintf` then reads whatever follows.
//!   Here such a conversion is sent as it is written. No line ported so far has a translation,
//!   so neither case is reachable yet.

use gamedata::locale_string::LocaleStrings;
use protocol::gc_chat::{GcChat, CHAT_MAX_LEN, CHAT_TYPE_COMMAND};

/// The character a line goes to, as its descriptor knows it.
#[derive(Debug, Clone, Copy)]
pub struct Recipient<'a> {
    /// Every language's locale strings.
    pub strings: &'a LocaleStrings,
    /// The descriptor's language (`DESC::GetLanguage`).
    pub language: u8,
    /// The descriptor's empire (`DESC::GetEmpire`), which the record carries.
    pub empire: u8,
}

/// One `vsnprintf` argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arg<'a> {
    /// An integer, for `%d`.
    Int(i64),
    /// A C string, for `%s`. It ends at its first NUL.
    Text(&'a [u8]),
}

/// The encoded `GC_CHAT` line `ChatPacket(chat_type, format, args...)` sends.
///
/// # Panics
///
/// Never: the text is cut to `CHAT_MAX_LEN`, so the record builds, and a record that short
/// always fits its size field.
#[must_use]
pub fn chat_packet(to: Recipient<'_>, chat_type: u8, format: &[u8], args: &[Arg<'_>]) -> Vec<u8> {
    let text = chat_text(to, chat_type, format, args);
    GcChat::notice(chat_type, to.empire, &text)
        .expect("a text cut to the chat length limit builds a line")
        .encode()
        .expect("a line within the chat length limit fits the record's size field")
}

/// The rendered text `ChatPacket(chat_type, format, args...)` speaks: the format
/// looked up in the recipient's language (unless the line is a command), the
/// arguments substituted, and the text cut to `CHAT_MAX_LEN`. The duel's whisper
/// notify carries this text with its NUL (`G/pvp.cpp:212-231`).
#[must_use]
pub fn chat_text(to: Recipient<'_>, chat_type: u8, format: &[u8], args: &[Arg<'_>]) -> Vec<u8> {
    let format = c_string(format);
    let format = if chat_type == CHAT_TYPE_COMMAND {
        format
    } else {
        to.strings.find(format, to.language)
    };
    notice_text(format, args)
}

/// The rendered text `SendLocaleNotice` speaks (`G/cmd_gm.cpp:1252-1313`): the
/// arguments substituted into a format that skips the locale-strings lookup, and
/// the text cut to `CHAT_MAX_LEN`.
#[must_use]
pub fn notice_text(format: &[u8], args: &[Arg<'_>]) -> Vec<u8> {
    let mut text = vsnprintf(c_string(format), args);
    text.truncate(CHAT_MAX_LEN);
    text
}

/// The `%s` widths of one `%-16s ` cell: the name, then spaces to 16, then one more.
const USER_CELL: usize = 17;

/// The `/user` lines for `names` (`do_user` and `user_func`,
/// `G/cmd_gm.cpp:1694-1753`): each online Name in a `%-16s ` cell, four names
/// a line, then `Total N`. A caller sends each through [`chat_packet`] with
/// `CHAT_TYPE_INFO`, so a Name packs no format conversion.
///
/// The cells are cut where legacy's 128-byte buffer cuts them: a cell that
/// would run past byte 127 is cut to the bytes left (`:1719-1723`), and a
/// 24-letter Name fills three cells' worth of room before the next cell
/// starts in the bytes left.
#[must_use]
pub fn user_rows(names: &[String]) -> Vec<Vec<u8>> {
    let mut rows = Vec::new();
    let mut line = Vec::new();
    let mut shown = 0usize;
    for name in names {
        let mut cell = name.as_bytes().to_vec();
        cell.resize(USER_CELL, b' ');
        let room = 128usize.saturating_sub(line.len() + 1);
        line.extend_from_slice(&cell[..cell.len().min(room)]);
        shown += 1;
        if shown % 4 == 0 {
            rows.push(std::mem::take(&mut line));
        }
    }
    if shown % 4 != 0 {
        rows.push(line);
    }
    rows.push(format!("Total {}", names.len()).into_bytes());
    rows
}
/// The bytes of a C string: those before its first NUL.
fn c_string(bytes: &[u8]) -> &[u8] {
    bytes
        .iter()
        .position(|&byte| byte == 0)
        .map_or(bytes, |nul| &bytes[..nul])
}

/// `snprintf` on a format the caller has chosen (`snprintf`, `G/char_state.cpp:1213`):
/// the ported lines' formats, whose conversions the arguments fill.
///
/// A line the legacy builds with `LC_TEXT` is built here and not looked up: the
/// multi-language build's `LC_TEXT` (`G/locale.hpp:31`) looks the text up with
/// `LOCALE_YMIR`, which answers the text itself (`G/locale.cpp:73-74`).
#[must_use]
pub(crate) fn format_line(format: &[u8], args: &[Arg<'_>]) -> Vec<u8> {
    vsnprintf(format, args)
}

/// The part of `vsnprintf` the ported lines use: `%%`, `%d`, `%i`, `%u`, `%ld`, `%lu`, `%lld`,
/// `%02d`, `%3d`, `%x` and `%s`.
///
/// Each conversion takes the next argument. A conversion with no argument left, or with one of
/// the other kind, is written as it is, and so is a `%` before any other byte. A line that needs
/// another conversion adds it with its ledger section.
fn vsnprintf(format: &[u8], args: &[Arg<'_>]) -> Vec<u8> {
    let mut out = Vec::with_capacity(format.len());
    let mut args = args.iter();
    let mut bytes = format.iter().copied();
    while let Some(byte) = bytes.next() {
        if byte != b'%' {
            out.push(byte);
            continue;
        }
        let conversion = bytes.clone().next();
        match conversion {
            Some(b'%') => out.push(b'%'),
            Some(b'0')
                if bytes.clone().nth(1) == Some(b'2') && bytes.clone().nth(2) == Some(b'd') =>
            {
                match args.next() {
                    Some(Arg::Int(value)) => {
                        let text = value.to_string();
                        if *value >= 0 && *value < 10 {
                            out.push(b'0');
                        }
                        out.extend_from_slice(text.as_bytes());
                    }
                    _ => out.extend_from_slice(b"%02d"),
                }
                bytes.next();
                bytes.next();
            }
            Some(b'3') if bytes.clone().nth(1) == Some(b'd') => {
                match args.next() {
                    Some(Arg::Int(value)) => {
                        let text = value.to_string();
                        let pad = 3usize.saturating_sub(text.len());
                        out.extend(std::iter::repeat_n(b' ', pad));
                        out.extend_from_slice(text.as_bytes());
                    }
                    _ => out.extend_from_slice(b"%3d"),
                }
                bytes.next();
            }
            Some(b'l')
                if bytes.clone().nth(1) == Some(b'l') && bytes.clone().nth(2) == Some(b'd') =>
            {
                match args.next() {
                    Some(Arg::Int(value)) => out.extend_from_slice(value.to_string().as_bytes()),
                    _ => out.extend_from_slice(b"%lld"),
                }
                bytes.next();
                bytes.next();
            }
            Some(b'l') if matches!(bytes.clone().nth(1), Some(b'd' | b'u')) => {
                let literal: &[u8] = if bytes.clone().nth(1) == Some(b'u') {
                    b"%lu"
                } else {
                    b"%ld"
                };
                match args.next() {
                    Some(Arg::Int(value)) => out.extend_from_slice(value.to_string().as_bytes()),
                    _ => out.extend_from_slice(literal),
                }
                bytes.next();
            }
            Some(b'd' | b'u' | b'i') => {
                let literal = match conversion {
                    Some(b'u') => b"%u",
                    Some(b'i') => b"%i",
                    _ => b"%d",
                };
                match args.next() {
                    Some(Arg::Int(value)) => out.extend_from_slice(value.to_string().as_bytes()),
                    _ => out.extend_from_slice(literal),
                }
            }
            Some(b's') => match args.next() {
                Some(Arg::Text(text)) => out.extend_from_slice(c_string(text)),
                _ => out.extend_from_slice(b"%s"),
            },
            _ => {
                out.push(b'%');
                continue;
            }
        }
        bytes.next();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamedata::locale_string::LanguageTable;
    use protocol::gc_chat::CHAT_TYPE_INFO;

    fn strings() -> LocaleStrings {
        LocaleStrings::default().with_table(
            5,
            LanguageTable::parse(b"\"hello\";\"hallo\";\n\"lvl %d\";\"Stufe %d\";\n"),
        )
    }

    /// The text of an encoded `GC_CHAT` line, after checking its fixed fields.
    fn text_of(line: &[u8], chat_type: u8, empire: u8) -> Vec<u8> {
        let size = usize::from(u16::from_le_bytes([line[1], line[2]]));
        assert_eq!(size, line.len());
        assert_eq!(line[0], 4, "HEADER_GC_CHAT");
        assert_eq!(line[3], chat_type);
        assert_eq!(&line[4..8], &[0; 4], "id");
        assert_eq!(line[8], empire);
        assert_eq!(line[9], 1, "bCanFormat");
        line[10..].to_vec()
    }

    #[test]
    fn a_line_is_translated_into_the_descriptors_language_then_formatted() {
        let strings = strings();
        let german = Recipient {
            strings: &strings,
            language: 5,
            empire: 2,
        };
        let line = chat_packet(german, CHAT_TYPE_INFO, b"hello", &[]);
        assert_eq!(text_of(&line, CHAT_TYPE_INFO, 2), b"hallo");
        let line = chat_packet(german, CHAT_TYPE_INFO, b"lvl %d", &[Arg::Int(15)]);
        assert_eq!(text_of(&line, CHAT_TYPE_INFO, 2), b"Stufe 15");
        // A text the table lacks goes out as it is, formatted.
        let line = chat_packet(german, CHAT_TYPE_INFO, b"[LS;444;%s]", &[Arg::Text(b"Axe")]);
        assert_eq!(text_of(&line, CHAT_TYPE_INFO, 2), b"[LS;444;Axe]");
        // Another language, and `LOCALE_YMIR`, read no German.
        for language in [0, 1] {
            let other = Recipient { language, ..german };
            let line = chat_packet(other, CHAT_TYPE_INFO, b"hello", &[]);
            assert_eq!(text_of(&line, CHAT_TYPE_INFO, 2), b"hello");
        }
    }

    #[test]
    fn a_command_line_is_not_translated() {
        let strings = strings();
        let german = Recipient {
            strings: &strings,
            language: 5,
            empire: 1,
        };
        let line = chat_packet(german, CHAT_TYPE_COMMAND, b"hello", &[]);
        assert_eq!(text_of(&line, CHAT_TYPE_COMMAND, 1), b"hello");
    }

    #[test]
    fn a_line_is_cut_to_the_chat_length_limit() {
        let strings = LocaleStrings::default();
        let to = Recipient {
            strings: &strings,
            language: 1,
            empire: 1,
        };
        let long = vec![b'x'; CHAT_MAX_LEN];
        let line = chat_packet(to, CHAT_TYPE_INFO, b"ab%s", &[Arg::Text(&long)]);
        let mut expected = b"ab".to_vec();
        expected.extend_from_slice(&long[..CHAT_MAX_LEN - 2]);
        assert_eq!(text_of(&line, CHAT_TYPE_INFO, 1), expected);
        // A format is a C string.
        let line = chat_packet(to, CHAT_TYPE_INFO, b"ab\0cd", &[]);
        assert_eq!(text_of(&line, CHAT_TYPE_INFO, 1), b"ab");
    }

    #[test]
    fn vsnprintf_writes_each_conversion_with_its_argument() {
        assert_eq!(vsnprintf(b"plain", &[]), b"plain");
        assert_eq!(vsnprintf(b"100%%", &[]), b"100%");
        // `%lld` renders the `long long` (`/set`'s confirmation, ledger 351).
        assert_eq!(
            vsnprintf(b"[%lld]", &[Arg::Int(-9_223_372_036_854_775_808)]),
            b"[-9223372036854775808]"
        );
        assert_eq!(vsnprintf(b"[%lld]", &[]), b"[%lld]");
        // `%lu` and `%ld` render the number (the champion experience line, ledger 371).
        assert_eq!(
            vsnprintf(b"got %lu, %ld", &[Arg::Int(400_000), Arg::Int(-3)]),
            b"got 400000, -3"
        );
        assert_eq!(vsnprintf(b"[%lu]", &[]), b"[%lu]");
        assert_eq!(vsnprintf(b"[%lx]", &[]), b"[%lx]");
        assert_eq!(
            vsnprintf(b"%s has %d%%", &[Arg::Text(b"Alpha"), Arg::Int(-7)]),
            b"Alpha has -7%"
        );
        // `%i` is `%d` (`/ds_change_attr`'s yang line, ledger 359).
        assert_eq!(vsnprintf(b"%i", &[Arg::Int(50000)]), b"50000");
        assert_eq!(vsnprintf(b"%i", &[]), b"%i");
        assert_eq!(vsnprintf(b"[%s]", &[Arg::Text(b"a\0b")]), b"[a]");
        // No argument left, or one of the other kind: the conversion is written as it is.
        assert_eq!(vsnprintf(b"%d and %s", &[]), b"%d and %s");
        assert_eq!(
            vsnprintf(b"%d %s", &[Arg::Text(b"t"), Arg::Int(1)]),
            b"%d %s"
        );
        // A `%` before any other byte, or at the end, is written as it is.
        assert_eq!(vsnprintf(b"%x %", &[Arg::Int(1)]), b"%x %");
        // `%02d` pads one digit with a zero, as the comment countdown's minutes and
        // seconds print (`G/guild.cpp:1061`).
        assert_eq!(vsnprintf(b"%02d", &[]), b"%02d");
        // `%3d` pads left with spaces to three columns, as `/state`'s resistances
        // print (`G/cmd_gm.cpp:1096-1122`); a longer number overflows.
        assert_eq!(vsnprintf(b"[%3d%%]", &[Arg::Int(5)]), b"[  5%]");
        assert_eq!(vsnprintf(b"%3d", &[Arg::Int(-42)]), b"-42");
        assert_eq!(vsnprintf(b"%3d", &[Arg::Int(1234)]), b"1234");
        assert_eq!(vsnprintf(b"%3d", &[]), b"%3d");
    }

    #[test]
    fn user_rows_pack_four_cells_a_line_cut_at_the_buffer() {
        let names: Vec<String> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(ToString::to_string)
            .collect();
        let rows = user_rows(&names);
        assert_eq!(rows.len(), 3, "a full line, a short line, and the total");
        assert_eq!(rows[0].len(), 68, "four cells of 17");
        assert!(rows[1].starts_with(b"e"));
        assert_eq!(rows[2], b"Total 5");
        let none: Vec<String> = Vec::new();
        assert_eq!(user_rows(&none), vec![b"Total 0".to_vec()]);
    }
}
