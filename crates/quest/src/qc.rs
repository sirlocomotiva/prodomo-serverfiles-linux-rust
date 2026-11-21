//! The quest compiler: legacy's `qc` (`server/server/quest/qc.cpp`), which turns one quest
//! source into the files legacy loads from `object/`.
//!
//! A source is `quest NAME [with COND] begin`, then `state NAME begin` blocks holding `when`
//! blocks and `function`s, then `end`. `qc` reads it with the legacy lexer ([`crate::lex`]) and
//! prints the body of every `when` and function back token by token: a name as written, a
//! number as C++ streams a `double` (`%g`, 6 digits), a string between double quotes with its
//! bytes unescaped, `do` as `begin`, a space after every token and a line end wherever the lexer
//! moved to another line between two tokens. That text is what legacy's Lua state loads, so the
//! port reproduces it byte for byte, including where legacy's look-ahead moves a line end one
//! token late. The argument of `set_state`, `newstate` and `setstate` is printed as a string
//! and must name a state of the quest.
//!
//! The files, keyed by their path below `object/`:
//! - `state/QUEST`: `QUEST={["start"]=0,["NAME"]=CRC,...,FUNCTION= function (ARGS)BODY}`, a
//!   state's number being its name's CRC-32 as an `int` (the next free number on a clash);
//! - `begin_condition/QUEST`: `return COND`, for a quest with a `with` condition;
//! - `WHO/EVENT/QUEST.STATE` (`notarget/EVENT/` for an event without a target), the bodies of
//!   the state's `when` blocks without an argument, one after another, each inside
//!   `if COND then ... return end` when it has a condition;
//! - `WHO/EVENT/QUEST.STATE.N.script`, `.when` (`return COND`, or empty) and `.arg` (the
//!   argument without its first `.`) for the N-th `when` block with an argument
//!   (`when 20011.chat."text"`), counted per event and state.
//!
//! `when a or b or c` compiles the body once per name, in the order c, a, b. `when X.target.Y`
//! names the event `target` with the argument `X.Y`. `qc` checks every condition and body with
//! its Lua parser; the port checks them with Lua 5.1 after [`crate::dialect::translate`].
//!
//! Where the printed text would read back as something else, the port refuses the source
//! instead of printing it: a number `%g` rounds, a string holding a `"` or a line end, a string
//! ending in a byte above 0x7f (which would take the closing quote along) and a string holding
//! a zero byte (which C++ stops printing at). Divergences: legacy loops forever when a file ends
//! inside a `with` condition or a body, and the port reports it; a `when` number outside
//! `unsigned int` (undefined in C++) is refused.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use mlua::{ChunkMode, Lua};

use crate::dialect;
use crate::lex::{self, LexError, Lexer, Reserved, Token};

/// Files below `object/`, keyed by their path there.
pub type Objects = BTreeMap<Vec<u8>, Vec<u8>>;

/// One compiled quest source.
#[derive(Debug, Default)]
pub struct Compiled {
    /// The quest's name.
    pub quest: Vec<u8>,
    /// The files the source compiles to.
    pub files: Objects,
    /// `QUEST.NAME` for every function the quest defines.
    pub defined_functions: BTreeSet<Vec<u8>>,
    /// The names the bodies call, as `qc` spots them (a name before `(`, or `a.b` before `(`).
    pub called_functions: BTreeSet<Vec<u8>>,
}

/// A source `qc` refuses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QcError {
    /// The line of the source.
    pub line: usize,
    /// What is refused.
    pub message: String,
}

impl fmt::Display for QcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for QcError {}

impl From<LexError> for QcError {
    fn from(error: LexError) -> Self {
        QcError {
            line: error.line,
            message: error.to_string(),
        }
    }
}

/// The compiler, holding the Lua state it checks conditions and bodies with.
pub struct Compiler {
    lua: Lua,
}

impl Default for Compiler {
    fn default() -> Self {
        Compiler::new()
    }
}

impl Compiler {
    /// A compiler with a fresh Lua state.
    pub fn new() -> Self {
        Compiler { lua: Lua::new() }
    }

    /// Compiles one quest source.
    ///
    /// # Errors
    ///
    /// Everything legacy's `qc` refuses, and the sources the module docs name.
    pub fn compile(&self, source: &[u8]) -> Result<Compiled, QcError> {
        Parser::new(self, source).parse()
    }

    /// `check_syntax`: whether Lua 5.1 parses the translated chunk.
    fn check_syntax(&self, chunk: &[u8], line: usize) -> Result<(), QcError> {
        let translated = dialect::translate(chunk).map_err(|error| QcError {
            line,
            message: format!("syntax error : {error}"),
        })?;
        self.lua
            .load(&translated)
            .set_name("quest")
            .set_mode(ChunkMode::Text)
            .into_function()
            .map(drop)
            .map_err(|error| QcError {
                line,
                message: format!("syntax error : {error}"),
            })
    }
}

