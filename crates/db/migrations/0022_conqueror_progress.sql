-- The conqueror level's progress (ledger section 371).
--
-- Legacy keeps both in `TPlayerTable` under `__CONQUEROR_LEVEL__` (`common/tables.h`):
-- `conqueror_level_step`, the quarters of the next conqueror level's cost banked, and
-- `conqueror_point`, the unspent points `/conqueror_point` moves into a sungma slot.
-- `CHARACTER::Save` writes them from `POINT_CONQUEROR_LEVEL_STEP` and
-- `POINT_CONQUEROR_POINT` (`G/char.cpp:1620-1628`); `SetPlayerProto` reads them back
-- (`:2337-2340`). A fresh row holds none.
ALTER TABLE player
-- `conqueror_level_step`: 0 to 4, as `level_step`.
ADD COLUMN conqueror_level_step smallint NOT NULL DEFAULT 0
    CHECK (conqueror_level_step BETWEEN 0 AND 4),
-- `conqueror_point`: the unspent conqueror points.
ADD COLUMN conqueror_point smallint NOT NULL DEFAULT 0 CHECK (conqueror_point >= 0);
