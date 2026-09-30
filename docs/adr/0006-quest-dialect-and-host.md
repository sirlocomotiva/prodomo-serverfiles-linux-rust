# Quests keep their dialect and load the way legacy loads them

ADR-0004 puts quests on Lua 5.1 and keeps their sources unchanged. The sources are not plain
Lua 5.0: legacy compiles them and loads every library with its own Lua 5.0 lexer
(`server/server/liblua/llex.c`), which adds the reserved words `quest`, `state`, `with` and
`when`, reads `begin` as `do`, `!` as `not` and `!=` as `~=`, and prints `do` back as `begin`.
`qc` re-emits the bodies of `when` blocks and functions token by token with that lexer, so the
compiled chunks in `object/` still hold the dialect. The libraries hold it too
(`luaLibrary/luaFunctions.lua:103` compares with `!=`). Lua 5.1 accepts none of it.

The Game data holds 61 quest files outside `object/`: 51 quest scripts (25 in `_basic`, 10 in
`_basic/guild`, 13 in `event`, 1 in `dungeon`, 2 in `rank`) and 10 libraries (`questlib.lua`,
`questlib_extra.lua`, `questing.lua`, `GFquestlib.lua`, `locale.lua`, `multiLocale.lua` and the 4
files of `luaLibrary/`). All 51 scripts open with `quest NAME begin` (the positive control) and no
library does (the negative control). `quest_list` names 16 of the scripts, one per line (the last
has no line end). Legacy also loads `settings.lua` and `translate.lua` from the locale directory.

One script cannot be live. In `_basic/change_empire.lua` an `end` on line 132 closes the
`if ret == 1` chain before its `elseif ret == 4`, so the file is not Lua, and legacy's `qc` aborts
on it (`expecting 'when' or 'function'`, line 139). It is not in `quest_list` and never reached
`object/`. The other 50 compile.

## Decision

- **The Rust `qc` reproduces legacy's output byte for byte**, including `begin` for `do`, `%g`
  numbers and strings printed between double quotes without escapes. Where that output would
  change what a source means (a number `%g` rounds, or a string holding a quote, a backslash
  or a line end), the Rust `qc` refuses the source instead: none of the 999 numbers and 1056
  strings of the 51 scripts does (a synthetic `1234567` and `'a"b'` are the positive controls).
- **Every chunk is translated at load**, never on disk. A lexer that follows legacy's rewrites
  `begin` to `do`, `!` to `not` and `!=` to `~=`, re-emits every string as an escaped Lua 5.1
  literal with the bytes legacy's lexer read, replaces comments with whitespace, and keeps every
  line where it was. A legacy-only reserved word (`quest`, `state`, `with`, `when`) outside a
  string is refused, as legacy's parser refuses it. This covers the compiled chunks, the `when`
  conditions, `arg` and `begin_condition` files, the libraries, `dofile` and `loadstring`.
- **Lua 5.0 semantics that the sources use get shims**:
  - A generic `for` over a table (`for k, v in t do`, 8 places in scripts, 11 in libraries)
    iterates with `next`, as Lua 5.0's `TFORPREP` did: the translator wraps every generic `for`
    list in `__compat_iter(...)`, which returns `next, t` for a table and passes anything else
    through.
  - `table.getn`, `table.setn`, `table.insert`, `table.remove` and `table.foreachi` use Lua 5.0's
    length: a numeric `n` field, then the size `setn` or `insert` recorded, then a count up to
    the first nil. Live code calls `getn` 40 times, `insert` 32 times and `foreach`/`foreachi` 6
    times. `table.concat`, `table.sort` and `unpack` keep Lua 5.1's length: live code never
    calls them.
