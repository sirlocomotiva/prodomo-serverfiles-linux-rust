-- A character's battle-pass missions (ledger section 473).
--
-- Legacy keeps the ten `TPlayerBattlePass` rows in the player table blob
-- (`tab.battlePass`, `C/tables.h:663`): daily in slots 0-4, weekly in 5-9,
-- written only when both buckets hold at least five (`G/char.cpp:1635-1650`).
-- Here each mission row is its own row, keyed by slot 0-9; a character that
-- never opened the pass holds no rows, which loads as an empty mission set.
-- The quest flags (`battlepass.points`, `battlepass.isPremium`) ride the
-- existing `quest_flag` rows.
CREATE TABLE battle_pass (
player_id integer NOT NULL REFERENCES player (id) ON DELETE CASCADE,
-- The stored slot, 0-4 daily and 5-9 weekly, as `CreatePlayerProto` lays them.
slot smallint NOT NULL CHECK (slot BETWEEN 0 AND 9),
-- `missionID`.
mission_id integer NOT NULL CHECK (mission_id BETWEEN 0 AND 65535),
-- `progress`.
progress bigint NOT NULL CHECK (progress >= 0),
-- `type`: 0 daily, 1 weekly.
kind smallint NOT NULL CHECK (kind BETWEEN 0 AND 1),
-- `endTime`.
end_time bigint NOT NULL CHECK (end_time >= 0),
PRIMARY KEY (player_id, slot)
);