/// `qc`'s view of the lexer: the current token, one token of look-ahead and the line of the
/// last token taken (`next`, `lookahead` in `qc.cpp`).
struct LexState<'a> {
    lexer: Lexer<'a>,
    t: Token,
    lookahead: Token,
    lastline: usize,
}

impl<'a> LexState<'a> {
    fn new(source: &'a [u8]) -> Self {
        LexState {
            lexer: Lexer::new(source),
            t: Token::Eos,
            lookahead: Token::Eos,
            lastline: 1,
        }
    }

    fn line(&self) -> usize {
        self.lexer.line()
    }

    /// Takes the look-ahead token, or reads one when there is none.
    fn next(&mut self) -> Result<(), LexError> {
        self.lastline = self.lexer.line();
        if self.lookahead == Token::Eos {
            self.t = self.lexer.lex()?;
        } else {
            self.t = std::mem::replace(&mut self.lookahead, Token::Eos);
        }
        Ok(())
    }

    /// Reads the look-ahead token, replacing one already read (`lua_assert` is empty).
    fn lookahead(&mut self) -> Result<(), LexError> {
        self.lookahead = self.lexer.lex()?;
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ParseState {
    Start,
    Quest,
    QuestWithOrBegin,
    StateList,
    StateName,
    StateBegin,
    WhenListOrFunction,
    WhenName,
    WhenWithOrBegin,
    WhenBody,
    FunctionName,
    FunctionArg,
    FunctionBody,
}

/// A `when` block with an argument (`AScript`).
struct ArgScript {
    condition: Vec<u8>,
    argument: Vec<u8>,
    script: Vec<u8>,
}

/// `parse` in `qc.cpp`, its locals as fields.
struct Parser<'a, 'c> {
    compiler: &'c Compiler,
    ls: LexState<'a>,
    state: ParseState,
    nested: i32,
    quest_name: Vec<u8>,
    start_condition: Vec<u8>,
    current_state_name: Vec<u8>,
    current_when_name: Vec<u8>,
    current_when_condition: Vec<u8>,
    current_when_argument: Vec<u8>,
    defined_states: BTreeSet<Vec<u8>>,
    used_states: BTreeMap<usize, Vec<u8>>,
    state_scripts: BTreeMap<Vec<u8>, BTreeMap<Vec<u8>, Vec<u8>>>,
    arg_scripts: BTreeMap<Vec<u8>, BTreeMap<Vec<u8>, Vec<ArgScript>>>,
    when_names: Vec<(Vec<u8>, Vec<u8>)>,
    current_function_name: Vec<u8>,
    current_function_arg: Vec<u8>,
    all_functions: Vec<u8>,
    defined_functions: BTreeSet<Vec<u8>>,
    called_functions: BTreeSet<Vec<u8>>,
}

impl<'a, 'c> Parser<'a, 'c> {
    fn new(compiler: &'c Compiler, source: &'a [u8]) -> Self {
        Parser {
            compiler,
            ls: LexState::new(source),
            state: ParseState::Start,
            nested: 0,
            quest_name: Vec::new(),
            start_condition: Vec::new(),
            current_state_name: Vec::new(),
            current_when_name: Vec::new(),
            current_when_condition: Vec::new(),
            current_when_argument: Vec::new(),
            defined_states: BTreeSet::new(),
            used_states: BTreeMap::new(),
            state_scripts: BTreeMap::new(),
            arg_scripts: BTreeMap::new(),
            when_names: Vec::new(),
            current_function_name: Vec::new(),
            current_function_arg: Vec::new(),
            all_functions: Vec::new(),
            defined_functions: BTreeSet::new(),
            called_functions: BTreeSet::new(),
        }
    }

    fn error(&self, message: impl Into<String>) -> QcError {
        QcError {
            line: self.ls.line(),
            message: message.into(),
        }
    }

    fn assert_nested(&self, nested: i32) -> Result<(), QcError> {
        if self.nested == nested {
            Ok(())
        } else {
            Err(self.error(format!("assertion failure : nested=={nested}")))
        }
    }

    /// Prints `token` as `qc` does, refusing text that would read back as something else.
    fn print(&self, out: &mut Vec<u8>, token: &Token) -> Result<(), QcError> {
        let text = display(token);
        let checked = matches!(token, Token::Number { .. } | Token::String(_));
        if checked && !reads_back_as(&text, token) {
            return Err(self.error(format!(
                "qc would print {} as `{}`, which reads back as something else",
                describe(token),
                String::from_utf8_lossy(&text)
            )));
        }
        out.extend_from_slice(&text);
        Ok(())
    }

    fn parse(mut self) -> Result<Compiled, QcError> {
        loop {
            self.ls.next()?;
            if self.ls.t == Token::Eos {
                break;
            }
            match self.state {
                ParseState::Start => {
                    self.assert_nested(0)?;
                    if !self.ls.t.is(Reserved::Quest) {
                        return Err(self.error("must start with 'quest'"));
                    }
                    self.state = ParseState::Quest;
                }
                ParseState::Quest => {
                    self.assert_nested(0)?;
                    let Some(name) = name_or_string(&self.ls.t) else {
                        return Err(self.error("quest name must be given"));
                    };
                    self.quest_name = name;
                    self.state = ParseState::QuestWithOrBegin;
                }
                ParseState::QuestWithOrBegin => self.quest_with_or_begin()?,
                ParseState::StateList => {
                    self.assert_nested(1)?;
                    if self.ls.t.is(Reserved::State) {
                        self.state = ParseState::StateName;
                    } else if self.ls.t.is(Reserved::End) {
                        self.nested -= 1;
                        self.state = ParseState::Start;
                    } else {
                        return Err(self.error("expecting 'state'"));
                    }
                }
                ParseState::StateName => {
                    self.assert_nested(1)?;
                    let Some(name) = name_or_string(&self.ls.t) else {
                        return Err(self.error("state name must be given"));
                    };
                    self.defined_states.insert(name.clone());
                    self.current_state_name = name;
                    self.state = ParseState::StateBegin;
                }
                ParseState::StateBegin => {
                    self.assert_nested(1)?;
                    if !self.ls.t.is(Reserved::Do) {
                        return Err(self.error("state doesn't have begin-end clause."));
                    }
                    self.nested += 1;
                    self.state = ParseState::WhenListOrFunction;
                }
                ParseState::WhenListOrFunction => self.when_list_or_function()?,
                ParseState::WhenName => self.when_name()?,
                ParseState::WhenWithOrBegin => self.when_with_or_begin()?,
                ParseState::WhenBody => self.when_body()?,
                ParseState::FunctionName => {
                    if let Token::Name(name) = &self.ls.t {
                        self.current_function_name.clone_from(name);
                        let defined = [&self.quest_name[..], b".", name].concat();
                        self.defined_functions.insert(defined);
                        self.state = ParseState::FunctionArg;
                    }
                }
                ParseState::FunctionArg => self.function_arg()?,
                ParseState::FunctionBody => self.function_body()?,
            }
        }
        self.assert_nested(0)?;
        self.finish()
    }

    fn quest_with_or_begin(&mut self) -> Result<(), QcError> {
        self.assert_nested(0)?;
        if self.ls.t.is(Reserved::With) {
            let line = self.ls.line();
            self.start_condition = self.condition()?;
            let check = [b"if ", &self.start_condition[..], b" then end"].concat();
            self.compiler.check_syntax(&check, line)?;
        }
        if !self.ls.t.is(Reserved::Do) {
            let token = String::from_utf8_lossy(&display(&self.ls.t)).into_owned();
            return Err(self.error(format!("quest doesn't have begin-end clause. ({token})")));
        }
        self.state = ParseState::StateList;
        self.nested += 1;
        Ok(())
    }

    /// The tokens after `with`, up to `begin`, joined by spaces.
    fn condition(&mut self) -> Result<Vec<u8>, QcError> {
        self.ls.next()?;
        let mut condition = Vec::new();
        self.print(&mut condition, &self.ls.t)?;
        self.ls.next()?;
        while !self.ls.t.is(Reserved::Do) {
            if self.ls.t == Token::Eos {
                return Err(self.error("the file ends inside a with condition"));
            }
            condition.push(b' ');
            self.print(&mut condition, &self.ls.t)?;
            self.ls.next()?;
        }
        Ok(condition)
    }

    fn when_list_or_function(&mut self) -> Result<(), QcError> {
        self.assert_nested(2)?;
        if self.ls.t.is(Reserved::When) {
            self.state = ParseState::WhenName;
            self.when_names.clear();
        } else if self.ls.t.is(Reserved::End) {
            self.nested -= 1;
            self.state = ParseState::StateList;
        } else if self.ls.t.is(Reserved::Function) {
            self.state = ParseState::FunctionName;
        } else {
            return Err(self.error("expecting 'when' or 'function'"));
        }
        Ok(())
    }

    fn when_name(&mut self) -> Result<(), QcError> {
        self.assert_nested(2)?;
        match &self.ls.t {
            Token::Number { value, .. } => {
                self.current_when_name = when_number(*value)
                    .ok_or_else(|| self.error("a when number outside unsigned int"))?;
                self.ls.lookahead = Token::Char(b'.');
            }
            Token::Name(name) => {
                self.current_when_name.clone_from(name);
                self.ls.lookahead()?;
            }
            Token::String(bytes) => {
                self.current_when_name = c_str(bytes).to_vec();
                self.ls.lookahead()?;
            }
            _ => return Err(self.error("when name must be given")),
        }
        self.state = ParseState::WhenWithOrBegin;
        self.current_when_argument.clear();
        if self.ls.lookahead == Token::Char(b'.') {
            self.ls.next()?;
            self.current_when_name.push(b'.');
            self.ls.next()?;
            let mut event = Vec::new();
            self.print(&mut event, &self.ls.t)?;
            if event == b"target" {
                self.current_when_argument = [b".", &self.current_when_name[..]].concat();
                self.current_when_argument.pop();
                self.current_when_name = event;
            } else {
                self.current_when_name.extend_from_slice(&event);
            }
            self.ls.lookahead()?;
        }
        let mut argument = Vec::new();
        while self.ls.lookahead == Token::Char(b'.') {
            self.ls.next()?;
            argument.push(b'.');
            self.ls.next()?;
            self.print(&mut argument, &self.ls.t)?;
            self.ls.lookahead()?;
        }
        self.current_when_argument.extend_from_slice(&argument);
        if self.ls.lookahead.is(Reserved::Or) {
            self.state = ParseState::WhenName;
            self.when_names.push((
                self.current_when_name.clone(),
                self.current_when_argument.clone(),
            ));
            self.ls.next()?;
        }
        Ok(())
    }

    fn when_with_or_begin(&mut self) -> Result<(), QcError> {
        self.assert_nested(2)?;
        self.current_when_condition.clear();
        if self.ls.t.is(Reserved::With) {
            let line = self.ls.line();
            self.current_when_condition = self.condition()?;
            let check = [b"if ", &self.current_when_condition[..], b" then end"].concat();
            self.compiler.check_syntax(&check, line)?;
        }
        if !self.ls.t.is(Reserved::Do) {
            let token = String::from_utf8_lossy(&display(&self.ls.t)).into_owned();
            return Err(self.error(format!("when doesn't have begin-end clause. ({token})")));
        }
        self.state = ParseState::WhenBody;
        self.nested += 1;
        Ok(())
    }

    /// Counts a block opening or closing (`for` and `while` open with their `do`).
    fn count_nesting(&mut self) {
        let t = &self.ls.t;
        if t.is(Reserved::Do) || t.is(Reserved::If) || t.is(Reserved::Function) {
            self.nested += 1;
        } else if t.is(Reserved::End) {
            self.nested -= 1;
        }
    }

    /// The first body token as `prev`: `qc` makes a `.` any other token.
    fn body_start(&self) -> Token {
        if self.ls.t == Token::Char(b'.') {
            Token::Reserved(Reserved::Do)
        } else {
            self.ls.t.clone()
        }
    }

    /// Spots the functions a body calls, as `qc` does.
    fn track_calls(
        &mut self,
        prev: &Token,
        callname: &mut Vec<u8>,
        registered: &mut bool,
    ) -> Result<(), QcError> {
        if !callname.is_empty() {
            self.ls.lookahead()?;
            if self.ls.lookahead == Token::Char(b'(') {
                self.called_functions.insert(callname.clone());
                *registered = true;
            }
            callname.clear();
        } else if self.ls.t == Token::Char(b'(') {
            if !*registered {
                if let Token::Name(name) = prev {
                    self.called_functions.insert(name.clone());
                }
            }
            *registered = false;
        }
        if self.ls.t == Token::Char(b'.') {
            self.ls.lookahead()?;
            *callname = [display(prev), b".".to_vec(), display(&self.ls.lookahead)].concat();
        }
        Ok(())
    }

    /// Takes the next body token, and a line end when the lexer moved to another line.
    fn body_next(&mut self, body: &mut Vec<u8>) -> Result<(), QcError> {
        self.ls.next()?;
        if self.ls.t == Token::Eos {
            return Err(self.error("the file ends inside a body"));
        }
        if self.ls.line() != self.ls.lastline {
            body.push(b'\n');
        }
        Ok(())
    }

    fn when_body(&mut self) -> Result<(), QcError> {
        self.assert_nested(3)?;
        let line = self.ls.line();
        let mut body = Vec::new();
        let mut state_check = 0_u8;
        let mut prev = self.body_start();
        let mut callname = Vec::new();
        let mut registered = false;
        loop {
            self.count_nesting();
            self.track_calls(&prev, &mut callname, &mut registered)?;
            if state_check > 0 {
                state_check -= 1;
                if state_check == 0 {
                    if let Some(name) = name_or_string(&self.ls.t) {
                        self.used_states.insert(self.ls.line(), name.clone());
                        self.ls.t = Token::String(name);
                    }
                }
            }
            if let Token::Name(name) = &self.ls.t {
                if [&b"set_state"[..], b"newstate", b"setstate"].contains(&&name[..]) {
                    state_check = 2;
                }
            }
            if self.nested == 2 {
                break;
            }
            self.print(&mut body, &self.ls.t)?;
            body.push(b' ');
            prev = self.ls.t.clone();
            self.body_next(&mut body)?;
        }
        self.compiler.check_syntax(&body, line)?;
        self.store_when(&body);
        self.state = ParseState::WhenListOrFunction;
        Ok(())
    }

    /// Files the body under every name of the `when`: the last name first, then the others in
    /// the order written.
    fn store_when(&mut self, body: &[u8]) {
        self.when_names.reverse();
        loop {
            let state = self.current_state_name.clone();
            let condition = &self.current_when_condition;
            if self.current_when_argument.is_empty() {
                let script = self
                    .state_scripts
                    .entry(self.current_when_name.clone())
                    .or_default()
                    .entry(state)
                    .or_default();
                if condition.is_empty() {
                    script.extend_from_slice(body);
                } else {
                    for part in [b"if ", &condition[..], b" then ", body, b" return end "] {
                        script.extend_from_slice(part);
                    }
                }
            } else {
                self.arg_scripts
                    .entry(self.current_when_name.clone())
                    .or_default()
                    .entry(state)
                    .or_default()
                    .push(ArgScript {
                        condition: condition.clone(),
                        argument: self.current_when_argument.clone(),
                        script: body.to_vec(),
                    });
            }
            let Some((name, argument)) = self.when_names.pop() else {
                break;
            };
            self.current_when_name = name;
            self.current_when_argument = argument;
        }
    }

    fn function_arg(&mut self) -> Result<(), QcError> {
        if self.ls.t != Token::Char(b'(') {
            return Err(self.error("assertion failure : t.token == '('"));
        }
        self.ls.next()?;
        self.current_function_arg = b"(".to_vec();
        if self.ls.t != Token::Char(b')') {
            loop {
                let Token::Name(name) = &self.ls.t else {
                    let token = String::from_utf8_lossy(&display(&self.ls.t)).into_owned();
                    let function = String::from_utf8_lossy(&self.current_function_name);
                    return Err(self.error(format!(
                        "invalud argument name {token} for function {function}"
                    )));
                };
                self.current_function_arg.extend_from_slice(name);
                self.ls.next()?;
                if self.ls.t != Token::Char(b')') {
                    self.current_function_arg.push(b',');
                }
                if self.ls.t != Token::Char(b',') {
                    break;
                }
                self.ls.next()?;
            }
        }
        self.current_function_arg.push(b')');
        self.state = ParseState::FunctionBody;
        self.nested += 1;
        Ok(())
    }

    fn function_body(&mut self) -> Result<(), QcError> {
        self.assert_nested(3)?;
        let mut body = Vec::new();
        let mut prev = self.body_start();
        let mut callname = Vec::new();
        let mut registered = false;
        while self.nested >= 3 {
            self.count_nesting();
            self.track_calls(&prev, &mut callname, &mut registered)?;
            self.print(&mut body, &self.ls.t)?;
            body.push(b' ');
            if self.nested == 2 {
                break;
            }
            prev = self.ls.t.clone();
            self.body_next(&mut body)?;
        }
        self.state = ParseState::WhenListOrFunction;
        self.all_functions.push(b',');
        self.all_functions
            .extend_from_slice(&self.current_function_name);
        self.all_functions.extend_from_slice(b"= function ");
        self.all_functions
            .extend_from_slice(&self.current_function_arg);
        self.all_functions.extend_from_slice(&body);
        Ok(())
    }

    /// Checks the state names used and writes the files.
    fn finish(self) -> Result<Compiled, QcError> {
        for (line, name) in &self.used_states {
            if !self.defined_states.contains(name) {
                return Err(QcError {
                    line: *line,
                    message: format!("state name not found : {}", String::from_utf8_lossy(name)),
                });
            }
        }
        let quest = &self.quest_name[..];
        let mut files = Objects::new();
        if !self.defined_states.is_empty() {
            let numbers = state_numbers(&self.defined_states);
            let mut table = [quest, b"={[\"start\"]=0"].concat();
            for name in self.defined_states.iter().filter(|name| *name != b"start") {
                let number = numbers.get(&name[..]).copied().unwrap_or_default();
                table.extend_from_slice(b",[\"");
                table.extend_from_slice(name);
                table.extend_from_slice(format!("\"]={number}").as_bytes());
            }
            table.extend_from_slice(&self.all_functions);
            table.push(b'}');
            files.insert([b"state/", quest].concat(), table);
        }
        if !self.start_condition.is_empty() {
            let condition = [b"return ", &self.start_condition[..]].concat();
            files.insert([b"begin_condition/", quest].concat(), condition);
        }
        for (when, states) in &self.arg_scripts {
            let directory = event_directory(when);
            for (state, scripts) in states {
                for (index, script) in scripts.iter().enumerate() {
                    let base = [
                        &directory[..],
                        quest,
                        b".",
                        state,
                        b".",
                        index.to_string().as_bytes(),
                    ]
                    .concat();
                    let condition = if script.condition.is_empty() {
                        Vec::new()
                    } else {
                        [b"return ", &script.condition[..]].concat()
                    };
                    files.insert([&base[..], b".script"].concat(), script.script.clone());
                    files.insert([&base[..], b".when"].concat(), condition);
                    files.insert([&base[..], b".arg"].concat(), script.argument[1..].to_vec());
                }
            }
        }
        for (when, states) in &self.state_scripts {
            let directory = event_directory(when);
            for (state, script) in states {
                files.insert(
                    [&directory[..], quest, b".", state].concat(),
                    script.clone(),
                );
            }
        }
        Ok(Compiled {
            quest: self.quest_name,
            files,
            defined_functions: self.defined_functions,
            called_functions: self.called_functions,
        })
    }
}

/// A name, or a string as C reads it (up to its first zero byte).
fn name_or_string(token: &Token) -> Option<Vec<u8>> {
    match token {
        Token::Name(name) => Some(name.clone()),
        Token::String(bytes) => Some(c_str(bytes).to_vec()),
        _ => None,
    }
}

/// The bytes before the first zero byte, as `getstr` gives them to C++.
fn c_str(bytes: &[u8]) -> &[u8] {
    bytes
        .iter()
        .position(|byte| *byte == 0)
        .map_or(bytes, |end| &bytes[..end])
}

/// `(unsigned int)` of a `when` number, `None` where C++ leaves it undefined.
fn when_number(value: f64) -> Option<Vec<u8>> {
    let truncated = value.trunc();
    if !(0.0..4_294_967_296.0).contains(&truncated) {
        return None;
    }
    Some(format!("{truncated:.0}").into_bytes())
}

/// The directory of an event's files: `notarget/EVENT/`, or `WHO/EVENT/` with the event lower
/// case and `WHO` as written.
fn event_directory(when: &[u8]) -> Vec<u8> {
    let lower = when.to_ascii_lowercase();
    match when.iter().position(|byte| *byte == b'.') {
        None => [b"notarget/", &lower[..], b"/"].concat(),
        Some(dot) => [&when[..dot], b"/", &lower[dot + 1..], b"/"].concat(),
    }
}

/// Every state's number: its name's CRC-32 as an `int`, or the next free number on a clash
/// (`qc.cpp:852-871`). The table writes 0 for `start` (`qc.cpp:845`), but its CRC is still
/// taken, so a later name with the same CRC moves on, as it did in legacy's `crc_set`.
fn state_numbers(names: &BTreeSet<Vec<u8>>) -> BTreeMap<&[u8], i32> {
    let mut numbers = BTreeMap::new();
    let mut taken = BTreeSet::new();
    for name in names {
        let mut number = i32::from_le_bytes(crc32(c_str(name)).to_le_bytes());
        while !taken.insert(number) {
            number = number.wrapping_add(1);
        }
        numbers.insert(&name[..], number);
    }
    numbers
}

/// The table `crc32.cpp` holds, the IEEE polynomial reflected.
const CRC_TABLE: [u32; 256] = crc_table();

const fn crc_table() -> [u32; 256] {
    let mut table = [0_u32; 256];
    let mut index = 0;
    while index < 256 {
        let mut crc = index;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ 0xEDB8_8320
            };
            bit += 1;
        }
        table[index as usize] = crc;
        index += 1;
    }
    table
}

