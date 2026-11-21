-- The map the character was last saved on (ledger section 239).
--
-- Legacy's player row holds `lMapIndex` (`TPlayerTable`, `G/tables.h`): the map the
-- character stands on. It matters for a private map, whose index names the copy of a base
-- map a guild war is fought on (`SECTREE_MANAGER::CreatePrivateMap`,
-- `G/sectree_manager.cpp:943-1026`): the position of a private map lies in the base map's
-- coordinates, so the position alone cannot name the copy the character stands on.
--
-- The Rewrite derived the map from the saved position, which is the atlas's answer
-- (`SECTREE_MANAGER::GetMapIndex`, `G/sectree_manager.cpp:666-687`) and therefore always the
-- public map. This column carries the index the character was saved on, and the load reads it
-- first, as legacy's `PlayerLoad` does (`G/input_db.cpp:335-354`).
--
-- The exit location (`lExitX`, `lExitY`, `lExitMapIndex`, `TPlayerTable`, `G/tables.h:596-597`)
-- is the place the character returns to when it leaves a private map: `SaveExitLocation`
-- (`G/char.cpp:6679-6683`) holds it in the character's memory, and `CHARACTER::Save`
-- (`G/char.cpp:1567-1578`) writes it beside the position. A character whose row names a
-- private map comes back here instead, because the private map is gone with the process
-- that held it (`G/char.cpp:2289-2294`).

ALTER TABLE player
    ADD COLUMN map_index integer NOT NULL DEFAULT 0 CHECK (map_index >= 0),
    ADD COLUMN exit_x integer NOT NULL DEFAULT 0,
    ADD COLUMN exit_y integer NOT NULL DEFAULT 0,
    ADD COLUMN exit_map_index integer NOT NULL DEFAULT 0 CHECK (exit_map_index >= 0);
