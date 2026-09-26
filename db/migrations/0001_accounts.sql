-- Accounts, GM grants, and the two account currencies (ADR-0003, ledger section 179).
--
-- The store is fresh: nothing here is copied from the legacy MySQL `account` or `gmlist`
-- tables. Each column that replaces a legacy one names it.

-- One account: the credential the Reference client logs in with.
--
-- `id` reaches the client and the game as a 32-bit value, so it is an integer identity rather
-- than a UUID. The legacy column was `int unsigned`; an `integer` identity starts at 1 and stays
-- positive, so every value fits both.
CREATE TABLE account (
    id integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY,

    -- Legacy lowercases the login the client sends (`trim_and_lower`) and then accepts only
    -- 2 to 30 ASCII letters and digits (`FN_IS_VALID_LOGIN_STRING`, `LOGIN_MAX_LEN = 30`).
    -- Storing only that form makes the unique constraint case-insensitive.
    login text NOT NULL UNIQUE CHECK (login ~ '^[a-z0-9]{2,30}$'),

    -- An argon2id PHC string. It replaces MySQL `PASSWORD()`, which PostgreSQL does not have.
    password_hash text NOT NULL CHECK (password_hash LIKE '$argon2id$%'),

    -- The code a player types to delete a character. Legacy compares the last seven bytes of
    -- `social_id` with the seven the client sends (`ClientManagerPlayer.cpp:1363`).
    delete_code text NOT NULL CHECK (delete_code ~ '^[A-Za-z0-9]{7}$'),

    -- `OK`, or the status the client is sent as the login failure (`status`, `char[9]`).
    status text NOT NULL DEFAULT 'OK' CHECK (status ~ '^[!-~]{1,8}$'),

    -- The client's language, 1 to 11 (`ELocale` without `LOCALE_YMIR`); 1 is English.
    language smallint NOT NULL DEFAULT 1 CHECK (language BETWEEN 1 AND 11),

    -- Login is refused while this is in the future (legacy `availDt`, `NOTAVAIL`).
    available_at timestamptz NOT NULL DEFAULT now(),
    created_at timestamptz NOT NULL DEFAULT now(),
    last_play_at timestamptz,

    -- Coins: the item-shop currency (legacy `coins`, read and written as `long long`).
    coins bigint NOT NULL DEFAULT 0 CHECK (coins >= 0),

    -- Cash: the daily-gift currency (legacy `cash`). The client is sent it as a `DWORD`.
    cash bigint NOT NULL DEFAULT 0 CHECK (cash BETWEEN 0 AND 4294967295)
);

-- A GM grant: the authority one character Name holds.
--
-- Legacy keys `gmlist` by character name and also requires the account to match
-- (`gm_new_get_level`), so a grant names both. The Name may belong to a character that does not
-- exist yet. Names are unique regardless of case, so one Name has at most one grant.
CREATE TABLE gm_grant (
    id uuid PRIMARY KEY DEFAULT uuidv7(),
    account_id integer NOT NULL REFERENCES account (id) ON DELETE CASCADE,

    -- A character Name: 2 to 24 ASCII letters and digits (`check_name_alphabet`,
    -- `CHARACTER_NAME_MAX_LEN = 24`).
    name text NOT NULL CHECK (name ~ '^[A-Za-z0-9]{2,24}$'),

    -- The legacy `mAuthority` spellings, without `PLAYER`: a player needs no grant.
    authority text NOT NULL CHECK (
        authority IN ('LOW_WIZARD', 'WIZARD', 'HIGH_WIZARD', 'GOD', 'IMPLEMENTOR')
    ),
    granted_at timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX gm_grant_name_key ON gm_grant (lower(name));
CREATE INDEX gm_grant_account_id_idx ON gm_grant (account_id);
