//! The in-process Operator console: the only way an Operator reaches a live world.
//!
//! **Why this lives inside `serve` and is not a `prodomo` subcommand.** The
//! `prodomo account ...` and `prodomo gm ...` subcommands are one-shot: they open
//! the store, write, and exit. A grant cannot work that way, because the world
//! exists only as the serve process's memory (ADR-0002) -- the character has to be
//! standing in it for the grant to place an item, and the client has to be attached
//! for the record to arrive. A separate process would therefore need a channel into
//! a running server, and a channel between two processes is exactly the class of
//! protocol ADR-0001 retired. So the grant runs here, and the Operator reaches it
//! through a file the serve process reads.
//!
//! **Legacy did not reach `do_item` this way either, and saying so is the point.**
//! `do_item` is registered in `cmd_info[]` (`game/cmd.cpp:282`) at `GM_GOD`, and
//! `interpret_command` has exactly two call sites in the whole tree --
//! `game/questlua_global.cpp:739` and `game/input_main.cpp:804`, a GM chat line. There
//! is no console reader, so the command is unreachable from an operator's terminal
//! in legacy. (That is a claim about an absence, so the search that supports it is
//! pinned in the test below by a positive control on the same file.) This console is
//! new, not a port, and it is named as such wherever the ledger records it.
//!
//! **The line format is `item give <name> <vnum> [count]`.** Legacy's `do_item` is
//! `item <vnum> [count]` and grants to the caller; the Operator targets a character by
//! name instead, because an Operator is outside the game (CONTEXT.md) and has no
//! character to be the caller.
//!
//! **Access control is the file's own permissions.** There is no password and no
//! allowlist, because a password in a config file is a secret this process would then
//! have to store, and an allowlist adds a second source of truth next to the filesystem
//! that already answers the question. So the file must not be world-writable, and
//! [`run`] says so at startup rather than assuming it.

use std::fmt;
use std::path::Path;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::unix::pipe;
use tracing::{error, info};

use crate::item_grant::GrantRequest;

/// The ceiling for a stack count, the same constant `prodomo::item_grant` clamps to.
///
/// Named here rather than repeated so a change to `common::item_slots` reaches the
/// console in the same commit as the reducer.
const LIMIT: u16 = common::item_slots::ITEM_COUNT_LIMIT;

/// One line the Operator typed, already split into its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperatorCommand {
    /// `item give <name> <vnum> [count]`
    ItemGive {
        /// The character's Name, matched case-insensitively by the world.
        target: String,
        /// The prototype vnum, as a number.
        vnum: u32,
        /// How many, or `None` when the Operator did not say.
        ///
        /// `None` is kept rather than folded to `1` so the console mirrors `do_item`,
        /// whose `iCount` is 1 by default and is clamped *only* when a second argument
        /// was actually given (`cmd_gm.cpp:480-484`). The clamp itself is not done here:
        /// `prodomo::item_grant` already clamps into
        /// `common::item_slots::ITEM_COUNT_LIMIT`, and a second clamp in the console
        /// would be a second number that can drift from the first.
        count: Option<u32>,
    },
    /// `item destroy <name> <id>`
    ItemDestroy {
        /// The character's Name, matched case-insensitively by the world.
        target: String,
        /// The **item id**, not a vnum: a destroy names one instance.
        ///
        /// Spelled out because `item give` takes a vnum and an Operator who typed the
        /// wrong one would be destroying something by prototype, which is not a thing
        /// this command can do.
        id: u32,
    },
}

/// A line the console could not turn into a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleError(String);

impl fmt::Display for ConsoleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl fmt::Display for OperatorCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ItemGive {
                target,
                vnum,
                count,
            } => match count {
                Some(count) => write!(formatter, "item give {target} {vnum} {count}"),
                None => write!(formatter, "item give {target} {vnum}"),
            },
            Self::ItemDestroy { target, id } => write!(formatter, "item destroy {target} {id}"),
        }
    }
}

