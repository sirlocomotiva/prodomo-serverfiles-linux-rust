-- A character's quickslots (ledger section 219).
--
-- Legacy keeps them in `TPlayerTable::quickslot[QUICKSLOT_MAX_NUM]` (`common/tables.h:615`), 36
-- packed `TQuickslot { BYTE type; BYTE pos; }` records, and writes the array as the `quickslot`
-- blob of the player row. Here each set slot is a row, and an empty slot is no row, because
-- legacy's empty slot is the zeroed record `DelQuickslot` writes (`G/char_quickslot.cpp:107`)
-- and `SetQuickslot` refuses type 0 at the load (`G/input_db.cpp:439-440`), so it is never
-- sent.

CREATE TABLE quickslot (
    player_id integer NOT NULL REFERENCES player (id) ON DELETE CASCADE,
    -- The slot's index, below `QUICKSLOT_MAX_NUM` (36, `common/length.h:51`).
    slot smallint NOT NULL CHECK (slot BETWEEN 0 AND 35),
    -- `QUICKSLOT_TYPE_ITEM` (1), `QUICKSLOT_TYPE_SKILL` (2) or `QUICKSLOT_TYPE_COMMAND` (3)
    -- (`common/length.h:376-380`). `QUICKSLOT_TYPE_NONE` is the absent row.
    kind smallint NOT NULL CHECK (kind BETWEEN 1 AND 3),
    -- What the slot names: an inventory cell, a skill or a command. A `BYTE`.
    pos smallint NOT NULL CHECK (pos BETWEEN 0 AND 255),
    PRIMARY KEY (player_id, slot)
);
