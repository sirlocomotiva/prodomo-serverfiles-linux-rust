# Rewrite status

Last reviewed: 2026-09-26, after ledger section 181 (the Parity inventory and the scripted client).
Steps 1 and 2 are done; step 3 is next.

This page records where the Rewrite stands, the build order, and the next step. Rules live in
`AGENTS.md`, terms in `CONTEXT.md`, and the change history in `docs/REWRITE_LEDGER.md`. When this page
and the ledger disagree, the most recent ledger section wins; update this page in the same change.

## Summary

**No client can log in yet.** The workspace has broad, well-tested client wire codecs (CG 91 of 92,
GC 96 of 134), TEA, and transport-free descriptor reducers. The two-process layout (a `game-server`
and a `db-server` talking the legacy DB-peer protocol over MySQL) was retired in ledger section 177;
sections 1-175 record how it was built and stay as history. Since section 178 the one `prodomo`
binary reads `prodomo.toml` and binds the auth listener and every Channel port. Since section 179 it migrates a PostgreSQL 18 store before admitting
anyone, and `prodomo account` and `prodomo gm` create accounts, set passwords, change Coins and
Cash, and grant GM authority. Since section 182 every connection gets the legacy handshake, TEA,
time sync, and the ping cycle.

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
| 1. Restructure | Retire the DB-peer and GG code and move the pure rule modules into `gamedata`. Rename `game-server` to the single `prodomo` binary with TOML configuration for the auth and Channel listeners, the Channel map sets, and PostgreSQL. First PostgreSQL schema and migrations. The Operator command that creates accounts and GMs. | **Done.** Retirement and `gamedata` (177). The rename, the TOML document, and the listeners (178). The account and GM schema, store readiness, and the Operator commands (179). |
| 2. Parity inventory | Every legacy system and handler, listed from the source in `.scratch/parity/`, each with a porting status. The scripted-client test crate. | **Done** (181). 1,643 rows in nine tables; `.scratch/parity/spec.md` has the statuses, the regeneration command, and four findings for the owner. The scripted client is the `parity` crate; its scenarios are `prodomo/tests/parity.rs`. Two rows are `ported` (the keepalive and unknown-header framing rules). |
| 3. Vertical slice | Handshake and TEA, auth (`LOGIN3`), login by key, character select, create, and delete, loading, entering the game, movement and chat, a Warp between maps, and logout with save. | **In progress.** The handshake, TEA, time sync, and the ping cycle are live (182), and so are the Channel status list (183) auth `LOGIN3` with its login keys (184), and the Channel login by key (`LOGIN2`) with the character list (185). Next: character select, create, and delete. |
| 4. Game systems | In dependency order: items and inventory; NPCs, shops, and Transfers (trade, safebox); the quest runtime (`qc` port and Lua 5.1, with its API growing as each later system lands); monsters, combat, drops, and exp; skills and affects; party, guild, messenger, and the cross-Channel bus; dungeons, events, guild war, and OX; the Prodomo custom systems (sash, aura, pets, battle pass, switchbot, item shop, premium shop, and the rest); GM commands, the adminpage, and logs. | Not started. |
| 5. Game data and play test | Finish the importers, fix what the full data set breaks, then the owner's play test with the Reference client. | Not started. |

The Game data arrived before step 1 (`legacy/`), so each system's reader or importer is built with
the system that first needs it, starting with the maps and protos in the vertical slice. Step 5 is
what is left over.

Quests come early in step 4 because NPC clicks, level-up rewards, and skill selection all go
through them.

### Acceptance

A Parity inventory item is ported only when its scripted-client scenario passes. The scripted client
is the test-only `parity` crate (181): `parity::Server` starts the real `prodomo` binary on ports the
operating system picks, and `parity::Client` plays raw client bytes over TCP, built with the
`protocol` codecs (and, from step 3, `DescriptorCrypto` in the client-key role). The scenarios are
the tests in `prodomo/tests/parity.rs`, because only a `prodomo` test is told where the binary is;
`inventory_rows_keep_the_rules` checks that every `ported` row names one of them. Each scenario asserts an expected server-to-client
sequence taken from the legacy source. Transfers and cross-Channel features get scenarios with
several clients.

### Play test setup

The owner runs the play test on a Linux machine or VM with a `compose.yaml` (Podman or Docker) that
starts PostgreSQL 18 and `prodomo`. The README covers creating an account and making it a GM
(179); it will also cover matching the auth and Channel ports to the Reference client's
`serverinfo`. Ports default to the legacy ones.

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

