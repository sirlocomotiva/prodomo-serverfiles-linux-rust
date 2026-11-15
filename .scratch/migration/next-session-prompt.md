# Next session (2026-11-15, after ledger 685)

Run `/goal .scratch/migration/migration-goal.md`.

Read `.scratch/migration/progress.md`, then `docs/STATUS.md` from its top to the end of "Waiting on
the owner". The direction review (ledger 679) changed how the work is cut: start with package A,
which turns the 65 `partial` rows into one countable list, before porting anything.

Prodomo is a from-scratch rewrite. There is no live server, no player data and no owner
`prodomo.toml`. The test PostgreSQL is a fresh database.

Before the first scenario, read `AGENTS.md`, "SCENARIO TIMING" and "TEST HYGIENE". A scenario that
fails in a full run and passes alone has a defect of its own: read its failure against the legacy
rule, fix it in the same section, and name it in the receipt.

The machine: `cargo` is `~/.cargo/bin/cargo` (Rust 1.100.0 from `rust-toolchain.toml`), PostgreSQL
18 is the Podman container `prodomo-pg18` on `127.0.0.1:55432`, and the gates are
`DATABASE_URL=postgres://prodomo:prodomo-test@127.0.0.1:55432/prodomo tools/gates.sh <dir>`, about
nine minutes, never beside another test run.
