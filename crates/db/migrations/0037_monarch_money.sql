-- The monarch treasuries (ledger section 459).
--
-- Legacy keeps four `money` cells (`TMonarchInfo.money`, an `int64_t[4]`,
-- `C/tables.h:1574-1580`), one per empire slot 0 to 3, in memory on the game and the DB
-- servers, and persists them in the `monarch` table. The game reads a cell as an `int`
-- (`CMonarch::GetMoney`, `G/monarch.cpp:175-182`), 0 for a slot past 3, and the DB
-- server refuses a move that would leave a cell below 0 or above 2000000000
-- (`D/Monarch.cpp:122-147`). Here the four cells are rows, seeded at 0, behind the
-- checks the game relies on.
CREATE TABLE monarch_money (
-- The empire slot, 0 to 3 (`_countof(m_MonarchInfo.money)`).
empire smallint PRIMARY KEY CHECK (empire BETWEEN 0 AND 3),
-- The treasury, within the moves the DB server allows.
money bigint NOT NULL DEFAULT 0 CHECK (money BETWEEN 0 AND 2000000000)
);
INSERT INTO monarch_money (empire) VALUES (0), (1), (2), (3);
