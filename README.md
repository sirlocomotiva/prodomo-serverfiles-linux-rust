# Prodomo server, Rust rewrite

A Linux-native Rust rewrite of the Prodomo Metin2 server. The goal is that the unmodified Prodomo
5.4 client, pointed at this server, plays exactly as it does against the legacy C++ server.

**It has not been played with the real client yet.** The scripted client of the test suite logs
in, enters the world, fights, trades and runs quests against the real binary; the owner's play
test with the Prodomo client comes once the remaining systems are closed. See
[`docs/STATUS.md`](docs/STATUS.md) for where it stands and what comes next.

## How it differs from the legacy server

Only what the client can see has to match. Everything behind the client protocol is redesigned:

- **One process.** A single `prodomo` binary runs auth and every Channel, including the Shared
  Channel (legacy channel 99). There is no separate DB server and no P2P mesh between game processes.
- **PostgreSQL 18, starting empty.** Passwords are hashed with argon2id. Trades, shop sales, and every
  other exchange of items or gold commit in one transaction, so a crash can never duplicate or lose
  an item.
- **Lua 5.1 for quests**, with the quest compiler ported to Rust. The quest sources are used as
  they are.
- **TOML configuration**, with the legacy ports as defaults.

The reasons are in [`docs/adr/`](docs/adr/).

## Repository layout

| path | contents |
|---|---|
| `protocol/`, `common/`, `net/`, `db/`, `gamedata/`, `world/`, `quest/`, `prodomo/`, `parity/` | The Rust workspace. `prodomo/` is the one server binary and `parity/` the scripted client its scenarios use; see `AGENTS.md` for what each crate holds. |
| `config/prodomo.toml.example` | The commented configuration, with the legacy ports. Channel 1 and the Shared Channel run; Channels 2-4 are commented out. |
| `legacy/` | The owner's legacy Game data (protos, maps, quests, locale strings, drop and shop tables), configuration, and SQL schema and table rows. See [`legacy/README.md`](legacy/README.md). |
| `server/` | The frozen legacy C++ server source. It is the behavioural reference and is never modified or built. |
| `compose.yaml`, `Dockerfile` | PostgreSQL 18 and the server together, for Podman or Docker. |
| `tools/gates.sh` | Runs every gate below and writes a summary: `tools/gates.sh <output-dir>`. |
| `.scratch/` | Working files that are tracked on purpose: the Parity inventory, the agents' goal and progress, the owner's questions, the width probes and open issues. |

## Documentation

| document | read it for |
|---|---|
| [`docs/STATUS.md`](docs/STATUS.md) | Where the rewrite stands, the next steps in order, and what waits on the owner. Start here. |
| [`CONTEXT.md`](CONTEXT.md) | The glossary: Parity, Quirk, Defect, Divergence, Channel, Warp, Pulse, and the rest. |
| [`docs/adr/`](docs/adr/) | The architecture decisions and why they were made. |
| [`AGENTS.md`](AGENTS.md) | Rules for anyone changing the code: build gates, verification method, compatibility and security rules. |
| [`docs/PLAYTEST.md`](docs/PLAYTEST.md) | First-session play-test script for the Reference client (starting with owner questions 7 and 27) and how to read refused-frame logs. |
| [`docs/PROTOCOL_NOTES.md`](docs/PROTOCOL_NOTES.md) | Per-record findings for the client wire codecs and the legacy defects behind the rules. |
| [`docs/REWRITE_LEDGER.md`](docs/REWRITE_LEDGER.md) | Append-only record of every change and gate receipt. |

## Prerequisites

- Rust 1.100.0, through rustup: `rust-toolchain.toml` pins it, and rustup installs it on the
  first `cargo` call.
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
PostgreSQL 18 server whose role may create databases; each test creates and drops its own:

```bash
DATABASE_URL=postgres://prodomo:CHANGE-ME@127.0.0.1:55432/prodomo \
  cargo test --workspace --all-targets --locked --offline --no-fail-fast
```

## Running it

### With Compose

`compose.yaml` starts PostgreSQL 18 and the server together, with Podman or Docker:

```bash
printf 'POSTGRES_PASSWORD=%s\n' 'CHANGE-ME' > .env      # pick your own password
cp config/prodomo.toml.example prodomo.toml
# in prodomo.toml set public_ip to this machine's address, and
# [store].url to postgres://prodomo:CHANGE-ME@postgres:5432/prodomo
podman compose up -d --build                            # or: docker compose up -d --build
podman compose run --rm prodomo account create alice    # asks for the password twice
podman compose run --rm prodomo gm grant alice Admin implementor
podman compose logs -f prodomo
```

