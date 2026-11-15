# Next session (2026-11-15)

Run `/goal .scratch/migration/migration-goal.md`.

Read `.scratch/migration/progress.md`, then `docs/STATUS.md` from its top to the end of "Waiting on
the owner". The direction review (ledger 679) changed how the work is cut: start with package A,
which turns the 65 `partial` rows into one countable list, before porting anything.

Prodomo is a from-scratch rewrite. There is no live server, no player data and no owner
`prodomo.toml`. The test PostgreSQL is a fresh database.
