//! The quest dialect translated to Lua 5.1 at load (ADR-0006).
//!
//! Legacy loads every quest chunk (the compiled `object/` files, the `when` conditions, the
//! arguments and every library) with its own lexer ([`crate::lex`]), so the chunks are written
//! in its dialect: `begin` for `do`, `!` for `not`, `!=` for `~=`, strings whose bytes follow
//! legacy's rules and long strings that nest. [`translate`] reads a chunk with that lexer and
//! writes the same tokens back in Lua 5.1: `do`, `not` and `~=`; every string as a double-quoted
//! Lua 5.1 literal holding the bytes legacy read; every numeral as written; comments as
//! whitespace. A token stays on the line where it started, so a Lua 5.1 error names the line
//! legacy would name and the rule that a call's `(` must stay on the line of its function
//! (`ambiguous syntax`) holds as it did.
//!
//! Lua 5.0 read a generic `for` over a table (`for k, v in t do`) as a walk with `next`, which
//! Lua 5.1 refuses at run time: the translator wraps every generic `for` list in
//! `__compat_iter( ... )`, which the host defines to give `next, t` for a table and to pass
//! anything else through. A list holding a `function` is refused, because its `do` could not be
//! told from the loop's.
//!
//! Refused as legacy's parser refuses them: the legacy-only words `quest`, `state`, `with` and
//! `when`, and `#` and `%`, which are not operators in Lua 5.0 but are in Lua 5.1. A name
//! holding a byte above 0x7f is refused too: legacy reads it, Lua 5.1 does not, and no quest
//! file holds one.

use std::collections::BTreeSet;
use std::fmt;

use crate::lex::{LexError, Lexer, Reserved, Token};

/// The function the host defines for generic `for` lists.
pub const COMPAT_ITER: &str = "__compat_iter";

/// A chunk the translator refuses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialectError {
    /// The line of the chunk.
    pub line: usize,
    /// What is refused.
    pub message: String,
}

impl fmt::Display for DialectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for DialectError {}

impl From<LexError> for DialectError {
    fn from(error: LexError) -> Self {
        DialectError {
            line: error.line,
            message: error.to_string(),
        }
    }
}

/// A token with the line where it starts.
type Lexed = (Token, usize);

/// Translates one chunk of the quest dialect to Lua 5.1.
///
/// # Errors
///
/// A chunk legacy's lexer refuses, or one the module docs name.
pub fn translate(source: &[u8]) -> Result<Vec<u8>, DialectError> {
    let tokens = lex_all(source)?;
    let (opens, closes) = generic_for_lists(&tokens)?;
    let mut output = Output::default();
    for (index, (token, line)) in tokens.iter().enumerate() {
        if closes.contains(&index) {
            output.emit(*line, b")");
        }
        if opens.contains(&index) {
            output.emit(*line, format!("{COMPAT_ITER}(").as_bytes());
        }
        output.emit(*line, &lua_text(token, *line)?);
    }
    Ok(output.text)
}

fn lex_all(source: &[u8]) -> Result<Vec<Lexed>, LexError> {
    let mut lexer = Lexer::new(source);
    let mut tokens = Vec::new();
    loop {
        let token = lexer.lex()?;
        if token == Token::Eos {
            return Ok(tokens);
        }
        tokens.push((token, lexer.token_line()));
    }
}

/// The indexes where a generic `for` list opens and where its `do` closes it.
fn generic_for_lists(tokens: &[Lexed]) -> Result<(BTreeSet<usize>, BTreeSet<usize>), DialectError> {
    let mut opens = BTreeSet::new();
    let mut closes = BTreeSet::new();
    for index in 0..tokens.len() {
        if !tokens[index].0.is(Reserved::For) {
            continue;
        }
        let Some(in_index) = generic_for_in(tokens, index) else {
            continue;
        };
        let start = in_index + 1;
        let mut depth = 0_i32;
        let mut end = None;
        for (position, (token, line)) in tokens.iter().enumerate().skip(start) {
            match token {
                Token::Char(b'(' | b'[' | b'{') => depth += 1,
                Token::Char(b')' | b']' | b'}') => depth -= 1,
                Token::Reserved(Reserved::Function) => {
                    return Err(DialectError {
                        line: *line,
                        message: "a function inside a generic for list".to_owned(),
                    });
                }
                Token::Reserved(Reserved::Do) if depth == 0 => {
                    end = Some(position);
                    break;
                }
                _ => {}
            }
        }
        if let Some(end) = end.filter(|end| *end > start) {
            opens.insert(start);
            closes.insert(end);
        }
    }
    Ok((opens, closes))
}

/// The index of the `in` of the generic `for` at `index`: `for NAME {, NAME} in`.
fn generic_for_in(tokens: &[Lexed], index: usize) -> Option<usize> {
    let is_name = |position: usize| matches!(tokens.get(position), Some((Token::Name(_), _)));
    if !is_name(index + 1) {
        return None;
    }
    let mut position = index + 2;
    while matches!(tokens.get(position), Some((Token::Char(b','), _))) && is_name(position + 1) {
        position += 2;
    }
    tokens
        .get(position)
        .filter(|(token, _)| token.is(Reserved::In))
        .map(|_| position)
}

