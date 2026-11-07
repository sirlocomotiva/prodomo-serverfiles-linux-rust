-- The biologist mission counters (ledger section 445).
--
-- Legacy keeps them in `player` (`player.sql`): `biologist_state`, `biologist_items_taken`
-- and `biologist_completed`, each a smallint defaulting to 0. The game reads and writes
-- them as unsigned (`TPlayerTable`, `C/tables.h:651-653`), and the point arms clamp the
-- state to mission 9 (`G/char.cpp:4731-4751`). Here the columns are the legacy ones,
-- with the mission behind the check the game relies on; the other two counters hold any
-- smallint the legacy DWORD arms could leave behind a hand-written row.
ALTER TABLE player
ADD COLUMN biologist_state smallint NOT NULL DEFAULT 0 CHECK (biologist_state BETWEEN 0 AND 9),
ADD COLUMN biologist_items_taken smallint NOT NULL DEFAULT 0 CHECK (biologist_items_taken >= 0),
ADD COLUMN biologist_completed smallint NOT NULL DEFAULT 0 CHECK (biologist_completed >= 0);
