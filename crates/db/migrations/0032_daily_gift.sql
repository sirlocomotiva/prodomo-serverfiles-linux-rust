-- A character's daily-gift week (ledger section 441).
--
-- Legacy keeps one `daily_gift` row per character (`G/char_daily_gifts.cpp:251-266`):
-- the player, the week, the seven day statuses, and the renew time as the `datetime`.
-- The load's row counts only for the current week while its renew time is past the
-- week's start (`SELECT ... WHERE player_id AND week AND UNIX_TIMESTAMP(datetime) >
-- start_time`, `:241-244`); any other row is replaced by a fresh one with day 1
-- waiting. Here the columns are the legacy ones, with the week and the statuses
-- smallints behind the checks the game relies on.
CREATE TABLE daily_gift (
player_id integer PRIMARY KEY REFERENCES player (id) ON DELETE CASCADE,
-- The week the statuses belong to (`daily_gift_week`).
week smallint NOT NULL CHECK (week BETWEEN 0 AND 255),
-- The seven day statuses: 0 locked, 1 waiting, 2 collected
-- (`DAILY_GIFT_COLLECT_LOCK/WAIT/SUCCESS`, `C/length.h`).
day1 smallint NOT NULL DEFAULT 0 CHECK (day1 BETWEEN 0 AND 2),
day2 smallint NOT NULL DEFAULT 0 CHECK (day2 BETWEEN 0 AND 2),
day3 smallint NOT NULL DEFAULT 0 CHECK (day3 BETWEEN 0 AND 2),
day4 smallint NOT NULL DEFAULT 0 CHECK (day4 BETWEEN 0 AND 2),
day5 smallint NOT NULL DEFAULT 0 CHECK (day5 BETWEEN 0 AND 2),
day6 smallint NOT NULL DEFAULT 0 CHECK (day6 BETWEEN 0 AND 2),
day7 smallint NOT NULL DEFAULT 0 CHECK (day7 BETWEEN 0 AND 2),
-- The week's renew time, the `datetime` legacy's `REPLACE` writes
-- (`FROM_UNIXTIME(renew)`, `:258`).
renew_time bigint NOT NULL DEFAULT 0
);
