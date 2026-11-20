# PostgreSQL 18 with a fresh store

The Rewrite stores everything in PostgreSQL 18 instead of MySQL/MariaDB and starts from an empty
database, because there is nothing to migrate (ADR-0001). This choice follows the owner's
preference. It also means the schema is designed for the Rewrite rather than copied from legacy.

## Consequences

- Any entity whose ID reaches the Reference client (players, guilds, and anything else the client
  sees by ID) keeps a 32-bit integer key, because 22 fields in `packet.h` carry such IDs. Rows the
  client never sees by ID use `uuidv7()`.
- `jsonb` is used only for sparse or variable data that legacy packs into columns (quest flags,
  affect lists). Unlogged tables are used only for transient data that may be lost in a crash.
- Free text is stored and relayed as the exact bytes the client sent (`bytea`) and is never
  transcoded, because the multi-language client may send different code pages. Names are ASCII and
  unique regardless of case.
- Passwords are hashed with argon2id, replacing MySQL `PASSWORD()` (`input_auth.cpp:125-127`), which
  PostgreSQL does not have. An operator command creates accounts, GMs, and item-shop currency,
  because no website exists.
- Every Transfer commits in one transaction when it happens, and other player state is written in
  the background at most a few seconds late. This is a Divergence from the legacy DB cache, which
  flushes on a timer and can lose or duplicate items in a crash.
- Database tests run only when `DATABASE_URL` is set, against PostgreSQL 18 in Podman, so the
  offline gates stay green without a database.

## Amendments

- **2026-11-15 (owner, direction review, ledger 679).** "At most a few seconds late" is binding.
  Until ledger 679 the per-character save event ran on legacy's cycle (120 seconds compiled in,
  180 in the owner's files), so a crash could lose that much experience, points and position;
  items and gold were already written when they moved. `[game] save_event_second_cycle` now
  defaults to 10.
- A test run without `DATABASE_URL` proves no Parity inventory row: every scripted-client scenario
  returns at its first line and is counted as passed. A gate receipt names the `DATABASE_URL` run
  and its counts, or says that it did not run.
- Drift from this decision, closed in ledger 738 (Package C5, `db/migrations/0048_guild_free_text_bytea.sql`):
  `guild_comment.content` and `guild_grade.name` were `text` with `char_length` checks
  (`0012_guild_comment.sql`, `0011_guild.sql`) and are now `bytea` with `octet_length` checks.
- `player_deleted.player` is `jsonb` holding the whole deleted row
  (`db/migrations/0003_character_select.sql`). That is an accepted exception: nothing reads it
  back, and typed columns would have to follow every later `player` migration.
