-- The account's empire and the characters it holds (ledger section 185).
--
-- Only the columns the Channel login reads are here; each later system adds its own in a new
-- migration. Every byte-wide column is range-checked to the width the client is sent, so a row
-- written by hand is never truncated on the wire.

-- The empire every character of the account belongs to (legacy `player_index.empire`), or 0
-- before the first character is created.
ALTER TABLE account
    ADD COLUMN empire smallint NOT NULL DEFAULT 0 CHECK (empire BETWEEN 0 AND 3);

-- One character. Legacy `player_index` held up to four character IDs per account (`pid1` to
-- `pid4`); here each character names its account and its slot.
CREATE TABLE player (
    -- Reaches the client as a 32-bit value (`TSimplePlayer::dwID`).
    id integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    account_id integer NOT NULL REFERENCES account (id) ON DELETE CASCADE,

    -- The select-screen slot, 0 to 3 (`PLAYER_PER_ACCOUNT = 4`).
    slot smallint NOT NULL CHECK (slot BETWEEN 0 AND 3),

    -- A character Name: 2 to 24 ASCII letters and digits, unique regardless of case.
    name text NOT NULL CHECK (name ~ '^[A-Za-z0-9]{2,24}$'),

    -- The race, 0 to 7 (`MAIN_RACE_MAX_NUM = 8`; the wolfman is not built).
    job smallint NOT NULL CHECK (job BETWEEN 0 AND 7),
    level smallint NOT NULL DEFAULT 1 CHECK (level BETWEEN 0 AND 255),
    -- Minutes played (`playtime`, sent as a `DWORD`).
    playtime_minutes integer NOT NULL DEFAULT 0 CHECK (playtime_minutes >= 0),
    st smallint NOT NULL DEFAULT 0 CHECK (st BETWEEN 0 AND 255),
    ht smallint NOT NULL DEFAULT 0 CHECK (ht BETWEEN 0 AND 255),
    dx smallint NOT NULL DEFAULT 0 CHECK (dx BETWEEN 0 AND 255),
    iq smallint NOT NULL DEFAULT 0 CHECK (iq BETWEEN 0 AND 255),
    conqueror_level smallint NOT NULL DEFAULT 0 CHECK (conqueror_level BETWEEN 0 AND 255),
    sungma_str smallint NOT NULL DEFAULT 0 CHECK (sungma_str BETWEEN 0 AND 255),
    sungma_hp smallint NOT NULL DEFAULT 0 CHECK (sungma_hp BETWEEN 0 AND 255),
    sungma_move smallint NOT NULL DEFAULT 0 CHECK (sungma_move BETWEEN 0 AND 255),
    sungma_immune smallint NOT NULL DEFAULT 0 CHECK (sungma_immune BETWEEN 0 AND 255),

    -- The armour and hair shown on the select screen (`WORD` parts).
    part_main integer NOT NULL DEFAULT 0 CHECK (part_main BETWEEN 0 AND 65535),
    part_hair integer NOT NULL DEFAULT 0 CHECK (part_hair BETWEEN 0 AND 65535),
    part_sash integer NOT NULL DEFAULT 0 CHECK (part_sash BETWEEN 0 AND 65535),

    -- The last saved position, in world units.
    x integer NOT NULL DEFAULT 0,
    y integer NOT NULL DEFAULT 0,

    skill_group smallint NOT NULL DEFAULT 0 CHECK (skill_group BETWEEN 0 AND 255),
    -- The player must choose a new Name before playing this character.
    change_name boolean NOT NULL DEFAULT false,

    created_at timestamptz NOT NULL DEFAULT now(),

    UNIQUE (account_id, slot)
);

CREATE UNIQUE INDEX player_name_key ON player (lower(name));
