//! The quest lexer: Lua 5.0's lexer as legacy modified it (`server/server/liblua/llex.c`).
//!
//! Legacy's `qc` reads quest sources with this lexer, and its Lua state loads the compiled
//! chunks and every library with it too, so the quest sources are written in its dialect. It
//! adds the reserved words `quest`, `state`, `with` and `when`, reads both `begin` and `do` as
//! [`Reserved::Do`] (and prints that token back as `begin`), reads `!` as `not` and `!=` as
//! `~=`, and lets a name hold any byte from 0xa0 up. Inside a quoted string, a byte with the high
//! bit set takes the byte after it along unchecked, whatever that byte is (a quote or a line end
//! included, and a line end taken this way is not counted), so the second byte of a two-byte
//! character never closes a string. Long strings `[[ ]]` nest.
//!
//! The port keeps every rule, and records the line where each token starts, which the
//! translator ([`crate::dialect`]) needs to keep lines where they were. One Divergence: when a
//! file ends inside a quoted string right after a high byte (or right after a `\`), legacy
//! saves the end of the stream as the byte 0xff and never returns, and the port reports
//! "unfinished string" instead.

use std::fmt;

/// A reserved word of the legacy lexer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reserved {
    /// `and`.
    And,
    /// `break`.
    Break,
    /// `do`, and `begin`, which legacy reads as `do`.
    Do,
    /// `else`.
    Else,
    /// `elseif`.
    Elseif,
    /// `end`.
    End,
    /// `false`.
    False,
    /// `for`.
    For,
    /// `function`.
    Function,
    /// `if`.
    If,
    /// `in`.
    In,
    /// `local`.
    Local,
    /// `nil`.
    Nil,
    /// `not`, and `!`.
    Not,
    /// `or`.
    Or,
    /// `repeat`.
    Repeat,
    /// `return`.
    Return,
    /// `then`.
    Then,
    /// `true`.
    True,
    /// `until`.
    Until,
    /// `while`.
    While,
    /// `quest`, legacy only.
    Quest,
    /// `state`, legacy only.
    State,
    /// `with`, legacy only.
    With,
    /// `when`, legacy only.
    When,
}

/// Every word the legacy lexer reserves, in `token2string` order (`llex.c:30-39`).
const RESERVED_WORDS: [(&[u8], Reserved); 26] = [
    (b"and", Reserved::And),
    (b"break", Reserved::Break),
    (b"begin", Reserved::Do),
    (b"else", Reserved::Else),
    (b"elseif", Reserved::Elseif),
    (b"end", Reserved::End),
    (b"false", Reserved::False),
    (b"for", Reserved::For),
    (b"function", Reserved::Function),
    (b"if", Reserved::If),
    (b"in", Reserved::In),
    (b"local", Reserved::Local),
    (b"nil", Reserved::Nil),
    (b"not", Reserved::Not),
    (b"or", Reserved::Or),
    (b"repeat", Reserved::Repeat),
    (b"return", Reserved::Return),
    (b"then", Reserved::Then),
    (b"true", Reserved::True),
    (b"until", Reserved::Until),
    (b"while", Reserved::While),
    (b"quest", Reserved::Quest),
    (b"state", Reserved::State),
    (b"with", Reserved::With),
    (b"when", Reserved::When),
    (b"do", Reserved::Do),
];

