//! The command interpreter: `interpret_command` (`G/cmd.cpp:610-717`), which runs a chat line
//! that starts with `/`.
//!
//! [`interpret`](crate::command::interpret) is legacy's walk. The anti-flood check
//! (`ENABLE_ANTI_CMD_FLOOD`) counts every line of a character that is not a GM, and the tenth
//! within a second disconnects it. Then the line is copied with each `$` doubled (`double_dollar`),
//! its first word is lowered (`first_cmd`), and the walk takes the first entry of the table
//! (`table::COMMANDS`) the word names: a `do_cmd` entry by its whole name, every other entry by a
//! prefix, so an empty word names `who`, the first. The character's position is checked against the
//! entry's before the walk's end is, so a position refusal answers a word that names nothing too;
//! then a word that names nothing, and an entry above the character's GM level, answer the
//! does-not-exist line.
//!
//! The entries this build ports are [`Command`](crate::command::Command)'s; every other entry
//! is [`Command::NotPorted`](crate::command::Command::NotPorted), which the caller logs and
//! answers with nothing.
//!
//! # Divergences
//!
//! - **A command this build does not port answers nothing.** Legacy runs its handler.

mod table;

/// `POS_DEAD` (`G/char.h:360`).
pub const POS_DEAD: u8 = 0;
/// `POS_SLEEPING`.
pub const POS_SLEEPING: u8 = 1;
/// `POS_RESTING`.
pub const POS_RESTING: u8 = 2;
/// `POS_SITTING`.
pub const POS_SITTING: u8 = 3;
/// `POS_FISHING`.
pub const POS_FISHING: u8 = 4;
/// `POS_FIGHTING`.
pub const POS_FIGHTING: u8 = 5;
/// `POS_MOUNTING`.
pub const POS_MOUNTING: u8 = 6;
/// `POS_STANDING`: the position of a character of this build that is not sitting; it tracks
/// no other yet.
pub const POS_STANDING: u8 = 7;

/// `GM_PLAYER` (`common/length.h:447`).
pub const GM_PLAYER: u8 = 0;
/// `GM_LOW_WIZARD`.
pub const GM_LOW_WIZARD: u8 = 1;
/// `GM_WIZARD`.
pub const GM_WIZARD: u8 = 2;
/// `GM_HIGH_WIZARD`.
pub const GM_HIGH_WIZARD: u8 = 3;
/// `GM_GOD`.
pub const GM_GOD: u8 = 4;
/// `GM_IMPLEMENTOR`.
pub const GM_IMPLEMENTOR: u8 = 5;
/// `GM_DISABLE`: no level runs the entry.
pub const GM_DISABLE: u8 = 6;

/// The line for a word that names no command, or a command above the character's level.
pub const NO_SUCH_COMMAND: &str = "@@(cmd.cpp)tradus:This command does not exist.";

/// `do_inputall`'s line: the command has to be typed in full.
pub const TYPE_IN_FULL: &str = "[LS;916]";

/// The Pulses a flood count lasts: `PASSES_PER_SEC(1)`.
pub const FLOOD_PULSES: u64 = 25;

/// The count of lines within [`FLOOD_PULSES`] that disconnects.
pub const FLOOD_LIMIT: u32 = 10;

/// `first_cmd`'s buffer, less its terminator.
const WORD_MAX: usize = 128;

/// `double_dollar`'s buffer, less its terminator.
const LINE_MAX: usize = 256;

/// `one_argument`'s `char arg[256]`, less its terminator.
const ARGUMENT_MAX: usize = 255;

/// One entry of `cmd_info`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandEntry {
    /// The name.
    pub name: &'static str,
    /// The handler legacy calls, by its function name.
    pub handler: &'static str,
    /// The lowest position that runs it.
    pub position: u8,
    /// The lowest GM level that runs it; 0 for every character.
    pub gm_level: u8,
}

