-- The character's horse and the time it was last saved (ledger section 392).
--
-- Legacy keeps the horse in `TPlayerTable.horse` (`THorseInfo`, `common/tables.h:473-480`)
-- and the horse-skill points in `TPlayerTable.horse_skill_point`; the login query
-- (`D/ClientManagerPlayer.cpp:475-481`) reads them with
-- `UNIX_TIMESTAMP(NOW())-UNIX_TIMESTAMP(last_play)` as the logoff interval, and the save
-- (`:122-146`) writes them with `last_play = NOW()`. The level indexes the 31 rows of
-- `c_aHorseStat`, so it is kept to 0..30; the drop time is a `DWORD` of seconds.
ALTER TABLE player
ADD COLUMN horse_level smallint NOT NULL DEFAULT 0 CHECK (horse_level BETWEEN 0 AND 30),
ADD COLUMN horse_riding boolean NOT NULL DEFAULT false,
ADD COLUMN horse_hp smallint NOT NULL DEFAULT 0,
ADD COLUMN horse_hp_droptime bigint NOT NULL DEFAULT 0
    CHECK (horse_hp_droptime BETWEEN 0 AND 4294967295),
ADD COLUMN horse_stamina smallint NOT NULL DEFAULT 0,
ADD COLUMN horse_skill_point smallint NOT NULL DEFAULT 0,
ADD COLUMN last_play_at timestamptz;