fn lua_text(token: &Token, line: usize) -> Result<Vec<u8>, DialectError> {
    let refuse = |message: String| Err(DialectError { line, message });
    Ok(match token {
        Token::Char(byte @ (b'#' | b'%')) => {
            return refuse(format!("`{}` is not a Lua 5.0 operator", char::from(*byte)));
        }
        Token::Char(byte) => vec![*byte],
        Token::Reserved(
            word @ (Reserved::Quest | Reserved::State | Reserved::With | Reserved::When),
        ) => {
            return refuse(format!("`{}` outside a quest header", word.lua_text()));
        }
        Token::Reserved(word) => word.lua_text().as_bytes().to_vec(),
        Token::Name(name) if !name.is_ascii() => {
            return refuse(format!(
                "a name with a byte above 0x7f: {}",
                String::from_utf8_lossy(name)
            ));
        }
        Token::Name(name) => name.clone(),
        Token::Concat => b"..".to_vec(),
        Token::Dots => b"...".to_vec(),
        Token::Eq => b"==".to_vec(),
        Token::Ge => b">=".to_vec(),
        Token::Le => b"<=".to_vec(),
        Token::Ne => b"~=".to_vec(),
        Token::Number { text, .. } => text.clone(),
        Token::String(bytes) => string_literal(bytes),
        Token::Eos => Vec::new(),
    })
}

/// A double-quoted Lua 5.1 literal holding `bytes`; a line end stays a line end (`\` before it).
pub fn string_literal(bytes: &[u8]) -> Vec<u8> {
    let mut literal = vec![b'"'];
    for &byte in bytes {
        match byte {
            b'"' => literal.extend_from_slice(b"\\\""),
            b'\\' => literal.extend_from_slice(b"\\\\"),
            b'\n' => literal.extend_from_slice(b"\\\n"),
            b'\r' => literal.extend_from_slice(b"\\r"),
            _ if byte < 0x20 || byte == 0x7f => {
                literal.extend_from_slice(format!("\\{byte:03}").as_bytes());
            }
            _ => literal.push(byte),
        }
    }
    literal.push(b'"');
    literal
}

/// The translated text and the line it has reached.
struct Output {
    text: Vec<u8>,
    line: usize,
}

impl Default for Output {
    fn default() -> Self {
        Output {
            text: Vec::new(),
            line: 1,
        }
    }
}

impl Output {
    /// Writes `text` on `line` when the output has not passed it, after a space otherwise.
    fn emit(&mut self, line: usize, text: &[u8]) {
        if line > self.line {
            self.text
                .resize(self.text.len() + (line - self.line), b'\n');
            self.line = line;
        } else if !self.text.is_empty() {
            self.text.push(b' ');
        }
        self.text.extend_from_slice(text);
        self.line += text.split(|byte| *byte == b'\n').count() - 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn translated(source: &str) -> String {
        String::from_utf8(translate(source.as_bytes()).unwrap()).unwrap()
    }

    fn refused(source: &str) -> String {
        translate(source.as_bytes()).unwrap_err().message
    }

    #[test]
    fn begin_bang_and_bang_equals_become_lua() {
        assert_eq!(
            translated("if !a != b then begin end end"),
            "if not a ~= b then do end end"
        );
    }

    #[test]
    fn lines_are_kept_and_comments_become_space() {
        let source = "a = 1 -- one\n\n--[[ two\nthree ]] b = 2";
        assert_eq!(translated(source), "a = 1\n\n\nb = 2");
    }

    #[test]
    fn strings_become_lua_51_literals() {
        assert_eq!(translated(r#"x = 'a"b\\c'"#), r#"x = "a\"b\\c""#);
        assert_eq!(
            translated("x = [[a\r\nb]] y = 1"),
            "x = \"a\\r\\\nb\" y = 1"
        );
        assert_eq!(translated("x = '\\0009'"), "x = \"\\0009\"");
        assert_eq!(translated("x = '\u{e9}'"), "x = \"\u{e9}\"");
        assert_eq!(translated("x = '\u{7f}\u{1f}'"), "x = \"\\127\\031\"");
    }

    #[test]
    fn a_line_end_inside_a_string_keeps_the_next_line() {
        assert_eq!(translated("x = 'a\\\nb'\ny = 1"), "x = \"a\\\nb\"\ny = 1");
    }

    #[test]
    fn numerals_stay_as_written() {
        assert_eq!(translated("x = 20011. + .5 + 1E3"), "x = 20011. + .5 + 1E3");
    }

    #[test]
    fn generic_for_lists_are_wrapped() {
        assert_eq!(
            translated("for k, v in t begin end"),
            "for k , v in __compat_iter( t ) do end"
        );
        assert_eq!(
            translated("for k in f(a, b), c do end"),
            "for k in __compat_iter( f ( a , b ) , c ) do end"
        );
        assert_eq!(translated("for i = 1, 2 do end"), "for i = 1 , 2 do end");
    }

    /// Only a `do` outside every bracket ends the list; an empty list or one no `do` ends is
    /// left for Lua 5.1 to refuse as written.
    #[test]
    fn a_list_without_its_own_do_is_left_alone() {
        assert_eq!(translated("for k in (x do end"), "for k in ( x do end");
        assert_eq!(translated("for k in do end"), "for k in do end");
    }

    #[test]
    fn a_function_inside_a_generic_for_list_is_refused() {
        let message = refused("for k in (function() return t end)() do end");
        assert_eq!(message, "a function inside a generic for list");
    }

    #[test]
    fn legacy_words_and_lua_51_operators_are_refused() {
        assert_eq!(refused("x = quest"), "`quest` outside a quest header");
        assert_eq!(refused("x = when"), "`when` outside a quest header");
        assert_eq!(refused("x = #t"), "`#` is not a Lua 5.0 operator");
        assert_eq!(refused("x = 5 % 2"), "`%` is not a Lua 5.0 operator");
        assert!(refused("\u{e9}t\u{e9} = 1").starts_with("a name with a byte above 0x7f"));
    }

    #[test]
    fn lex_errors_carry_their_line() {
        let error = translate(b"a = 1\nb = 'x").unwrap_err();
        assert_eq!(error.line, 2);
    }
}
