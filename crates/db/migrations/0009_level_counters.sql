-- The experience counters the level-up chain reads and writes (ledger section 229m).
--
-- Legacy keeps them in `TPlayerTable` (`common/tables.h`), which `QUERY_PLAYER_LOAD`
-- (`D/ClientManagerPlayer.cpp:275-400`) loads whole and `CHARACTER::SaveReal`
-- (`G/char_save.cpp:284-468`) writes whole: the random hit and spell points the level-ups
-- added (`sRandomHP`, `sRandomSP`), the unspent stat, skill and sub-skill points
-- (`stat_point`, `skill_point`, `sub_skill_point`), and the level step (`level_step`).
ALTER TABLE player
-- `sRandomHP`: the hit points the level-ups rolled. Legacy's own column comment says a
-- level-0 row can hold a negative, so the smallint range is the only bound.
ADD COLUMN random_hp smallint NOT NULL DEFAULT 0,
-- `sRandomSP`, the spell-point counterpart.
ADD COLUMN random_sp smallint NOT NULL DEFAULT 0,
-- `stat_point`: the unspent stat points.
ADD COLUMN stat_point smallint NOT NULL DEFAULT 0 CHECK (stat_point >= 0),
-- `skill_point`: the unspent skill points.
ADD COLUMN skill_point smallint NOT NULL DEFAULT 0 CHECK (skill_point >= 0),
-- `sub_skill_point`: the unspent sub-skill points.
ADD COLUMN sub_skill_point smallint NOT NULL DEFAULT 0 CHECK (sub_skill_point >= 0),
-- `level_step`: the quarters of the next level's cost banked, 0 to 4.
ADD COLUMN level_step smallint NOT NULL DEFAULT 0 CHECK (level_step BETWEEN 0 AND 4);
