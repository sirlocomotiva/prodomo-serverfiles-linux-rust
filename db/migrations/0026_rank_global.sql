-- The global rank (ledger section 402).
--
-- Legacy's `ENABLE_GLOBAL_RANK` (`C/prodomodefines.h:170-175`) keeps three tables
-- (`legacy/sql/schema/player.sql:1119-1180`): `rank_global`, one row per character with its
-- nine counters, `rank_global_reward`, the prizes the Operator lists for a place in a
-- counter, and `rank_global_winner`, the prizes a season's end awarded, each claimed once by
-- `pc.global_rank_get_reward`. Its DB process kept the counters in memory, wrote them every
-- 20 minutes, and emptied `rank_global` at a season's end (`D/ClientManager.cpp:5225-5525`).
--
-- Here a character's row is written with the character, as the rest of its state is
-- (ADR-0003). `rank_global_state` holds the season number the client is shown and a
-- generation the season's end raises: a row is written only while its generation is the
-- current one, so a save taken before a season's end cannot put an old row back after it.

CREATE TABLE rank_global_state (
    only_row boolean PRIMARY KEY DEFAULT true CHECK (only_row),
    -- `RankGlobalSeason`, the event flag legacy shows and raises at a season's end.
    season integer NOT NULL DEFAULT 0,
    -- How many seasons ended here: the rows' guard.
    generation bigint NOT NULL DEFAULT 0 CHECK (generation >= 0)
);

INSERT INTO rank_global_state DEFAULT VALUES;

CREATE TABLE rank_global (
    -- `pid`. Legacy keeps a deleted character's row until the season ends, so there is no
    -- reference to `player`.
    player_id integer PRIMARY KEY CHECK (player_id > 0),
    -- The `rank_global_state` generation the row was written in.
    generation bigint NOT NULL CHECK (generation >= 0),
    -- The Name the character had when it first scored.
    name text NOT NULL CHECK (name ~ '^[A-Za-z0-9]{1,24}$'),
    level integer NOT NULL,
    empire smallint NOT NULL CHECK (empire BETWEEN 0 AND 255),
    -- `type0` .. `type8`.
    counts integer[] NOT NULL CHECK (array_ndims(counts) = 1 AND cardinality(counts) = 9)
);

CREATE TABLE rank_global_reward (
    id uuid PRIMARY KEY DEFAULT uuidv7(),
    -- The counter, `type0` .. `type8`.
    type smallint NOT NULL CHECK (type BETWEEN 0 AND 8),
    -- The place, from 1; a season's end awards places 1 to 10.
    pos integer NOT NULL CHECK (pos > 0),
    vnum integer NOT NULL CHECK (vnum > 0),
    -- `AutoGiveItem` takes a `WORD` count.
    count integer NOT NULL CHECK (count BETWEEN 1 AND 65535)
);

CREATE TABLE rank_global_winner (
    id uuid PRIMARY KEY DEFAULT uuidv7(),
    player_id integer NOT NULL CHECK (player_id > 0),
    name text NOT NULL,
    position integer NOT NULL CHECK (position BETWEEN 1 AND 10),
    type smallint NOT NULL CHECK (type BETWEEN 0 AND 8),
    vnum integer NOT NULL CHECK (vnum > 0),
    count integer NOT NULL CHECK (count BETWEEN 1 AND 65535),
    won_at timestamptz NOT NULL DEFAULT now(),
    done boolean NOT NULL DEFAULT false
);

CREATE INDEX rank_global_winner_unclaimed ON rank_global_winner (player_id) WHERE NOT done;