/// A table row.
const fn entry(
    name: &'static str,
    handler: &'static str,
    position: u8,
    gm_level: u8,
) -> CommandEntry {
    CommandEntry {
        name,
        handler,
        position,
        gm_level,
    }
}

impl CommandEntry {
    /// Whether the walk takes only the whole name (`do_cmd`).
    fn exact(&self) -> bool {
        self.handler == "do_cmd"
    }

    /// What this build runs for the entry.
    #[must_use]
    pub fn command(&self) -> Command {
        match self.handler {
            "do_inputall" => Command::TypeInFull,
            "do_click_safebox" => Command::ClickSafebox,
            "do_click_mall" => Command::ClickMall,
            "do_safebox_close" => Command::SafeboxClose,
            "do_safebox_password" => Command::SafeboxPassword,
            "do_safebox_change_password" => Command::SafeboxChangePassword,
            "do_mall_password" => Command::MallPassword,
            "do_mall_close" => Command::MallClose,
            _ => Command::NotPorted(self.name),
        }
    }
}

/// A command this build runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// `do_inputall`: [`TYPE_IN_FULL`].
    TypeInFull,
    /// `do_click_safebox`: the client is told to ask for the safebox password.
    ClickSafebox,
    /// `do_click_mall`: the client is told to ask for the mall password.
    ClickMall,
    /// `do_safebox_close`.
    SafeboxClose,
    /// `do_safebox_password`: open the safebox.
    SafeboxPassword,
    /// `do_safebox_change_password`.
    SafeboxChangePassword,
    /// `do_mall_password`: open the mall.
    MallPassword,
    /// `do_mall_close`.
    MallClose,
    /// An entry this build does not port, by name.
    NotPorted(&'static str),
}

/// The flood count of one character (`m_dwCmdAntiFloodPulse`, `m_dwCmdAntiFloodCount`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommandFlood {
    pulse: u64,
    count: u32,
}

/// Who runs a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caller {
    /// The Pulse now.
    pub pulse: u64,
    /// The character's GM level.
    pub gm_level: u8,
    /// Its position.
    pub position: u8,
}

/// What a line does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Interpreted {
    /// Nothing: an empty line, or a position legacy has no line for.
    Nothing,
    /// The flood limit was reached: `DelayedDisconnect(0)`.
    Disconnect,
    /// A `CHAT_TYPE_INFO` line to the character.
    Notice(&'static str),
    /// Run the entry with the rest of the line.
    Run {
        /// The entry.
        entry: CommandEntry,
        /// The line after the command word, spaces and all.
        rest: Vec<u8>,
    },
}

/// `interpret_command(ch, line, len)` for the text after the `/`.
#[must_use]
pub fn interpret(line: &[u8], flood: &mut CommandFlood, caller: Caller) -> Interpreted {
    if caller.gm_level == GM_PLAYER {
        if caller.pulse > flood.pulse.saturating_add(FLOOD_PULSES) {
            flood.count = 0;
            flood.pulse = caller.pulse;
        }
        flood.count = flood.count.saturating_add(1);
        if flood.count >= FLOOD_LIMIT {
            return Interpreted::Disconnect;
        }
    }
    let line = double_dollar(line);
    if line.is_empty() {
        return Interpreted::Nothing;
    }
    let (word, rest) = first_cmd(&line);
    let found = table::COMMANDS.iter().find(|entry| {
        let name = entry.name.as_bytes();
        if entry.exact() {
            name == word.as_slice()
        } else {
            name.get(..word.len()) == Some(word.as_slice())
        }
    });
    let position = found.map_or(POS_DEAD, |entry| entry.position);
    if caller.position < position {
        return match caller.position {
            POS_MOUNTING => Interpreted::Notice("[LS;917]"),
            POS_DEAD => Interpreted::Notice("[LS;918]"),
            POS_SLEEPING => Interpreted::Notice("[LS;919]"),
            POS_RESTING | POS_SITTING => Interpreted::Notice("[LS;920]"),
            _ => Interpreted::Nothing,
        };
    }
    let Some(entry) = found else {
        return Interpreted::Notice(NO_SUCH_COMMAND);
    };
    if entry.gm_level != GM_PLAYER
        && (entry.gm_level > caller.gm_level || entry.gm_level == GM_DISABLE)
    {
        return Interpreted::Notice(NO_SUCH_COMMAND);
    }
    Interpreted::Run {
        entry: *entry,
        rest: rest.to_vec(),
    }
}

