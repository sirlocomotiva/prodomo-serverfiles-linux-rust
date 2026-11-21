-- One item instance (ledger section 196).
--
-- Legacy `player.item` (legacy/sql/schema/player.sql:351-395) is MyISAM, latin1, and carries
-- the item's six sockets and seven attributes as six and seven sibling columns. This table keeps
-- that shape, because the shape is the record: `world::character::items` addresses a socket and an
-- attribute by index, and a normalised child table would mean a second set of ordinals to keep in
-- step. The index is the only thing that names them, and it is named in one place
-- (`common::constants`).
--
-- The SQL schema is ours to redesign (ADR-0001), so the things that changed and why are listed
-- under the table. Nothing here restates a constant that `common` already measures: every CHECK
-- below is on a bound the legacy *field* enforces, not on one `common` derives.

CREATE TABLE item (
    -- The id the client sees (`TPacketGCItemSet::dwID`): 32 bits, and the value 0 is `NO_ITEM`,
    -- so it is never 0. It is assigned from the item-ID range pool (`db::item_id_range`), not
    -- generated, because the pool owns the allocation and an identity would collide with it.
    --
    -- `MAX_ITEM_ID` is 4,290,000,000 (`ItemIDRangeManager::cs_dwMaxItemID` (`ItemIDRangeManager.h:8`)), so the ceiling is
    -- that, not the unsigned-32 ceiling.
    -- `bigint`, and **not** `integer`, which cannot hold the value: PostgreSQL's `integer` is
    -- signed 32-bit and `4290000000 > 2147483647`. Probed, not reasoned: an insert of
    -- `2141000001` -- a legal pool id -- into an `integer` column returns `integer out of range`
    -- from PostgreSQL 18, and so does the constant itself. This does not contradict
    -- ADR-0003's "IDs the client sees stay 32-bit integers": that is the **unsigned** 32-bit
    -- space, which is what `TPacketGCItemSet::dwID` is on the wire, and which the frozen 32-bit
    -- target's `DWORD` satisfied. A signed column would not.
    id bigint PRIMARY KEY CHECK (id > 0 AND id <= 4290000000),

    -- The owning **character**, or NULL for an item lying on the ground. Legacy has the same
    -- information in `owner_id` but no way to express "nobody owns this": it uses 0, and
    -- `CItem::GetOwner` walks the character manager looking for it. Here the absence is a NULL,
    -- which is why the last CHECK in this table can be a real biconditional.
    --
    -- The foreign key is deliberate and it is a difference from legacy. Legacy's `owner_id` is
    -- **polymorphic**: the award path writes an *account* id into it
    -- (`db/ClientManager.cpp:997-1008`, `pi->account_id`) for a safebox or mall row, and a
    -- character id for everything else, with nothing in the schema distinguishing the two. A
    -- typed reference cannot express that, which is the point: an awarded item belongs to a
    -- safebox, a safebox needs its own table, and `sys.item.safebox` is a later unit. Until that
    -- table exists this column means exactly one thing.
    owner_id integer REFERENCES player (id) ON DELETE CASCADE,

    -- The window byte (`EWindows`, `length.h:657-677`: RESERVED_WINDOW 0 .. GROUND 10, eleven
    -- members with `__ATTR_6TH_7TH__`, `__AURA_SYSTEM__` and `ENABLE_SWITCHBOT` all live at
    -- `prodomodefines.h:31,30,191`). The range is checked; the per-window meaning of `pos` is
    -- **not**, because it is not one rule. Five of the eleven windows name a cell of the flat
    -- array, `DRAGON_SOUL_INVENTORY` names a cell of the other array, `ATTR67_ADD` and
    -- `SWITCHBOT` name their own one- and five-element windows, `SAFEBOX` names a cell of a
    -- container this table does not have yet, and `GROUND` names a world position. A single range
    -- check would be wrong for two of them, so the rule lives in `common::item_slots`, where it is
    -- measured and tested, and this column only refuses a byte the enum cannot hold.
    -- Named `window_type` and not `window`, because `window` is a reserved word
    -- (SQL:2011 window functions) and PostgreSQL refuses it unquoted -- caught by
    -- running the migration, not by reading it. It matches `ItemPos::window_type`
    -- and `common::item_slots::stored_window`, so the vocabulary is one word.
    window_type smallint NOT NULL CHECK (window_type BETWEEN 0 AND 10),

    -- The cell, or whatever the window says it is. `integer` and not `smallint`: the field is
    -- `TPlayerItem::pos`, a `WORD` (`tables.h:436`), so a value of 32768 is representable in the
    -- game and in legacy's own `smallint(5) unsigned` column (`player.sql:355`) -- and a
    -- PostgreSQL `smallint` rejects it (`smallint out of range`, probed). `item_slots` is the
    -- only caller that knows the bound, and it is under 1370, so this column is deliberately the
    -- wider of the two.
    pos integer NOT NULL CHECK (pos >= 0),

    -- The stack size. `TItemData::count` is a `WORD` (`packet.h:2974`), so a value above 65535 is
    -- not representable at all; and `ITEM_MAX_COUNT` is 5000 (`common::constants`), which is the
    -- bound the game enforces. Zero is refused, because an empty stack is not an item: legacy's
    -- `SetCount` treats 0 as a destruction request (`item.cpp:307-338`), so a stored 0 would be a
    -- row the load path has to interpret as "delete me".
    count smallint NOT NULL CHECK (count BETWEEN 1 AND 5000),

    -- The prototype this instance was made from (`TItemData::vnum`, `packet.h:2973`). A `DWORD`,
    -- and 0 is the "no prototype" value the load path has to notice, so it is allowed here and
    -- refused in Rust where the prototype table can answer.
    -- `bigint` for the same reason as `id`: the field is a `DWORD` (`tables.h:439`) and legacy's
    -- column is `int(11) unsigned` (`player.sql:357`). A `DWORD` of 3000000000 is a legal item
    -- vnum and a PostgreSQL `integer` refuses it (probed).
    vnum bigint NOT NULL CHECK (vnum BETWEEN 0 AND 4294967295),

    -- `TItemData::dwRefineElement` (`packet.h:2976`), under `ENABLE_REFINE_ELEMENT`
    -- (`prodomodefines.h:28`). A `DWORD` that this build does not bound anywhere: `EItemElement`
    -- does not exist in the frozen tree (swept with a positive control for `GetRefineElement` and
    -- a negative control for `EItemElement`), so the element names live in the Game data tables
    -- and a range check here would be a guess. Only the field's own width is checked.
    refine_element bigint NOT NULL DEFAULT 0 CHECK (refine_element BETWEEN 0 AND 4294967295),

    -- `TItemData::transmutation` (`packet.h:2979`), under `__CHANGELOOK_SYSTEM__`
    -- (`prodomodefines.h:16`). Also a `DWORD` vnum.
    transmutation bigint NOT NULL DEFAULT 0 CHECK (transmutation BETWEEN 0 AND 4294967295),

    -- `TItemData::flags` and `anti_flags` (`packet.h:2981-2982`), two independent bitmaps that
    -- are **not** inverses of each other. Both reach the client: `TPacketGCItemSet`
    -- (`packet.h:1439-1440`) carries them, which is two of the four bytes that make that record
    -- 72 wide, and `protocol/src/gc_item_window.rs` encodes them at offsets 18 and 22. The
    -- narrower `TPacketGCItemUpdate` (`packet.h:1470-1484`) does not, which is why the 59-byte
    -- update has no place to change them and a flag change needs a full set.
    --
    -- The bit layout lives in `ITEM_FLAG` and `ITEM_ANTIFLAG` (`item_length.h:500+`), which this
    -- migration does not restate: the columns are bitmaps, so a range check would be meaningless
    -- and a per-bit check is a transcription of that enumeration waiting to drift.
    -- `bigint` again, and here bit 31 set is the *ordinary* case rather than an edge: these are
    -- `DWORD` bitmaps and half of every bitmap's range is above 2147483648. Sizing these columns
    -- from legacy's `int(11) unsigned` (which is 4 unsigned bytes) as a PostgreSQL `integer`
    -- (which is 4 signed bytes) would silently refuse half of all flags.
    flags bigint NOT NULL DEFAULT 0 CHECK (flags BETWEEN 0 AND 4294967295),
    anti_flags bigint NOT NULL DEFAULT 0 CHECK (anti_flags BETWEEN 0 AND 4294967295),

    -- `TItemData::alSockets[6]` (`packet.h:2983`), **signed**. Legacy's column is
    -- `int(10) unsigned` (`player.sql:360-365`), which does not match the field it stores:
    -- Sockets 0 and 1 hold a `time_t` and a first-used marker, and a socket can be **negative**,
    -- so these columns are signed. The path is the limit dispatch loop
    -- (`item_manager.cpp:280-318`), which writes socket 0 from a limit value with no floor on the
    -- result: `SetSocket(0, time(0) + item->GetLimitValue(i))` at line 287, and
    -- `SetSocket(0, duration)` at line 315, where `duration` is `GetSocket(0)` or
    -- `GetLimitValue(i)` and the only substitution is for `0`, not for a negative value. Both
    -- operands are a signed `long` and `time(0)` is 4 bytes on the 32-bit target, so a prototype
    -- whose `LIMIT_VALUE` is very negative persists a negative socket 0. An unsigned column would
    -- refuse that row or wrap it to a 32-bit unsigned value that the comparison against
    -- `time(0)` would then read as "far in the future".
    --
    -- This is a deliberate difference from legacy's DDL, not an oversight: legacy declares
    -- `int(10) unsigned` (`player.sql:360-365`) for a field its own code writes signed.
    --
    -- Note for the next reader, because the two tables disagree: `player.item` defaults a socket
    -- to **0** and `player.item_proto` defaults it to **-1** (`player.sql:524-529`). Zero is the
    -- instance's empty-socket marker, because `CItem::GetSocketCount` (`item.cpp:1776-1784`) counts
    -- up to the first socket that is 0. The prototype's -1 is never read, because
    -- `TItemTable::alSockets` is never assigned by the loader and the array is zeroed at
    -- `ClientManagerBoot.cpp:592,1581`. Here 0 is the default for both, and the empty marker is
    -- stated once, here.
    socket0 integer NOT NULL DEFAULT 0,
    socket1 integer NOT NULL DEFAULT 0,
    socket2 integer NOT NULL DEFAULT 0,
    socket3 integer NOT NULL DEFAULT 0,
    socket4 integer NOT NULL DEFAULT 0,
    socket5 integer NOT NULL DEFAULT 0,

    -- `TItemData::aAttr[7]` (`TPlayerItemAttribute`, `packet.h:2984`) as seven `(type, value)`
    -- pairs, under `ITEM_ATTRIBUTE_MAX_NUM` = 7. `TPlayerItemAttribute` is
    -- `{ BYTE bType; short sValue; }` (`tables.h:426-430`), so the type is an **unsigned** byte
    -- and the value is a signed 16-bit -- which is exactly what `smallint` is, on both counts.
    -- A type of 0 means the slot is empty, which is why 0 is allowed and why the value is
    -- allowed to be anything at the same time.
    --
    -- The range is **0..255, not -128..127**, and that is a deliberate difference from legacy's
    -- column. `player.attrtypeN` is a MySQL `tinyint`, so legacy's *table* can hold a negative
    -- type, while legacy's *field* cannot: the load does `str_to_number(bType, row[cur++])`
    -- (`db/ClientManagerPlayer.cpp:60`) into a `BYTE`, so a stored -1 arrives as 255. This
    -- column refuses the negative value instead of wrapping it, which turns a corrupt row into
    -- an error rather than into a real attribute. (`attrtype0 BETWEEN -128 AND 127` was in the
    -- first draft of this migration; it is what PostgreSQL enforces for a signed `smallint`, and
    -- it was sizing the column from legacy's SQL type rather than from the C++ field it stores.)
    --
    -- Legacy's table also has `apply_path`/`apply_value`/`apply_type` for attributes 0..3 only
    -- (`player.sql:367-378`). This table does not have them yet, because
    -- `world::item::ItemAttribute` has no apply triple and the apply system is `sys.item.attr`,
    -- which is a later unit. A later migration adds them; it must not edit this one.
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
    attrvalue6 smallint NOT NULL DEFAULT 0,

    -- There are deliberately **no** limit columns, and that is the correction this migration
    -- carries. Ledger 195 gave the instance a `limits: [ItemLimit; 2]`, but `aLimits` is a member
    -- of `TItemTable` (`tables.h:879`) -- the prototype -- and not of `TItemData`
    -- (`packet.h:2971-2985`), which is the instance record and has no such member. `CItem` has no
    -- `SetLimit`; `GetLimitType` and `GetLimitValue` (`item.h:110-111`) both read
    -- `m_pProto->aLimits[idx]`. Legacy's own `player.item` table carries no limit columns either.
    -- The limits live in `gamedata::item_proto::ItemProto::limits`, read from `item_proto.txt`,
    -- and an instance that had its own would have had nothing to save it to.

    -- Legacy's `TItemTable::bSize` decides the grid footprint, not the instance, so it is not a
    -- column here: the prototype table answers it and `world::item::Item::size` carries it. Legacy
    -- does not store it per instance either.

    created_at timestamptz NOT NULL DEFAULT now(),

    -- The load path is "every item of this character", and the save path touches one row by id.
    -- A partial index on the owned rows keeps the load from scanning the ground items, which are
    -- the large table once drops exist.
    CHECK ((window_type = 10) = (owner_id IS NULL))
);

CREATE INDEX item_owner_idx ON item (owner_id) WHERE owner_id IS NOT NULL;

-- A character's flat and dragon-soul storage is addressed by cell, and a cell may hold only one
-- item. Without this, two rows can name the same cell and the load path would silently keep the
-- one it read last, which is exactly the "the grid says one thing and the id array says another"
-- state that ledger 195's `GridConflict` exists to refuse.
CREATE UNIQUE INDEX item_owner_cell_key
    ON item (owner_id, window_type, pos) WHERE owner_id IS NOT NULL;

-- **Do not write this table with `REPLACE`.** Legacy's character save is a
-- `REPLACE INTO item` (`db/Cache.cpp:178`, and the safebox and mall save at
-- `db/ClientManager.cpp:1545-1579`), and that worked because its only unique key was the id.
-- This table has a second one: an item that moves to a cell another item already holds is a
-- conflict on `item_owner_cell_key`, and MySQL's `REPLACE` answers a unique conflict by
-- **deleting the conflicting row** and inserting the new one. A save that moves an item would
-- therefore destroy the item it landed on. The save is
-- `INSERT ... ON CONFLICT (id) DO UPDATE SET ...`, which touches exactly the row named by the
-- id and leaves every other row alone.
--
-- The delete is `WHERE id = ? AND owner_id = ?`, never `WHERE id = ?`. Legacy's
-- `QUERY_ITEM_DESTROY` is handed the owner's pid and uses it for the log line and for choosing
-- the async or sync branch, then deletes on the id alone
-- (`db/ClientManager.cpp:1833,1838,1843-1846`), and the cached path issues the same id-only
-- delete (`db/Cache.cpp:57-61`). The predicate ignores the ownership it was handed, and the
-- item id space is global across every character, every safebox, the mall and the ground. The
-- pid is not client-supplied -- items are addressed to the client by window and cell, never by
-- id -- so legacy's defect is not client-reachable, but there is no reason to copy a
-- destructive predicate that ignores its own guard.
