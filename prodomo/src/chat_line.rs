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
    let format = c_string(format);
    let format = if chat_type == CHAT_TYPE_COMMAND {
        format
    } else {
        to.strings.find(format, to.language)
    };
    let mut text = vsnprintf(format, args);
    text.truncate(CHAT_MAX_LEN);
    GcChat::notice(chat_type, to.empire, &text)
        .expect("a text cut to the chat length limit builds a line")
        .encode()
        .expect("a line within the chat length limit fits the record's size field")
}

/// The bytes of a C string: those before its first NUL.
fn c_string(bytes: &[u8]) -> &[u8] {
    bytes
        .iter()
        .position(|&byte| byte == 0)
        .map_or(bytes, |nul| &bytes[..nul])
}

/// The part of `vsnprintf` the ported lines use: `%%`, `%d` and `%s`.
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
            Some(b'd') => match args.next() {
                Some(Arg::Int(value)) => out.extend_from_slice(value.to_string().as_bytes()),
                _ => out.extend_from_slice(b"%d"),
            },
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
        assert_eq!(
            vsnprintf(b"%s has %d%%", &[Arg::Text(b"Alpha"), Arg::Int(-7)]),
            b"Alpha has -7%"
        );
        assert_eq!(vsnprintf(b"[%s]", &[Arg::Text(b"a\0b")]), b"[a]");
        // No argument left, or one of the other kind: the conversion is written as it is.
        assert_eq!(vsnprintf(b"%d and %s", &[]), b"%d and %s");
        assert_eq!(
            vsnprintf(b"%d %s", &[Arg::Text(b"t"), Arg::Int(1)]),
            b"%d %s"
        );
        // A `%` before any other byte, or at the end, is written as it is.
        assert_eq!(vsnprintf(b"%x %", &[Arg::Int(1)]), b"%x %");
    }
}
