-- The empire changes (ledger section 405).
--
-- Legacy's `change_empire` (`legacy/sql/schema/player.sql:65-70`) holds one row per account
-- that ever changed its empire, with the change count; `CHARACTER::GetChangeEmpireCount`
-- reads it (`G/char_change_empire.cpp:131-162`) and `SetChangeEmpireCount` writes it
-- (`:164-185`), and the change quest refuses a count at or above 1. Legacy's key is the
-- count itself (`PRIMARY KEY (change_count)`), so two accounts that each changed once
-- cannot both hold a row; that is a Defect, and here the account is the key.

CREATE TABLE change_empire (
    -- The account, as `account.id`.
    account_id integer PRIMARY KEY REFERENCES account (id) ON DELETE CASCADE,
    -- How many times the account changed its empire.
    change_count integer NOT NULL CHECK (change_count > 0),
    -- When the last change landed.
    changed_at timestamptz NOT NULL DEFAULT now()
);