/// `isnhspace`: the C locale's white space.
const fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | 0x09..=0x0d)
}

/// `double_dollar`: the line up to its first NUL, each `$` doubled, cut at [`LINE_MAX`] bytes
/// without splitting a pair.
fn double_dollar(line: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(line.len().min(LINE_MAX));
    for &byte in line.iter().take_while(|&&byte| byte != 0) {
        let width = if byte == b'$' { 2 } else { 1 };
        if out.len() + width > LINE_MAX {
            break;
        }
        out.resize(out.len() + width, byte);
    }
    out
}

/// `first_cmd`: the first word lowered, at most [`WORD_MAX`] bytes, and the line after it.
fn first_cmd(line: &[u8]) -> (Vec<u8>, &[u8]) {
    let start = line
        .iter()
        .position(|&byte| !is_space(byte))
        .unwrap_or(line.len());
    let line = &line[start..];
    let length = line
        .iter()
        .take(WORD_MAX)
        .take_while(|&&byte| !is_space(byte))
        .count();
    let word = line[..length].to_ascii_lowercase();
    (word, &line[length..])
}

/// `one_argument`: the first argument, where a `"` toggles quoting and is dropped, at most
/// `ARGUMENT_MAX` (255) bytes, and the rest after the spaces that follow it.
#[must_use]
pub fn one_argument(line: &[u8]) -> (Vec<u8>, &[u8]) {
    let skip = |line: &[u8]| {
        line.iter()
            .position(|&byte| !is_space(byte))
            .unwrap_or(line.len())
    };
    let mut at = skip(line);
    let mut quoted = false;
    let mut argument = Vec::new();
    while let Some(&byte) = line.get(at) {
        if byte == 0 || argument.len() >= ARGUMENT_MAX {
            break;
        }
        if byte == b'"' {
            quoted = !quoted;
            at += 1;
            continue;
        }
        if !quoted && is_space(byte) {
            break;
        }
        argument.push(byte);
        at += 1;
    }
    let rest = &line[at..];
    (argument, &rest[skip(rest)..])
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYER: Caller = Caller {
        pulse: 1_000,
        gm_level: GM_PLAYER,
        position: POS_STANDING,
    };

    fn run(line: &[u8], caller: Caller) -> Interpreted {
        interpret(line, &mut CommandFlood::default(), caller)
    }

    fn ran(line: &[u8]) -> (&'static str, Vec<u8>) {
        match run(line, PLAYER) {
            Interpreted::Run { entry, rest } => (entry.name, rest),
            other => panic!("{line:?} ran nothing: {other:?}"),
        }
    }

    #[test]
    fn the_table_is_legacys_under_the_owners_defines() {
        assert_eq!(table::COMMANDS.len(), 267);
        assert_eq!(table::COMMANDS[0].name, "who");
        let named = |name: &str| table::COMMANDS.iter().find(|entry| entry.name == name);
        // `ENABLE_NEWSTUFF` is on and `OFFLINE_SHOP` off.
        assert!(named("click_safebox").is_some());
        assert!(named("click_mall").is_some());
        assert!(table::COMMANDS
            .iter()
            .all(|entry| !entry.name.contains("offline")));
        let password = named("safebox_password").expect("an entry");
        assert_eq!(
            (password.position, password.gm_level),
            (POS_DEAD, GM_PLAYER)
        );
        assert_eq!(
            named("safebox").map(|entry| entry.gm_level),
            Some(GM_HIGH_WIZARD)
        );
    }

    #[test]
    fn a_word_names_the_first_entry_it_is_a_prefix_of() {
        assert_eq!(ran(b"safebox_password 123456").0, "safebox_password");
        assert_eq!(ran(b"safebox_password 123456").1, b" 123456");
        assert_eq!(ran(b"click_saf").0, "click_safebox");
        assert_eq!(ran(b"SAFEBOX_CLOSE").0, "safebox_close");
        assert_eq!(ran(b"  \tmall_close").0, "mall_close");
        // An empty word names `who`, which only an implementor runs.
        assert_eq!(run(b" ", PLAYER), Interpreted::Notice(NO_SUCH_COMMAND));
        let implementor = Caller {
            gm_level: GM_IMPLEMENTOR,
            ..PLAYER
        };
        assert!(matches!(
            run(b" zz", implementor),
            Interpreted::Notice(NO_SUCH_COMMAND)
        ));
        assert!(matches!(
            run(b" ", implementor),
            Interpreted::Run { entry, .. } if entry.name == "who"
        ));
    }

    #[test]
    fn a_prefix_stops_at_the_type_it_in_full_entries() {
        for line in [
            &b"safebox_passwor 1"[..],
            b"safebox_p",
            b"mall_pass",
            b"safebox_change",
        ] {
            let Interpreted::Run { entry, .. } = run(line, PLAYER) else {
                panic!("{line:?}");
            };
            assert_eq!(entry.command(), Command::TypeInFull, "{line:?}");
        }
        assert_eq!(
            table::COMMANDS
                .iter()
                .find(|entry| entry.name == "safebox_password")
                .map(CommandEntry::command),
            Some(Command::SafeboxPassword)
        );
    }

    #[test]
    fn a_do_cmd_entry_takes_only_its_whole_name() {
        let quit = ran(b"quit").0;
        assert_eq!(quit, "quit");
        // `qui` is the type-it-in-full entry before `quit`; `quitt` names nothing.
        assert_eq!(ran(b"qui").0, "qui");
        assert_eq!(run(b"quitt", PLAYER), Interpreted::Notice(NO_SUCH_COMMAND));
    }

    #[test]
    fn a_word_that_names_nothing_answers_the_does_not_exist_line() {
        assert_eq!(
            run(b"no_such", PLAYER),
            Interpreted::Notice(NO_SUCH_COMMAND)
        );
        assert_eq!(run(b"", PLAYER), Interpreted::Nothing);
        assert_eq!(run(b"\0who", PLAYER), Interpreted::Nothing);
    }

    #[test]
    fn a_command_above_the_level_does_not_exist() {
        assert_eq!(
            run(b"safebox 3", PLAYER),
            Interpreted::Notice(NO_SUCH_COMMAND)
        );
        let high = Caller {
            gm_level: GM_HIGH_WIZARD,
            ..PLAYER
        };
        assert!(matches!(run(b"safebox 3", high), Interpreted::Run { .. }));
        let wizard = Caller {
            gm_level: GM_WIZARD,
            ..PLAYER
        };
        assert_eq!(
            run(b"safebox 3", wizard),
            Interpreted::Notice(NO_SUCH_COMMAND)
        );
    }

    #[test]
    fn the_position_is_checked_before_the_word_is() {
        let fighting = table::COMMANDS
            .iter()
            .find(|entry| entry.position == POS_FIGHTING)
            .expect("an entry");
        let at = |position| Caller { position, ..PLAYER };
        let line = fighting.name.as_bytes();
        for (position, notice) in [
            (POS_DEAD, Interpreted::Notice("[LS;918]")),
            (POS_SLEEPING, Interpreted::Notice("[LS;919]")),
            (POS_RESTING, Interpreted::Notice("[LS;920]")),
            (POS_SITTING, Interpreted::Notice("[LS;920]")),
            (POS_FISHING, Interpreted::Nothing),
        ] {
            assert_eq!(run(line, at(position)), notice, "{position}");
        }
        assert!(matches!(
            run(line, at(POS_FIGHTING)),
            Interpreted::Run { .. }
        ));
        // A word that names nothing meets the terminator's `POS_DEAD`, which every position
        // passes.
        assert_eq!(
            run(b"no_such", at(POS_DEAD)),
            Interpreted::Notice(NO_SUCH_COMMAND)
        );
    }

    #[test]
    fn the_tenth_line_within_a_second_disconnects_a_player() {
        let mut flood = CommandFlood::default();
        for pulse in 1..10 {
            let caller = Caller { pulse, ..PLAYER };
            assert_ne!(
                interpret(b"mall_close", &mut flood, caller),
                Interpreted::Disconnect
            );
        }
        let caller = Caller {
            pulse: 10,
            ..PLAYER
        };
        assert_eq!(interpret(b"", &mut flood, caller), Interpreted::Disconnect);
        // 26 Pulses on, the count starts over.
        let mut flood = CommandFlood::default();
        for pulse in (0..40).map(|step| step * 3) {
            let caller = Caller { pulse, ..PLAYER };
            assert_ne!(
                interpret(b"", &mut flood, caller),
                Interpreted::Disconnect,
                "{pulse}"
            );
        }
        let gm = Caller {
            gm_level: GM_LOW_WIZARD,
            ..PLAYER
        };
        let mut flood = CommandFlood::default();
        for _ in 0..20 {
            assert_ne!(interpret(b"", &mut flood, gm), Interpreted::Disconnect);
        }
    }

    #[test]
    fn the_count_lasts_through_its_twenty_fifth_pulse() {
        // `thecore_pulse() > pulse + PASSES_PER_SEC(1)` (`G/cmd.cpp:615`): a line 25 Pulses
        // after the count started still counts, and one 26 Pulses after starts a new count.
        let (start, mut flood) = (
            Caller {
                pulse: 100,
                ..PLAYER
            },
            CommandFlood::default(),
        );
        assert_ne!(interpret(b"", &mut flood, start), Interpreted::Disconnect);
        let edge = Caller {
            pulse: 100 + FLOOD_PULSES,
            ..PLAYER
        };
        for _ in 0..8 {
            assert_ne!(interpret(b"", &mut flood, edge), Interpreted::Disconnect);
        }
        assert_eq!(interpret(b"", &mut flood, edge), Interpreted::Disconnect);
        let mut flood = CommandFlood::default();
        assert_ne!(interpret(b"", &mut flood, start), Interpreted::Disconnect);
        let past = Caller {
            pulse: 101 + FLOOD_PULSES,
            ..PLAYER
        };
        for _ in 0..9 {
            assert_ne!(interpret(b"", &mut flood, past), Interpreted::Disconnect);
        }
    }

    #[test]
    fn a_dollar_is_doubled_and_the_line_cut_at_256_bytes() {
        assert_eq!(double_dollar(b"a$b"), b"a$$b");
        assert_eq!(double_dollar(&[b'x'; 300]).len(), LINE_MAX);
        let mut line = vec![b'x'; 255];
        line.push(b'$');
        assert_eq!(double_dollar(&line).len(), 255);
        assert_eq!(double_dollar(b"ab\0cd"), b"ab");
    }

    #[test]
    fn the_word_is_at_most_128_bytes() {
        let long = [b'a'; 200];
        let (word, rest) = first_cmd(&long);
        assert_eq!((word.len(), rest.len()), (WORD_MAX, 72));
    }

    #[test]
    fn an_argument_reads_quotes_and_skips_spaces() {
        assert_eq!(one_argument(b"  abc def"), (b"abc".to_vec(), &b"def"[..]));
        assert_eq!(one_argument(b" \"a b\"c d"), (b"a bc".to_vec(), &b"d"[..]));
        assert_eq!(one_argument(b""), (Vec::new(), &b""[..]));
        let (argument, rest) = one_argument(&[b'z'; 300]);
        assert_eq!((argument.len(), rest.len()), (ARGUMENT_MAX, 45));
    }
}
