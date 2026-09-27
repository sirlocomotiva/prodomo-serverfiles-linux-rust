-- The state the loading phase and the game phase read (ledger section 187).
--
-- The select screen only needed the columns of `0002` and `0003`. A character that has been
-- selected needs the rest of what `CHARACTER::PointsPacket` sends: the experience it has
-- banked, the gold it carries, the conqueror experience, and the voice its points record
-- holds. Legacy keeps them in `TPlayerTable` (`common/tables.h`), which `QUERY_PLAYER_LOAD`
-- (`D/ClientManagerPlayer.cpp:275-400`) loads whole.

ALTER TABLE player
    -- Experience banked towards the next level. Legacy is `long long`; the next-level cost is
    -- a separate read of the compiled-in table (`common::levels`), never a stored column, so a
    -- row written by hand cannot disagree with its level.
    ADD COLUMN exp bigint NOT NULL DEFAULT 0,
    -- Conqueror experience (`__CONQUEROR_LEVEL__` is on).
    ADD COLUMN conqueror_exp bigint NOT NULL DEFAULT 0,
    -- Gold carried. `ENABLE_REMOVE_LIMIT_GOLD` is on, so legacy's `TPlayerTable::gold` and
    -- `CHARACTER::GetGold()` are `unsigned long long` and cannot be negative; gold is
    -- therefore checked at or above zero here.
    --
    -- The legacy field is unsigned 64-bit. A PostgreSQL `bigint` is signed, so this column
    -- holds the low 63 bits of that range. A value a `bigint` cannot hold is refused by the
    -- check rather than wrapped: the store never clamps a currency value silently.
    ADD COLUMN gold bigint NOT NULL DEFAULT 0 CHECK (gold >= 0),
    -- `POINT_VOICE`. Legacy never writes this slot of `TPacketGCPoints` and sends stack
    -- garbage in its place; the Rewrite sends the real value instead, so the value has to
    -- exist. Recorded as a Defect in `docs/PROTOCOL_NOTES.md`.
    ADD COLUMN voice smallint NOT NULL DEFAULT 0 CHECK (voice BETWEEN 0 AND 255);
