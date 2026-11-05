-- The stat-reset count `/stat-` spends (ledger section 293).
--
-- Legacy keeps it in `TPlayerTable.stat_reset_count` (`common/tables.h:625`), which
-- `QUERY_PLAYER_LOAD` (`D/ClientManagerPlayer.cpp:451`) loads whole and `CHARACTER::Save`
-- (`G/char.cpp:1592`) writes from `POINT_STAT_RESET_COUNT`. The reset count is the
-- only counter `/stat-` reads; a fresh row holds none.
ALTER TABLE player
ADD COLUMN stat_reset_count smallint NOT NULL DEFAULT 0 CHECK (stat_reset_count >= 0);
