-- The inventory unlocks (ledger section 449).
--
-- Legacy keeps them in `player` (`player.sql:866`): `envanter`, an int
-- defaulting to 0. The game reads it as an `INT` (`TPlayerTable`,
-- `C/tables.h:633`) into `m_points.envanter` (`G/char.cpp:2346`) and clamps
-- nothing: the unlock is the stat `Update_Inven` raises (`G/char.cpp:11763`)
-- and the burst sends (`:2051`). The Rewrite holds no more than the cover
-- count (18, `INVENTORY_LOCK_COVER_COUNT`), which is the legacy text the
-- `Update_Inven` guard and the `PointChange(POINT_INVEN)` refusal share, so
-- the check keeps the same range.
ALTER TABLE player
ADD COLUMN envanter integer NOT NULL DEFAULT 0 CHECK (envanter BETWEEN 0 AND 18);
