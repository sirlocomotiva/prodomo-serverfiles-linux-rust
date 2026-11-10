//! The OX quiz file: `oxquiz.lua` through a Lua 5.1 state of its own.
//!
//! Legacy runs the file on the live quest state at OX start
//! (`CEventManager::SetOXEvent`, `G/event_manager.cpp:1495-1503`) and at
//! `oxevent.open` (`G/questlua_oxevent.cpp:67-79`), and every `add_ox_quiz` call
//! lands in the OX list (`_add_ox_quiz`, `G/questlua_global.cpp:1144-1156`).
//! The Rewrite cannot run it there: the start is judged after the quest step
//! that queues it, when the step's session — the only [`Game`](crate::manager::Game)
//! that collects the calls — is gone, and running it during the step would land
//! quizzes even when the world then refuses the start. So the loader below runs
//! the file on a fresh state that knows only `add_ox_quiz` and answers the
//! calls in order. On the owner's data the file is 567 pure calls and the answers
//! are what the live state would collect; a file that reaches for anything else
//! fails the load, as a failed `lua_dofile` fails the start.
//!
//! The level is `(int)` of the first argument's low byte, the question is the
//! second argument as `lua_tostring` keeps it, and the third argument's truth is
//! the answer, as the bridge reads them. A missing question faults `AddQuiz` in
//! legacy (`strlcpy` of NULL), so the bridge stays quiet without one; the loader
//! refuses the file instead, failing closed.

use std::sync::{Arc, Mutex};

use mlua::{Lua, MultiValue, Value};

use crate::host;

/// One `add_ox_quiz` call: the level, the question and the answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OxQuizCall {
    /// `(int)` of the first argument's low byte.
    pub level: u8,
    /// The question, as `lua_tostring` keeps it.
    pub question: Vec<u8>,
    /// The third argument's truth.
    pub answer: bool,
}

/// Why `oxquiz.lua` did not load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OxQuizError {
    /// The file could not be read.
    Io(String),
    /// Lua refused the chunk, or the chunk failed: `lua_dofile`'s nonzero result.
    Lua(String),
    /// A call without a question, which would fault `AddQuiz`.
    MissingQuestion,
}

impl std::fmt::Display for OxQuizError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "cannot read oxquiz.lua: {error}"),
            Self::Lua(error) => write!(f, "oxquiz.lua failed: {error}"),
            Self::MissingQuestion => write!(f, "oxquiz.lua calls add_ox_quiz without a question"),
        }
    }
}

impl std::error::Error for OxQuizError {}

/// Run the quiz file `path` and answer its `add_ox_quiz` calls in order.
///
/// The state knows `add_ox_quiz` only: any other global is `nil`, so anything
/// but the pure calls fails the load.
///
/// # Errors
///
/// [`OxQuizError::Io`] when the file cannot be read, [`OxQuizError::Lua`] when
/// Lua refuses it, and [`OxQuizError::MissingQuestion`] for a questionless call.
pub fn load(path: &std::path::Path) -> Result<Vec<OxQuizCall>, OxQuizError> {
    let source = std::fs::read(path).map_err(|error| OxQuizError::Io(error.to_string()))?;
    run(&source, &format!("@{}", path.display()))
}

/// [`load`] over bytes already read, for tests.
fn run(source: &[u8], name: &str) -> Result<Vec<OxQuizCall>, OxQuizError> {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let lua = Lua::new();
    {
        let collected = Arc::clone(&calls);
        lua.globals()
            .set(
                "add_ox_quiz",
                lua.create_function(move |lua, args: MultiValue| -> mlua::Result<Value> {
                    let args: Vec<Value> = args.into_iter().collect();
                    let level = args
                        .first()
                        .and_then(|value| host::c_int(lua, value.clone()).ok())
                        .unwrap_or(0)
                        .to_le_bytes()[0];
                    let Some(question) =
                        host::c_string(lua, args.get(1).cloned().unwrap_or(Value::Nil))?
                    else {
                        return Err(mlua::Error::runtime("add_ox_quiz without a question"));
                    };
                    let answer = !matches!(
                        args.get(2).cloned().unwrap_or(Value::Nil),
                        Value::Nil | Value::Boolean(false)
                    );
                    collected
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(OxQuizCall {
                            level,
                            question,
                            answer,
                        });
                    Ok(Value::Number(f64::from(level)))
                })
                .map_err(|error| OxQuizError::Lua(error.to_string()))?,
            )
            .map_err(|error| OxQuizError::Lua(error.to_string()))?;
    }
    lua.load(source).set_name(name).exec().map_err(|error| {
        let text = error.to_string();
        if text.contains("add_ox_quiz without a question") {
            OxQuizError::MissingQuestion
        } else {
            OxQuizError::Lua(text)
        }
    })?;
    let done = calls
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::{load, OxQuizError};

    /// The owner's quiz file path, below the crate's locale fixture root.
    fn owner_quiz() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe/oxquiz.lua")
    }

    /// The owner's file loads whole: 567 pure calls, every one level 1, in order.
    #[test]
    fn the_owners_quiz_file_loads_whole() {
        let calls = load(&owner_quiz()).expect("oxquiz.lua loads");
        assert_eq!(calls.len(), 567, "every line is a call");
        assert!(
            calls.iter().all(|call| call.level == 1),
            "every call is level 1"
        );
        assert_eq!(
            calls[0].question, b"Aura of Sword is a skill for Sura.",
            "the first question"
        );
        assert!(!calls[0].answer, "the first answer is false");
        assert!(calls[5].answer, "the sixth answer is true");
        assert!(
            calls.iter().all(|call| !call.question.is_empty()),
            "no empty question"
        );
    }

    /// Anything but the pure calls fails the load, as a failed `lua_dofile` fails
    /// the start.
    #[test]
    fn anything_but_the_pure_calls_fails_the_load() {
        let dir = std::env::temp_dir().join("prodomo-ox-quiz-probe");
        std::fs::create_dir_all(&dir).expect("the probe dir");
        let file = dir.join("oxquiz.lua");
        std::fs::write(&file, b"add_ox_quiz(1, \"Q?\", true)\nno_such_global()\n")
            .expect("the probe file");
        assert!(
            matches!(load(&file), Err(OxQuizError::Lua(_))),
            "an unknown global fails"
        );
        std::fs::write(&file, b"add_ox_quiz(1)\n").expect("the probe file");
        assert_eq!(
            load(&file),
            Err(OxQuizError::MissingQuestion),
            "a questionless call fails closed"
        );
        std::fs::remove_dir_all(&dir).expect("the probe dir goes");
    }
}