impl Reserved {
    /// The word legacy prints for the token (`luaX_token2str`): `begin` for [`Reserved::Do`].
    pub fn legacy_text(self) -> &'static str {
        match self {
            Reserved::Do => "begin",
            other => other.lua_text(),
        }
    }

    /// The word Lua 5.1 reads as the token; the legacy-only words keep their spelling.
    pub fn lua_text(self) -> &'static str {
        match self {
            Reserved::And => "and",
            Reserved::Break => "break",
            Reserved::Do => "do",
            Reserved::Else => "else",
            Reserved::Elseif => "elseif",
            Reserved::End => "end",
            Reserved::False => "false",
            Reserved::For => "for",
            Reserved::Function => "function",
            Reserved::If => "if",
            Reserved::In => "in",
            Reserved::Local => "local",
            Reserved::Nil => "nil",
            Reserved::Not => "not",
            Reserved::Or => "or",
            Reserved::Repeat => "repeat",
            Reserved::Return => "return",
            Reserved::Then => "then",
            Reserved::True => "true",
            Reserved::Until => "until",
            Reserved::While => "while",
            Reserved::Quest => "quest",
            Reserved::State => "state",
            Reserved::With => "with",
            Reserved::When => "when",
        }
    }

    fn from_word(word: &[u8]) -> Option<Reserved> {
        RESERVED_WORDS
            .iter()
            .find(|(text, _)| *text == word)
            .map(|(_, reserved)| *reserved)
    }
}

/// A token of the legacy lexer.
#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    /// A single-byte token such as `(`, `+` or `.`.
    Char(u8),
    /// A reserved word.
    Reserved(Reserved),
    /// A name.
    Name(Vec<u8>),
    /// `..`.
    Concat,
    /// `...`.
    Dots,
    /// `==`.
    Eq,
    /// `>=`.
    Ge,
    /// `<=`.
    Le,
    /// `~=`, and `!=`.
    Ne,
    /// A number, with the text it was read from.
    Number {
        /// The value `strtod` gives the text.
        value: f64,
        /// The numeral as written (a leading `.` included).
        text: Vec<u8>,
    },
    /// A string, with its escapes resolved.
    String(Vec<u8>),
    /// The end of the chunk.
    Eos,
}

impl Token {
    /// Whether the token is the reserved word `word`.
    pub fn is(&self, word: Reserved) -> bool {
        *self == Token::Reserved(word)
    }
}

