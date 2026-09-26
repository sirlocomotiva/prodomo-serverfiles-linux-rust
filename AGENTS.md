# PROJECT KNOWLEDGE BASE

## OVERVIEW

This repository is an active Linux-native Rust rewrite of the Metin2 game and database servers. The legacy Windows client and C++ server remain the behavioral, data-layout, and wire-format oracle until parity is demonstrated.

Do not use old plan checkboxes as proof that a subsystem is complete. Do not call a subsystem complete because it compiles or has a type with the right name.

This file holds **rules**. History, per-record findings, and receipts live elsewhere:

| document | what it is for |
|---|---|
| `docs/STATUS.md` | Where the rewrite stands, what is wired and what is not, and the next milestones. Read it first. |
| `docs/PROTOCOL_NOTES.md` | Per-module and per-record findings, legacy defects, and the verification lessons behind the rules below. Read the relevant section before touching a codec. |
| `docs/REWRITE_LEDGER.md` | Authoritative, append-only record of every change and receipt (sections 1-171). Append a new section for each change. |

## AGENT RUNTIME ENVIRONMENT

The agent and repository tools run inside a container. The agent may launch other containers with Podman when an isolated environment is useful. Ports exposed by those containers are reachable from the agent container through `host.docker.internal`; do not assume another container listens on the agent container's localhost.

No MySQL/MariaDB server runs inside the agent container. Use one on the host or in a Podman container, reached through `host.docker.internal` (see `docs/STATUS.md`).

## ACTIVE RUST WORKSPACE

Eight crates. The status column is honest: **no crate is wired end to end yet.**

| crate | contents | live status |
|---|---|---|
| `protocol/` | Transport-free CG, GC, GG, and DB-peer record codecs, the CG/GC/GG inventories, TEA, and `db_setup` (the shared `TPacketGDSetup`/`TPacketLoginOnSetup` record, its field offsets, and the game-to-DB encoder). | Codecs only. Most CG codecs have no caller outside `protocol`. `db_setup` is used by `db-server` and by `game-server/src/db_client.rs`. |
| `common/` | TOML config, logging, enums, constants, legacy table/domain types. | `src/tables.rs` still uses `repr(C, packed)` structs; do not size wire records from them. |
| `net/` | Tokio transport helpers, DB-peer and fixed-client framing. | The generic length-prefixed `ReadBuffer` is **not** the legacy client framing. |
| `db/` | SQLx/MySQL pool and bounded, retry-preserving query streaming. | Never exercised against a real server. |
| `world/` | Spatial model, characters, events, bounded combat/monster rules. | `game-server` lists it as a dependency but never imports it. |
| `quest/` | Quest/Lua scaffold. | Not a working legacy quest runtime; also a dependency of `game-server` that is never imported. |
| `game-server/` | Binary plus transport-free reducers: handshake, heartbeat, SyncPosition, `ClientLifecycle`, `AccountPlayerSession`, `DescriptorCrypto`, the game-side DB client `GameDbClient` with its `BootReadyGate`, and the live socket adapter `DbLink`. | The binary accepts sockets but only recognises keepalive and pong. It sends no handshake and has a no-op game loop. It **does** open a DB socket: `DbLink` connects on the legacy three-second window, sends `GD_BOOT` then `GD_SETUP` at handle 0, validates the boot reply, records the map-location reply, and drives the gate, which refuses clients until boot and re-closes when the link drops. `db-server/tests/game_server_db_gate.rs` starts both real binaries and walks that sequence. It arms SIGTERM and SIGINT **before** binding the listener, so an open port implies a handled signal. |
| `db-server/` | Binary plus peer framing, boot composition, SQLx acquisition adapters, peer policy, peer transport-identity classification, item-ID range pool, cache core, `DbRequestService`, `BootTableLoader`, `BootTableCache`, `gm`/`gm_sqlx`, `protocol::db_map_locations`. | The binary builds a real `DbRequestService` and answers two requests. `BOOT` is composed **per request** from the live `BootTableCache`: `BootTableLoader` reads the 11 always-present sections plus the profile-selected `renewal_shop`, `event`, and `premium_market_price` sections from `SQL_PLAYER`, and the GM tail is read from `SQL_COMMON` inside each request because legacy filters the administrator list on the requesting peer's `szIP`. A missing common pool or an unloaded cache **refuses** the boot rather than serving empty sections. An ordinary game `SETUP` gets one `HEADER_DG_MAP_LOCATIONS` frame holding the requesting peer's own location. Every other request is authorized and then logged as unimplemented; none is answered. The setup reply does not list other connected peers, because there is no peer registry. Tick, drain, and shutdown are `TODO`. |