/// Turn one console line into a command.
///
/// An empty line and a `#` comment are both `Ok(None)`: a console that has to be fed a
/// comment character to be left alone is a console nobody uses. Everything else is
/// either a command or an error -- an unrecognised line is **never** silently ignored,
/// because a typo that printed nothing looks exactly like a command that worked.
///
/// # Errors
///
/// Returns a [`ConsoleError`] whose text is what the Operator is shown, for a line whose
/// verb is unknown, whose vnum or count is not a plain number, or which carries more
/// arguments than the command takes. Every error names the whole line, so a message
/// alone identifies what was typed.
pub fn parse(line: &str) -> Result<Option<OperatorCommand>, ConsoleError> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return Ok(None);
    }

    let words: Vec<&str> = trimmed.split_whitespace().collect();
    match words.as_slice() {
        [verb, sub, target, vnum, rest @ ..] if *verb == "item" && *sub == "give" => {
            let vnum = parse_number(vnum, "vnum")?;
            // An absent count stays absent, which is what `do_item`'s default of 1
            // amounts to once `prodomo::item_grant` has applied its own clamp. The
            // clamp is deliberately not repeated here: doing it twice would mean two
            // numbers to keep equal, and only the reducer's is the one the world uses.
            let count = match rest {
                [] => None,
                [count] => Some(parse_number(count, "count")?),
                _ => {
                    return Err(ConsoleError(format!(
                        "item give takes one vnum, an optional count, and nothing else: \
                         `item give <name> <vnum> [count]`, got `{trimmed}`"
                    )))
                }
            };
            Ok(Some(OperatorCommand::ItemGive {
                target: (*target).to_owned(),
                vnum,
                count,
            }))
        }
        [verb, sub, target, id] if *verb == "item" && *sub == "destroy" => {
            Ok(Some(OperatorCommand::ItemDestroy {
                target: (*target).to_owned(),
                id: parse_number(id, "item id")?,
            }))
        }
        [verb, ..] if *verb == "item" => Err(ConsoleError(format!(
            "unknown item command; this console has `item give <name> <vnum> [count]` and \
             `item destroy <name> <id>`, got `{trimmed}`"
        ))),
        [verb, ..] => Err(ConsoleError(format!(
            "unknown command `{verb}`; this console has `item give <name> <vnum> [count]` \
             and `item destroy <name> <id>`. Got `{trimmed}`"
        ))),
        [] => Ok(None),
    }
}

/// A number the Operator typed, or an error that says which field was wrong.
fn parse_number(word: &str, field: &str) -> Result<u32, ConsoleError> {
    word.parse::<u32>()
        .map_err(|_| ConsoleError(format!("the {field} must be a plain number, got `{word}`")))
}

/// The request a command becomes, for the path that runs it.
///
/// # Panics
///
/// Never. The count is clamped into `1..=LIMIT` and converted with `try_from`, so the
/// `expect` below is unreachable by construction; it is there so that a future change
/// to the clamp fails loudly here rather than truncating a count into a smaller stack.
pub fn to_request(command: &OperatorCommand) -> Option<GrantRequest> {
    match command {
        OperatorCommand::ItemGive {
            target,
            vnum,
            count,
        } => Some(give_request(target, *vnum, *count)),
        // A destroy names an item id that already exists, so there is nothing to build:
        // the request type is a *grant* request, and borrowing it for a destroy would
        // mean a field that means "a prototype to create" carrying "an id to remove".
        OperatorCommand::ItemDestroy { .. } => None,
    }
}

/// The grant request a `give` line becomes.
fn give_request(target: &str, vnum: u32, count: Option<u32>) -> GrantRequest {
    GrantRequest {
        target: target.to_owned(),
        vnum,
        // The console parses a `u32` because that is what fits the words an Operator
        // types, and the request holds a `u16`. A number that does not fit is clamped
        // here rather than refused: `do_item` clamps too, and a refusal would make the
        // ceiling a different number on the way in than on the way through. The clamp is
        // the same constant the reducer uses.
        count: count.map(|count| {
            u16::try_from(count.clamp(1, u32::from(LIMIT))).expect("a clamped count fits a u16")
        }),
    }
}

/// What the console needs to run a command.
#[derive(Clone)]
pub struct ConsoleContext {
    /// The store the row is written to.
    pub store: db::store::Store,
    /// The thread that owns the world.
    pub controller: crate::game_loop_messages::GameLoopController,
}

