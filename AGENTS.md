# PROJECT KNOWLEDGE BASE

## OVERVIEW

This repository is a Linux-native Rust rewrite of the Prodomo Metin2 server. The goal is Parity:
the unmodified Prodomo 5.4 client, pointed at the Rewrite, plays exactly as it does against the
legacy C++ server, apart from recorded Divergences. The frozen C++ source under `server/server` is
the behavioural reference and is never modified. The owner's legacy Game data and configuration are
under `legacy/`.

Read these before changing anything:

| document | what it is for |
|---|---|
| `CONTEXT.md` | The glossary. Use its terms (Parity, Quirk, Defect, Divergence, Channel, Shared Channel, Warp, Pulse, Transfer, Operator, and the rest) and avoid the synonyms it lists. |
| `docs/adr/` | The eight architecture decisions and their dated amendments. Do not work against one without a new ADR that supersedes it. |
| `docs/STATUS.md` | Where the Rewrite stands, the next steps in order, what waits on the owner, and the Divergence and Defect registers. It is not a changelog: a ledger section updates its numbers, its next steps and its registers, and never adds a per-section paragraph. `docs/STATUS_HISTORY.md` holds what it narrated until ledger 678. |
| `legacy/README.md` | What the owner's deployment snapshot contains and what was removed. |
| `docs/PROTOCOL_NOTES.md` | Per-record findings for the client wire codecs, legacy defects, and the verification lessons behind the rules below. A `client/...` path or a GG note in it is history; the sections about the retired DB-peer protocol and the old module lists are in `docs/PROTOCOL_NOTES_HISTORY.md`, which is never updated. |
| `docs/REWRITE_LEDGER.md` | Append-only record of every change and gate receipt. Append a new section for each change; never edit an old one. A section is a slice of a package in `docs/STATUS.md`, not one `case` arm. Its receipt names the run with `DATABASE_URL` and the run without it, and lists every test that failed on the first run; "passes alone" does not close a failure. |

Do not call a system ported because it compiles, has a type with the right name, or has a codec. A
system is ported when its Parity inventory item has a passing scripted-client scenario.

## THE DECISIONS THAT SHAPE EVERYTHING

ADR-0001 to ADR-0005 were settled by the owner on 2026-09-26; ADR-0006, ADR-0007 and ADR-0008
were accepted on 2026-11-15, with the amendments of the direction review (ledger 679). The ADRs
hold the reasoning. Do not reopen them without the owner.

- **ADR-0001: the client protocol is the only contract.** Only the client wire records (every CG
  and GC record, the framing, and the TEA boundaries) and the legacy Game data file formats must
  stay byte-compatible. The SQL schema, process layout, configuration, and every server-to-server
  protocol are ours to redesign. The DB-peer protocol and the game-to-game (P2P/GG) protocol are
  retired.
- **ADR-0002: one process hosts auth and every Channel.** A single `prodomo` binary runs the auth
  listener and every Channel, including the Shared Channel (legacy channel 99). Each Channel is one
  world holding all of its maps; there are no Cores. All worlds step on one game thread at 25
  Pulses per second. Network and database work run on async tasks that feed the game thread through
  queues. Channels talk only over an in-process bus.
- **ADR-0003: PostgreSQL 18, fresh store.** No MySQL or MariaDB anywhere. IDs the client sees stay
  32-bit integers; other rows use `uuidv7()`. Passwords are argon2id. Free text is `bytea`, stored and
  relayed as the exact bytes the client sent. Every Transfer commits in one transaction when it
  happens; other player state is written in the background at most a few seconds late and always on
  logout, Warp, Channel change, and shutdown.
- **ADR-0004: quests run on Lua 5.1** through `mlua`, and `qc` is ported to Rust. All 51 quest
  scripts in `legacy/gamedata/locale/europe/quest` are live. A 5.0 idiom that 5.1 lacks gets a
  shim. A quest source is edited only to fix a mistake that stops a script (it does not compile,
  or a path always raises): the smallest edit, listed in `legacy/README.md`, with its bytes and
  hashes in the ledger, as for `_basic/change_empire.lua` (ledger 680). What a working path
  says, gives or costs is content and is never edited.
- **ADR-0005: the safebox and the mall belong to the account.** Their item rows carry
  `item.account_id`, never `owner_id`; a checkin or checkout is one Transfer between a character
  and its account. The `safebox` table holds only an argon2id password hash.