/// `get_crc32` (`server/server/quest/crc32.cpp`): only the low byte of `crc ^ byte` picks the
/// table entry, so a byte's sign does not matter.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for &byte in bytes {
        crc = CRC_TABLE[usize::from((crc.to_le_bytes()[0]) ^ byte)] ^ (crc >> 8);
    }
    crc ^ 0xffff_ffff
}

/// A token as `qc` prints it (`operator<<`).
fn display(token: &Token) -> Vec<u8> {
    match token {
        Token::Name(name) => name.clone(),
        Token::Number { value, .. } => format_g(*value).into_bytes(),
        Token::String(bytes) => [b"\"", c_str(bytes), b"\""].concat(),
        Token::Char(byte) => vec![*byte],
        Token::Reserved(word) => word.legacy_text().as_bytes().to_vec(),
        Token::Concat => b"..".to_vec(),
        Token::Dots => b"...".to_vec(),
        Token::Eq => b"==".to_vec(),
        Token::Ge => b">=".to_vec(),
        Token::Le => b"<=".to_vec(),
        Token::Ne => b"~=".to_vec(),
        Token::Eos => b"<eof>".to_vec(),
    }
}

fn describe(token: &Token) -> String {
    match token {
        Token::Number { text, .. } => format!("the number {}", String::from_utf8_lossy(text)),
        Token::String(bytes) => format!("the string {:?}", String::from_utf8_lossy(bytes)),
        other => format!("{other:?}"),
    }
}