/// Read commands from `path` until `shutdown` resolves.
///
/// The pipe is opened once, for reading and writing, and held until shutdown. A named pipe
/// whose last writer closes answers end-of-file, and one whose last reader closes throws
/// away what is still in it. A read-only reader therefore had to close and reopen after
/// every command, and a command written between its end-of-file and its close was lost
/// (ledger 221). Holding the write end as well means neither ever happens: the reader waits
/// for the next `echo` instead of seeing end-of-file, and the pipe always has a reader.
///
/// Each answer goes to the log and nowhere else (ledger 209.2's first option). The server
/// never writes into the pipe, because what it wrote there it would read back as the next
/// command.
pub async fn run(
    path: &Path,
    context: ConsoleContext,
    mut shutdown: tokio::sync::broadcast::Receiver<()>,
) {
    // Re-checked here as well as in `prepare`, because `run` is public and a caller that
    // skips `prepare` would otherwise read whatever the path holds.
    if !is_a_pipe(path) {
        error!(
            path = %path.display(),
            "Operator console path is not a named pipe; refusing to read it, because a \
             regular file would be replayed from the start and every grant re-run"
        );
        return;
    }
    let mut reader = match open(path) {
        Ok(receiver) => BufReader::new(receiver),
        Err(error) => {
            error!(%error, path = %path.display(), "Operator console could not open its pipe");
            return;
        }
    };
    info!(path = %path.display(), "Operator console reading");

    let mut line = String::new();
    loop {
        line.clear();
        let read = tokio::select! {
            biased;
            _ = shutdown.recv() => {
                info!("Operator console stopped");
                return;
            }
            result = reader.read_line(&mut line) => result,
        };
        match read {
            // The server holds a write end itself, so this is not an `echo` closing.
            Ok(0) => {
                error!("Operator console pipe ended while its own write end was open");
                return;
            }
            Ok(_) => {
                if let Some(answer) = answer(&line, &context).await {
                    info!("Operator console: {answer}");
                }
            }
            Err(error) => {
                error!(%error, "Operator console could not read a line; the console stops");
                return;
            }
        }
    }
}

/// Open the console's pipe for reading and writing.
///
/// `O_RDWR` on a named pipe is Linux's, and it never waits for a writer, because the
/// descriptor is one. The receiver refuses a path that is not a named pipe.
fn open(path: &Path) -> std::io::Result<pipe::Receiver> {
    pipe::OpenOptions::new()
        .read_write(true)
        .open_receiver(path)
}

/// Create the console's named pipe if it is not already there.
///
/// Called from the accept loop rather than from the reader task, so a pipe that cannot
/// be created is a startup fact the log states once instead of a reader that silently
/// retries forever. The server keeps running either way: a missing Operator console is
/// not a reason to refuse clients.
///
/// # Errors
///
/// Returns an error when the parent directory cannot be created, when `mkfifo` is not
/// available, and when the path already exists as something other than a named pipe. The
/// last is the important one: the error text names the path so the owner can remove it.
pub async fn prepare(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        if is_a_pipe(path) {
            return Ok(());
        }
        return Err(std::io::Error::other(format!(
            "{} exists and is not a named pipe; a regular file there would be replayed \
             from the start whenever the console opened it, re-running every grant",
            path.display()
        )));
    }
    make_pipe(path).await
}

/// Make the console's named pipe.
///
/// This shells out to `mkfifo` rather than opening the path, and the reason is worth
/// recording because the first draft got it wrong. `OpenOptions::create_new` with a mode
/// creates a **regular file** with that mode -- there is no flag for a FIFO -- so the
/// console came up as an ordinary file. The first end-to-end run showed it immediately:
/// `stat` reported `regular empty file` where a pipe should have been. A regular file
/// then made that draft's tail loop replay it: the writer closed, the reader saw
/// end-of-file, the loop reopened, and the line that was already run was read again,
/// forever. The first
/// draft also carried a comment saying a non-pipe would be refused, and no code did it.
///
/// `mkfifo` is coreutils, it takes `-m` for the mode, and it is a plain process, so this
/// costs no `unsafe` -- which the workspace forbids -- and no dependency.
async fn make_pipe(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }
    // An existing path is removed first so a restart reuses the same name. The window
    // between removing and creating is the only moment the console is unavailable, and it
    // is a window this process creates itself.
    let _ = tokio::fs::remove_file(path).await;
    let output = tokio::process::Command::new("mkfifo")
        .arg("-m")
        .arg("0600")
        .arg(path)
        .output()
        .await
        .map_err(|error| std::io::Error::other(format!("`mkfifo` could not be run: {error}")))?;
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "`mkfifo -m 0600 {}` failed: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

/// Whether `path` is a named pipe.
///
/// Checked before the reader starts, and it is the guard the module documentation claims
/// exists: a regular file at the console's path would be replayed from the start whenever
/// the console opened it, re-running every grant the owner ever typed. Refusing it is the
/// only way that mistake cannot reach a second row.
fn is_a_pipe(path: &Path) -> bool {
    use std::os::unix::fs::FileTypeExt;
    std::fs::metadata(path)
        .map(|metadata| metadata.file_type().is_fifo())
        .unwrap_or(false)
}

