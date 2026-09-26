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