`tools/` is outside the workspace. `tools/packet_compare` cannot build: `clap` is missing from `Cargo.lock` and the registry cache, and it imports the deleted `protocol::cg`. `ported-client/` is an empty Godot stub.

## BUILD AND TEST

`Cargo.lock` is present but not tracked by Git. Run every gate locked and offline:

```bash
cargo fmt --all -- --check
cargo build --workspace --locked --offline
cargo test --workspace --all-targets --locked --offline --no-fail-fast
cargo test --workspace --doc --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline
```

`--all-targets` does **not** run doc-tests, so the `--doc` run is a separate gate. Fix Clippy findings by refactoring, never by `#[allow]`. `unsafe_code` is forbidden workspace-wide.

The workspace was last validated with Rust 1.85.1, which is not pinned by a toolchain file. The current receipt, re-run on 2026-09-26 after section 175, is **2,111 tests across 31 test binaries plus 6 doc-tests (2,117 total), 187 workspace Rust files, and 137,576 scanned lines**, with zero failures and all six gates green. Record a new receipt in the ledger whenever the numbers change.

## PROTOCOL COVERAGE

Key every coverage metric on the **wire byte**, never on an identifier name.

| direction | registered | implemented | missing |
|---|---|---|---|
| client to game | 92 | 91 | 1 (`KEY_AGREEMENT`, dead: `_IMPROVED_PACKET_ENCRYPTION_` is never defined) |
| game to client | 134 | 96 | 38 (19 fixed-size, 19 dynamic) |
| game to game | 36 | 4 | 32 |

- The game-to-client registration table is the **client's**, in `client/Client/UserInterface/PythonNetworkStream.cpp`. `server/server/game/packet_info.cpp` registers only inbound records and must never be used to measure game-to-client coverage.
- `protocol/src/cg_inventory.rs`, `gc_inventory.rs`, and `gg_inventory.rs` are the machine-readable tables. Each row's implemented flag is tested, so update the flag in the same change as the codec.
- Eight GC bytes are decoded by the client but never sent by the checked-in server: 15, 18, 72, 73, 84, 112, 117, and 213. Treat them as reachability questions, not porting work.
- The DB server consumes no GG record. GG framing is `[u8 header][body]` on a game-to-game socket; the DB peer framing is `[u8 header][u32 handle][u32 size]`. Never share the two tables.

## NEXT PRIORITY

Codec coverage is far ahead of integration. The recommended next work is the login-to-game **vertical slice** in `docs/STATUS.md` (milestones M1-M11). The DB-side answers for `BOOT` and `SETUP` exist (sections 170 and 171), M3 is done (sections 172-174: the game-side DB client, its live socket, the boot gate, and the DB-side table loader with its per-request GM tail). The immediate next step is a **live descriptor with TEA**, then auth, login, select, loading, enter game, and logout. Do not assume the boot tables are correct yet: the GM tail has been checked source-by-source, but the other profile-selected sections still need a source-by-source check against the legacy loader rather than a blanket assumption that "loaded" means "correct". Add a new codec only when a milestone needs it. Record each milestone as a ledger section with its receipt.

## OWNER DECISIONS

Decided by the project owner on 2026-09-26. Details and source citations are in `docs/STATUS.md` ("Owner decisions"). Do not reopen them without the owner.