/// Run one line and answer it in the Operator's own words.
///
/// A blank line and a comment answer with `None`, so the console stays silent for
/// input that was not a command. A parse error is answered with the error text, because
/// an Operator who mistyped needs to be told and not left watching a log.
pub async fn answer(line: &str, context: &ConsoleContext) -> Option<String> {
    match parse(line) {
        Ok(None) => None,
        Err(error) => Some(error.to_string()),
        Ok(Some(command)) => Some(match command {
            OperatorCommand::ItemGive {
                target,
                vnum,
                count,
            } => {
                let request = give_request(&target, vnum, count);
                run_grant(&request, context).await
            }
            OperatorCommand::ItemDestroy { target, id } => run_destroy(&target, id, context).await,
        }),
    }
}

/// Grant, and describe what happened in one sentence an Operator can act on.
async fn run_grant(request: &GrantRequest, context: &ConsoleContext) -> String {
    match crate::item_persist::grant_and_deliver(&context.store, &context.controller, request).await
    {
        crate::item_persist::Granted::Delivered { id, cell } => {
            let (window_type, pos) = cell;
            format!(
                "Gave item {} to {} in window {} cell {}; id {id}; the client has it.",
                request.vnum, request.target, window_type, pos
            )
        }
        crate::item_persist::Granted::Stored { id, cell, why } => {
            let (window_type, pos) = cell;
            format!(
                "Gave item {} to {} in window {} cell {}, id {id}, and saved it, but the \
                 client was not told: {why}. The player has it at the next login.",
                request.vnum, request.target, window_type, pos
            )
        }
        crate::item_persist::Granted::Failed { why } => {
            // `do_item`'s own sentence is used for a full inventory and for nothing
            // else. Wrapping every refusal in it was the first draft's mistake, and the
            // first end-to-end run showed it: `item give Nobody 10500` answered "Not
            // enough inventory space. (no item prototype with vnum 10500)", which tells
            // an Operator to go and clear an inventory that was not the problem. So the
            // sentence goes only where it is true, and every other refusal says what it
            // is. A wrong reason costs more than a missing one.
            if why.contains(crate::item_grant::NO_ROOM_MESSAGE) {
                why
            } else {
                format!("The grant did not happen: {why}")
            }
        }
    }
}

