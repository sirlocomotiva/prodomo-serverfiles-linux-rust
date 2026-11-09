-- A character's personal shop and its wares (ledger section 493).
--
-- Legacy keeps these in `player.private_shop` and `player.private_shop_item`
-- (`legacy/sql/schema/player.sql:995-1055`), keyed by the owner's pid: one shop row
-- with the stand, the earnings and the page count, and one row per ware with the
-- item's full instance data (three sockets on disk, because the private-shop
-- tables predate `ENABLE_EXTENDED_SOCKETS`; the instance's six sockets ride the
-- game-side `TPlayerPrivateShopItem`, and only the first three are stored).
--
-- Here the shape follows `item` (`0005_items.sql`), not the MyISAM tables: six
-- sockets and seven attributes as sibling columns, because `world` addresses them
-- by index. The wares keep their global item ids, since a checkout returns the
-- same instance to its owner. The SQL schema is ours (ADR-0001).
CREATE TABLE private_shop (
-- The owning character. One shop per character, as legacy's `PRIMARY KEY
-- (owner_id)` says.
owner_id integer PRIMARY KEY REFERENCES player (id) ON DELETE CASCADE,
-- The raw title bytes, `TPrivateShop::szTitle` (`C/tables.h:1897`). Free text is
-- `bytea` (ADR-0003).
title bytea NOT NULL CHECK (octet_length(title) <= 33),
-- The owner's name as the entity record shows it.
owner_name bytea NOT NULL CHECK (octet_length(owner_name) <= 25),
-- The shop's state: 0 unavailable, 1 closed, 2 open, 3 modify
-- (`EPrivateShopState`, `C/length.h:155-161`).
state smallint NOT NULL CHECK (state BETWEEN 0 AND 3),
-- The stand's polymorph vnum.
vnum bigint NOT NULL CHECK (vnum BETWEEN 0 AND 4294967295),
-- The title's type byte.
title_type smallint NOT NULL CHECK (title_type BETWEEN 0 AND 255),
-- The stand's map and position.
map_index integer NOT NULL,
x integer NOT NULL,
y integer NOT NULL,
-- The Channel the stand is on.
channel smallint NOT NULL CHECK (channel BETWEEN 0 AND 255),
-- The earnings in yang and cheques.
gold bigint NOT NULL CHECK (gold >= 0),
cheque bigint NOT NULL CHECK (cheque BETWEEN 0 AND 4294967295),
-- The page count, 1 or 2 (`PRIVATE_SHOP_PAGE_MAX_NUM`).
page_count smallint NOT NULL CHECK (page_count BETWEEN 1 AND 2),
-- The premium expiry as a unix time, `tPremiumTime`.
premium_time bigint NOT NULL CHECK (premium_time >= 0),
-- The unlocked slots, `wUnlockedSlots`.
unlocked_slots integer NOT NULL CHECK (unlocked_slots >= 0)
);

-- One ware of a personal shop: the instance's full data, addressed by its global
-- item id, at the shop slot the client chose.
CREATE TABLE private_shop_item (
-- The global item id, as `item.id`: the same instance, moved tables.
id bigint PRIMARY KEY CHECK (id > 0 AND id <= 4290000000),
-- The shop it stands in.
owner_id integer NOT NULL REFERENCES private_shop (owner_id) ON DELETE CASCADE,
-- The shop slot, 0 to 127 (`PRIVATE_SHOP_HOST_ITEM_MAX_NUM`).
pos smallint NOT NULL CHECK (pos BETWEEN 0 AND 127),
-- The stack size.
count smallint NOT NULL CHECK (count BETWEEN 1 AND 5000),
-- The prototype.
vnum bigint NOT NULL CHECK (vnum BETWEEN 0 AND 4294967295),
-- The asking price in yang and cheques.
gold bigint NOT NULL CHECK (gold >= 0),
cheque bigint NOT NULL CHECK (cheque BETWEEN 0 AND 4294967295),
-- The checkin time, `tCheckin`.
checkin bigint NOT NULL CHECK (checkin >= 0),
socket0 integer NOT NULL DEFAULT 0,
socket1 integer NOT NULL DEFAULT 0,
socket2 integer NOT NULL DEFAULT 0,
socket3 integer NOT NULL DEFAULT 0,
socket4 integer NOT NULL DEFAULT 0,
socket5 integer NOT NULL DEFAULT 0,
attrtype0 smallint NOT NULL DEFAULT 0 CHECK (attrtype0 BETWEEN 0 AND 255),
attrvalue0 smallint NOT NULL DEFAULT 0,
attrtype1 smallint NOT NULL DEFAULT 0 CHECK (attrtype1 BETWEEN 0 AND 255),
attrvalue1 smallint NOT NULL DEFAULT 0,
attrtype2 smallint NOT NULL DEFAULT 0 CHECK (attrtype2 BETWEEN 0 AND 255),
attrvalue2 smallint NOT NULL DEFAULT 0,
attrtype3 smallint NOT NULL DEFAULT 0 CHECK (attrtype3 BETWEEN 0 AND 255),
attrvalue3 smallint NOT NULL DEFAULT 0,
attrtype4 smallint NOT NULL DEFAULT 0 CHECK (attrtype4 BETWEEN 0 AND 255),
attrvalue4 smallint NOT NULL DEFAULT 0,
attrtype5 smallint NOT NULL DEFAULT 0 CHECK (attrtype5 BETWEEN 0 AND 255),
attrvalue5 smallint NOT NULL DEFAULT 0,
attrtype6 smallint NOT NULL DEFAULT 0 CHECK (attrtype6 BETWEEN 0 AND 255),
attrvalue6 smallint NOT NULL DEFAULT 0
);

-- A shop's grid is addressed by slot, and a slot may hold only one ware.
CREATE UNIQUE INDEX private_shop_item_slot_key ON private_shop_item (owner_id, pos);