- **Auth keeps the legacy split.** The game server in auth mode runs the `account` lookup on its own SQL connection, as legacy does, and then sends `GD_AUTH_LOGIN`. `db-server` only registers the login data and answers `DG_AUTH_LOGIN`; it never queries `account`. Use bound parameters for the lookup.
- **Configuration is TOML.** **Done, section 169.** Both binaries read TOML through `serde` and the `toml` crate; the `key=value` parsers in `common/src/config.rs` are gone and the legacy `CONFIG` and `conf.txt` files are not read. Defaults are `game.toml` and `db.toml`, with commented examples in `config/`. An unknown key is an error and a boolean must be `true`/`false`, both stricter than the old parser. The legacy-key-to-TOML-key mapping is in ledger sections 169.5-169.6.

## VERIFICATION AND SEARCH RULES

These rules exist because each one was broken at least once in this repository. `docs/PROTOCOL_NOTES.md` ("Verification lessons") has the incidents.

- **Every negative search needs a positive control and a negative control.** An absence claim without a control is not evidence. If the harness provides the `metin2-verify-absence-source-sweep` skill, use it before reporting an absence.
- **Search tooling traps.** If `grep` is a shell function or alias in your shell, call `command grep`. Pass `-a`, because GNU grep reports `server/server/common/VnumHelper.h` as a binary match and hides its lines. Never filter with `--include=*.h` alone: the client's feature header is `client/Client/UserInterface/LOCALE_INC.H`, with an uppercase extension. Use both `*.h` and `*.H`, or read files in code with `errors="replace"`.
- **Resolve a constant across every definition form**: `#define NAME n`, `const T NAME = n;`, a multiline anonymous-enum member, a single-line `enum { NAME = n };`, an enum member whose value is an expression, and an auto-incremented enum member with no `=`. Then check which `#ifdef` gates the use site sits behind.
- **Sweep the struct tag and the typedef alias as two separate searches** before calling a legacy struct unused. Call sites mostly spell the tag.
- **Search the whole legacy tree for struct definitions**, not only `packet.h`. Records live under other names and in other headers. Handle both the `} NAME;` typedef form and `struct NAME { ... };`, including classes with constructors.
- **Measure packed widths by compiling the verbatim struct body for i686**, never by summing fields. Use `i686-linux-gnu-g++-12 -static` under `#pragma pack(1)`. `gcc -m32` is not installable here, and a non-static 32-bit binary cannot run without `/lib/ld-linux.so.2`. Every probe run re-measures known widths and a packed two-`long` struct that must be 8 bytes. Keep a hand sum only as the value you check the probe against. The `gg_inventory` widths are still hand-summed and should be re-measured.
- **The legacy target is 32-bit x86** (`server/server/premake5.lua:12`). C++ `long`, `time_t`, and the `QWORD` typedef (`unsigned long`) are 4 bytes; `bool` is 1 byte. A 64-bit reading silently breaks framing.
- **Pair client and server records by byte value, never by name.** The trees name the same byte differently (`_OLD` on the server versus `_NEW` on the client, and many renames), and five client rows have no server enumerator at all. Keep both names in Rust as aliases rather than picking one.
- **Never share a header table between directions**, and never infer a width from the fact that two directions use the same number. Sort every cross-direction collision into one of the four buckets in `docs/PROTOCOL_NOTES.md` ("Cross-direction header collisions"). The client's `CNetworkPacketHeaderMap::Set` is last-wins; the server's `CPacketInfo::Set` is first-wins.
- **A round trip is not an independent witness.** It only proves that the encoder and decoder agree. Pin every record with golden bytes taken from the source field order. Use test values with distinct byte halves; `u16::MAX` is byte-symmetric and cannot catch an endianness bug.
- **Mutation sweeps:** apply each mutant to the pristine file, assert the text changed, and restore by checksum. Report compile-error kills separately from semantic kills. When a survivor is an equivalent mutant, test the class of real defect it hides.
- **A client-only array bound is not automatically a shared constant.** Record which tree defines it.

## COMPATIBILITY RULES

