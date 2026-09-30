-- The safebox and the mall, owned by the account (ledger section 226, ADR-0005).
--
-- Legacy keeps a safebox or mall item in `player.item` with the **account** id in `owner_id`
-- (`db/ClientManager.cpp:790-814`), the column that holds a character id for every other window.
-- `0005_items.sql` refused that: its `owner_id` references `player`. The account gets a column of
-- its own, so every row names exactly one holder of one kind, and the holder follows the window.

ALTER TABLE item ADD COLUMN account_id integer REFERENCES account (id) ON DELETE CASCADE;

-- `0005_items.sql`'s biconditional, `(window_type = 10) = (owner_id IS NULL)`, took the
-- generated name `item_check` (read back from PostgreSQL 18's `pg_constraint`). The three-way
-- rule replaces it: a ground row (`GROUND`, 10) has no holder, a `SAFEBOX` (3) or `MALL` (4) row
-- has only the account, and every other window has only the character.
-- `db::items::check` states the same rule in Rust, so a refused row names its fields.
ALTER TABLE item DROP CONSTRAINT item_check;

ALTER TABLE item ADD CONSTRAINT item_holder_check CHECK (
    CASE
        WHEN window_type = 10 THEN owner_id IS NULL AND account_id IS NULL
        WHEN window_type IN (3, 4) THEN owner_id IS NULL AND account_id IS NOT NULL
        ELSE owner_id IS NOT NULL AND account_id IS NULL
    END
);

-- One item per safebox or mall cell, deferrable for the same reason as `item_owner_cell_key`
-- (`0006_item_cell_deferrable.sql`): a checkin, a checkout and a move inside the safebox are
-- written as row moves, and only the end state has to hold. A NULL `account_id` is distinct, so
-- the character and ground rows never meet this key.
ALTER TABLE item
    ADD CONSTRAINT item_account_cell_key UNIQUE (account_id, window_type, pos)
    DEFERRABLE INITIALLY IMMEDIATE;

CREATE INDEX item_account_idx ON item (account_id) WHERE account_id IS NOT NULL;

-- The safebox password of one account.
--
-- Legacy's `safebox` row (`player.sql`, read at `db/ClientManager.cpp:706-760`) holds the
-- password in plain text next to a size and a gold column. The size is never read (the owner's
-- build always opens one page, `input_db.cpp:1135-1150`) and the gold is never shown, so only
-- the password is kept, as an argon2id PHC string (ADR-0003). An account without a row opens
-- with `000000`, which is what legacy answers for an account without one.
CREATE TABLE safebox (
    account_id integer PRIMARY KEY REFERENCES account (id) ON DELETE CASCADE,
    password_hash text NOT NULL CHECK (password_hash LIKE '$argon2id$%')
);
