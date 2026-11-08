-- The reward window's rows (ledger section 462).
--
-- Legacy keeps the solo counts in one file per pid
-- (`/root/pd2_game/srv1/share/reward_solo/%u`, `G/char.cpp:12991-13049`) and the
-- global first-claim names in one file per mission
-- (`reward_global/%u`, `G/char_manager.cpp:1831-1863`), both read and written as
-- plain text, and neither in the SQL schema. Here the counts are rows of
-- `reward_solo` (pid, mission, count) and the claims are rows of
-- `reward_global` (mission, name), behind the checks the world relies on.
CREATE TABLE reward_solo (
-- The character's pid.
player_id integer NOT NULL CHECK (player_id > 0),
-- The solo mission, 0 to 17 (`m_mapRewardSolo`, `G/char_manager.cpp:1566-1782`).
mission smallint NOT NULL CHECK (mission BETWEEN 0 AND 17),
-- The count, within the table's clamps (the largest max is 100000).
count integer NOT NULL DEFAULT 0 CHECK (count BETWEEN 0 AND 100000),
PRIMARY KEY (player_id, mission)
);
CREATE TABLE reward_global (
-- The global mission, 0 to 16 (`m_mapRewardGlobal`, `G/char_manager.cpp:1413-1561`);
-- the source names 1 twice, so the map holds 0-16 with the second 1 lost.
mission smallint PRIMARY KEY CHECK (mission BETWEEN 0 AND 16),
-- The first claimer's name, as `SetGlobalReward` writes it.
name bytea NOT NULL
);
