# Rewrite status

Last reviewed: 2026-09-26, after the planning decisions recorded in `docs/adr/` (ADR-0001 to
ADR-0004) and ledger section 176.

This page records where the Rewrite stands, the build order, and the next step. Rules live in
`AGENTS.md`, terms in `CONTEXT.md`, and the change history in `docs/REWRITE_LEDGER.md`. When this page
and the ledger disagree, the most recent ledger section wins; update this page in the same change.

## Summary

**No client can log in yet.** The workspace has broad, well-tested client wire codecs (CG 91 of 92,
GC 96 of 134), TEA, and transport-free descriptor reducers. It also has a two-process layout, a
`game-server` and a `db-server` talking the legacy DB-peer protocol over MySQL adapters, which the
owner's decisions of 2026-09-26 retire. Ledger sections 1-175 record how that code was built; they
stay as history.

The direction since then:

- Only the client protocol and the Game data file formats are a contract (ADR-0001).
- One `prodomo` process runs auth, Channels 1-4, and the Shared Channel on one game thread at 25
  Pulses per second (ADR-0002).
- The store is a fresh PostgreSQL 18 database (ADR-0003).
- Quests run on Lua 5.1 through `mlua`, compiled by a Rust port of `qc` (ADR-0004).

## Build order

Each step lands as one or more ledger sections with a gate receipt.

| step | scope | state |
|---|---|---|
| 1. Restructure | Retire the DB-peer and GG code and move the pure rule modules into `gamedata`. Rename `game-server` to the single `prodomo` binary with TOML configuration for the auth and Channel listeners, the Channel map sets, and PostgreSQL. First PostgreSQL schema and migrations. The Operator command that creates accounts and GMs. | **Next.** |
| 2. Parity inventory | Every legacy system and handler, listed from the source in `.scratch/parity/`, each with a porting status. The scripted-client test crate. | Not started. |
| 3. Vertical slice | Handshake and TEA, auth (`LOGIN3`), login by key, character select, create, and delete, loading, entering the game, movement and chat, a Warp between maps, and logout with save. | Not started. Codecs and reducers exist; see below. |
| 4. Game systems | In dependency order: items and inventory; NPCs, shops, and Transfers (trade, safebox); the quest runtime (`qc` port and Lua 5.1, with its API growing as each later system lands); monsters, combat, drops, and exp; skills and affects; party, guild, messenger, and the cross-Channel bus; dungeons, events, guild war, and OX; the Prodomo custom systems (sash, aura, pets, battle pass, switchbot, item shop, premium shop, and the rest); GM commands, the adminpage, and logs. | Not started. |
| 5. Game data and play test | Finish the importers, fix what the full data set breaks, then the owner's play test with the Reference client. | Not started. |

The Game data arrived before step 1 (`legacy/`), so each system's reader or importer is built with
the system that first needs it, starting with the maps and protos in the vertical slice. Step 5 is
what is left over.

Quests come early in step 4 because NPC clicks, level-up rewards, and skill selection all go
through them.

### Acceptance

A Parity inventory item is ported only when its scripted-client scenario passes. The scripted client
is a test-only crate that drives the real `prodomo` binary over TCP, using the `protocol` codecs and
`DescriptorCrypto` in the client-key role. Each scenario asserts an expected server-to-client
sequence taken from the legacy source. Transfers and cross-Channel features get scenarios with
several clients.

### Play test setup

The owner runs the play test on a Linux machine or VM with a `compose.yaml` (Podman or Docker) that
starts PostgreSQL 18 and `prodomo`. The README will cover creating an account, making it a GM, and
matching the auth and Channel ports to the Reference client's `serverinfo`. Ports default to the
legacy ones.

## Topology

Taken from `legacy/config` (see `legacy/README.md` for the full table):

| Channel | legacy ports | notes |
|---|---|---|
| auth | 30001 | Not a Channel. |
| 1 | 30003, 30005 | Hosts 8 maps that Channels 2-4 lack. |
| 2, 3, 4 | 30007/30009, 30011/30013, 30015/30017 | |
| 99 (Shared Channel) | 30019 | 29 maps that exist once for everyone. Never picked at login. |

In legacy each port belongs to one Core, and a Warp to a map on the other Core of the same Channel
reconnects the client to that Core's port. The Rewrite has no Cores: each Channel listens on all of
its ports, and any of them admits the Channel's players.

