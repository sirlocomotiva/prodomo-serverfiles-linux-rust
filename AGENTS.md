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
| `docs/adr/` | The four architecture decisions. Do not work against one without a new ADR that supersedes it. |
| `docs/STATUS.md` | Where the Rewrite stands, the build order, and the next step. |
| `legacy/README.md` | What the owner's deployment snapshot contains and what was removed. |
| `docs/PROTOCOL_NOTES.md` | Per-record findings for the client wire codecs, legacy defects, and the verification lessons behind the rules below. Its DB-peer and GG sections are history. |
| `docs/REWRITE_LEDGER.md` | Append-only record of every change and gate receipt. Append a new section for each change; never edit an old one. |

Do not call a system ported because it compiles, has a type with the right name, or has a codec. A
system is ported when its Parity inventory item has a passing scripted-client scenario.

## THE DECISIONS THAT SHAPE EVERYTHING

Settled by the owner on 2026-09-26. The ADRs hold the reasoning. Do not reopen them without the
owner.

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
  shim; a quest source is never edited.

Further owner decisions, recorded in `CONTEXT.md` and `docs/STATUS.md`:

- Only the `europe` Locale is ported. Names are ASCII letters and digits and unique regardless of
  case.
- Leaving a Shared Channel map returns the player to the Channel they came from (a Divergence;
  legacy sends them to Channel 1). Each Channel keeps its legacy map set as configuration.
- The protos are the text files in `legacy/gamedata/proto` (`PROTO_FROM_DB = 0`). Game data files are
  read in place from the legacy layout; the Game data SQL tables are imported from
  `legacy/sql/gamedata`.
- Accounts, GMs, and item-shop currency are created by an Operator command in the `prodomo`
  binary. There is no website.
- The adminpage has no default password and stays off until one is configured (legacy defaults to
  `SHOWMETHEMONEY`, a Defect).
- Listener ports are configurable and default to the legacy ones: auth 30001, and the Channel ports
  in `legacy/README.md` (Channel 1: 30003 and 30005, up to the Shared Channel at 30019).
- Never reproduce a Defect. A bug an honest client can trigger is a Defect unless the owner names it
  a Quirk. Every deliberate difference is a recorded Divergence.

## AGENT RUNTIME ENVIRONMENT

The agent and repository tools run inside a container. The agent may launch other containers with
Podman. Ports those containers expose are reachable from the agent container through
`host.docker.internal`; do not assume another container listens on the agent container's localhost.

PostgreSQL 18 runs in Podman (`postgres:18`), never inside the agent container. Database tests run
only when `DATABASE_URL` is set, so every gate stays green without a database.

The i686 cross compiler used by the width probe (`i686-linux-gnu-g++-12`) is not installed on every
machine. Check before relying on it, and say so in the receipt when a width could not be measured.
The same goes for `rustfmt` and `cargo-clippy`: when one is missing, say in the receipt that its gate
did not run, and check formatting by hand (rustfmt style, 100 columns).

## WORKSPACE

The layout below is the target of build step 1 (see `docs/STATUS.md`). `docs/STATUS.md` records which
parts exist today.

| crate | contents |
|---|---|
| `protocol/` | Transport-free client wire codecs: CG and GC records, framing helpers, TEA, and the machine-readable CG and GC inventories. Nothing server-to-server. |
| `common/` | Configuration, logging, legacy enums and constants, and the GM list rules. `src/tables.rs` uses `repr(C, packed)` structs; never size a wire record from them. |
| `net/` | Tokio transport helpers for the fixed-size client framing. |
| `db/` | The PostgreSQL store: pool, migrations, the item-ID range pool, and the queries each system needs. |
| `gamedata/` | Readers for the legacy Game data formats, the importer for `legacy/sql/gamedata`, and the pure rule modules kept from the retired DB server. |
| `world/` | Maps, sectors, characters, events, and the gameplay rules that act on them. |
| `quest/` | The `qc` port and the Lua 5.1 quest runtime. |
| `prodomo/` | The one binary: listeners, descriptors, the game thread, the in-process bus, and the Operator commands. |

A scripted-client test crate that drives the real `prodomo` binary over TCP arrives with the Parity
inventory (build step 2). `server/` is the frozen legacy source; it is not part of the workspace.

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
findings by refactoring, never by `#[allow]`. `unsafe_code` is forbidden workspace-wide, which is
why Lua 5.0.3 through FFI was rejected (ADR-0004).

The toolchain is Rust 1.85.1. Adding or upgrading a dependency needs an online `cargo fetch`; ask
the owner first, keep new crates compatible with 1.85, record the fetch in the ledger, and run every
gate offline afterwards. Record a new receipt in the ledger whenever the test counts change.

## PROTOCOL COVERAGE

Key every coverage metric on the **wire byte**, never on an identifier name.

| direction | registered | implemented | missing |
|---|---|---|---|
| client to game | 92 | 91 | 1 (`KEY_AGREEMENT`, dead: `_IMPROVED_PACKET_ENCRYPTION_` is never defined) |
| game to client | 134 | 96 | 38 (19 fixed-size, 19 dynamic) |

- The game-to-client registration table was taken from the client's `PythonNetworkStream.cpp`
  while the client source was in the repository. It is recorded in `protocol/src/gc_inventory.rs`.
  `server/server/game/packet_info.cpp` registers only inbound records and must never be used to
  measure game-to-client coverage.
- The Reference client source is no longer in the repository (ADR-0001). `server/server/game/packet.h`
  and the server's send sites are the reference for record widths; the owner calibrates them against
  the Reference client in the play test.
- `protocol/src/cg_inventory.rs` and `gc_inventory.rs` are the machine-readable tables. Each row's
  implemented flag is tested, so update the flag in the same change as the codec.
- Eight GC bytes are decoded by the client but never sent by the checked-in server: 15, 18, 72, 73,
  84, 112, 117, and 213. Treat them as reachability questions, not porting work.
- Add a codec only when the system being ported needs it.

## VERIFICATION AND SEARCH RULES

These rules exist because each one was broken at least once in this repository. `docs/PROTOCOL_NOTES.md`
("Verification lessons") has the incidents.

- **Every negative search needs a positive control and a negative control.** An absence claim
  without a control is not evidence. If the harness provides the `metin2-verify-absence-source-sweep`
  skill, use it before reporting an absence.
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
- A client is not accepted until the world it would enter has finished loading its Game data.
  Legacy accepts clients before its DB boot completes; that is a Defect.
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
- Generated output is disposable: root `target/` and enumerated `Debug`/`Release` build trees.
- `.gitignore` re-includes directories under `server/extern/include/` whose names (`debug/`,
  `log/`, `x86/`, `win32/`, `arm/`) match build-output rules. Keep that exception when editing ignore
  rules.
- `.overlord/` is an ignored agent runtime directory. Do not delete it wholesale when it exists.
- Stray `.DS_Store` files are ignored; do not add them to Git.
- Before a large edit to a Rust file, make sure it is committed or copied aside. An edit once left
  `db-server/src/main.rs` truncated while it was untracked, and no copy could restore it.
- Run `cargo fmt` before compiling, not after. A file that fails to parse reports misleading
  delimiter errors.
- Write generated files only inside the intended test root. Run probes from a scratch directory and
  check the workspace root for stray `*.core` files afterwards.

## Agent skills

### Issue tracker

Issues and specs are local markdown files under `.scratch/<feature>/`. See
`docs/agents/issue-tracker.md`.

### Triage labels

The five default roles (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`,
`wontfix`), recorded on each issue file's `Status:` line. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one root `CONTEXT.md` and `docs/adr/`. See `docs/agents/domain.md`.
