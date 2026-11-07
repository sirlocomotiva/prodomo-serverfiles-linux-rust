-- The premium-players flag and its timer (ledger section 447).
--
-- Legacy keeps them in `player` (`player.sql:870-871`): `premium`, a smallint
-- defaulting to 0, and `premium_time`, a bigint defaulting to 0. The game reads
-- them as a `BYTE` and a `long` (`TPlayerTable`, `C/tables.h:643-644`): the flag
-- is 1 while premium runs, and the timer is the `time(0)` it ends at
-- (`G/char.cpp:11591`). The flag keeps the byte range the setter keeps; the
-- timer is 32-bit here, as the game holds it. The activation needs two MD
-- vouchers (80015); the four affects (549-552) lay 2000 max HP, 15 monster, 10
-- boss and 10 metin bonus while the flag holds.
ALTER TABLE player
ADD COLUMN premium smallint NOT NULL DEFAULT 0 CHECK (premium BETWEEN 0 AND 255),
ADD COLUMN premium_time integer NOT NULL DEFAULT 0;