## What exists and what happens to it

| code | today | in the Rewrite |
|---|---|---|
| `protocol` CG and GC codecs, TEA, inventories | 91 of 92 CG and 96 of 134 GC records, golden-byte tested. Most CG codecs have no caller. | Kept. |
| `protocol` `db_*` and `gg*` modules | DB-peer records, the boot stream, setup, map locations, and 4 of 36 GG records. | Retired in step 1. |
| `net` | Client framing and DB-peer transport, plus the unused `buffer.rs`. | Client framing kept; the rest retired. |
| `db` | SQLx over MySQL, never run against a real server. | Replaced by the PostgreSQL store. |
| `world` | Spatial model, characters, events, and some combat rules. Never imported by a binary. | Kept; grows with step 4. |
| `quest` | An 8-line scaffold. | Replaced by the `qc` port and the Lua 5.1 runtime. |
| `game-server` | Binary that accepts clients but answers only keepalive and pong. Reducers: `ClientLifecycle`, handshake, heartbeat, `DescriptorCrypto`, `client_live`, `AccountPlayerSession` and its router, `sync_position`. A game-side DB client (`GameDbClient`, `DbLink`) with a boot gate. | Renamed `prodomo`. The reducers are kept; `AccountPlayerSession` is re-based on store results. The DB client and its gate are retired; the new gate is "the world has loaded its Game data". |
| `db-server` | Binary that answers the DB-peer `BOOT` and `SETUP` requests from MySQL. Peer policy, peer trust, boot composition, caches, and one SQLx adapter per boot table. | Retired. The pure rule modules (GM list rules, item-ID ranges, and the event, shop, item_attr, banword, refine, skill, land, object, and proto row rules) move to `gamedata` where they still apply. |
| `tools/packet_compare` | Does not build: `clap` is missing and it imports the deleted `protocol::cg`. | Deleted. |

## Vertical slice: what exists

| stage | present in Rust | missing |
|---|---|---|
| Handshake and TEA | `ClientLifecycle`, `handshake`, `handshake_dispatch`, `DescriptorCrypto`, `client_live` | A listener that runs them; scripted-client coverage. |
| Auth | `CgLogin3` (66 bytes), `GcLoginFailure` | Account lookup in PostgreSQL with argon2id; the login-key registry; `0x96`. |
| Login by key | `CgLoginByKey`, `AccountPlayerSession::on_login*`, `GcEmpire`, the 357-byte `GcLoginSuccess` | The login checks from `D/ClientManagerLogin.cpp:82-150`; the player summaries from the store. |
| Select, create, delete | `on_select`; the create and delete CG codecs | Player and item tables; name rules; the create defaults. |
| Loading | Loading-phase GC records 15, 16, 76, and 28-30; `gc_actors` | `ITEM_SET2` (21), `ENTITY` (249), map data from `legacy/gamedata`. |
| Enter game, movement, chat | `CgEnterGame`, `on_enter_game`, `CharacterAdd`, `GcTime`, `GcChannel`, `sync_position`, the move codecs | Game-phase dispatch; world placement; view range; `CHAT` (4). |
| Warp | The GC warp record | Map-to-Channel routing; the reconnect. |
| Logout and save | Nothing live | Background save; the logon record. |

## Legacy flow references

`G/` is `server/server/game/` and `D/` is `server/server/db/`.

- **Handshake.** The default key is `1234abcd5678efgh` (`G/desc.cpp:225-248`). `PHASE` (253) is
  written before TEA is enabled (`G/desc.cpp:494-540`). The echo is at `G/input.cpp:130-157`, and the
  key switch is at `G/input_login.cpp:208`.
- **Auth.** `LOGIN3` (66 bytes) is handled at `G/input_auth.cpp:67-195`, with SQL at `:170-190`. The
  login data is registered at `D/ClientManager.cpp:1998-2050`, and `G/input_db.cpp:1706-1737` sends
  `0x96`. The client then reconnects to a Channel port.
- **Login by key.** `LOGIN2` is handled at `G/input_login.cpp:153-219`. The login checks are at
  `D/ClientManagerLogin.cpp:82-150`. `G/input_db.cpp:109-213` sends `EMPIRE` (90), the 357-byte
  `0x20` record, and `PHASE(SELECT)`.