/// A chunk the legacy lexer refuses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LexError {
    /// The line the lexer had reached.
    pub line: usize,
    /// Legacy's message, such as "unfinished string".
    pub message: &'static str,
    /// The text legacy names after "near".
    pub near: Vec<u8>,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {}: {} near `{}'",
            self.line,
            self.message,
            String::from_utf8_lossy(&self.near)
        )
    }
}

impl std::error::Error for LexError {}

/// `isspace` in the C locale.
fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// Whether a name may start with the byte (`llex.c:421`).
fn starts_name(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0xa0
}

/// Whether a name may go on with the byte (`readname`).
fn continues_name(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0xa0
}

/// The legacy lexer over one chunk.
pub struct Lexer<'a> {
    source: &'a [u8],
    position: usize,
    current: Option<u8>,
    line: usize,
    token_line: usize,
    buffer: Vec<u8>,
}

impl<'a> Lexer<'a> {
    /// Starts reading `source` on line 1, skipping a first line that opens with `#` as
    /// `luaX_setinput` does.
    pub fn new(source: &'a [u8]) -> Self {
        let mut lexer = Lexer {
            source,
            position: 0,
            current: None,
            line: 1,
            token_line: 1,
            buffer: Vec::new(),
        };
        lexer.advance();
        if lexer.current == Some(b'#') {
            loop {
                lexer.advance();
                if matches!(lexer.current, Some(b'\n') | None) {
                    break;
                }
            }
        }
        lexer
    }

    /// The line the lexer has reached (`linenumber`).
    pub fn line(&self) -> usize {
        self.line
    }

    /// The line where the last token [`Lexer::lex`] returned starts.
    pub fn token_line(&self) -> usize {
        self.token_line
    }

    fn advance(&mut self) {
        self.current = self.source.get(self.position).copied();
        if self.current.is_some() {
            self.position += 1;
        }
    }

    fn current_is(&self, test: fn(u8) -> bool) -> bool {
        self.current.is_some_and(test)
    }

    /// Saves the current byte and reads the next; at the end of the chunk it saves nothing.
    fn save_and_next(&mut self) {
        if let Some(byte) = self.current {
            self.buffer.push(byte);
        }
        self.advance();
    }

    /// Skips a line end and counts it (`inclinenumber`).
    fn new_line(&mut self) {
        self.advance();
        self.line += 1;
    }

    fn error(&self, message: &'static str, near: &[u8]) -> LexError {
        LexError {
            line: self.line,
            message,
            near: near.to_vec(),
        }
    }

    fn error_near_buffer(&self, message: &'static str) -> LexError {
        self.error(message, &self.buffer)
    }

    /// Reads the next token (`luaX_lex`).
    ///
    /// # Errors
    ///
    /// Everything legacy's lexer refuses: a malformed number, an unfinished string or long
    /// comment, an escape above 255 and a control byte outside a string.
    pub fn lex(&mut self) -> Result<Token, LexError> {
        loop {
            self.token_line = self.line;
            let Some(byte) = self.current else {
                return Ok(Token::Eos);
            };
            match byte {
                b'\n' => self.new_line(),
                b'-' => {
                    self.advance();
                    if self.current != Some(b'-') {
                        return Ok(Token::Char(b'-'));
                    }
                    self.advance();
                    if self.current == Some(b'[') {
                        self.advance();
                        if self.current == Some(b'[') {
                            self.long_string(false)?;
                            continue;
                        }
                    }
                    while !matches!(self.current, Some(b'\n') | None) {
                        self.advance();
                    }
                }
                b'[' => {
                    self.advance();
                    if self.current != Some(b'[') {
                        return Ok(Token::Char(b'['));
                    }
                    return self.long_string(true).map(Token::String);
                }
                b'=' => return Ok(self.maybe_equals(b'=', Token::Eq)),
                b'<' => return Ok(self.maybe_equals(b'<', Token::Le)),
                b'>' => return Ok(self.maybe_equals(b'>', Token::Ge)),
                b'~' => return Ok(self.maybe_equals(b'~', Token::Ne)),
                b'!' => {
                    self.advance();
                    if self.current != Some(b'=') {
                        return Ok(Token::Reserved(Reserved::Not));
                    }
                    self.advance();
                    return Ok(Token::Ne);
                }
                b'"' | b'\'' => return self.string(byte).map(Token::String),
                b'.' => {
                    self.advance();
                    if self.current == Some(b'.') {
                        self.advance();
                        if self.current == Some(b'.') {
                            self.advance();
                            return Ok(Token::Dots);
                        }
                        return Ok(Token::Concat);
                    }
                    if !self.current_is(|next| next.is_ascii_digit()) {
                        return Ok(Token::Char(b'.'));
                    }
                    return self.numeral(true);
                }
                _ if is_space(byte) => self.advance(),
                _ if byte.is_ascii_digit() => return self.numeral(false),
                _ if starts_name(byte) => return Ok(self.name()),
                _ if byte < 0x20 || byte == 0x7f => {
                    let near = format!("char({byte})");
                    return Err(self.error("invalid control char", near.as_bytes()));
                }
                _ => {
                    self.advance();
                    return Ok(Token::Char(byte));
                }
            }
        }
    }

    /// `byte` alone, or `byte=` as `pair`.
    fn maybe_equals(&mut self, byte: u8, pair: Token) -> Token {
        self.advance();
        if self.current != Some(b'=') {
            return Token::Char(byte);
        }
        self.advance();
        pair
    }

    fn name(&mut self) -> Token {
        self.buffer.clear();
        loop {
            self.save_and_next();
            if !self.current_is(continues_name) {
                break;
            }
        }
        match Reserved::from_word(&self.buffer) {
            Some(reserved) => Token::Reserved(reserved),
            None => Token::Name(self.buffer.clone()),
        }
    }

    /// `read_numeral`; `comma` when the `.` before the digits is already read.
    fn numeral(&mut self, comma: bool) -> Result<Token, LexError> {
        self.buffer.clear();
        if comma {
            self.buffer.push(b'.');
        }
        while self.current_is(|byte| byte.is_ascii_digit()) {
            self.save_and_next();
        }
        if self.current == Some(b'.') {
            self.save_and_next();
            if self.current == Some(b'.') {
                self.save_and_next();
                return Err(self
                    .error_near_buffer("ambiguous syntax (decimal point x string concatenation)"));
            }
        }
        while self.current_is(|byte| byte.is_ascii_digit()) {
            self.save_and_next();
        }
        if matches!(self.current, Some(b'e' | b'E')) {
            self.save_and_next();
            if matches!(self.current, Some(b'+' | b'-')) {
                self.save_and_next();
            }
            while self.current_is(|byte| byte.is_ascii_digit()) {
                self.save_and_next();
            }
        }
        match parse_numeral(&self.buffer) {
            Some(value) => Ok(Token::Number {
                value,
                text: self.buffer.clone(),
            }),
            None => Err(self.error_near_buffer("malformed number")),
        }
    }

    /// `read_long_string`, entered on the second `[`; `keep` for a string (a comment's text is
    /// dropped).
    fn long_string(&mut self, keep: bool) -> Result<Vec<u8>, LexError> {
        self.buffer.clear();
        self.buffer.push(b'[');
        self.save_and_next();
        if self.current == Some(b'\n') {
            self.new_line();
        }
        let mut depth = 0_usize;
        loop {
            match self.current {
                None => {
                    let message = if keep {
                        "unfinished long string"
                    } else {
                        "unfinished long comment"
                    };
                    return Err(self.error(message, b"<eof>"));
                }
                Some(b'[') => {
                    self.save_and_next();
                    if self.current == Some(b'[') {
                        depth += 1;
                        self.save_and_next();
                    }
                }
                Some(b']') => {
                    self.save_and_next();
                    if self.current == Some(b']') {
                        if depth == 0 {
                            break;
                        }
                        depth -= 1;
                        self.save_and_next();
                    }
                }
                Some(b'\n') => {
                    self.buffer.push(b'\n');
                    self.new_line();
                    if !keep {
                        self.buffer.clear();
                    }
                }
                Some(_) => self.save_and_next(),
            }
        }
        self.save_and_next();
        if !keep {
            return Ok(Vec::new());
        }
        Ok(self.buffer[2..self.buffer.len() - 2].to_vec())
    }

    /// `read_string`, entered on the opening delimiter.
    fn string(&mut self, delimiter: u8) -> Result<Vec<u8>, LexError> {
        self.buffer.clear();
        self.save_and_next();
        loop {
            let Some(byte) = self.current else {
                return Err(self.error("unfinished string", b"<eof>"));
            };
            if byte == delimiter {
                break;
            }
            if byte & 0x80 != 0 {
                self.save_and_next();
                if self.current.is_none() {
                    return Err(self.error("unfinished string", b"<eof>"));
                }
                self.save_and_next();
                continue;
            }
            match byte {
                b'\n' => return Err(self.error_near_buffer("unfinished string")),
                b'\\' => self.escape()?,
                _ => self.save_and_next(),
            }
        }
        self.save_and_next();
        Ok(self.buffer[1..self.buffer.len() - 1].to_vec())
    }

    /// One escape in a quoted string, entered on the `\`.
    fn escape(&mut self) -> Result<(), LexError> {
        self.advance();
        let Some(byte) = self.current else {
            return Ok(());
        };
        let simple = match byte {
            b'a' => Some(0x07),
            b'b' => Some(0x08),
            b'f' => Some(0x0c),
            b'n' => Some(b'\n'),
            b'r' => Some(b'\r'),
            b't' => Some(b'\t'),
            b'v' => Some(0x0b),
            _ => None,
        };
        if let Some(resolved) = simple {
            self.buffer.push(resolved);
            self.advance();
        } else if byte == b'\n' {
            self.buffer.push(b'\n');
            self.new_line();
        } else if byte.is_ascii_digit() {
            let mut code = 0_u32;
            let mut digits = 0;
            while let Some(digit) = self.current.filter(u8::is_ascii_digit) {
                if digits == 3 {
                    break;
                }
                code = 10 * code + u32::from(digit - b'0');
                digits += 1;
                self.advance();
            }
            let Ok(resolved) = u8::try_from(code) else {
                return Err(self.error_near_buffer("escape sequence too large"));
            };
            self.buffer.push(resolved);
        } else {
            self.save_and_next();
        }
        Ok(())
    }
}