/// Whether the legacy lexer reads `text` back as `token` alone.
fn reads_back_as(text: &[u8], token: &Token) -> bool {
    let mut lexer = Lexer::new(text);
    let same = match (lexer.lex(), token) {
        (Ok(Token::Number { value: read, .. }), Token::Number { value, .. }) => {
            read.to_bits() == value.to_bits()
        }
        (Ok(Token::String(read)), Token::String(bytes)) => read == *bytes,
        _ => false,
    };
    same && lexer.lex() == Ok(Token::Eos)
}

/// A `double` as a C++ stream prints it by default: `%g` with 6 significant digits.
pub fn format_g(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value < 0.0 { "-inf" } else { "inf" }.to_owned();
    }
    if value == 0.0 {
        return if value.is_sign_negative() { "-0" } else { "0" }.to_owned();
    }
    let scientific = format!("{value:.5e}");
    let Some((mantissa, exponent)) = scientific.split_once('e') else {
        return scientific;
    };
    let Ok(exponent) = exponent.parse::<i32>() else {
        return scientific;
    };
    if !(-4..6).contains(&exponent) {
        let sign = if exponent < 0 { '-' } else { '+' };
        return format!(
            "{}e{sign}{:02}",
            without_trailing_zeros(mantissa),
            exponent.unsigned_abs()
        );
    }
    let decimals = usize::try_from(5 - exponent).unwrap_or_default();
    without_trailing_zeros(&format!("{value:.decimals$}")).to_owned()
}