- **Select.** `G/input_login.cpp:265-304`. `G/input_db.cpp:328-457` sends `PHASE(LOADING)`, then
  249, 15, 28, 16, and 76, then the items (21) via `:1451`.
- **Enter game.** `G/input_login.cpp:562` onward sends 1, 136, and 19 (`G/char.cpp:1060-1293`),
  `PHASE(GAME)`, 106, 121, and `CHAT` (4).
- **What is loaded at start-up.** `D/ClientManagerBoot.cpp` lists every table the legacy DB loaded,
  and `G/input_db.cpp:459-990` shows how the game consumed them. `PROTO_FROM_DB = 0` in
  `legacy/config/db/conf.txt`, so the protos come from the text files.

## Divergences decided so far

| Divergence | legacy behaviour |
|---|---|
| Leaving a Shared Channel map returns the player to the Channel they came from. | Channel 1, because channel 99 learns only the Channel 1 and 99 map locations (`D/ClientManager.cpp:1298-1400`). |
| Every Transfer commits in one transaction when it happens. | The DB cache flushes on a timer; a crash can lose or duplicate items. |
| Passwords are argon2id. | MySQL `PASSWORD()` (`G/input_auth.cpp:125-127`). |
| The adminpage has no default password. | `SHOWMETHEMONEY` (`G/config.cpp:110`). |
| Clients are refused until the world has loaded its Game data. | Clients are accepted before the DB boot completes (`G/main.cpp:691-695`). |
| An unknown or malformed client frame closes the descriptor. | Logged and consumed, or ignored (ledger 160.5). |

## Legacy defects not to reproduce

- The select index is used before its bound check (`G/input_login.cpp:281` before `:288`). The
  create path forwards an unchecked index and uses `strncpy`.
- Auth SQL is built by string formatting (`G/input_auth.cpp:133-152`), and `TABLE_POSTFIX` is
  interpolated into SQL.
- The proxy `lAddr` is written after the `memcpy` that sends it (`G/desc.cpp:874-907`).
- A use-after-free at `G/char_item.cpp:7438-7439` on every successful item destroy.
- A 17-byte desync in `PrivateShopItemCheckin` (ledger 148).
- An out-of-bounds write at `D/ClientManagerBoot.cpp:357-362`.
- Never send the Panama packet (151).
- Three client Python wrappers send uninitialized stack data; the server must not trust those
  bytes.

## Environment

- PostgreSQL 18 runs in Podman (`postgres:18`), reached through `host.docker.internal`. The image is
  not pulled yet; the owner approved one pull (planning Q23).
- The owner approved one online `cargo fetch` for the PostgreSQL, `uuid`, `argon2`, and Lua 5.1
  dependencies (planning Q23). On the current machine the offline cache also lacks
  `tracing-appender`, so the offline gates cannot build until that fetch runs.
- `i686-linux-gnu-g++-12`, used by the width probe, is not installed on the current machine.

## Code-quality backlog

Take these on when a step touches the code.

- **Duplication.** 62 error enums, 42 `check_exact` helpers, and 37 little-endian readers. `TItemPos`
  is defined 4 times, and `TSimplePlayer`, `TQuickslot`, and `TPlayerSkill` 3 times each. There are
  4 phase enums and a duplicate `ServerState`. A shared `wire` reader and writer, one `CodecError`, a
  `FixedRecord` trait, and a `CgPacket` enum for dispatch would remove most of it. Retiring the
  DB-peer code removes some of these counts.
- **`common/src/tables.rs`** declares 72 `repr(C, packed)` structs. Do not size wire records from
  them.
- **Unused dependencies:** `bytemuck` and `ring`.
- **Silent data loss:** `bytes_to_str` (`protocol/src/lib.rs:629`) returns `""` on invalid UTF-8.
  Legacy names are raw bytes, so this hides data rather than rejecting it.
- **Panic points in non-test code.** `common/src/logging.rs:59` panics if the log directory cannot be
  created. The rest are invariant panics that cannot fire today: 8 `.expect` calls in
  `protocol/src/cg_account.rs`, and one each at `protocol/src/cg_attack.rs:134` and
  `game-server/src/sync_position.rs:177`.
- **Toolchain:** Rust is not pinned. Consider a `rust-toolchain.toml` for 1.85.1.