`config/prodomo.toml.example` holds this topology with each Channel's legacy map set. The binary
refuses a topology it cannot run before binding anything (ledger 178.3). Two legacy settings the
owner should know about (178.5):

- `test_server` defaults to on in legacy and none of the owner's `CONFIG` files turns it off, so the
  deployment ran in test-server mode. The example keeps it on for Parity. One of its effects: legacy
  gives **every** character IMPLEMENTOR authority while it is on (`G/gm.cpp:53`), so on the owner's
  deployment every player was a GM. The GM grants of section 179 do not reproduce that yet; the
  owner decides when `test_server` is audited.
- `BLOCK_LOGIN` refuses accounts created on or after its date. Its legacy default is `30000705`; an
  empty value would refuse every account.

## What exists and what happens to it

| code | today | in the Rewrite |
|---|---|---|
| `protocol` CG and GC codecs, TEA, inventories | 91 of 92 CG and 96 of 134 GC records, golden-byte tested. Most CG codecs have no caller. | Kept. |
| `protocol` `db_*` and `gg*` modules | Deleted in 177. `TSimplePlayer` moved to `protocol::simple_player`. | Done. |
| `net` | Client framing. `buffer.rs` and the DB-peer transport were deleted in 177. | Kept. |
| `db` | `store` (the PostgreSQL pool, the embedded migrations, and the transient-error rule), `credentials` (logins, argon2id passwords, delete codes), `accounts` (accounts, Coins and Cash, GM grants), and `item_id_range`. Tested against PostgreSQL 18.6 when `DATABASE_URL` is set. | Grows with each system's tables. |
| `gamedata` | Packed table record layouts and nine table rules (banword, event, item_attr, land, object_proto, refine, renewal_shop, shop, skill) as typed builders over caller-supplied rows. New in 177. | Readers and importers for `legacy/gamedata` are added with the system that first needs them. |
| `world` | Spatial model, characters, events, and some combat rules. Never imported by a binary. | Kept; grows with step 4. |
| `quest` | An 8-line scaffold. | Replaced by the `qc` port and the Lua 5.1 runtime. |
| `prodomo` | Renamed from `game-server` in 178. `prodomo serve` reads `prodomo.toml`, creates a lazy store, binds the auth listener and every Channel port (`listeners`), starts the game loop, migrates the store (retrying while it is unreachable), and only then opens `ReadyGate` (179). Every admitted connection runs `client_live` (182): the handshake on accept, TEA, time sync, keepalive, and the ping cycle. `prodomo account` and `prodomo gm` are the Operator commands (`operator`, 179). Reducers: `ClientLifecycle`, handshake, heartbeat, `DescriptorCrypto`, `client_live`, `AccountPlayerSession` and its router (re-based on the typed `account_records` in 177), `sync_position`. | `ReadyGate` will also wait for the loaded Game data once step 3 loads it. |
| `db-server` | Deleted in 177. GM list rules moved to `common::gm`, item-ID ranges to `db::item_id_range`, and the nine table rules to `gamedata`. The `item_proto` and `mob_proto` SQL decoders did not move, because the protos are read from the text files; the `object`, `market_price`, `monarch`, `player`, `player_index`, `quest`, and `login` modules did not move, because that state lives in the fresh store. | Done. |
| `tools/packet_compare` | Deleted in 177. | Done. |

## Vertical slice: what exists

