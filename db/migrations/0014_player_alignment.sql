-- The character's alignment (ledger section 234).
--
-- Legacy keeps two values in `CHARACTER`: `m_iAlignment`, the one the client is shown, and
-- `m_iRealAlignment`, the one the rules read (`G/char.h:1444-1445`). The store holds one
-- column, `player.alignment` (`legacy/sql/schema/player.sql:847`), and `SetPlayerProto` puts
-- it in both values at the load (`G/char.cpp:2244-2245`) while `SaveReal` writes the real one
-- back (`G/char.cpp:1546`).
--
-- The two legacy values differ only while the hide-alignment unique (70048,
-- `UNIQUE_ITEM_HIDE_ALIGNMENT_TITLE`, `G/game/unique_item.h:28`) is worn: it calls
-- `ShowAlignment(false)`, which shows a zero alignment while the rules still read the real
-- one (`G/char_battle.cpp:3457-3475`). The Rewrite has one value, so the shown value and
-- the real value are the same number, which is what the store keeps. The hide is not
-- ported (ledger 234).
ALTER TABLE player
-- `tab.lAlignment`: `UpdateAlignment` clamps it to -200000..200000
-- (`G/char_battle.cpp:3486`), and the check refuses a row a hand wrote past that.
ADD COLUMN alignment integer NOT NULL DEFAULT 0 CHECK (alignment BETWEEN -200000 AND 200000);
