# Rewrite status

Last reviewed: 2026-09-26, against ledger sections 1-175 and the owner decisions below.

This page records where the Rust rewrite stands, what is and is not wired, and the recommended
next milestones. Rules live in `AGENTS.md`. Per-record findings live in `docs/PROTOCOL_NOTES.md`.
The change history lives in `docs/REWRITE_LEDGER.md`. When this page and the ledger disagree, the
most recent ledger section wins; update this page in the same change.

## Summary

The ledger sections built transport-free wire codecs and pure reducers, and sections 170-174 gave a
running `game-server` a real DB socket, made its client listener wait for boot, and gave
`db-server` a live boot table loader with a per-request GM tail. Codec coverage is broad and well
tested: 2,111 tests across 31 test binaries, and every gate is green. **No client can yet log in,
select a character, or enter the game against the Rust servers.** The end-to-end exchange that works
is a real `game-server` DB client connecting to a real `db-server` process: it composes `GD_BOOT`
and `GD_SETUP`, the DB answers both, the game client validates the boot reply, reaches `Booted`,
opens its boot-ready gate, and records the returned map location. A process test asserts exactly
that. What is still missing is the client-facing path: handshake, TEA, login, and character
selection.

| direction | registered | implemented | missing |
|---|---|---|---|
| client to game (CG) | 92 | 91 | 1 (dead: `KEY_AGREEMENT`) |
| game to client (GC) | 134 | 96 | 38 (19 fixed-size, 19 dynamic) |
| game to game (GG) | 36 | 4 | 32 |

Most of the remaining GC and GG records are not needed for a first login. The recommendation is to
**stop adding codecs that no milestone needs**, and build the vertical slice below.

## What the binaries actually do

| binary | behavior today | evidence |
|---|---|---|
| `game-server` | Accepts TCP clients and recognises only keepalive and pong. It sends no handshake and never installs TEA. It has a complete, tested, **transport-free** game-to-DB client (`GameDbClient`) and a live socket adapter (`DbLink`) behind it, so a running process really connects to `db-server`, composes `GD_BOOT` then `GD_SETUP`, and **refuses clients until the boot gate opens**. The game loop runs a no-op tick. It arms SIGTERM and SIGINT **before** binding the listener, so an open port implies a handled signal. | `game-server/src/main.rs` (only `KeepAlive` and `Pong` are matched); `arm_shutdown_signal()` precedes `setup_tcp_listener` (ledger 170.11); `db_client.rs`, `db_client_live.rs`, and `db-server/tests/game_server_db_gate.rs` (ledger 172.2, 172.4, 173) |
| `db-server` | Accepts peers, classifies each connection's transport identity, authorizes every request through the fail-closed peer policy, and **answers `BOOT` from a live table cache**: `BootTableLoader` reads the 11 always-present sections plus the profile-selected `renewal_shop`, `event`, and `premium_market_price` sections from `SQL_PLAYER`, and `LiveBootResponder` composes the frame per request. The GM tail (`gmhost` and `gmlist`) is read from `SQL_COMMON` **inside each boot request**, because legacy filters the administrator list on the requesting peer's `szIP`. A missing common pool refuses the boot rather than serving empty GM lists. **Open:** still answers an ordinary game `SETUP` with one `0xfe` map-locations frame; every other request is authorized and then logged as unimplemented. | Ledger 170.8 records a live BOOT run: reply header `43`, handle `0`, declared length `415`, version `6`, real clock, `ff ff` terminator. Ledger 171.9 records five process tests. Ledger 174 records the GM tail, 17 killed mutations, and the fail-closed rules. Ledger 175 audits all 14 statements against the legacy loader and records the one real divergence: the mob and item tables are read from SQL, which is the legacy `PROTO_FROM_DB=1` path, not the default `ClientManagerBoot.cpp` text-file path. Neither text file is checked into this repository, so the default legacy path is not reproducible here. **A match with any given legacy deployment is conditional on that deployment running `PROTO_FROM_DB=1`.** `db-server/tests/boot_process.rs` now asserts an unavailable-table case gets **no** boot reply. |

Other facts that matter for integration:

- Only 5 of the 91 CG codecs are used outside `protocol`. `game-server/src/handshake_dispatch.rs`
  and `account_player_router.rs` are complete and tested but have no caller.
- `game-server` depends on `world` and `quest` in `Cargo.toml` but never imports either.
- `db-server` opens **lazy** pools through `ConnectionPool::lazy`; the game server still never
  constructs a pool.
- Process-level tests: `game-server/tests/game_server_process.rs` and
  `db-server/tests/boot_process.rs`.

## Vertical slice: what exists and what is missing