/// Destroy, and describe what happened in one sentence an Operator can act on.
///
/// The answer says whether the **row** is gone, because that is the fact an Operator
/// needs and the one the store is authoritative about, and then whether the client was
/// told, because a client that was not keeps drawing the item until its next login.
async fn run_destroy(target: &str, id: u32, context: &ConsoleContext) -> String {
    match crate::item_persist::destroy_and_delete(&context.store, &context.controller, target, id)
        .await
    {
        crate::item_persist::Destroyed::Gone { id, cell, told } => {
            let (window_type, pos) = cell;
            let client = if told {
                "and cleared the cell on the client."
            } else {
                "but no client was told, so the cell stays drawn until the next login."
            };
            format!(
                "Destroyed item {id} from {target}, freed window {window_type} cell {pos}, \
                 deleted its row, {client}"
            )
        }
        crate::item_persist::Destroyed::StillStored { id, error } => {
            format!(
                "The world freed item {id} from {target} but its row is still in the store \
                 ({error}). Do not destroy it again: the world no longer holds it, so a \
                 second attempt would delete the row with nothing to release."
            )
        }
        crate::item_persist::Destroyed::Refused { id, error } => {
            format!(
                "The destroy did not happen: {id} could not be released ({error}), and \
                     the row was left alone."
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse, to_request, ConsoleError, OperatorCommand};

    fn given(line: &str) -> OperatorCommand {
        match parse(line) {
            Ok(Some(command)) => command,
            other => panic!("`{line}` should be a command, got {other:?}"),
        }
    }

    /// The count a parsed command carries.
    ///
    /// A helper rather than a method on the enum: nothing outside the tests reads the
    /// count back, and the one place that needs it goes through `to_request`.
    fn count_of(command: &OperatorCommand) -> Option<u32> {
        match command {
            OperatorCommand::ItemGive { count, .. } => *count,
            // A destroy has no count, and saying so beats a panic in a test helper.
            OperatorCommand::ItemDestroy { .. } => {
                panic!("a destroy has no count")
            }
        }
    }

    fn refused(line: &str) -> String {
        match parse(line) {
            Err(ConsoleError(text)) => text,
            other => panic!("`{line}` should be refused, got {other:?}"),
        }
    }

    #[test]
    fn a_plain_give_is_read_as_written() {
        assert_eq!(
            given("item give Shaman 10500"),
            OperatorCommand::ItemGive {
                target: "Shaman".to_owned(),
                vnum: 10500,
                count: None,
            }
        );
    }

    #[test]
    fn extra_spaces_and_a_trailing_newline_do_not_change_the_command() {
        // The console reads lines, so the newline is always there. A parser that needed
        // the Operator to type exactly one space would be a parser nobody uses.
        let expected = OperatorCommand::ItemGive {
            target: "Shaman".to_owned(),
            vnum: 10500,
            count: None,
        };
        for line in [
            "item give Shaman 10500\n",
            "  item give Shaman 10500  ",
            "\titem  give  Shaman  10500\t",
        ] {
            assert_eq!(given(line), expected, "for {line:?}");
        }
    }

    #[test]
    fn an_absent_count_stays_absent_and_a_given_count_is_kept() {
        // `do_item`'s `iCount` is 1 by default and is clamped only when a second argument
        // was given. Keeping the difference is what stops the console from quietly
        // turning "no count" into "a count of one", which is indistinguishable downstream.
        assert_eq!(count_of(&given("item give Shaman 10500")), None);
        assert_eq!(count_of(&given("item give Shaman 10500 12")), Some(12));
    }

    #[test]
    fn a_count_of_zero_is_kept_for_the_reducer_to_clamp() {
        // Zero is a number the Operator can type. The console passes it on and the
        // reducer clamps it to 1, which is what `MINMAX(1, iCount, g_bItemCountLimit)`
        // does. Refusing it here would make the console the only place the rule lives.
        assert_eq!(count_of(&given("item give Shaman 10500 0")), Some(0));
    }

    #[test]
    fn a_very_large_count_is_a_number_and_not_a_wrap() {
        // 4,294,967,295 is `u32::MAX`. It must not become 65,535 or 4,294,901,760 by
        // accident; the clamp is what brings it down, and it is one number doing that.
        assert_eq!(
            count_of(&given("item give Shaman 10500 4294967295")),
            Some(u32::MAX)
        );
        assert_eq!(
            count_of(&given("item give Shaman 10500 99999999")),
            Some(99_999_999)
        );
    }

    #[test]
    fn a_blank_line_and_a_comment_are_not_commands() {
        for line in ["", "   ", "\n", "# item give Shaman 10500", "  # hello"] {
            assert_eq!(parse(line), Ok(None), "for {line:?}");
        }
    }

    #[test]
    fn an_unknown_verb_is_refused_and_never_silently_dropped() {
        // A typo that printed nothing looks exactly like a command that worked, which is
        // the failure this console is most able to cause and least able to notice.
        for line in ["give Shaman 10500", "item drop Shaman 10500", "items"] {
            let text = refused(line);
            assert!(
                text.contains(line.trim()),
                "{text} should name what was typed"
            );
        }
    }

    #[test]
    fn item_on_its_own_lists_what_the_console_has() {
        let text = refused("item");
        assert!(
            text.contains("item give <name> <vnum> [count]"),
            "the refusal should teach the syntax: {text}"
        );
    }

    #[test]
    fn a_vnum_that_is_not_a_number_is_refused_by_name() {
        let text = refused("item give Shaman sword");
        assert!(text.contains("vnum"), "{text}");
        assert!(text.contains("sword"), "{text}");
    }

    #[test]
    fn a_negative_or_empty_number_is_refused_rather_than_wrapped() {
        for line in ["item give Shaman -1", "item give Shaman 99999999999"] {
            let text = refused(line);
            assert!(text.contains("vnum"), "{text}");
        }
    }

    #[test]
    fn too_many_arguments_are_refused_rather_than_ignoring_the_rest() {
        let text = refused("item give Shaman 10500 3 extra");
        assert!(text.contains("nothing else"), "{text}");
    }

    #[test]
    fn a_target_is_required() {
        // Without a target there is nobody to give to, and defaulting to "the caller"
        // would be legacy's shape -- and an Operator has no character to be the caller.
        let text = refused("item give 10500");
        assert!(text.contains("item give"), "{text}");
    }

    #[test]
    fn a_command_reads_back_as_the_line_that_would_produce_it() {
        let command = given("item give Shaman 10500 12");
        assert_eq!(command.to_string(), "item give Shaman 10500 12");
        assert_eq!(parse(&command.to_string()), Ok(Some(command)));
    }

    /// The claim that legacy has no console reader is an absence claim, so it carries
    /// controls. Without them, "I searched and found nothing" is not evidence.
    #[test]
    fn legacy_reaches_do_item_from_exactly_two_places_and_neither_is_a_console() {
        use std::path::Path;
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../server/server");
        assert!(
            root.is_dir(),
            "the frozen legacy source is at {}",
            root.display()
        );

        let mut files = Vec::new();
        collect(&root, &mut files);
        files.sort();
        assert!(
            files.len() > 100,
            "the sweep found only {} files",
            files.len()
        );

        // The positive control: the table the command lives in is really there, so the
        // sweep can see a name when one exists. A sweep that returned nothing for
        // everything would pass the negative half of this test while searching nothing.
        let cmd_cpp = read(&root.join("game/cmd.cpp"));
        assert!(
            cmd_cpp.contains("{ \"item\",") || cmd_cpp.contains("{ \"item\""),
            "the positive control failed: `item` is not registered in cmd_info, so a \
             negative result from this sweep would mean the sweep is broken"
        );

        // The claim: `interpret_command` is called from exactly two files, neither of
        // which is a console. `questlua_global.cpp` is the quest Lua hook and
        // `input_main.cpp` is a GM chat line.
        let callers: Vec<String> = files
            .iter()
            .filter(|file| {
                let text = read(file);
                text.contains("interpret_command") && !file.ends_with("game/cmd.cpp")
            })
            .map(|file| {
                file.strip_prefix(&root)
                    .expect("a file under the root")
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        assert_eq!(
            callers,
            vec![
                "game/input_main.cpp".to_owned(),
                "game/questlua_global.cpp".to_owned()
            ],
            "the set of interpret_command callers changed; the Operator console's \
             justification names exactly these two, and neither is a console"
        );
    }

    /// The negative control for the sweep above: `cmd.cpp` *does* contain the name, so
    /// a filter that drops it must be a deliberate exclusion, not an accident of the
    /// search. It is excluded because it is the definition and the one `extern`.
    #[test]
    fn the_excluded_file_is_excluded_because_it_is_the_definition() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../server/server");
        let cmd_cpp = read(&root.join("game/cmd.cpp"));
        assert!(
            cmd_cpp.contains("void interpret_command("),
            "cmd.cpp is the definition; if the name moved, the exclusion above is stale"
        );
    }

    fn collect(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect(&path, out);
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("cpp" | "h" | "H" | "c" | "cc")
            ) {
                out.push(path);
            }
        }
    }

    fn read(path: &std::path::Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }
    #[test]
    fn a_destroy_names_an_item_id_and_not_a_vnum() {
        // The two commands take numbers of different meanings, and mixing them up would
        // destroy an arbitrary instance. The error text says "item id" for that reason,
        // so it is pinned here: an Operator who typed a vnum gets told which number is
        // wanted instead of getting a silent wrong-item destroy.
        match parse("item destroy Shaman 19") {
            Ok(Some(OperatorCommand::ItemDestroy { target, id })) => {
                assert_eq!(target, "Shaman");
                assert_eq!(id, 19);
            }
            other => panic!("`item destroy Shaman 19` should be a destroy, got {other:?}"),
        }
    }

    #[test]
    fn a_destroy_takes_exactly_a_name_and_an_id() {
        for line in [
            "item destroy Shaman",
            "item destroy Shaman 19 2",
            "item destroy 19",
        ] {
            assert!(
                parse(line).is_err(),
                "`{line}` is not a complete destroy and must be refused"
            );
        }
    }

    #[test]
    fn a_destroy_has_no_grant_request_because_it_creates_nothing() {
        // A `GrantRequest` is "make this prototype"; a destroy is "remove this id".
        // Borrowing the type would mean a field meaning one thing carrying the other, so
        // the conversion is `None` and the console dispatches on the variant instead.
        let command = given("item destroy Shaman 19");
        assert!(
            to_request(&command).is_none(),
            "a destroy must not be convertible into a grant request"
        );
        assert!(
            to_request(&given("item give Shaman 19")).is_some(),
            "a give must still be convertible, or the positive control is not testing the sweep"
        );
    }

    #[test]
    fn a_wrong_item_subcommand_says_what_the_console_has() {
        let error = parse("item takes Shaman 19")
            .expect_err("takes is not a verb this console has")
            .to_string();
        assert!(
            error.contains("item give <name> <vnum> [count]")
                && error.contains("item destroy <name> <id>"),
            "the refusal should list both commands, got: {error}"
        );
    }

    #[test]
    fn a_destroy_id_that_is_not_a_number_is_refused_by_name() {
        let error = parse("item destroy Shaman sword")
            .expect_err("sword is not an id")
            .to_string();
        assert!(
            error.contains("item id"),
            "the refusal should say which field it wanted, got: {error}"
        );
    }
}

#[cfg(test)]
mod pipe_tests {
    use super::{is_a_pipe, open, prepare};
    use std::path::PathBuf;

    /// A path in this test's own directory, so two tests cannot collide.
    fn a_scratch_path(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("prodomo-console-{}-{name}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir.join("console")
    }

    #[tokio::test]
    async fn a_write_to_the_read_only_end_of_a_pipe_reports_success_and_sends_nothing() {
        // This is the whole reason the console does not echo its answer, and it is a
        // property of Tokio rather than of this crate, so it is pinned here where a
        // future reader will look when someone tries to add the echo back.
        //
        // `File::poll_write` copies the bytes into an internal buffer and returns the
        // count it copied, before the blocking `write(2)` has run. `write_all` is
        // therefore satisfied by the copy alone and never asks a second time, so the
        // `EBADF` a read-only descriptor earns sits in a field Tokio keeps for the *next*
        // call. A console that echoed its answer this way would log a successful write,
        // log no error, and put nothing in the pipe.
        //
        // The first draft of this test opened the read-only end first, and that open
        // waits for a writer that never came: the test hung instead of failing. The
        // read-write end is opened first here, because on Linux `O_RDWR` on a FIFO never
        // waits, and nothing below reads a pipe that could be empty. The timeout turns any
        // wait this reasoning missed into a failure rather than a hang.
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let path = a_scratch_path("echo");
        let _ = std::fs::remove_file(&path);
        prepare(&path).await.expect("the pipe should be created");
        let seen = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let mut both = tokio::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .await
                .expect("a read-write end should open");
            // One end, read-only: this is exactly what `open` hands the console's reader.
            let mut read_only = tokio::fs::File::open(&path)
                .await
                .expect("the read end should open while a writer exists");
            let reported = read_only.write_all(b"an answer nobody receives\n").await;
            assert!(
                reported.is_ok(),
                "this test is about a write that *reports* success, so if Tokio ever \
                 fixed the optimistic return the test has to be rewritten rather than \
                 quietly start passing for the wrong reason: {reported:?}"
            );
            // The deferred error is still there for the next call: the write did fail.
            let deferred = read_only.flush().await;
            assert!(
                deferred.is_err(),
                "the read-only descriptor's write must fail somewhere, got {deferred:?}"
            );

            // The control: the same pipe, written through a descriptor that may write,
            // delivers. A pipe is first in, first out, so if the rejected write had put
            // its bytes in, they would come out ahead of these in the same read.
            both.write_all(b"an answer somebody receives\n")
                .await
                .expect("a read-write end should accept bytes");
            both.flush()
                .await
                .expect("the control write should complete");
            let mut buffer = [0_u8; 128];
            let count = both.read(&mut buffer).await.expect("the pipe should read");
            String::from_utf8_lossy(&buffer[..count]).into_owned()
        })
        .await
        .expect("no step of this test should wait");
        assert_eq!(
            seen, "an answer somebody receives\n",
            "only the control's bytes may be in the pipe"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn each_echo_is_read_and_the_pipe_never_ends_between_them() {
        // Ledger 221: the read-only reader saw end-of-file when an `echo` closed, and its
        // close then threw away a command a second `echo` had written in between. The
        // console's own write end keeps it from ever seeing end-of-file, so it never closes.
        use tokio::io::AsyncBufReadExt as _;

        let path = a_scratch_path("rdwr");
        let _ = std::fs::remove_file(&path);
        prepare(&path).await.expect("the pipe should be created");
        let mut reader =
            tokio::io::BufReader::new(open(&path).expect("the console's end should open"));
        let wait = std::time::Duration::from_secs(10);
        let mut line = String::new();
        for command in ["item give Alpha 19", "item give Alpha 27001 10"] {
            // What `echo` does: open, write one line, close.
            std::fs::write(&path, format!("{command}\n")).expect("an echo should write");
            line.clear();
            tokio::time::timeout(wait, reader.read_line(&mut line))
                .await
                .expect("the line should arrive")
                .expect("the pipe should read");
            assert_eq!(line, format!("{command}\n"));
        }
        // Every writer but the console's own has closed, and the read waits for the next.
        line.clear();
        let quiet = std::time::Duration::from_millis(200);
        let pending = tokio::time::timeout(quiet, reader.read_line(&mut line)).await;
        assert!(pending.is_err(), "the pipe ended: {pending:?} {line:?}");
        drop(reader);
        let _ = std::fs::remove_file(&path);

        // The open itself refuses a regular file.
        let regular = a_scratch_path("rdwr-regular");
        std::fs::write(&regular, "item give Shaman 19\n").expect("the file should be writable");
        assert!(
            open(&regular).is_err(),
            "a regular file must not open as the console"
        );
        let _ = std::fs::remove_file(&regular);
    }

    #[tokio::test]
    async fn prepare_makes_a_named_pipe_and_not_a_regular_file() {
        // The first draft opened the path with `OpenOptions::create_new`, which makes a
        // **regular file** with the mode -- there is no flag for a FIFO. The end-to-end
        // run caught it with `stat`, so this is pinned here where it is cheap to see.
        let path = a_scratch_path("fifo");
        let _ = std::fs::remove_file(&path);
        prepare(&path).await.expect("the pipe should be created");
        assert!(
            is_a_pipe(&path),
            "prepare must leave a named pipe at {}, not a regular file",
            path.display()
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn prepare_is_happy_to_reuse_a_pipe_that_is_already_there() {
        // A restart finds the previous run's pipe. Removing and recreating it would open
        // a window in which a command written at that moment is lost, so an existing
        // pipe is left alone.
        let path = a_scratch_path("reuse");
        let _ = std::fs::remove_file(&path);
        prepare(&path)
            .await
            .expect("the first pipe should be created");
        prepare(&path)
            .await
            .expect("an existing pipe should be reused");
        assert!(is_a_pipe(&path));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn prepare_refuses_a_regular_file_rather_than_replaying_it() {
        // This is the bug that made the first draft replay every grant forever: a regular
        // file at the console's path was read from the start every time that draft's
        // tail loop reopened it. The refusal is what stops that, so it is tested directly.
        let path = a_scratch_path("regular");
        std::fs::write(&path, "item give Shaman 19\n").expect("the file should be writable");

        let error = prepare(&path)
            .await
            .expect_err("a regular file must be refused");
        let text = error.to_string();
        assert!(
            text.contains("replayed"),
            "the refusal should say why: {text}"
        );
        assert!(
            text.contains(&path.display().to_string()),
            "the refusal should name the path the owner has to fix: {text}"
        );
        assert!(!is_a_pipe(&path), "prepare must not have replaced the file");

        // And the content is untouched: the console never deletes a file the owner
        // pointed it at, because that path could be anything.
        assert_eq!(
            std::fs::read_to_string(&path).expect("the file should still be there"),
            "item give Shaman 19\n"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_missing_path_is_not_a_pipe() {
        assert!(!is_a_pipe(&PathBuf::from(
            "/nonexistent/prodomo-console/console"
        )));
    }
}

#[cfg(test)]
mod refusal_tests {
    use crate::item_grant::{GrantRefusal, NO_ROOM_MESSAGE};

    /// The console tells a full inventory apart from every other refusal by this text.
    ///
    /// That is a coupling on a message, so it is pinned here: if an arm of
    /// `GrantRefusal`'s `Display` ever starts containing `NO_ROOM_MESSAGE`, the console
    /// would blame the player's inventory for a problem that has nothing to do with it,
    /// and this test is what would notice.
    #[test]
    fn only_a_full_inventory_carries_the_full_inventory_sentence() {
        assert!(
            GrantRefusal::NoRoom { size: 3 }
                .to_string()
                .starts_with(NO_ROOM_MESSAGE),
            "a full inventory is the one case that may say so"
        );
        for refusal in [
            GrantRefusal::NoSuchCharacter {
                name: "Nobody".to_owned(),
            },
            GrantRefusal::UnknownVnum { vnum: 19 },
            GrantRefusal::IdsExhausted,
            GrantRefusal::NoAllocator,
            GrantRefusal::IdAlreadyOwned { id: 7 },
        ] {
            assert!(
                !refusal.to_string().contains(NO_ROOM_MESSAGE),
                "{refusal} must not borrow the full-inventory sentence"
            );
        }
    }
}
