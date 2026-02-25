# Metin2 Server Rust Rewrite

This repository contains an active, Linux-native Rust rewrite of the Metin2 game and database servers. It is **not yet a playable replacement**: the codecs and reducers are tested, but nothing is wired end to end. The legacy C++ server under `server/` and the Windows client under `client/` remain the behavioral and wire-format reference until parity is proven.

## Documentation

| document | read it for |
|---|---|
| [`docs/STATUS.md`](docs/STATUS.md) | Where the rewrite stands, what the binaries actually do, and the next milestones. Start here. |
| [`AGENTS.md`](AGENTS.md) | Rules for anyone changing the code: build gates, verification method, compatibility and security rules, hygiene. |
| [`docs/PROTOCOL_NOTES.md`](docs/PROTOCOL_NOTES.md) | Per-module and per-record findings and the legacy defects behind the rules. |
| [`docs/REWRITE_LEDGER.md`](docs/REWRITE_LEDGER.md) | Authoritative, append-only record of every change and gate receipt (sections 1-168). |

## Workspace

| crate | purpose | state |
|---|---|---|
| `protocol/` | Legacy packet records, header inventories, TEA | Codecs for 91 of 92 client-to-game, 96 of 134 game-to-client, and 4 of 36 game-to-game records, plus the shared game-to-DB setup record. Transport-free. |
| `common/` | Shared config, logging, enums, legacy table types | Partial. |
| `net/` | Tokio framing for DB peers and fixed client records | Used by read-only accept loops. |
| `db/` | SQLx/MySQL pool and bounded query streaming | Never run against a real database. |
| `world/` | Spatial model, characters, events, combat rules | Not used by the server binary. |
| `quest/` | Quest/Lua integration | Scaffold only. |
| `game-server/` | Game server binary, transport-free session reducers, and the game-side DB client | Accepts sockets and answers only keepalive and pong. It does open a real DB socket: it sends `GD_BOOT`/`GD_SETUP`, validates the boot reply, and refuses clients until boot. No table loader, so a booted server still has empty boot sections. |
| `db-server/` | Database server binary, boot composition, SQL adapters, peer policy | Answers `GD_BOOT` with a real 415-byte empty boot payload and an ordinary game `GD_SETUP` with one `DG_MAP_LOCATIONS` frame. All table sections are empty because no table loader exists. Every other request is authorized and then logged as unimplemented. |

`tools/` is outside the workspace, and `tools/packet_compare` does not build. `ported-client/` is an empty stub.

## Prerequisites

- A current stable Rust toolchain. The workspace was last verified with Rust 1.85.1.
- The current `Cargo.lock`. It is present in this worktree but not tracked by Git.
- For database work, a MySQL or MariaDB server. None runs in the agent container; run one on the host or in a Podman container and reach it through `host.docker.internal`.
- The legacy C++ build has its own requirements; see `server/server/` and its `build_*.sh` scripts.

## Build and test

```bash
cargo fmt --all -- --check
cargo build --workspace --locked --offline
cargo test --workspace --all-targets --locked --offline --no-fail-fast
cargo test --workspace --doc --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline
```

The latest receipt (2026-09-26) is 1,936 tests across 28 test binaries plus 6 doc-tests, all passing, with every gate green. These are unit and slice checks, not evidence of client, DB, world, quest, or gameplay parity.

## Configuration

Both binaries are configured by a **TOML** file. The legacy C++ `CONFIG` and `conf.txt`
`key=value` files are no longer read. Start from the commented examples:

```bash
cp config/game.toml.example game.toml
cp config/db.toml.example db.toml
```

Every key is optional and keeps the compiled-in default when omitted, but an **unknown** key is
an error, so a typo fails at startup instead of silently selecting a default. Booleans must be real
TOML booleans (`true` / `false`); the legacy `1` / `0` forms are rejected. The three SQL connections
of each binary are sub-tables (`[player_sql]`, `[sql_player]`, and so on) instead of one
space-separated string, which also removes the legacy trap where the game and DB servers read the
same key in a different field order.

`SqlConfig::connection_url()` turns a connection table into a `mysql://` URL, percent-encoding the
user and password so a credential containing `@` or `/` cannot redirect the connection.

### Database peer trust

`db-server` also reads `trusted_game_peers` and `trusted_auth_peers`. **Both default to empty, which
refuses every connection.** With no entry, each peer is denied every request, including the read-only
`BOOT` request, and is closed without a reply.

This setting is **transport identity, not authentication**. The legacy DB protocol has no
credential exchange, and a source address can be spoofed. Assigning a role removes only the
"authentication required" denial: it does not bind the auth slot, prove the peer is the process it
claims to be, or bind an account or player. The binary logs a warning at startup whenever the lists
are non-empty. See `db-server::peer_auth` and `db-server::peer_policy`.

To convert an existing `CONFIG` or `conf.txt` by hand, use the legacy-key to TOML-key mapping in
section 169 of [`docs/REWRITE_LEDGER.md`](docs/REWRITE_LEDGER.md).

## Run the current binaries

```bash
cargo run -p game-server -- --config game.toml
cargo run -p db-server  -- --config db.toml
```

The default paths are `game.toml` and `db.toml`, so `--config` may be omitted. Use `--help` to
list the options, and `--port` to override the listen port.

`db-server` currently accepts a TCP peer, classifies its transport identity, authorizes every request
through the fail-closed peer policy, and answers a `BOOT` request with a real boot payload whose
table sections are **empty** and whose tail carries the clock, the item-ID range pair, the monarch
record, and the `0xffff` terminator. No other request has a response yet, and the binary does **not**
provide a game session. Its SQL pools are lazy: they store the URL at startup and attempt no
connection until a query runs, so a missing database does not stop the listener.

## Contributing

Follow `AGENTS.md`. In short: derive every layout and behavior from the legacy source, encode `#pragma pack(1)` records field by field, back each codec with golden bytes, keep all six gates green, and record each change as a new ledger section. Most of the tree is untracked, so never run an unrestricted `git clean`.