| stage | present in Rust | missing |
|---|---|---|
| Login by key | `CgLoginByKey`; `AccountPlayerSession::on_login*`; the 362-byte DB login adapter; `player_index` and its SQLx adapter; `GcEmpire`; the 357-byte `GcLoginSuccess` | DB-side checks from `server/server/db/ClientManagerLogin.cpp:82-150`; player-summary SQL; any wiring |
| Select and load | `on_select`; `PlayerLoadRequest`; `PlayerResultRecord` (2,007 bytes); affect load | Player and item SQL; `DG_ITEM_LOAD` and `TPlayerItem`; building a character from the table |
| Loading-phase GC | Bytes 15, 16, 76, and 28-30; `gc_actors` | `ITEM_SET2` (21); `ENTITY` (249); map data |
| Enter game | `CgEnterGame`; `on_enter_game`; `CharacterAdd`; `GcTime`; `GcChannel` | Game-phase dispatch; world placement; `CHAT` (4) |
| Logout and save | The cache core, unwired | `GD_LOGOUT`; save; removing the logon record |

## Test infrastructure gaps

- No SQL schema (DDL) and no seed data exist in the repository.
- No MySQL/MariaDB runs in the agent container. Use a server on the host or in a Podman container,
  reached through `host.docker.internal` (see `AGENTS.md`, runtime environment).
- No packet captures, no scripted client, and no `sqlx::test` tests. The five `db-server` process tests prove
  both live answers against an **unreachable** SQL endpoint, which is why they pass without a database.
- Recommended scripted client: the existing `protocol` codecs plus `DescriptorCrypto` in the client
  key role, driving a single-map fixture with one seeded account.

## Milestones

Each milestone should land as its own ledger section, with golden bytes and a receipt.

| milestone | scope | acceptance |
|---|---|---|
| M1 | **Done (config half, section 169):** TOML configuration for both binaries, `SqlConfig::connection_url`, strict rejection of malformed and unknown keys. **Open (database half):** a MariaDB instance reachable from the container, minimal DDL for `account`, `player_index`, `player`, `item`, `quest`, and `affect`, one seeded account with one character, and a `DATABASE_URL`-gated test. | Both binaries start from a TOML file and reject a malformed one. **Met.** A test gated on `DATABASE_URL` connects and reads the seed. **Not met: no database is reachable from the container.** |
| M2 | **Done (section 170):** `db-server` opens **lazy** SQL pools, classifies a connection's transport identity through a fail-closed allowlist, and authorizes every request through `DbPeerPolicy`. **Done (reply half, section 171):** the live process builds a real `DbRequestService` and answers an ordinary game `SETUP` with one `HEADER_DG_MAP_LOCATIONS` frame, and the auth-mode request with no bytes at all. **Done (loader, sections 174 and the boot-loader work):** `BootTableLoader` reads the 11 always-present sections plus the profile-selected `renewal_shop`, `event`, and `premium_market_price` sections from `SQL_PLAYER` into `BootTableCache`, and the GM tail is resolved per request from `SQL_COMMON`. **Open:** only the requesting peer's own location is listed in SETUP, because there is no connected-peer registry. | **Met.** A test spawns the binary, sends `BOOT`, and parses the reply with `protocol::db_boot`. Ledger 170.8 also records a live run. Ledger 171.9 adds the map-locations reply, the silent auth branch, and the unlisted-peer refusal. Ledger 174 adds the per-request GM tail with 17 killed mutations, including the fail-open case. |
| M3 | **Done (sections 172, 173, and 174):** `GameDbClient` sends `GD_BOOT` then `GD_SETUP` at handle 0 with the legacy three-second retry window, validates the real boot reply, records the real `DG_MAP_LOCATIONS` reply, and drives a `BootReadyGate`. `db_client_live.rs` owns the socket and `main.rs` consults the gate, so a running `game-server` really connects and really refuses clients before boot. The shared setup record moved to `protocol/src/db_setup.rs`. **Open:** nothing on the client-facing path. | **Met.** `db-server/tests/game_server_db_gate.rs` starts both real binaries and walks the whole sequence: clients refused with no DB, clients served once the real DB boots, clients refused again after the DB is killed, and the game server still running. |
| M4 | Live descriptor: `GC_HANDSHAKE`, the 13-byte echo, a plaintext `PHASE` and then TEA with the default key, per-phase framing, and close on an unknown header. | Encrypted PING/PONG works, including with fragmented input. |
| M5 | Auth, with the legacy split (owner decision 1): measure `GD_AUTH_LOGIN`/`DG_AUTH_LOGIN` with the i686 probe. `LOGIN3`, then the `account` lookup in the game server with bound-parameter SQL, then `GD_AUTH_LOGIN`. The login-data registry lives in `db-server`. Finish with `0x96`. | Golden bytes; a wrong password returns `WRONGPWD`; `db-server` runs no `account` query. |
| M6 | Login by key: key switch, validations, and summaries; `EMPIRE`, the 357-byte record, then `PHASE(SELECT)`. | The seeded character name decodes; a key mismatch and a replayed key both fail. |
| M7 | `DG_ITEM_LOAD` and `TPlayerItem`; composite DB frames. | Select indices 4-255 fail without a panic. |
| M8 | Loading: build a character from `TPlayerTable`; send `PHASE(LOADING)`, then 15, 16, 76, 28, and 21. | A golden GC sequence. |
| M9 | Enter game: `PHASE(GAME)`, then 1, 136, 19, 106, and 121. | The client stays in GAME for N pings, and MOVE works. |
| M10 | `GD_LOGOUT` and removal of the logon record. | Two sequential logins work; a concurrent login receives `ALREADY`. |
| M11 | Plaintext transcripts as test fixtures; a Windows client smoke test; optionally, a diff against the C++ server in an i686 container. | Transcripts match per header and length. |