- **ADR-0006: quests keep their dialect and load the way legacy loads them.** The `quest` crate
  holds the legacy lexer, `qc`, the translator from the 5.0 dialect to Lua 5.1, the shims and the
  loader. A quest API function no ported system backs stays a "not ported" error with a Parity
  row.
- **ADR-0007: the Rewrite ships with the test-server mode off.** `[game] test_server` defaults to
  `false`, so game masters come only from `prodomo gm`. A new scenario runs in the shipped mode
  and grants the GM level it needs.
- **ADR-0008: Parity carries the game over; legacy's mistakes are fixed, not ported.** A bug, an
  inconsistency, a vulnerability or a backdoor is fixed as the code is ported, without asking,
  and recorded: a Defect row, a Divergence row when a player can tell, the ledger, and a test.
  Odd but deliberate behaviour is a Quirk and stays. A difference the Rewrite's own design
  causes is not an improvement: match legacy or argue the Divergence. A fix that changes
  numbers every player meets is also listed for the owner, and the work does not wait.

Further owner decisions, recorded in `CONTEXT.md` and `docs/STATUS.md`:

- Only the `europe` Locale is ported. Names are ASCII letters and digits and unique regardless of
  case.
- Leaving a Shared Channel map returns the player to the Channel they came from (a Divergence;
  legacy sends them to Channel 1). Each Channel keeps its legacy map set as configuration.
- The protos are the text files in `legacy/gamedata/proto` (`PROTO_FROM_DB = 0`). Game data files are
  read in place from the legacy layout; the Game data SQL tables are imported from
  `legacy/sql/gamedata`.
- Accounts, GMs, and item-shop currency are created by an Operator command in the `prodomo`
  binary (`prodomo account ...` and `prodomo gm ...`, ledger 179). There is no website.
- The adminpage has no default password and stays off until one is configured (legacy defaults to
  `SHOWMETHEMONEY`, a Defect).
- Listener ports are configurable and default to the legacy ones: auth 30001, and the Channel ports
  in `legacy/README.md` (Channel 1: 30003 and 30005, up to the Shared Channel at 30019).
- Never reproduce a Defect, and never leave one "reproduced, provisional" (ADR-0008). A bug an
  honest client can trigger is a Defect unless the owner names it a Quirk. Every deliberate
  difference is a recorded Divergence.
- The play test is the last step: the owner tests with the Reference client once every Parity
  row is `ported`, `unused` or waiting on the owner (2026-11-15).
- A character is saved every 10 seconds by default, not legacy's compiled 120 or the 180 of the
  owner's files (2026-11-15, ADR-0003).
- The `unused` rows stand without a row-by-row confirmation (2026-11-15). A row the play test
  reaches is reopened.

## AGENT RUNTIME ENVIRONMENT

The agent and repository tools may run inside a container, and the agent may launch other
containers with Podman. Where a published port is reachable depends on the machine: on some,
`host.docker.internal` reaches the host; on the machine used for ledger 179 that name does not
resolve (`getent hosts` fails while `localhost` resolves) and a published port is reached on
`127.0.0.1`. Check with `getent hosts host.docker.internal` before choosing the address.

PostgreSQL 18 runs in Podman (`postgres:18`), never inside the agent container. Database tests run
only when `DATABASE_URL` is set, so every gate stays green without a database. To run them:

```bash
podman run -d --name prodomo-pg18 -e POSTGRES_USER=prodomo -e POSTGRES_PASSWORD=prodomo-test \
  -e POSTGRES_DB=prodomo -p 127.0.0.1:55432:5432 postgres:18
DATABASE_URL=postgres://prodomo:prodomo-test@127.0.0.1:55432/prodomo \
  cargo test --workspace --all-targets --locked --offline --no-fail-fast
```

Each database test creates its own scratch database (`prodomo_test_*`, `prodomo_proc_*` or
`prodomo_async_*`), so the role needs `CREATEDB`, and drops it when the test ends, even on a
panic. Tests may run in parallel against one server, all with the same `DATABASE_URL`. After a
run, `SELECT datname FROM pg_database WHERE datname ~ '^prodomo_(test|proc|async)_'` should
return nothing ("TEST HYGIENE").

The i686 cross compiler used by the width probe (`i686-linux-gnu-g++-12`) is not installed on every
machine. Check before relying on it, and say so in the receipt when a width could not be measured.
The same goes for `rustfmt` and Clippy, which `rust-toolchain.toml` asks rustup to install: when
one is missing, say in the receipt that its gate did not run, and check formatting by hand
(rustfmt style, 100 columns). `clippy.toml` lists product names that `doc_markdown` must accept.

