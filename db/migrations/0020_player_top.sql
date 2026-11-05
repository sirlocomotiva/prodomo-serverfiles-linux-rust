-- The top-player rows `SetTopPlayerEffect` keeps (ledger section 313).
--
-- Legacy keeps them in `player_top` (`player.sql:974-985`): one row per
-- character that reached `TOP_PLAYER_MAX_LEVEL` (120) while fewer than three
-- rows stood, with its PID and the level. The first, second and third row
-- grant `AFFECT_PLAYER_TOP_1`, `_2` and `_3`. The level column stays: legacy
-- writes the reached level, and every row but a hand-written one holds 120.
CREATE TABLE player_top (
    id uuid PRIMARY KEY DEFAULT uuidv7(),
    player_id integer NOT NULL REFERENCES player(id) ON DELETE CASCADE,
    level integer NOT NULL CHECK (level >= 1),
    UNIQUE (player_id)
);