- The game-to-DB setup record lives in `protocol/src/db_setup.rs` because both
  binaries need it: the game server encodes it, the DB server decodes it. The
  decoder, the feature profile, the record cap, and the auth-mode base-only
  receive policy stay in `db-server/src/setup.rs`; they are receive policy, not
  wire layout. `encode_setup_payload` derives `dwLoginCount` from the record
  slice it is given and never from a caller-supplied field.
- `game-server::db_client` is transport-free and deliberately diverges from
  legacy in eight documented ways, recorded in ledger section 172.3 and
  `docs/PROTOCOL_NOTES.md`. The two that matter most: **the legacy tree has no
  boot-ready gate at all** (clients are accepted and logged in while the DB link
  is retrying), and **`static bool bSentBoot` means legacy never resends
  `GD_BOOT`**, so after a DB restart it serves stale tables and never re-requests
  the item-ID range. The Rust client resends both bootstrap frames on every
  connection and closes the link on an unknown DB header.
- `game-server::db_client` is transport-free policy; `game-server::db_client_live`
  is the only code that owns a socket. Keep protocol decisions in the first. The
  live adapter may add `DbClientError::Io` for a real cause, but it must not
  decide anything about framing, ordering, or acceptance.
- `DbLinkConfig` takes a host **name**, not a `SocketAddr`, because legacy
  `socket_connect` uses `gethostbyname` for any host not starting with a digit
  (`libthecore/socket.cpp:255-265`). An unusable `db_addr` is a warning, not a
  startup failure: the compiled-in default is the empty string with port 0, and
  legacy retries forever. The gate stays closed, so an unconfigured server
  refuses every client.
- `szPublicIP` is a NUL-terminated ASCII address, packed with
  `db_client::public_ip_field`, not four octets from `Ipv4Addr::octets`. The
  legacy `bind_ip` token is what writes `g_szPublicIP` (`config.cpp:1347-1350`),
  and the same string is both the client bind address and the setup field.
- `game-server` is a **dev-dependency** of `db-server` so that one test can drive
  both real binaries across the real socket. Neither shipped binary depends on
  the other. Do not promote that edge to a normal dependency.
- `HEADER_DG_BOOT` is 43 and `HEADER_DG_MAP_LOCATIONS` is `0xfe`. `0xfe` is also
  `HEADER_CG_PONG` and `HEADER_GC_BINDUDP`; these are three unrelated tables and
  the constants must never be substituted for each other. These values were
  measured by compiling the verbatim enum, not by summing.
- `ENABLE_ITEMSHOP` is defined in the checked-in `prodomodefines.h`, and the DB
  sends a `DG_ITEMSHOP` frame at the end of every boot. The Rust game client
  currently reports that header as unknown and closes the link, so the boot-ready
  divergence is only exercised against the empty-snapshot profile.


- Derive layouts and behavior from the legacy source, not from Rust type names.
- Legacy records use `#pragma pack(1)`. Use explicit field-by-field wire codecs with explicit little-endian conversion. Do not make Rust packed structs the compatibility representation, and never use a non-packed `#[repr(C)]` struct for sizing.
- Legacy client packets use a one-byte header plus a header-specific payload size, not a generic length prefix. Variable records use the sub-header and count rules in `protocol::cg_variable`.
- Legacy DB peer frames use a one-byte protocol header, a four-byte handle, and a four-byte payload length, all little-endian.
- A header-dependent frame adapter checks the **payload** width (the frame excludes the header byte); the record decoder checks the full width. Check the exact length before reading any field or the header.
- Fixed `char[N]` fields stay raw `[u8; N]`: no `&str`, no `CStr`, no NUL or UTF-8 requirement, unless a record's own note says otherwise (for example `GcLoginFailure`). A stricter codec would reject bytes the legacy server accepts.
- A byte the legacy handler never range-checks stays opaque, and all 256 values round-trip. Sentinels, handler `switch` arms, and gameplay bounds (slot counts, window types, VID validity) belong above the codec. Read the handler before deciding whether a byte is opaque or sub-typed.
- Never reproduce a legacy defect for "parity" and never silently compensate for one in a codec. Record it in `docs/PROTOCOL_NOTES.md` and the ledger. Where the rewrite deliberately diverges (for example closing on an unknown header), record it as a divergence, not as parity.
- Select a DB boot feature profile explicitly. Keep ordinary table records opaque until their feature-dependent widths are verified. `BootSnapshot` is caller-resolved and must not query SQL or infer rows. Response adapters frame already-resolved outcomes only.
- Validate `TABLE_POSTFIX` through `db-server::postfix` before interpolating it into SQL. Every other SQL value is a bound parameter, never string formatting.
- Add golden bytes and malformed and fragmented-input tests before enabling a live descriptor.