## WORKSPACE

The workspace has nine crates under `crates/`:

| crate | contents |
|---|---|
| `crates/protocol/` | Transport-free client wire codecs: CG and GC records, framing helpers, TEA, and the machine-readable CG and GC inventories. Nothing server-to-server. |
| `crates/common/` | Configuration, logging, legacy enums and constants, and the GM list rules. `src/tables.rs` uses `repr(C, packed)` structs; never size a wire record from them. |
| `crates/net/` | Tokio transport helpers for the fixed-size client framing. |
| `crates/db/` | The PostgreSQL store: pool, migrations, the item-ID range pool, and the queries each system needs. |
| `crates/gamedata/` | Readers for the legacy Game data formats, the importer for `legacy/sql/gamedata`, and the pure rule modules kept from the retired DB server. |
| `crates/world/` | Maps, sectors, characters, events, and the gameplay rules that act on them. |
| `crates/quest/` | The `qc` port and the Lua 5.1 quest runtime. |
| `crates/prodomo/` | The one binary: listeners, descriptors, the game thread, the in-process bus, and the Operator commands. |
| `crates/parity/` | The scripted client (test-only): `Server` runs the real `prodomo` binary, `Client` plays raw client bytes over TCP, and `inventory` checks the Parity inventory in `docs/parity/`. The scenarios are `crates/prodomo/tests/parity.rs`. |

`server/` is the frozen legacy source; it is not part of the workspace.

The Parity inventory is `docs/parity/` (`spec.md` explains it). Its generated tables come from
`python3 docs/parity/tools/gen.py`; edit only their `status`, `scenario`, and `note` cells. When a
change ports a row, set it to `ported`, name the scenario, and add the scenario in the same change.

Outside the crates:

| path | contents |
|---|---|
| `docs/migration/` | The agents' working files: the goal (`migration-goal.md`), where the work stands (`progress.md`), and the owner's questions and standing decisions (`owner-questions.md`). They are tracked and go into each section's commit. |
| `docs/parity/` | The Parity inventory and its generator. |
| `tools/probes/` | The width and enumeration probes that code comments cite, by ledger section; its `README.md` says how to build one. |
| `docs/issues/<feature>/issues/` | Open issues (see "Agent skills"). |
| `.tmp/` | Ignored. A session's scratch output: gate receipts, logs, a copy of the tree for a mutation sweep. Never cite a file there from code or from a tracked document: it exists on one machine only. Delete what a session put there when its section lands. |
| `tools/gates.sh` | Runs every gate below and writes the receipt: `tools/gates.sh <output-dir>`. |
| `tools/mutate.py` | The mutation sweep driver; its header says how to run it on a copy of the tree. |
| `docker-compose.yml`, `Dockerfile`, `config/prodomo.toml.example` | The play-test kit and the commented configuration. |
| `legacy/`, `server/` | The owner's deployment snapshot and the frozen legacy source ("LEGACY REFERENCE"). |

## BUILD AND TEST

`Cargo.lock` is tracked. Run every gate locked and offline:

```bash
cargo fmt --all -- --check
cargo build --workspace --locked --offline
cargo test --workspace --all-targets --locked --offline --no-fail-fast
cargo test --workspace --doc --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline
```

`--all-targets` does **not** run doc-tests, so the `--doc` run is a separate gate. Fix Clippy
findings by refactoring, never by `#[allow]`. A lint the whole workspace declines is switched off
once, in `[workspace.lints]` of `Cargo.toml` with its reason (three are: `module_name_repetitions`,
`must_use_candidate`, and since ledger 684 `assert_is_empty`); adding one is the owner's decision.
A dependency that no target uses fails the build (`cargo::unused_dependencies`). `unsafe_code` is
forbidden workspace-wide, which is why Lua 5.0.3 through FFI was rejected (ADR-0004).

The toolchain is Rust 1.100.0, pinned in `rust-toolchain.toml` (ledger 684); the `Dockerfile`'s
base image carries the same version, and the two change together. rustup's `cargo` reads the
pin; a distribution's `/usr/bin/cargo` does not, and `tools/gates.sh` refuses to run with any
other compiler. A toolchain change is its own ledger section. Adding or upgrading a dependency
needs an online `cargo fetch`; ask the owner first, keep new crates compatible with the pinned
compiler, record the fetch in the ledger, and run every gate offline afterwards. Record a new
receipt in the ledger whenever the test counts change.

