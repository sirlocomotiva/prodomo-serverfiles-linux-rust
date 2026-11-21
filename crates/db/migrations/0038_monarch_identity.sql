-- The monarch identities (ledger section 460).
--
-- Legacy keeps the `pid` and `name` cells (`TMonarchInfo`, `C/tables.h:1574-1580`), one
-- per empire slot 0 to 3, in memory on the game and the DB servers, and persists them in
-- the `monarch` table (`player.sql:697-703`), the name read back through a join on
-- `player`. Here the four cells are rows on `monarch_money` (0 is no monarch, as the
-- zeroed struct is), the name read back through `player` the same way.
ALTER TABLE monarch_money
ADD COLUMN pid integer NOT NULL DEFAULT 0 CHECK (pid BETWEEN 0 AND 2147483647);