fn without_trailing_zeros(text: &str) -> &str {
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        text
    }
}

/// Whether a numeral reads as the value `qc` would print for it; exposed for the survey tests.
pub fn prints_exactly(text: &[u8]) -> bool {
    lex::parse_numeral(text).is_some_and(|value| {
        reads_back_as(
            format_g(value).as_bytes(),
            &Token::Number {
                value,
                text: text.to_vec(),
            },
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(source: &str) -> Result<Compiled, QcError> {
        Compiler::new().compile(source.as_bytes())
    }

    fn file(compiled: &Compiled, path: &str) -> String {
        let bytes = compiled
            .files
            .get(path.as_bytes())
            .unwrap_or_else(|| panic!("no {path} in {:?}", paths(compiled)));
        String::from_utf8(bytes.clone()).unwrap()
    }

    fn paths(compiled: &Compiled) -> Vec<String> {
        compiled
            .files
            .keys()
            .map(|path| String::from_utf8_lossy(path).into_owned())
            .collect()
    }

    #[test]
    fn crc32_matches_legacy() {
        let as_int = |name: &[u8]| i32::from_le_bytes(crc32(name).to_le_bytes());
        assert_eq!(as_int(b"start"), -1_619_438_193);
        assert_eq!(as_int(b"run"), 1_349_952_704);
        assert_eq!(as_int(b"done"), 271_442_091);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn format_g_matches_printf() {
        let cases = [
            (20011.0, "20011"),
            (100_000.0, "100000"),
            (1_000_000.0, "1e+06"),
            (1_234_567.0, "1.23457e+06"),
            (0.5, "0.5"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (1.5e-10, "1.5e-10"),
            (123.456_789, "123.457"),
            (1e300, "1e+300"),
            (0.0, "0"),
        ];
        for (value, text) in cases {
            assert_eq!(format_g(value), text, "{value}");
        }
    }

    #[test]
    fn a_number_printf_rounds_is_refused() {
        assert!(prints_exactly(b"20011"));
        assert!(prints_exactly(b"1000000"));
        assert!(!prints_exactly(b"1234567"));
        let source = "quest q begin state start begin when login begin x = 1234567 end end end";
        let error = compile(source).unwrap_err();
        assert!(error.message.contains("the number 1234567"), "{error}");
    }

    #[test]
    fn a_string_that_would_read_back_otherwise_is_refused() {
        for body in ["x = 'a\"b'", "x = 'a\\nb'", "x = 'a\\0b'", "x = '\\195'"] {
            let source =
                format!("quest q begin state start begin when login begin {body} end end end");
            let error = compile(&source).unwrap_err();
            assert!(error.message.contains("the string"), "{body}: {error}");
        }
        let source = "quest q begin state start begin when login begin x = 'a\\tb' end end end";
        assert!(compile(source).is_ok(), "a tab reads back as itself");
    }

    #[test]
    fn a_when_body_prints_token_by_token() {
        let source = "quest q begin\n state start begin\n  when login begin\n   say(\"hi\")\n   \
                      if a != 1 then\n    x = !b\n   end\n  end\n end\nend\n";
        let compiled = compile(source).unwrap();
        assert_eq!(compiled.quest, b"q");
        assert_eq!(file(&compiled, "state/q"), "q={[\"start\"]=0}");
        assert_eq!(
            file(&compiled, "notarget/login/q.start"),
            "say ( \"hi\" ) \nif a ~= 1 then \nx = not b \nend \n"
        );
    }

    #[test]
    fn a_numbered_when_with_an_argument_writes_three_files() {
        let source = "quest q begin state start begin\nwhen 20011.chat.\"Menu\" with a == 1 \
                      begin\nsay(1)\nend end end";
        let compiled = compile(source).unwrap();
        assert_eq!(
            file(&compiled, "20011/chat/q.start.0.script"),
            "say ( 1 ) \n"
        );
        assert_eq!(
            file(&compiled, "20011/chat/q.start.0.when"),
            "return a == 1"
        );
        assert_eq!(file(&compiled, "20011/chat/q.start.0.arg"), "\"Menu\"");
    }

    #[test]
    fn a_condition_wraps_a_plain_when() {
        let source = "quest q begin state start begin when Kill with x begin y() end end end";
        let compiled = compile(source).unwrap();
        assert_eq!(
            file(&compiled, "notarget/kill/q.start"),
            "if x then y ( )  return end "
        );
    }

    #[test]
    fn or_names_compile_last_first() {
        let source = "quest q begin state start begin\nwhen a.click or b.click or c.click \
                      begin\nx()\nend end end";
        let compiled = compile(source).unwrap();
        for who in ["a", "b", "c"] {
            assert_eq!(file(&compiled, &format!("{who}/click/q.start")), "x ( ) \n");
        }
    }

    #[test]
    fn the_event_is_lower_case_and_the_target_keeps_its_case() {
        let source = "quest q begin state start begin when Guard.Click begin x() end end end";
        let compiled = compile(source).unwrap();
        assert_eq!(file(&compiled, "Guard/click/q.start"), "x ( ) ");
    }

    #[test]
    fn target_moves_the_first_name_into_the_argument() {
        let source = "quest q begin state start begin when boss.target.kill begin x() end end end";
        let compiled = compile(source).unwrap();
        assert_eq!(
            file(&compiled, "notarget/target/q.start.0.arg"),
            "boss.kill"
        );
    }

    #[test]
    fn states_are_numbered_by_crc_and_named_states_are_checked() {
        let source = "quest q begin state start begin when login begin set_state(run) end end \
                      state run begin when login begin set_state(\"start\") end end end";
        let compiled = compile(source).unwrap();
        assert_eq!(
            file(&compiled, "state/q"),
            "q={[\"start\"]=0,[\"run\"]=1349952704}"
        );
        assert_eq!(
            file(&compiled, "notarget/login/q.start"),
            "set_state ( \"run\" ) "
        );
        let missing = "quest q begin state start begin when login begin set_state(gone) end \
                       end end";
        let error = compile(missing).unwrap_err();
        assert_eq!(error.message, "state name not found : gone");
    }

    /// `buckeroo` and `plumless` share a CRC-32: the later name takes the next number
    /// (`qc.cpp:861-868`).
    #[test]
    fn a_state_whose_crc_is_taken_takes_the_next_number() {
        let source = "quest q begin state start begin when login begin set_state(plumless) end \
                      end state plumless begin end state buckeroo begin end end";
        let compiled = compile(source).unwrap();
        assert_eq!(
            file(&compiled, "state/q"),
            "q={[\"start\"]=0,[\"buckeroo\"]=1306201125,[\"plumless\"]=1306201126}"
        );
    }

    /// `(unsigned int)` of a `when` number: the whole `unsigned int` range, and nothing past it.
    #[test]
    fn a_when_number_is_an_unsigned_int() {
        assert_eq!(when_number(3_000_000_000.0), Some(b"3000000000".to_vec()));
        assert_eq!(when_number(4_294_967_295.9), Some(b"4294967295".to_vec()));
        assert_eq!(when_number(20_011.5), Some(b"20011".to_vec()));
        assert_eq!(when_number(4_294_967_296.0), None);
        assert_eq!(when_number(-1.0), None);
    }

    #[test]
    fn functions_join_the_state_table() {
        let source = "quest q begin state start begin\nfunction f(a, b)\nreturn a\nend\n\
                      function g() end end end";
        let compiled = compile(source).unwrap();
        assert_eq!(
            file(&compiled, "state/q"),
            "q={[\"start\"]=0,f= function (a,b)return a \nend ,g= function ()end }"
        );
        assert!(compiled.defined_functions.contains(&b"q.f"[..]));
    }

    #[test]
    fn a_quest_condition_writes_begin_condition() {
        let source = "quest q with pc.level > 5 begin state start begin end end";
        let compiled = compile(source).unwrap();
        assert_eq!(
            file(&compiled, "begin_condition/q"),
            "return pc . level > 5"
        );
    }

    #[test]
    fn syntax_errors_are_refused() {
        let source = "quest q begin state start begin when login begin x = = 1 end end end";
        let error = compile(source).unwrap_err();
        assert!(error.message.starts_with("syntax error"), "{error}");
        let condition = "quest q begin state start begin when login with a == begin end end end";
        assert!(compile(condition).is_err());
    }

    #[test]
    fn an_unfinished_source_is_refused() {
        let body = "quest q begin state start begin when login begin x()";
        assert_eq!(
            compile(body).unwrap_err().message,
            "the file ends inside a body"
        );
        let condition = "quest q with a";
        assert_eq!(
            compile(condition).unwrap_err().message,
            "the file ends inside a with condition"
        );
        let open = "quest q begin state start begin";
        assert_eq!(
            compile(open).unwrap_err().message,
            "assertion failure : nested==0"
        );
        assert_eq!(compile("x").unwrap_err().message, "must start with 'quest'");
    }

    #[test]
    fn calls_are_spotted() {
        let source = "quest q begin state start begin when login begin pc.warp(1) say(2) \
                      x = t.y end end end";
        let compiled = compile(source).unwrap();
        let called: Vec<_> = compiled.called_functions.iter().cloned().collect();
        assert_eq!(called, vec![b"pc.warp".to_vec(), b"say".to_vec()]);
    }
}