`tools/gates.sh <output-dir>` runs all six, the tests both without and with `DATABASE_URL`, and
writes `summary.txt` with every failed test by name, everything the tests left behind, and the
number of `#[allow]` attributes, which may not rise; a ledger receipt is that summary.

## TEST HYGIENE

A test run leaves nothing behind: no scratch database, no server process, no file in the
temporary folder, no core file. Ledger 683 found 32,472 scenario directories, 15 databases and a
server three days old on the test machine. A test process that is killed runs no `Drop`, so
cleaning up in `Drop` alone is not enough.

- **A scratch database is named by `crates/db/tests/support/lease.rs`** (`lease::database_name`), never
  by hand. The name carries its process's owner token, and the process holds an advisory lock on
  that token while it lives; the next test process drops every database whose lock is free.
- **A scratch directory comes from `parity::scratch`** (`ScratchDir`, or `unique_path` for a
  caller with its own guard), never from `std::env::temp_dir()` joined by hand. The name carries
  the process ID; the next test process kills what still runs in a dead process's directory and
  removes it. `gamedata` and `quest` do not depend on `parity`; their unit tests remove their
  own files, and a new one must too.
- **A test that starts a process owns it**: a guard kills and waits for it in `Drop`, as
  `parity::Server` does.
- **`tools/gates.sh` fails on anything left.** It gives the tests a temporary folder of the
  run's own, so its `temp-files`, `servers`, `scratch-dbs` and `cores` lines count what that
  run left; each must be 0. Do not run it beside another test run on the same database server.
- **Stop a test run with Ctrl-C or SIGTERM to `cargo`, not `kill -9` on a test binary**; if one
  was killed anyway, any next test run cleans up after it.
- `PRODOMO_KEEP_SCENARIO_ROOT=1` keeps each scenario server's directory (log and configuration)
  for a look; the next test process removes it.
- A session's own scratch output goes to `.tmp/`, not to `/tmp`, and is deleted when its
  section lands. A copy of the tree (for a mutation sweep) is removed when the sweep ends; a Git
  worktree is removed with `git worktree remove`.

## SCENARIO TIMING

A scenario that fails in the full run and passes alone has a defect of its own until its
failure has been read against the legacy rule it tests. Ledgers 683 to 685 read every such loss
and swept all 519 scenarios: each cause was in the scenario or in an order the Rewrite does not
fix, none was the load (ledger 685.7 has each one). A full run keeps 16 cores busy, so a reply
can be a second late and a Pulse can be stepped late; a scenario must pass anyway.

- **A rule that draws gets enough tries.** Size the tries so that all of them missing is rarer
  than one run in a million, and write the odds in the comment (twenty-four dogs for
  `AggregateMonster`'s one in two, ten casts for the fish table's one in four, twelve digs at
  81%). Or accept every outcome the draw allows and assert each one's own records: read the
  whole table row first, its rare columns included (the grade refine's third outcome is 5%,
  and eleven catches of `fishing.txt` have no length).
- **A monster whose place is asserted stands still**: start the server with
  `no_wander = true`. Idle, a monster walks 300 to 700 one time in 7, so the longer a scenario
  takes to reach its step the further its monsters are from their lines.
- **A record sent on a clock of its own may land in any later read**: a revive's end, a
  mark's delete at its event's next check, a critical hit's effect on a swing back, a heal
  tick. A reader that names every header it allows names these too.
- **A bound on `DISTANCE_APPROX` allows its error**, 3.9% low to 4.04% high
  (`G/utils.h:17-40`), not the true distance.
- **A rule counted in whole seconds of the wall clock** (`get_global_time() - x < n`,
  `time(0)`) is tested either from the start of a second (`wait_for_a_fresh_second`) or with
  every value the straddle allows. An expiry read back from the store is bounded by a full
  run's delay, not by one second.
- **A legacy window starts at its event.** Nothing unrelated (a second login, a `quiet`, a
  `sleep`) goes between the event and the step that must land inside the window. A clock that
  proves a lower bound starts before the send that makes the server stamp.
- **A required record is read with `read_game()`** or a deadline of seconds. `QUIET_WINDOW`
  (300 ms) proves silence only; a wait sized to a Pulse count leaves half a second over it.