## Legacy flow references

`G/` is `server/server/game/` and `D/` is `server/server/db/`.

- **Start-up.** The game accepts clients before `DG_BOOT` arrives (`G/main.cpp:671-697`,
  `G/desc_manager.cpp:150-198`). It sends `GD_BOOT` (9) and `GD_SETUP` (0xff)
  (`G/desc_client.cpp:126-226`), handled at `D/ClientManager.cpp:436-642` and `:1284`.
  `CInputDB::Boot` is at `G/input_db.cpp:459-990`. The game also runs its own SQL
  (`G/config.cpp:568-641`).
- **Handshake.** The default key is `1234abcd5678efgh` (`G/desc.cpp:225-248`). `PHASE` (253) is
  written before TEA is enabled (`G/desc.cpp:494-540`). The echo is at `G/input.cpp:130-157`, and the
  key switch is at `G/input_login.cpp:208`.
- **Auth.** `LOGIN3` (66 bytes) is handled at `G/input_auth.cpp:67-195`, with SQL at `:170-190`.
  `GD_AUTH_LOGIN` is handled at `D/ClientManager.cpp:1998-2050`. `DG_AUTH_LOGIN` returns at
  `G/input_db.cpp:1706-1737`, which sends `0x96`. The client then reconnects.
- **Login by key.** `LOGIN2` is handled at `G/input_login.cpp:153-219` and sent on as
  `GD_LOGIN_BY_KEY` (101). The DB checks are at `D/ClientManagerLogin.cpp:82-150`.
  `DG_LOGIN_SUCCESS` (30) returns at `G/input_db.cpp:109-213`, which sends `EMPIRE` (90), the
  357-byte `0x20` record, and `PHASE(SELECT)`.
- **Select.** `G/input_login.cpp:265-304` sends `GD_PLAYER_LOAD` (3). The reply is DG 35 plus the
  item, affect, and quest loads. `G/input_db.cpp:328-457` sends `PHASE(LOADING)`, then 249, 15, 28,
  16, and 76, then the items (21) via `:1451`.
- **Enter game.** `G/input_login.cpp:562` onward sends 1, 136, and 19 (`G/char.cpp:1060-1293`),
  `PHASE(GAME)`, 106, 121, and `CHAT` (4).

## Legacy defects not to reproduce

- The select index is used before its bound check (`G/input_login.cpp:281` before `:288`). The
  create path forwards an unchecked index and uses `strncpy`.
- Auth SQL is built by string formatting (`G/input_auth.cpp:133-152`). Use bound parameters.
- The proxy `lAddr` is written after the `memcpy` that sends it (`G/desc.cpp:874-907`).
- Clients are accepted before boot completes (`G/main.cpp:691-695`).
- The boot payload size is never checked (`G/input_db.cpp:470-474`), and `bSentBoot` is a
  function-level `static`.
- The auth-peer designation is unauthenticated (`D/ClientManager.cpp:1289-1295`).
- `long alMaps` depends on the ABI. Encode it as an explicit 4-byte field.
- Never send the Panama packet (151).
- Closing on an unknown header is a deliberate divergence from legacy (ledger 160.5), not parity.
- The most serious recorded defects:
  - A use-after-free at `G/char_item.cpp:7438-7439` on every successful item destroy.
  - A 17-byte desync in `PrivateShopItemCheckin` (ledger 148).
  - An out-of-bounds write at `D/ClientManagerBoot.cpp:357-362`.
  - SQL injection through `TABLE_POSTFIX`.
  - The length wrap in `D/Peer.cpp`.
  - Three client Python wrappers that send uninitialized stack data.

