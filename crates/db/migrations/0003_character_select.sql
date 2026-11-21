-- Creating, deleting, and renaming characters on the select screen (ledger section 186).

-- The points a new character starts with (`NewPlayerTable2`, `input_login.cpp:403-449`). Legacy
-- keeps them as signed 32-bit values.
ALTER TABLE player
    ADD COLUMN hp integer NOT NULL DEFAULT 0,
    ADD COLUMN sp integer NOT NULL DEFAULT 0,
    ADD COLUMN stamina integer NOT NULL DEFAULT 0,
    -- The body shape chosen at creation (`part_base`, a `BYTE`; creation allows only 0 or 1).
    ADD COLUMN part_base smallint NOT NULL DEFAULT 0 CHECK (part_base BETWEEN 0 AND 255);

-- A deleted character. Legacy copied the row into `player_deleted` before deleting it
-- (`D/ClientManagerPlayer.cpp:1442`); here the whole row is kept as one document, so later
-- migrations that add columns to `player` need no twin here.
CREATE TABLE player_deleted (
    id uuid PRIMARY KEY DEFAULT uuidv7(),
    player_id integer NOT NULL,
    account_id integer NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    name text NOT NULL,
    player jsonb NOT NULL,
    deleted_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX player_deleted_account ON player_deleted (account_id);
