# Prodomo server, Rust rewrite

A Linux-native Rust rewrite of the Prodomo Metin2 server. The goal is that the unmodified Prodomo
5.4 client, pointed at this server, plays exactly as it does against the legacy C++ server.

**It is not playable yet.** The client wire codecs and descriptor reducers are built and tested, but
no client can log in. See [`docs/STATUS.md`](docs/STATUS.md) for where it stands and what comes next.

## How it differs from the legacy server

Only what the client can see has to match. Everything behind the client protocol is redesigned:

- **One process.** A single `prodomo` binary runs auth and every Channel, including the Shared
  Channel (legacy channel 99). There is no separate DB server and no P2P mesh between game processes.
- **PostgreSQL 18, starting empty.** Passwords are hashed with argon2id. Trades, shop sales, and every
  other exchange of items or gold commit in one transaction, so a crash can never duplicate or lose
  an item.
- **Lua 5.1 for quests**, with the quest compiler ported to Rust. The quest sources are used
  unchanged.
- **TOML configuration**, with the legacy ports as defaults.

The reasons are in [`docs/adr/`](docs/adr/).

## Repository layout

| path | contents |
|---|---|
| `protocol/`, `common/`, `net/`, `db/`, `world/`, `quest/`, `game-server/`, `db-server/` | The Rust workspace. It is being restructured into the single `prodomo` binary; see `AGENTS.md`. |
| `legacy/` | The owner's legacy Game data (protos, maps, quests, locale strings, drop and shop tables), configuration, and SQL schema and table rows. See [`legacy/README.md`](legacy/README.md). |
| `server/` | The frozen legacy C++ server source. It is the behavioural reference and is never modified or built. |

## Documentation

| document | read it for |
|---|---|
| [`docs/STATUS.md`](docs/STATUS.md) | Where the rewrite stands, the build order, and the next step. Start here. |
| [`CONTEXT.md`](CONTEXT.md) | The glossary: Parity, Quirk, Defect, Divergence, Channel, Warp, Pulse, and the rest. |
| [`docs/adr/`](docs/adr/) | The architecture decisions and why they were made. |
| [`AGENTS.md`](AGENTS.md) | Rules for anyone changing the code: build gates, verification method, compatibility and security rules. |
| [`docs/PROTOCOL_NOTES.md`](docs/PROTOCOL_NOTES.md) | Per-record findings for the client wire codecs and the legacy defects behind the rules. |
| [`docs/REWRITE_LEDGER.md`](docs/REWRITE_LEDGER.md) | Append-only record of every change and gate receipt. |

## Prerequisites

- Rust 1.85.1.
- Podman or Docker, to run PostgreSQL 18 for the database tests and the server.

## Build and test

```bash
cargo fmt --all -- --check
cargo build --workspace --locked --offline
cargo test --workspace --all-targets --locked --offline --no-fail-fast
cargo test --workspace --doc --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline
```

These gates need no database. Tests that do need one run only when `DATABASE_URL` points at a
PostgreSQL 18 server.

## Running it

Not yet. Build step 1 produces the `prodomo` binary, its configuration file, and the Operator
command that creates accounts and GMs; this section will then describe them. The play test will use
a `compose.yaml` that starts PostgreSQL 18 and `prodomo` together.

## Contributing

Follow `AGENTS.md`. In short: derive every layout and behaviour from the legacy source, encode
`#pragma pack(1)` records field by field, back each codec with golden bytes, never reproduce a legacy
defect, keep all six gates green, and record each change as a new ledger section.