- **A reader that takes a deadline reads with the `_until` forms** (`read_game_raw_until`,
  `read_wire_until`). `read_game()` and `read_wire()` panic after five quiet seconds
  (`IO_TIMEOUT`) whatever the deadline says, and a wire can be quiet that long: a monster
  cannot see a player for 5 s after the enter (`AFF_REVIVE_INVISIBLE`) and looks again only
  every 1 to 3 s.
- **After a client is dropped, SQL on a column the save writes waits for the save**:
  `server.wait_for("Character disconnected; wrote the row")`. "Character left the world" is
  logged before the save and is not a barrier. A second wait for a line the server already
  logged is `Server::wait_for_next`, since `wait_for` returns at once on a collected line. A
  character that left by a Warp logs no save line (the Warp wrote the row); wait for its
  "Client connection closed addr=" line.
- **A relogin needs no wait.** The login answers `ALREADY` until the old descriptor's save is
  written, and the login helpers retry it.
- **Dropping a `Server` kills it** (`SIGKILL`), so nothing is saved. Kill the server before
  dropping the client when a scenario wants the session unsaved, and wait for the save line
  when it wants it saved; `drop(client); drop(server);` leaves it to chance.
- **Name the state a step depends on.** A potion is refused silently on a full pool, so a
  drink that must land goes before the bites heal.
- **Two descriptors have no order between them.** Records one Pulse sends to one client keep
  their order; what two clients' descriptors each do on their own (a Warp ordered for a whole
  party, two saves) happens in either order. A reader accepts both (`read_own_removal`), and
  where legacy has one order the difference is a `docs/STATUS.md` item (C11), not a wider
  assert kept quiet.
