-- The silk bundle's price list (`MYSHOP_PRICE_LIST`).
--
-- Legacy keeps it in `myshop_pricelist` (`legacy/sql/schema/player.sql:744-749`): one row
-- per `(owner_id, item_vnum)` with the unit price. A bundle shop opened while the seller
-- holds the silk bundle 71049 sends the shop's unit prices (`G/char.cpp:915-937`), the DB
-- cache merges them in front of the stored ones up to `SHOP_PRICELIST_MAX_NUM` (40,
-- `CItemPriceListTableCache::UpdateList`, `server/server/db/Cache.cpp:226-277`), and the
-- botary's first use reads them back in that order (`MyshopPricelistRequest`,
-- `server/server/db/ClientManager.cpp:2273-2303`). A character's deletion deletes its rows
-- (`ClientManagerPlayer.cpp:1539`).
--
-- Here `slot` keeps the merged order, which legacy's cache holds and MyISAM's insertion
-- order mirrors, and `price` is a bigint: legacy wrapped the unit price at 32 bits (a
-- Defect the game fixes).
CREATE TABLE myshop_pricelist (
    owner_id integer NOT NULL REFERENCES player (id) ON DELETE CASCADE,
    slot smallint NOT NULL CHECK (slot BETWEEN 0 AND 39),
    item_vnum bigint NOT NULL CHECK (item_vnum BETWEEN 0 AND 4294967295),
    price bigint NOT NULL CHECK (price >= 0),
    PRIMARY KEY (owner_id, slot),
    UNIQUE (owner_id, item_vnum)
);
