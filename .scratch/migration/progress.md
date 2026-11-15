# Migration progress

Read this first after a compaction or in a new session. The goal is
`.scratch/migration/migration-goal.md`; this folder was `.scratch/equip/` until ledger 682. This
file is rewritten after every section and goes into the section's commit.

## Where it stands

- The last section is **ledger 684**, the toolchain: Rust 1.100.0, pinned in
  `rust-toolchain.toml`, with the new Clippy's findings fixed and Cargo's unused-dependency
  lint a gate. It ported nothing. Before it: 683 (the test hygiene: a killed test run leaves
  nothing, `tools/gates.sh` fails on anything left), 682 (the repository cleanup), 681
  (ADR-0008, mistakes are fixed, not ported, which queued seven fixes as package C10), 680 (the
  owner's answers) and 679 (the direction review).
- Parity: 1,136 `ported`, 65 `partial`, 444 `unused`, 0 `missing`, 0 `codec`, of 1,645 rows.
- Tests: 5,659 pass in 46 targets with `DATABASE_URL`; 519 are scenarios.
- **What is next is in `docs/STATUS.md`, "Next steps"**, in order: package A (make the tail
  countable) comes before any further `case` arm.

## Where things are

- `AGENTS.md` ("WORKSPACE") has the map of everything outside the crates; "TEST HYGIENE" has the
  rules a test and a session follow so nothing is left behind.
- Gates: `tools/gates.sh <output-dir>`, then read `<output-dir>/summary.txt`. It fails on a
  failed gate and on anything the tests left (`temp-files`, `servers`, `scratch-dbs`, `cores`),
  and prints the `allows` count, which may not rise.
- Mutation sweeps: `tools/mutate.py`, on a copy of the tree under `.scratch/tmp/`.
- Probes: `.scratch/probes/` (its README says how to build one).
- History, never a rule: `docs/STATUS_HISTORY.md`, `docs/PROTOCOL_NOTES_HISTORY.md`.
- A finished issue, a research note or a one-off script is deleted in the change that finishes
  the work. `.scratch/tmp/` is ignored and is emptied when a section lands; nothing tracked may
  cite a file in it.

## Owner decisions in force

- 2026-10-14: an in-memory test may prove a row when a binary scenario is impractical; Defects
  and backdoors are fixed, not carried over; Channel 1 and the Shared Channel run.
- 2026-10-15: mutation sweeps wait for a stronger machine. Answered 2026-11-15: this 16-core
  machine is the one, so package C7's sweeps by risk may start. The 450 deferred per-section
  sweeps are **not** owed any more.
- 2026-11-15: the play test is last and out of scope until the owner opens it; the test-server
  mode ships off (ADR-0007); the save cycle is 10 seconds; the `unused` rows stand; ADR-0006 is
  accepted; `change_empire.lua` is fixed, the one edit to a quest source (ADR-0004); the drop
  limit is 1000 ms; the game logs to keep are in `docs/STATUS.md`.
- 2026-11-15 (ADR-0008): a legacy bug, inconsistency, vulnerability or backdoor is fixed while
  porting, without a question, and recorded; never ported "provisionally". Odd but deliberate
  behaviour stays as a Quirk. A difference the Rewrite's own design causes is not an
  improvement. A quest source is edited only where a mistake stops a script (ADR-0004).

## The machine

- 16 cores, 30 GB. Rust 1.100.0 comes from rustup through `rust-toolchain.toml`: use
  `~/.cargo/bin/cargo` (put `~/.cargo/bin` first in `PATH`). `/usr/bin/cargo` is the
  distribution's 1.85.1 and must not build this tree; `tools/gates.sh` refuses it.
- PostgreSQL 18 is the Podman container `prodomo-pg18` on `127.0.0.1:55432`. There is no `psql`
  on the host: use `podman exec prodomo-pg18 psql -U prodomo -d prodomo`.
- `i686-linux-gnu-g++-12` 12.4.0 is installed (ledger 680); package C8 probes the records.
- `compose.yaml` builds and starts with Podman (ledger 680). The images
  `rust:1.100.0-bookworm`, `debian:bookworm-slim` and `localhost/prodomo:local` are on the
  machine. Never `compose restart`: it ended the server without the SIGTERM.
- A full run with `DATABASE_URL` still loses one scenario now and then, a different one each
  time: `.scratch/suite-flakes/issues/01-full-suite-first-run-losses.md`. Four losses have a
  cause in the scenario and a fix (ledgers 683 and 684); the issue says what to look for in
  the next. A second wait for a line the server already logged must be
  `Server::wait_for_next`.
- Clean since ledger 683: no scratch database, no test server, no `prodomo-*` entry in `/tmp`,
  no Git worktree but the main one, an empty `.scratch/tmp/`, and a `target/` rebuilt from
  nothing.

## Left running or uncommitted

Nothing is left running. Ledger 684 is committed and pushed.