- **A loss keeps its output.** `tools/gates.sh` keeps `test-db.log`; read the panic, the
  client's `GAME` records and the server's `SERVER` lines (a failing scenario prints its
  server's last 400 console lines) before running the scenario again. To reproduce one, run
  the built test binary many times at once (`seq 1 144 | xargs -P 24 ...` from `crates/prodomo/`),
  which is the load a full run gives it.

## PROTOCOL COVERAGE

Key every coverage metric on the **wire byte**, never on an identifier name.

| direction | registered | implemented | missing |
|---|---|---|---|
| client to game | 92 | 91 | 1 (`KEY_AGREEMENT`, dead: `_IMPROVED_PACKET_ENCRYPTION_` is never defined) |
| game to client | 134 | 125 | 9 (5 fixed-size, 4 dynamic; `REFINE_INFORMATION` and `SYNC_POSITION` have codecs and stale flags, `docs/STATUS.md` C8) |

- The game-to-client registration table was taken from the client's `PythonNetworkStream.cpp`
  while the client source was in the repository. It is recorded in `crates/protocol/src/gc_inventory.rs`.
  `server/server/game/packet_info.cpp` registers only inbound records and must never be used to
  measure game-to-client coverage. Bytes 100, 102 and 133 are outside the table by design:
  the mark downloader decodes them on its own path (`crates/protocol/src/gc_mark.rs`), so they are
  implemented but never rows.
- The Reference client source is no longer in the repository (ADR-0001). `server/server/game/packet.h`
  and the server's send sites are the reference for record widths; the owner calibrates them against
  the Reference client in the play test.
- `crates/protocol/src/cg_inventory.rs` and `gc_inventory.rs` are the machine-readable tables. Each row's
  implemented flag is tested, so update the flag in the same change as the codec.
- Nine GC bytes are decoded by the client but never sent by the checked-in server: 6, 15, 18, 72,
  73, 84, 112, 117, and 213. Treat them as reachability questions, not porting work. Byte 6 is the
  newest member: the server has the enumerator and the struct, but `DESC::SendLoginSuccessPacket`
  writes byte 32 into it (`server/server/game/desc.cpp:880`).
- Add a codec only when the system being ported needs it.

## VERIFICATION AND SEARCH RULES

These rules exist because each one was broken at least once in this repository. `docs/PROTOCOL_NOTES.md`
("Verification lessons") has the incidents.

- **Every negative search needs a positive control and a negative control.** An absence claim
  without a control is not evidence. If the harness provides the `metin2-verify-absence-source-sweep`
  skill, use it before reporting an absence.
- **A module's `//!` doc resolves intra-doc links in the crate root, not in its own module.**
  On this toolchain a bare `EWindows` in the module doc of `crates/common/src/item_slots.rs` fails
  `RUSTDOCFLAGS="-D warnings"` even though the enum is declared in the same file 200 lines below,
  while the same link inside an *item's* doc comment in that file resolves normally. Write
  `crate::item_slots::EWindows` in a module's `//!` block. The asymmetry is what makes this look
  like a typo rather than a scope rule.
- **Search tooling traps.** If `grep` is a shell function or alias in your shell, call `command grep`.
  Pass `-a`, because GNU grep reports `server/server/common/VnumHelper.h` and some Game data files as
  binary and hides their lines. Never filter with `--include=*.h` alone; use both `*.h` and `*.H`, or
  read files in code with `errors="replace"`.
- **Resolve a constant across every definition form**: `#define NAME n`, `const T NAME = n;`, a
  multiline anonymous-enum member, a single-line `enum { NAME = n };`, an enum member whose value is
  an expression, and an auto-incremented enum member with no `=`. Then check which `#ifdef` gates the
  use site sits behind; the server's feature switches are in `server/server/common/prodomodefines.h`.
- **Sweep the struct tag and the typedef alias as two separate searches** before calling a legacy
  struct unused. Call sites mostly spell the tag.
- **Search the whole legacy tree for struct definitions**, not only `packet.h`. Handle both the
  `} NAME;` typedef form and `struct NAME { ... };`, including classes with constructors.
- **Measure packed widths by compiling the verbatim struct body for i686**, never by summing fields.
  Use `i686-linux-gnu-g++-12 -static` under `#pragma pack(1)`, and have every probe run re-measure
  known widths and a packed two-`long` struct that must be 8 bytes. Keep a hand sum only as the value
  you check the probe against.
- **The legacy target is 32-bit x86** (`server/server/premake5.lua:12`). C++ `long`, `time_t`, and
  the `QWORD` typedef (`unsigned long`) are 4 bytes; `bool` is 1 byte. A 64-bit reading silently
  breaks framing.
- **Never share a header table between directions**, and never infer a width from the fact that two
  directions use the same number. Sort every cross-direction collision into one of the four buckets
  in `docs/PROTOCOL_NOTES.md` ("Cross-direction header collisions").
- **A round trip is not an independent witness.** It only proves that the encoder and decoder agree.
  Pin every record with golden bytes taken from the source field order. Use test values with
  distinct byte halves; `u16::MAX` is byte-symmetric and cannot catch an endianness bug.
- **Mutation sweeps:** apply each mutant to the pristine file, assert the text changed **in
  executable code** (a mutation that landed in a doc comment proves nothing), and restore by
  checksum. Report compile-error kills separately from semantic kills. When a survivor is an
  equivalent mutant, test the class of real defect it hides.
- **Game data is read byte-exact.** Several files are CRLF, the proto name column is Korean in a
  legacy code page, and only `skill_proto.szName` is UTF-8. Never transcode on read; never rewrite a
  file under `legacy/`.

## COMPATIBILITY RULES

- Derive layouts and behaviour from the legacy source, not from Rust type names.
- Legacy records use `#pragma pack(1)`. Use explicit field-by-field wire codecs with explicit
  little-endian conversion. Do not make Rust packed structs the compatibility representation, and
  never use a non-packed `#[repr(C)]` struct for sizing.
- Legacy client packets use a one-byte header plus a header-specific payload size, not a generic
  length prefix. Variable records use the sub-header and count rules in `protocol::cg_variable`.
- A header-dependent frame adapter checks the **payload** width (the frame excludes the header
  byte); the record decoder checks the full width. Check the exact length before reading any field
  or the header.
- Fixed `char[N]` fields stay raw `[u8; N]`: no `&str`, no `CStr`, no NUL or UTF-8 requirement,
  unless a record's own note says otherwise (for example `GcLoginFailure`). A stricter codec would
  reject bytes the legacy server accepts.
- A byte the legacy handler never range-checks stays opaque, and all 256 values round-trip.
  Sentinels, handler `switch` arms, and gameplay bounds (slot counts, window types, VID validity)
  belong above the codec. Read the handler before deciding whether a byte is opaque or sub-typed.
- Never reproduce a Defect and never silently compensate for one in a codec. Record it in
  `docs/PROTOCOL_NOTES.md` or `docs/STATUS.md` and in the ledger. Where the Rewrite deliberately
  differs, record a Divergence, not parity.
- Unknown and malformed client frames close the descriptor. Legacy mostly logs and consumes
  (`CInputLogin::Analyze` has its `SetPhase(PHASE_CLOSE)` commented out, and `CInputMain::Analyze` is
  a bare `return (0)`), so closing is a recorded Divergence (ledger 160.5), not parity.
- Timers, cooldowns, and regeneration are counted in Pulses at 25 per second, as legacy counts them.
  Do not convert a legacy Pulse count to wall-clock time.

## CONFIGURATION

- One `prodomo.toml` configures the whole process (`common::config::ServerConfig`); the commented
  example is `config/prodomo.toml.example`. The legacy `CONFIG` and `conf.txt` files are never read.
  Ledger 178.4 lists the legacy keys that were dropped and why.
- Every table refuses unknown keys, so a misspelling fails at startup. Keep `deny_unknown_fields` on
  every new table.
- `ServerConfig::validate` refuses an impossible topology before anything is bound. A new rule about
  Channels, ports, or maps belongs there, not in the listener code.
- A `[game]` key keeps the unit the legacy file was written in: the save and ping cycles are
  seconds, although legacy multiplied them into Pulses while parsing. Its default is the legacy
  compiled-in value unless that value is a Defect, which the ledger then records.
- Never log or print a secret. The store URL goes through `redact_url`; a password-like setting is a
  `Secret`. A test that feeds a known password must assert it never reaches the console.

## STORE AND OPERATOR COMMANDS

- A schema change is a new file in `crates/db/migrations/` (`NNNN_name.sql`); never edit a migration that
  has been applied, because sqlx refuses a changed checksum. `db::store::MIGRATOR` embeds the
  folder, `crates/db/build.rs` rebuilds when it changes, and a test pins that versions start at 1 with no
  gap.
- Keep a `CHECK` constraint behind every rule the Rust types enforce (login and Name shape, the
  argon2id prefix, currency ranges), so a row written by hand cannot break an invariant the game
  relies on.
- `prodomo serve` binds its ports with `ReadyGate` closed and then migrates. Only a transient error
  is retried (I/O, TLS, a pool timeout, SQLSTATE class `08`, `53`, or `57P`), with a pause that
  starts at 1 second and doubles up to 30; any other error exits non-zero. Do not widen
  `StoreError::is_transient`: a wrong password or a missing database never heals by waiting.
- Operator commands validate every argument and read the password **before** connecting, migrate
  once without retrying, print their result on stdout, print `error: ...` on stderr, and exit 1 on
  failure. A password is read from standard input (on a terminal, twice with echo off) and is never
  a command-line argument, which `ps` and shell history would expose.
- A currency change is a signed delta applied under `SELECT ... FOR UPDATE` in one transaction. A
  result outside the column's range is refused, never clamped.
- A GM grant belongs to one account. Ask `db::accounts::gm_authority` with the account **and** the
  character Name; never grant authority by Name alone.
- A database test is gated on `DATABASE_URL`, creates its own scratch database, and drops it on
  `Drop`, so tests never share rows and a panic leaves nothing behind.

## DESCRIPTOR CONTRACTS

These transport-free reducers live in `prodomo`.

- `ClientLifecycle` preserves the setup, heartbeat, handshake, and post-handshake phase-transition
  effect order and treats close as control-only. A descriptor must keep the recorded output
  boundary when writing a phase packet: legacy `DESC::SetPhase` writes `GC_PHASE` **before** enabling
  TEA for the Handshake-to-Login/Auth transition, while later encrypted transitions use the prior
  encrypted boundary.
- `DescriptorCrypto` covers the active non-improved TEA branch. It starts in explicit plaintext
  mode, derives the source Myevan key and the client/server pair without unsafe pointer arithmetic,
  buffers the largest aligned ciphertext prefix while retaining a 1-7-byte tail, and zero-pads each
  output unit. It owns no socket and no improved DH2 path.
- `AccountPlayerSession` sits beside `ClientLifecycle`, not inside it. Its local `Login` phase does
  not prove the lifecycle is in `ClientPhase::Login`; the descriptor must check the actual phase,
  then run blocked-IP, shutdown, and user-limit admission before `on_login`. Its DB-peer frame
  entry points were removed in ledger 177; the store calls `on_login_result` and `on_player_result`
  directly. Keep its generation correlation, which is what stops a late database answer from
  reaching a closed or reused descriptor.

## SECURITY AND TRUST BOUNDARIES

- Every length, count, index, and slot read from a client is bound-checked before use. Legacy
  `input_login.cpp:265-304` indexes `c_r.players[pinfo->index]` before its `PLAYER_PER_ACCOUNT`
  check, and `454-515` forwards an unchecked create index. Both are Defects.
- Every SQL value is a bound parameter. Never format a value into SQL. Legacy builds its auth query
  by string formatting (`input_auth.cpp:133-152`) and interpolates `TABLE_POSTFIX`; both are Defects,
  and `TABLE_POSTFIX` has no equivalent in the Rewrite.
- Passwords are verified with argon2id only. Never store, log, or compare a plaintext password, and
  never reproduce MySQL `PASSWORD()`.
- A client is not accepted until the store's schema is migrated and the world it would enter has
  finished loading its Game data. Legacy accepts clients before its DB boot completes; that is a
  Defect.
- The adminpage and every Operator path fail closed: no default password, no default allowlist.
- Credentials never enter the repository. `legacy/config` has every credential replaced by
  `REDACTED`; keep it that way, and use environment variables or an untracked local config for real
  secrets.
- The recorded legacy Defects not to reproduce are listed in `docs/STATUS.md`.

## LEGACY REFERENCE

- `server/server/game/`: legacy game behaviour, packet tables, descriptors, managers, and the main
  loop.
- `server/server/db/`: the legacy DB server. Its caches and boot stream are retired, but it is still
  the reference for what is loaded, when it is saved, and what the login checks are.
- `server/server/common/`: shared legacy structures, and the feature switches in `prodomodefines.h`.
- `server/server/quest/`: legacy `qc` and the quest runtime.
- `server/server/lib*`: legacy libraries.
- `legacy/config/`: the owner's per-process configuration; the source of the default topology.
- `legacy/gamedata/`: protos, maps, quests, locale strings, and drop and shop tables.
- `legacy/sql/`: the legacy schema, for reference only, and the Game data table rows to import.
- `server/extern/`: bundled dependencies. Preserve them.

Do not build or run the legacy server. The owner's play test against the Reference client is the
final check.

## LEGACY BEHAVIOURS TO KEEP

- `char_battle.cpp` resets `m_dwKillerPID = 0`; keep that reset in the port.
- `text_file_loader.cpp` and `group_text_parse_tree.cpp` do not allow spaces in group names; the
  Game data readers must parse the same way.

## REPOSITORY HYGIENE

- Commit on `main` and push right after; never create or switch branches. Commit only when the user
  asks or a plan they confirmed includes the commit.
- Never run an unrestricted `git clean`, `git checkout .`, or `git stash -u`.
- Keep the legacy C++ source until the play test demonstrates Parity.
- Generated output is disposable: root `target/`, the root `log/` folder a locally run server
  writes, and `.tmp/`.
- A research note, a survey, a finished issue or a one-off script is not kept in the tree. Its
  findings go into the ledger section, `docs/PROTOCOL_NOTES.md` or a code comment, and the file is
  deleted in the change that finishes the work; Git keeps the rest. A probe that code cites goes
  to `tools/probes/`. Nothing new goes at the repository root.
- `.gitignore` names no build-output directory (`debug/`, `log/`, `x86/`, `bin/`, `obj/`): nothing
  is built in `server/`, and `server/extern/include/` has tracked directories with such names.
  Do not add such a rule.
- `.overlord/` is an ignored agent runtime directory. Do not delete it wholesale while a session
  uses it; a worktree in it whose work has landed is removed with `git worktree remove`.
- Stray `.DS_Store` files are ignored; do not add them to Git.
- Before a large edit to a Rust file, make sure it is committed or copied aside. An edit once left
  `db-server/src/main.rs` truncated while it was untracked, and no copy could restore it.
- Run `cargo fmt` before compiling, not after. A file that fails to parse reports misleading
  delimiter errors.
- Write generated files only inside the intended test root. Run probes from a scratch directory and
  check the workspace root for stray `*.core` files afterwards.

## Agent skills

### Issue tracker

Issues and specs are local markdown files under `docs/issues/<feature>/`. See
`docs/agents/issue-tracker.md`. An issue whose work has landed is deleted in that change; a
`ready-for-agent` file is always work still to do.

### Triage labels

The five default roles (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`,
`wontfix`), recorded on each issue file's `Status:` line. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one root `CONTEXT.md` and `docs/adr/`. See `docs/agents/domain.md`.