## GAME-SERVER ADAPTER CONTRACTS

- `game-server::ClientLifecycle` is transport-free. It preserves setup/heartbeat/handshake/post-handshake phase-transition effect order and treats close as control-only. A future adapter must retain the recorded previous output boundary when writing a phase packet: legacy `DESC::SetPhase` writes `GC_PHASE` before enabling TEA for Handshake-to-Login/Auth, while later encrypted phase transitions use the prior encrypted boundary. It must also own descriptor stream buffering, per-phase analyzers, socket I/O, key installation, and teardown; those integration paths are not modeled yet.
- `game-server::DescriptorCrypto` is a transport-free adapter for the active non-improved TEA branch. It starts in explicit plaintext mode, derives the source Myevan key and client/server pair without unsafe pointer arithmetic, buffers the largest aligned ciphertext prefix while retaining a 1-7-byte tail, and zero-pads each output unit. It does not own a socket, phase analyzer, key lifecycle, or improved DH2 path.
- `game-server::AccountPlayerSession` is a transport-free reducer beside, not merged into, `ClientLifecycle`. Its local `Login` phase does not prove the adjacent lifecycle is in `ClientPhase::Login`; the adapter must check that actual phase, then run blocked-IP, shutdown, and user-limit admission checks before `on_login`. Apply its candidate bind/phase state transactionally, retain the full seeded correlation and key-installation policy for DB/key routing, use the retry helpers without allocating a new generation, close on `PlayerIdMissing` or `PlayerNotBound` where legacy does, and perform full descriptor teardown. The reducer does not consume supplementary quest frames or validate world/location.

## SECURITY AND TRUST BOUNDARIES