| stage | present in Rust | missing |
|---|---|---|
| Handshake and TEA | Live on every connection with scenarios (182). The Channel status list (183). The login's `SetSecurityKey` (185). | The status list counts characters in game once entering the game publishes the count (`ChannelStatusBoard::set_online`). |
| Auth | Live with a scenario (184): every legacy check in order, `AUTH_SUCCESS` (0x96) with a login key, and `LOGIN_FAILURE`. `prodomo::auth_login` holds the rules and the login-key registry. | Premium times on the login data (no columns yet). |
| Login by key | Live with scenarios (185): `SHUTDOWN` under the handshake key, then the client key pair, the login-key judgement (`NOID`), the logon registry (`ALREADY` and the kick of a holder without a character), `GC_EMPIRE`, the 357-byte character list with each map's Channel address from the map atlas (`gamedata::map_atlas`), and `PHASE(SELECT)`. `prodomo::channel_login` holds the rules; `db::players` reads the characters. | `FULL` (unit-tested; the online count arrives with entering the game). The delayed kick of a holder in game. The blocked-country IP list (`is_blocked_country_ip`, `G/block_country.cpp`; empty tables behave as today). The mark-login table keyed by handle and random key (guild marks). Guild ids and names (zeros until guilds exist). |
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
| Clients are refused until the store's schema is migrated (179) and, from step 3, until the world has loaded its Game data. While the store is unreachable the ports stay bound and each connection is closed at once. | Clients are accepted before the DB boot completes (`G/main.cpp:691-695`). |
| Logins are 2 to 30 ASCII letters and digits, stored in lowercase. | `account.login` is `varchar(16)` (`legacy/sql/schema/account.sql`), although its column comment says `LOGIN_MAX_LEN=30`. |
| A GM grant is one character Name of one account. Names are unique regardless of case, and a Name held by another account must be revoked before it is granted again. | `gmlist` rows are looked up by exact Name in a `std::map` (`G/gm.cpp:55`); nothing stops two rows whose Names differ only in case. |
| The GM host check and its `gmhost`, `mContactIP`, and `mServerIP` columns are not ported. | Used only when `gm_host_check` is set (`G/gm.cpp:62-105`, `G/config.cpp:45` and `:1212`); none of the owner's `CONFIG` files sets it. |
| An unknown or malformed client frame closes the descriptor. | Logged and consumed, or ignored (ledger 160.5). |
| A store or hashing error during auth closes the connection (ledger 184). | The query failure is logged and the client waits with no answer. |
| An auth login key is drawn only once the login succeeds, and the panama and hybrid-crypt records are not sent (ledger 184). | The key is drawn before the query (`G/input_auth.cpp:113`); `SendPanamaList` and the crypt keys follow the success, from data absent from `legacy/`. |
| An `index` line in the map folder with a map index but no name stops the server at start-up (ledger 185). | `sscanf` leaves the name buffer as it was: the previous line's name, or uninitialised bytes on the first line (`G/sectree_manager.cpp:691-775`). |
| The Channel status list is computed when it is asked for, with the ports in ascending order (ledger 183). | Each Core reports to the DB server at boot and then every five minutes, so a status can be five minutes old; the list is in `unordered_map` order (`G/desc_client.cpp:292-313`, `D/ClientManager.cpp:4455-4466`). |

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
- A second `LOGIN3` on one auth descriptor with a different login leaves the first login marked as
  connected (`ConnectAccount` overwrites the descriptor's login without releasing the first).
- A use-after-free on every successful auth: `G/db.cpp:456-457` logs `pinfo->login` after
  `M2_DELETE(pinfo)`.
- `QUERY_LOGIN_BY_KEY` checks for a logged-on account (`ALREADY`) before it compares the login and the
  client key, and the game then kicks the descriptor holding the login the client *typed*
  (`D/ClientManagerLogin.cpp:82-149`, `G/input_db.cpp` `LoginAlready`). Any live login key could
  disconnect any logged-on account by name. The Rewrite judges the key first (ledger 185).
- The DB server's player-cache branch of `CreateAccountPlayerDataFromRes` forces `bChangeName = 0`,
  so a pending forced rename disappears from the list once the character is cached. The Rewrite
  always sends the stored flag (ledger 185).
- Three client Python wrappers send uninitialized stack data; the server must not trust those
  bytes.

## Environment

- PostgreSQL 18 runs in Podman (`postgres:18`, PostgreSQL 18.6), pulled once in section 179 under
  the owner's approval (planning Q23). On the current machine `host.docker.internal` does not
  resolve, so the container publishes its port on `127.0.0.1`; `AGENTS.md` has the commands.
- The one online `cargo fetch` the owner approved (planning Q23) ran in section 177. The offline
  gates build from the local cache.
- `rustfmt` 1.8.0 and Clippy 0.1.85 were installed by the owner after section 179; section 180
  applied `cargo fmt` to the drift that built up while they were missing and cleared every Clippy
  finding. Both gates run again.
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
- **Silent data loss:** `bytes_to_str` (`protocol/src/lib.rs:624`) returns `""` on invalid UTF-8.
  Legacy names are raw bytes, so this hides data rather than rejecting it.
- **Panic points in non-test code.** `common/src/logging.rs:64` panics if the log directory cannot be
  created. The rest are invariant panics that cannot fire today: 8 `.expect` calls in
  `protocol/src/cg_account.rs`, and one each at `protocol/src/cg_attack.rs:134` and
  `prodomo/src/sync_position.rs:177`.
- **Toolchain:** Rust is not pinned. Consider a `rust-toolchain.toml` for 1.85.1.