## Allowed simplifications for the first slice

- Auth and channel may run as one process with two listeners.
- `DG_BOOT` table sections may start empty.
- P2P/GG, quests, social systems, `ENTITY`, and hybrid crypt may be skipped. Map locations are **not**
  skippable: `DG_MAP_LOCATIONS` is the SETUP answer and is implemented (section 171).

These must stay wire-compatible from the start: every CG size, every GC byte, the TEA boundaries,
the 357-byte and 362-byte records, and, if C++ peers must interoperate, the DB-peer framing and the
boot layout.

## Owner decisions

Decided by the project owner on 2026-09-26. Follow them; do not reopen them without the owner.

1. **The auth account lookup keeps the legacy split.** The game server in auth mode runs the
   `account` query on its own SQL connection, exactly as legacy does: `DBManager` connects with the
   `PLAYER_SQL` settings (`G/config.cpp:617`), `G/input_auth.cpp:135-190` issues the query, and
   `G/db.cpp:246-457` handles the result. Only then does it send `GD_AUTH_LOGIN` to the DB server
   (`G/db.cpp:191-238`). `db-server` does not look up accounts. It only registers the login data
   (`InsertLoginData`) and answers `DG_AUTH_LOGIN` (`D/ClientManager.cpp:1998-2050`). This keeps
   a Rust `db-server` usable by an unmodified C++ game server. The query itself must use bound
   parameters (see the legacy defects above).
2. **Configuration moves to TOML.** **Implemented in section 169.** Both binaries read TOML files.
   The `key=value` parsers in `common/src/config.rs` are replaced with `serde` plus the `toml`
   crate, and the legacy `CONFIG` and `conf.txt` files are not read. `toml 0.8.23` was added to
   `Cargo.lock` with one online `cargo fetch` on 2026-09-26; every gate afterwards ran `--locked
   --offline`. The legacy-key to TOML-key mapping is in sections 169.5 and 169.6, and the
   `README.md` run commands now use `game.toml` and `db.toml`. Defaults are `game.toml` and
   `db.toml`; commented examples are in `config/`.

   Two behaviours the owner should know about, both stricter than the old parser: an **unknown key
   is an error** (the old parser ignored unknown keys), and a boolean must be `true`/`false` (the
   old parser also accepted `1` and `t`). A single legacy scalar line such as `mother_port=50080`
   is still valid TOML and will be read; a whole legacy `CONFIG` is rejected.

## Code-quality backlog

Recommendations only. None of these block the slice. Take them on when a milestone touches the
code.

- **Duplication.** 62 error enums, 42 `check_exact` helpers, and 37 little-endian readers. `TItemPos`
  is defined 4 times, and `TSimplePlayer`, `TQuickslot`, and `TPlayerSkill` 3 times each. There are
  4 phase enums and a duplicate `ServerState`. A shared `wire` reader/writer, one `CodecError`, a
  `FixedRecord` trait, and a `CgPacket` enum for dispatch would remove most of it.
- **`common/src/tables.rs`** declares 72 `repr(C, packed)` structs. Do not size wire records from
  them.
- **Unused dependencies:** `bytemuck`, `serde`, `mlua`, and `ring`. The TOML configuration will
  put `serde` to use in `common`.
- **Dead code:** `net/src/buffer.rs` (960 lines).
- **Silent data loss:** `bytes_to_str` (`protocol/src/lib.rs:629`) returns `""` on invalid UTF-8.
  Legacy names are raw bytes, so this hides data rather than rejecting it.
- **Panic points in non-test code.** `common/src/logging.rs:59` panics at runtime if the log
  directory cannot be created; return an error instead. The rest are invariant panics that cannot
  fire today but would if a refactor broke the invariant: 8 `.expect` calls in
  `protocol/src/cg_account.rs`, and one each at `protocol/src/cg_attack.rs:134`,
  `db-server/src/shop.rs:1158`, `db-server/src/peer_policy.rs:365`, and
  `game-server/src/sync_position.rs:177`. Section 162 removed the same pattern from `CgHack` with a
  pre-sized array and `copy_from_slice`.
- **Broken tool:** `tools/packet_compare` needs `clap` and imports the deleted `protocol::cg`.
- **Toolchain:** Rust is not pinned. Consider a `rust-toolchain.toml` for 1.85.1.
- **Unverified widths:** the `gg_inventory` widths are hand-summed; re-measure them with the i686
  probe.
- **Version control:** almost the whole workspace is untracked, including `Cargo.lock`. Committing
  it is the owner's decision.
