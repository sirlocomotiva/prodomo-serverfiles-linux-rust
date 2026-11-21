-- The protected-inventory lock (ledger section 446).
--
-- Legacy keeps it in `player` (`player.sql:872-873`): `secured`, a smallint
-- defaulting to 0, and `secured_password`, a `varchar(6)` defaulting to '999999'.
-- The game reads them as a `BYTE` and an `int` (`TPlayerTable`, `C/tables.h:647-648`)
-- and narrows any stored state through the `BYTE` setter (`G/char.h:2598`). Here
-- the password is an integer, as the game holds it; the state keeps the byte range
-- the setter keeps. The lock refuses the twenty `IsSecured` sites while nonzero.
ALTER TABLE player
ADD COLUMN secured smallint NOT NULL DEFAULT 0 CHECK (secured BETWEEN 0 AND 255),
ADD COLUMN secured_password integer NOT NULL DEFAULT 999999;