- Legacy `server/server/game/input_login.cpp:265-304` indexes `c_r.players[pinfo->index]` before its `PLAYER_PER_ACCOUNT` check, and `454-515` forwards an unchecked create index into `TPlayerCreatePacket`. The select path is pre-auth normal-client reachable. These are separate C++ robustness follow-ups; the Rust reducer's select bound check does not patch the legacy server.
- **BOOT is composed per request, never replayed from a snapshot.** Legacy `QUERY_BOOT` calls `CItemIDRangeManager::GetRange()` twice per request, so every answer must consume two successive ranges from the persistent FIFO. `BootResponseAdapter` is an immutable snapshot and is only the fallback used when no `BootResponder` is installed. `impl BootResponder for ()` is fail-closed and is selected only when explicitly wrapped in `Some`. Live BOOT owns the item-range pool, the clock, and the injected time, and all peers share one `Arc<DbService>`, so ranges stay globally unique.
- **The SETUP reply is `HEADER_DG_MAP_LOCATIONS = 0xfe`, handle 0, and a `BYTE count` followed by 146-byte `TMapLocation` records.** There is no setup version byte, no per-table count word, and no echoed setup handle. A request with `bAuthServer != 0` gets **no bytes at all**; that is `SetupReplyOutcome::Silent`, which is reported as `DbServiceOutcome::NoResponse` and never as a zero-length `0xfe` frame, because a peer would then read a count byte that does not exist. The `bAuthServer` byte selects a branch and is never a credential: the role decision belongs to `DbPeerPolicy` and is made before the service is reached, and an auth-mode request from an ordinary game peer is rejected before dispatch.
- **Authorization precedes composition for SETUP.** The reply discloses a peer's public IP, listen port, and map list, so a denied request must write nothing and close the peer. `DbPeerOperation::GameSetup` is granted to authenticated Game and Auth peers with `AuthorizationEffect::None`, and an unlisted peer must receive no setup reply at all.
- **Only the requesting peer's own location is emitted today.** The legacy ordering rule is "other connected peers first, own peer last", but the multi-peer case needs a connected-peer registry that does not exist. Do not invent a global peer list to fill the gap, and do not describe the current reply as a complete SETUP.
- **`MapLocation` decisions already fixed, and not to be undone.** 32 signed 32-bit little-endian map indices at offset 0, 16 raw host bytes at 128, `u16` port at 144; a record is 146 bytes, a one-record payload 147, a complete frame 156. The count is checked with `u8::try_from`, so 256 records is an error rather than a truncation to 0. Decoding validates the count against the real payload length. The host tail is zero-filled so encoding is deterministic; legacy leaves those bytes uninitialized. The map list is capped at the 32 slots and stops at the first zero, because a 33rd entry would overwrite `dwLoginCount` in the packed request.
- **Keep the two `0xff` headers apart.** `HEADER_GD_SETUP` (game to DB) and `HEADER_DG_P2P` (DB to game) share a byte value and belong to opposite directions. `protocol::db_map_locations` declares the DB-to-game names; never merge the two tables.
- **The boot GM tail is per request, never cached with the tables.** Legacy `__GetHostInfo` and `__GetAdminInfo` both run inside `QUERY_BOOT`, and `__GetAdminInfo` filters `gmlist` on the requesting peer's `szIP` (`mServerIP='ALL' or mServerIP='<szIP>'`, with a NULL address becoming the literal `ALL`). Caching the administrator list with the tables would serve every peer the same GM list. Pass the decoded address as a per-request argument; never store one global request address on the responder.
- **The request address is validated, which is stricter than legacy on purpose.** Legacy interpolates `szIP` into an unescaped `'%s'`, so `db-server::gm::AdminQuery::new` accepts only a bare IP literal and refuses a statement at or above the legacy `char[512]`. `DbBootRequest::ip_text()` is fail-closed: an unterminated, empty, or non-ASCII `szIP` yields no address, which makes the composer use `ALL`. Reproducing the legacy interpolation would be a defect.
- **GM row conversion keeps the two different legacy rules apart.** The account goes through `trim_and_lower` (skip leading and trailing `isspace`, lowercase, always NUL-terminate inside `dest_size`), so `m_szAccount` is 31 usable bytes plus a terminator. The name goes through plain `strlcpy`, so its case and surrounding spaces survive. Only the account is normalized.
- **An unknown `mAuthority` drops the row.** Legacy `continue`s on anything that is not one of the six exact names. Defaulting to `GM_PLAYER` would invent an administrator the database never listed, so `GmAuthority::from_column` returns `None` and the row is skipped.
- **A missing `SQL_COMMON` refuses the boot rather than serving empty GM lists.** Legacy always runs both GM queries. Answering with zero rows would hand the game server a world with no GM hosts and no administrators while reporting success, which silently disarms every GM account. The same rule governs the item-ID range: it is taken only after the cache check and the GM read have both succeeded, so a refused boot does not shrink the supply for a later successful one.
- **The mob and item tables are read from SQL, which is the legacy `PROTO_FROM_DB=1` path, not the default.** `ClientManagerBoot.cpp:18-30` reads `mob_proto` and `item_proto` from the text files `mob_proto.txt` and `item_proto.txt` unless the config key `PROTO_FROM_DB` is non-zero; `bIsProtoReadFromDB` defaults to `false` at `ClientManager.cpp:71`. The wire format is identical either way, so the client cannot tell the sources apart, but the **data** can differ silently. `BootTableLoader` always reads SQL because neither text file is checked into this repository and there is no Rust text-table reader. Treat a match with a legacy deployment as conditional on that deployment running `PROTO_FROM_DB=1`. `ENABLE_AUTODETECT_VNUMRANGE` is defined at `ClientManagerBoot.cpp:1372` inside the `ENABLE_PROTO_FROM_DB` block, so `item_proto` selects 34 columns and not `vnum_range`; dropping that define shifts the whole item table by one cell.
- **A text replacement intended to change behavior must be confirmed to have landed on executable text.** A statement mutation that matched a doc comment before the constant produced a green run that proved nothing. When a mutation "survives", first check that it reached the code before concluding the test is weak.
- **The GM section count is a `WORD` on the wire, so the pure builders cap rows and bytes.** A source with more rows than `u16::MAX` would wrap the declared count and desynchronize the game server's parser for every later record. `build_host_list` and `build_admin_list` refuse with `GmSectionError::TooManyRows`/`TooManyBytes` rather than truncating.
- Treat legacy DB-peer framing as a separate trusted-peer hardening boundary. `CPeer::PeekPacket` and header-only dispatch can pass zero/short payloads to handlers that cast fixed records; `QUERY_SETUP` also trusts its login count and auth-peer designation without an authenticated role gate, `QUERY_EMPIRE_SELECT` trusts the empire index, and game-side `CInputDB::Process` narrows the length to signed `int` and can make no progress or move backward. These are not normal game-client reachability claims. The Rust setup decoder already bounds setup records, and `db-server/src/peer_policy.rs` now provides a transport-free fail-closed authorization boundary, but neither is live dispatch. Require checked lengths, per-handler minimum sizes, count bounds, range checks, and externally authenticated peer identity/role/ownership before any live DB adapter.
- An accepted DB socket is not an authenticated game session. The legacy listener defaults `BIND_IP` to `0` and `BIND_PORT` to 5300 unless configured, the optional source filter is not role authentication, and auth/login/player handlers trust packet account, player, and login fields before proving ownership. A live adapter must authenticate the peer, bind account/player ownership to the session, and recheck every mutating request before SQL. The current `DbPeerPolicy` permits only externally verified, generation-bound `Auth`-role base-only setup as an explicit effect; it must not be bypassed by `bAuthServer`, frame handles, setup IPs, login keys, account/player IDs, or source prefixes.
- `db-server::peer_auth` is **transport identity, not authentication**, and must never be described as authentication. The legacy DB protocol has no credential exchange, and a source address can be spoofed. Both `trusted_game_peers` and `trusted_auth_peers` **default to empty, which refuses every connection**; a peer left unclassified stays unauthenticated, is denied every request including the read-only `BOOT`, and is closed without a reply. Assigning a role removes only the `AuthenticationRequired` denial: it does not bind the auth slot, prove the peer is the process it claims to be, or bind an account or player. The one exception is an explicit effect: only `AuthorizationEffect::BindAuthPeer`, produced by base-only `Auth`-role `SETUP`, may bind the single auth slot, and a read-only authorization carries `AuthorizationEffect::None` and must leave shared state untouched. The decision is taken before the response is written and the effect is applied after, so a denied or failed request never claims the slot. Do not add a non-empty default allowlist, and do not let any other signal (a frame handle, `bAuthServer`, a setup IP, a login key, or a request header) stand in for the allowlist.
- `db::ConnectionPool::lazy` performs **no network I/O**. It parses the URL and stores it, so a malformed URL still fails at startup, but no TCP connection is attempted until a query runs. This is a deliberate difference from the legacy, which connects at startup and refuses to serve without a database; the agent container has no reachable MariaDB. A lazy pool is not a readiness signal, and the first query still fails if nothing is listening. Do not replace it with a connect-and-retry path, which would stop the listener from coming up.
- The active boot profile has **14** table sections, not 13, because the checked-in legacy build defines `ENABLE_RENEWAL_SHOPEX`, `__EVENT_MANAGER__`, and `__PREMIUM_PRIVATE_SHOP__`. Every section declares its verified record width even when it carries zero rows, because the legacy always writes the size `WORD`. The item-ID range block declares count `1` and then writes **two** records; the game-side reader consumes `size` records twice with no check on the count word, so the count is a per-half stride and not a total. Do not "fix" either quirk. An active-profile empty boot payload is exactly 415 bytes.
- Unknown DB headers are consumed and logged; retained client unknown/variable headers must close rather than retry in a loop. Note that this is a **rewrite requirement, not a legacy description**, and the legacy client path does not agree: `CInputHandshake::Analyze` has no switch and simply falls through; `CInputAuth::Analyze` (`input_auth.cpp:222`) logs via `sys_err` and consumes; `CInputLogin::Analyze` (`input_login.cpp:1260`) logs and returns 0 with its `d->SetPhase(PHASE_CLOSE)` line **commented out**, so it does not close; and `CInputMain::Analyze` (`input_main.cpp:4126`) is a bare `return (0)` with no log at all. Closing where legacy merely logs is a deliberate divergence and must be recorded as one, not presented as parity (ledger 160.5).
- Do not accept game clients before the DB boot has completed, and bound-check every length and count read from a peer. The legacy server does neither; see `docs/STATUS.md` for the list of legacy defects not to reproduce.

