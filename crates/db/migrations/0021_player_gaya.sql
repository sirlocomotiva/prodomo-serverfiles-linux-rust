-- The gaya currency `/set gaya` spends (ledger section 352).
--
-- Legacy keeps it in `TPlayerTable.gaya` (`common/tables.h:217`), an unsigned int the
-- login query (`D/ClientManagerPlayer.cpp:440,731`) loads whole and `CHARACTER::Save`
-- (`G/char.cpp`) writes from `POINT_GAYA`. The point arm refuses at `GAYA_MAX`
-- (999999, `common/length.h:104`); a fresh row holds none.
ALTER TABLE player
ADD COLUMN gaya integer NOT NULL DEFAULT 0 CHECK (gaya BETWEEN 0 AND 999999);