The image build fetches the locked crates once. `legacy/` is mounted read-only, the store keeps
its data in the `pgdata` volume, and the server's log files go to the `logs` volume. The
published ports are the legacy ones the client's `serverinfo` names: 30001 (auth), 30003 and
30005 (Channel 1) and 30019 (the Shared Channel). `podman compose stop` or `podman compose down`
stops the server with SIGTERM, and it saves every character before it exits. Do not use
`podman compose restart`: in the test of ledger 680 it ended the server without the SIGTERM.

### Pointing the Reference client (`serverinfo.py`)

Point the Prodomo 5.4 client's `serverinfo.py` at the `[network].public_ip` configured in
`prodomo.toml` (for example `127.0.0.1` on the same machine, or the host/VM LAN IP) using the
default ports from `config/prodomo.toml.example` and `compose.yaml`:

| `serverinfo.py` entry | address | port | notes |
|---|---|---|---|
| `AUTH` (`STATE_NONE` / auth server) | `<public_ip>` | `30001` | Login / authentication listener (`[auth].port`). |
| `CH1` (`CHANNEL_DICT[1]`) | `<public_ip>` | `30003` | Channel 1 primary port (`30005` also serves all Channel 1 maps). |
| `MARKADDR` (guild mark / symbol server) | `<public_ip>` | `30003` | Served on Channel 1's first port. |
| Shared Channel (channel 99) | `<public_ip>` | `30019` | Not listed in `CHANNEL_DICT`; reached automatically via `GC_WARP`. |

Channels 2–4 are disabled by default (commented out in `config/prodomo.toml.example`). Keep the
client's `CHECK_VERSION` / locale on `europe`. Follow [`docs/PLAYTEST.md`](docs/PLAYTEST.md) for
the first-session verification script and how to inspect refused-frame logs if the client
disconnects.

### From the source tree

Start PostgreSQL 18 (pick your own password; this one is only an example):

```bash
podman run -d --name prodomo-pg18 -e POSTGRES_USER=prodomo -e POSTGRES_PASSWORD=CHANGE-ME \
  -e POSTGRES_DB=prodomo -p 127.0.0.1:55432:5432 postgres:18
cp config/prodomo.toml.example prodomo.toml
# [store].url in the example already names this container; put your password in it
cargo run -p prodomo -- --config prodomo.toml serve
```

`--config` defaults to `prodomo.toml` in the working directory. Logs go to stdout and to a file in
`./log`, filtered by `RUST_LOG` when it is set; `serve --verbose` also reads `LOG_DIR` and `LOG_ANSI`.
The store URL must be `postgres://` or `postgresql://`, and its password is never logged. A root
`prodomo.toml` is ignored by Git, so a real password stays local. SIGTERM or Ctrl+C stops the
server.

On start the server binds every port, then creates or updates the schema. Until that has finished
it closes each client connection at once. While PostgreSQL cannot be reached it keeps retrying,
waiting 1 second at first and up to 30 seconds between attempts; a permanent error, such as a wrong
password or a missing database, stops it with a non-zero exit.

### Accounts, GMs, and currency

The Operator commands use the same configuration file and create the schema if it is missing.

```bash
prodomo account create alice                  # asks for the password twice
printf '%s\n' "$PASSWORD" | prodomo account create alice --delete-code 1234567
prodomo account password alice                # replaces the password
prodomo account coins alice 500               # item-shop Coins; a negative amount removes them
prodomo account cash alice 20                 # daily-gift Cash
prodomo gm grant alice Admin god              # low_wizard, wizard, high_wizard, god, implementor
prodomo gm list
prodomo gm revoke Admin
```

- A login is 2 to 30 ASCII letters and digits and is stored in lowercase. A password is 1 to 16
  printable characters, read from standard input, never from the command line. On a terminal it
  is asked for twice without echo; otherwise the first line is used.
- The delete code is the 7 letters and digits a player types to delete a character. Without
  `--delete-code`, a random 7-digit code is generated and printed once.
- A GM grant names one character of one account; the character need not exist yet. Names are
  unique regardless of case, so a Name granted to another account must be revoked first.
- A balance never goes below zero or above its limit; a change that would is refused and nothing
  is written.

Under Compose, run each of these as `podman compose run --rm prodomo account ...`.

## Contributing

Follow `AGENTS.md`. In short: derive every layout and behaviour from the legacy source, encode
`#pragma pack(1)` records field by field, back each codec with golden bytes, never reproduce a legacy
defect, keep all six gates green, and record each change as a new ledger section.
