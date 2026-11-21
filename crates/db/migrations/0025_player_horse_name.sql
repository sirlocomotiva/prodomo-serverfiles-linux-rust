-- The character's horse name (ledger section 395).
--
-- Legacy keeps it in `horse_name(id, name)` (`player.sql`), written by `REPLACE INTO` on
-- `HorseNameManager::UpdateHorseName` (`D/ClientManagerHorseName.cpp:5-13`) and read at
-- enter-game (`HEADER_GD_REQ_HORSE_NAME`, `G/input_login.cpp:891-892`). The name is a
-- 24-byte `char` array (`CHARACTER_NAME_MAX_LEN`, `C/length.h:15`), so the Rewrite keeps it
-- as `bytea` of at most 24 bytes on the player row; an empty name is no name, as a missing
-- row is in legacy.
ALTER TABLE player
ADD COLUMN horse_name bytea NOT NULL DEFAULT '\x' CHECK (octet_length(horse_name) <= 24);
