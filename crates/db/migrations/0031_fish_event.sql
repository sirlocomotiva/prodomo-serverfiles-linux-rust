-- The fish-event board (ledger section 440).
--
-- Legacy keeps the jigsaw board in the player row (`fish_slots`, a 48-byte blob of
-- `TPlayerFishEventSlot`, and `fishEventUseCount`, `server/server/common/tables.h:635-637`,
-- `server/server/db/ClientManagerPlayer.cpp:224,238,781-788`). Here the set cells are rows of
-- `fish_event_slot` and the use count is one row of `fish_event_count`, both read at select
-- and written with the character (`G/char.cpp:1613-1614`).
CREATE TABLE fish_event_slot (
    player_id integer NOT NULL REFERENCES player (id) ON DELETE CASCADE,
    -- The board cell, 0 to 23 (`FISH_EVENT_SLOTS_NUM`).
    cell integer NOT NULL CHECK (cell >= 0 AND cell < 24),
    -- `bIsMain`: the cell the shape was placed at.
    is_main boolean NOT NULL,
    -- `bShape`: the shape on the cell, 1 to 7. An empty cell is no row.
    shape integer NOT NULL CHECK (shape >= 1 AND shape <= 7),
    PRIMARY KEY (player_id, cell)
);

-- The boxes the character used on its current board (`m_dwFishUseCount`),
-- which the login report reads beside the enable flag.
CREATE TABLE fish_event_count (
    player_id integer PRIMARY KEY REFERENCES player (id) ON DELETE CASCADE,
    use_count integer NOT NULL CHECK (use_count >= 0)
);