- **The library chain is legacy's**: `settings.lua` (with its `dofile`s of `BlueDragon.lua` and
  `quest/GFquestlib.lua`), `quest/questlib.lua` (with its `dofile`s of the 4 `luaLibrary/` files
  and `multiLocale.lua`), `translate.lua`, `quest/locale.lua`, then one state table per quest.
  `get_locale_base_path()` names the locale directory as legacy's did, and `dofile` reads only
  below it. Legacy never loads `questlib_extra.lua` or `questing.lua`; the tests load them in a
  separate state and nothing else does (`questlib_extra.lua` ends by calling
  `check_event_flags()`, which refuses on `game.get_event_flag`).
- **Standard libraries**: base (with `coroutine`), `table`, `string` and `math`, as legacy opened
  them, and an `os` table holding only `date`, `time`, `clock` and `difftime`. The base library
  loses `load` and `loadfile`, which would read code the translator never sees, and `package`
  is not opened, so there is no `require`. Legacy also opened `io` and `debug` (marked `TEMP` in
  `questlua.cpp:652`), and with `io` all of `os`; live code reaches `io` only from the file
  loggers `regenWriteLine`, `writeSyserr` and `writeSyslog` in `luaFunctions.lua`, which no
  script calls, and never reaches `debug`, the other `os` names, `loadfile` or `require`. That is
  a Divergence: a quest cannot touch the host's files.
- **A script `qc` refuses is not loaded**: the loader logs its error and loads the rest, as legacy
  ran only what reached `object/`. `change_empire.lua` stays refused until the owner fixes its
  source (a STATUS row); the Rewrite does not edit it, and a test pins that it is the only one.
- **Quest order**: legacy numbers quests in the `readdir` order of `object/state/`, which the file
  system chooses. The Rewrite compiles `quest_list` in order and then the other scripts by path,
  and a quest's index is the position where its name first appears. Two scripts,
  `_basic/guild/guild_building.lua` and `_basic/guild/guild_manage.lua`, both declare the quest
  `guild_building`: as in legacy, they share one index and one state table (both have only the
  state `start` and no functions), and each keeps its own `when` scripts.
- **The host**: `mlua`'s `send` feature, which adds no crate, keeps the Lua state `Send` inside
  the game state. Every legacy API name is registered as a C function (the 699 names of the
  `questlua_*.cpp` tables and the global table, which a test holds equal to the Parity inventory
  `quest-api.md`, generated from `server/`), and no library replaces one with a Lua function.
  The calls `settings.lua` makes at load (`add_bgm_info`, `add_goto_info`,
  `set_bgm_volume_enable`, `arena.add_map`) are recorded into the host's settings. A call reaches
  the game through one function that lives only for the current script run (`Lua::scope`), so
  the API borrows the game state without `unsafe` or `'static` data. `q.yield` is
  `coroutine.yield`, and a name whose system is not ported raises a "not ported" error, which
  ends the script as any legacy script error does.
- **Quest records**: a character's quest flags (`quest.flag` values, `__status` for the state)
  live in memory while the character is logged in and end with it (`DisconnectPC`). No ported
  API writes one: `set_state`, `q.setstate`, `pc.setqf` and the rest are "not ported", so no
  script can change a state that a later login would see. Legacy's `quest` table, loaded with
  the character and saved with it, lands as a new table with the first of them, and until then
  the lost flags are a recorded Divergence with a STATUS row. Event flags stay in memory, as
  they are until an event system is ported.

## Considered options

- Editing the sources or the compiled chunks into Lua 5.1: ADR-0004 forbids it, and the reference
  output would no longer be legacy's.
- A second Lua 5.1 lexer patch (a vendored, modified Lua): the workspace vendors Lua through
  `mlua` and must not carry C changes.
- Loading only `quest_list`: ADR-0004 makes all 51 scripts live.

## Consequences

- `quest` holds the legacy lexer, `qc`, the translator, the shims and the loader. Its tests
  compile the 16 `quest_list` scripts against the 95 reference files in `object/`, compile the
  other 34 and refuse `change_empire.lua`, and load every compiled script and every library.
- The first client-visible trigger is a quest `chat` menu on an NPC click, before its shop.
- Every API function a ported system cannot back stays a "not ported" error with a STATUS row.