## LEGACY REFERENCE

- `server/server/game/`: legacy game behavior, packet tables, descriptors, managers, and main loop.
- `server/server/db/`: legacy DB peer protocol, request routing, caches, and boot stream.
- `server/server/common/`: shared legacy structures and utilities.
- `server/server/lib*`: legacy libraries.
- `server/server/quest/`: legacy quest and Lua code.
- `client/`: Windows client protocol and behavior reference. Its feature switches are in `client/Client/UserInterface/LOCALE_INC.H`; the server's are in `server/server/common/prodomodefines.h`.
- `server/extern/`, `client/Extern/`: bundled dependencies. Preserve source and required libraries.

The legacy C++ server uses Premake and Make:

```bash
cd server/server
./build_debug.sh
./build_live.sh
```

The server workspace is x86, C++17, and uses a static runtime. The client uses Visual Studio projects.

## REPOSITORY HYGIENE

- Keep the legacy C++ and client source until parity is demonstrated.
- Almost the whole workspace, including every Rust crate and this file, is **untracked** by Git. Do not use unrestricted `git clean`, `git checkout .`, or `git stash -u`; they would destroy user work. Commit only when the user asks.
- Generated output is disposable: root `target/`, client `.vs/`, and enumerated `Debug`/`Release` build trees.
- `.gitignore` re-includes directories under `server/extern/include/` and `client/Extern/include/` whose names (`debug/`, `log/`, `x86/`, `win32/`, `arm/`) match build-output rules. Keep that exception when editing ignore rules.
- Do not delete the active `.overlord/` runtime wholesale.
- Do not restore, stage, or delete the pre-existing Java removals without an explicit decision.
- Stray `.DS_Store` files are ignored; do not add them to Git.
- **Untracked means a bad edit is unrecoverable.** `db-server/src/main.rs` is untracked, and an edit in section 171 left it truncated with an unclosed delimiter that Git could not restore. Every `/tmp` copy was a pre-section-169 revision, and `target/debug/db-server` is a compiled receipt, not a source. Before a large edit to an untracked Rust file, copy it aside and keep the copy current, and never keep only a compiled binary as the recovery path.
- Run `cargo fmt` before compiling, not after. A file that fails to parse reports misleading delimiter errors, and a truncated file is far cheaper to diagnose from a clean parse.
- Write generated files only inside the intended test root. A crash during an i686 or `qemu` probe can drop a core file into the current directory; run probes from an explicit scratch directory under `/tmp` and check the workspace root for stray `*.core` files afterwards.

## LEGACY ANTI-PATTERNS

- `server/server/game/char_battle.cpp`: do not delete `m_dwKillerPID = 0`.
- `server/server/game/text_file_loader.cpp` and `group_text_parse_tree.cpp`: group names must not contain spaces.
- `client/Client/SphereLib/spherepack.cpp`: never remove the root node (`SPF_ROOTNODE`).

## Agent skills

### Issue tracker

Issues and specs are local markdown files under `.scratch/<feature>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

The five default roles (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`), recorded on each issue file's `Status:` line. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one root `CONTEXT.md` and `docs/adr/`, created lazily. See `docs/agents/domain.md`.