/// `luaO_str2d` over a numeral the lexer read: `strtod` must take the whole text.
///
/// A numeral here is digits with at most one `.` and an exponent, which Rust's parser reads
/// exactly as `strtod` does (both round correctly; both refuse an exponent without digits).
pub fn parse_numeral(text: &[u8]) -> Option<f64> {
    std::str::from_utf8(text).ok()?.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(source: &[u8]) -> Result<Vec<Token>, LexError> {
        let mut lexer = Lexer::new(source);
        let mut out = Vec::new();
        loop {
            let token = lexer.lex()?;
            if token == Token::Eos {
                return Ok(out);
            }
            out.push(token);
        }
    }

    fn name(text: &str) -> Token {
        Token::Name(text.as_bytes().to_vec())
    }

    fn number(value: f64, text: &str) -> Token {
        Token::Number {
            value,
            text: text.as_bytes().to_vec(),
        }
    }

    #[test]
    fn begin_and_do_are_both_do() {
        let lexed = tokens(b"begin do").unwrap();
        assert_eq!(lexed, vec![Token::Reserved(Reserved::Do); 2]);
        assert_eq!(Reserved::Do.legacy_text(), "begin");
        assert_eq!(Reserved::Do.lua_text(), "do");
    }

    #[test]
    fn quest_words_are_reserved() {
        let lexed = tokens(b"quest state with when").unwrap();
        let words = [
            Reserved::Quest,
            Reserved::State,
            Reserved::With,
            Reserved::When,
        ];
        assert_eq!(lexed, words.map(Token::Reserved).to_vec());
    }

    #[test]
    fn bang_is_not_and_bang_equals_is_ne() {
        let lexed = tokens(b"!a != b ~= c").unwrap();
        let expected = vec![
            Token::Reserved(Reserved::Not),
            name("a"),
            Token::Ne,
            name("b"),
            Token::Ne,
            name("c"),
        ];
        assert_eq!(lexed, expected);
    }

    #[test]
    fn a_number_takes_the_dot_before_a_name() {
        let lexed = tokens(b"20011.chat").unwrap();
        assert_eq!(lexed, vec![number(20011.0, "20011."), name("chat")]);
    }

    #[test]
    fn numerals_follow_strtod() {
        let lexed = tokens(b"5 .5 5.e2 1E-2").unwrap();
        let expected = vec![
            number(5.0, "5"),
            number(0.5, ".5"),
            number(500.0, "5.e2"),
            number(0.01, "1E-2"),
        ];
        assert_eq!(lexed, expected);
        assert_eq!(tokens(b"1e").unwrap_err().message, "malformed number");
        assert_eq!(tokens(b"1e+").unwrap_err().message, "malformed number");
        let ambiguous = tokens(b"1..2").unwrap_err();
        assert!(ambiguous.message.starts_with("ambiguous syntax"));
    }

    #[test]
    fn a_hex_numeral_is_a_number_then_a_name() {
        assert_eq!(
            tokens(b"0x10").unwrap(),
            vec![number(0.0, "0"), name("x10")]
        );
    }

    #[test]
    fn escapes_resolve() {
        let lexed = tokens(b"'a\\tb\\065\\0659\\q\\\\\\''").unwrap();
        assert_eq!(lexed, vec![Token::String(b"a\tbAA9q\\'".to_vec())]);
        let too_large = tokens(b"\"\\256\"").unwrap_err();
        assert_eq!(too_large.message, "escape sequence too large");
    }

    #[test]
    fn an_escaped_line_end_is_counted() {
        let mut lexer = Lexer::new(b"\"a\\\nb\" x");
        assert_eq!(lexer.lex().unwrap(), Token::String(b"a\nb".to_vec()));
        assert_eq!(lexer.lex().unwrap(), name("x"));
        assert_eq!(lexer.token_line(), 2);
    }

    #[test]
    fn a_high_byte_takes_the_next_byte_along() {
        assert_eq!(
            tokens(b"\"\xc3\"x\"").unwrap(),
            vec![Token::String(b"\xc3\"x".to_vec())]
        );
        let mut lexer = Lexer::new(b"\"\xc3\nx\" y");
        assert_eq!(lexer.lex().unwrap(), Token::String(b"\xc3\nx".to_vec()));
        assert_eq!(lexer.lex().unwrap(), name("y"));
        assert_eq!(
            lexer.token_line(),
            1,
            "the line end a high byte takes is not counted"
        );
    }

    #[test]
    fn a_string_left_open_is_unfinished() {
        assert_eq!(
            tokens(b"\"abc\n\"").unwrap_err().message,
            "unfinished string"
        );
        assert_eq!(tokens(b"\"abc").unwrap_err().message, "unfinished string");
        assert_eq!(
            tokens(b"\"ab\xc3").unwrap_err().message,
            "unfinished string"
        );
        assert_eq!(tokens(b"\"ab\\").unwrap_err().message, "unfinished string");
    }

    #[test]
    fn long_strings_nest_and_skip_a_first_line_end() {
        let lexed = tokens(b"[[\na[[b]]c\n]]").unwrap();
        assert_eq!(lexed, vec![Token::String(b"a[[b]]c\n".to_vec())]);
        let open = tokens(b"[[a").unwrap_err();
        assert_eq!(open.message, "unfinished long string");
    }

    #[test]
    fn comments_are_skipped_and_counted() {
        let mut lexer = Lexer::new(b"-- one\n--[[ two\nthree ]] a --[x\nb");
        assert_eq!(lexer.lex().unwrap(), name("a"));
        assert_eq!(lexer.token_line(), 3);
        assert_eq!(lexer.lex().unwrap(), name("b"));
        assert_eq!(lexer.token_line(), 4);
        let open = tokens(b"--[[ a").unwrap_err();
        assert_eq!(open.message, "unfinished long comment");
    }

    #[test]
    fn a_first_line_opening_with_a_hash_is_skipped() {
        let mut lexer = Lexer::new(b"#!/bin/lua x\ny");
        assert_eq!(lexer.lex().unwrap(), name("y"));
        assert_eq!(lexer.token_line(), 2);
    }

    #[test]
    fn names_take_bytes_from_0xa0() {
        assert_eq!(
            tokens(b"\xe9t\xe9").unwrap(),
            vec![Token::Name(b"\xe9t\xe9".to_vec())]
        );
        assert_eq!(tokens(b"\x9f").unwrap(), vec![Token::Char(0x9f)]);
    }

    /// `isspace` and `iscntrl` in the C locale (`llex.c:413`, `:432`): `\t`, `\n`, `\v`, `\f` and
    /// `\r` are space, and every other byte below 0x20, and 0x7f, is refused.
    #[test]
    fn control_bytes_are_refused_but_c_space_is_space() {
        assert_eq!(tokens(b"a\r\nb").unwrap(), vec![name("a"), name("b")]);
        assert_eq!(
            tokens(b"a\x0bb\x0cc\td").unwrap(),
            vec![name("a"), name("b"), name("c"), name("d")]
        );
        for byte in [0x01_u8, 0x1f, 0x7f] {
            let control = tokens(&[b'a', byte]).unwrap_err();
            assert_eq!(control.message, "invalid control char");
            assert_eq!(control.near, format!("char({byte})").as_bytes());
        }
    }

    #[test]
    fn operators() {
        let lexed = tokens(b"== <= >= = < > ~ .. ... . [ %").unwrap();
        let expected = vec![
            Token::Eq,
            Token::Le,
            Token::Ge,
            Token::Char(b'='),
            Token::Char(b'<'),
            Token::Char(b'>'),
            Token::Char(b'~'),
            Token::Concat,
            Token::Dots,
            Token::Char(b'.'),
            Token::Char(b'['),
            Token::Char(b'%'),
        ];
        assert_eq!(lexed, expected);
    }
}
