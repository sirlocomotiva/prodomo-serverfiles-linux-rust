
# Ledger 196 — survey of the legacy item database path

Scope: load, save, update, remove, destroy, award, find and the boot-time item-ID
range, in the frozen tree at `server/server`, plus the Rewrite's current state for
each. Read-only. Nothing in the repository was modified; the legacy server was
never built or run. Every conclusion carries a `file:line` citation. Negative
claims carry a positive control and a negative control (§11).

## 0. A correction to the task's own premise

The brief placed the item load in `server/server/game/input_db.cpp`. That file is
**not** where the item SQL is. It is 74,257 bytes of *game-side* parsing: it
consumes rows that the DB server already read.

- `server/server/game/input_db.cpp` includes `db.h` and implements
  `CInputDB::ItemLoad` (`input_db.cpp:1451-1567`), which walks a `TPlayerItem`
  array that arrived over the retired DB-peer socket. It contains no SQL and no
  `QUERY_*` handler.
- The SQL lives in the DB server: `server/server/db/ClientManagerPlayer.cpp`
  (character items) and `server/server/db/ClientManager.cpp` (safebox, mall,
  destroy, award).

Absence evidence, with controls:

| claim | command | result |
|---|---|---|
| positive control: `ITEM` is greppable in `input_db.cpp` | `command grep -c -a 'ITEM' server/server/game/input_db.cpp` | `43` |
| negative control: an invented token is not | `command grep -rc -a 'ZZZ_NOT_A_REAL_TOKEN_9f3a' server/server/game/input_db.cpp` | `0` |
| `QUERY_LOAD_ITEM` does not exist anywhere | `command grep -rn -a 'QUERY_LOAD_ITEM' server/server` | no output |
| `QUERY_ITEM_SAVE` is not a query | `command grep -rn -a '"QUERY_ITEM_SAVE' server/server` | one hit, and it is a log string: `ClientManager.cpp:1586` |
| `QID_ITEM_SAVE` does exist (so the QID enum is found by the same grep) | `command grep -rn -a 'QID_ITEM_SAVE' server/server/db/QID.h` | `16: QID_ITEM_SAVE, // 10` |

So the item query identifiers are `QID_ITEM` (load), `QID_ITEM_SAVE` and
`QID_ITEM_DESTROY` (`QID.h:15-17`), and the SQL is written inline with
`snprintf`, not held in named constants. The brief's `QUERY_*` names do not exist
in this tree.

## 1. Where the item SQL actually is

| purpose | file:line | statement shape |
|---|---|---|
| item load, account path | `db/ClientManagerPlayer.cpp:364-392` | `SELECT` 27 columns `FROM item%s WHERE owner_id=%d AND window in (...)` |
| item load, character path | `db/ClientManagerPlayer.cpp:501-524` | the same 27 columns, `WHERE owner_id=%d` |
| row decode | `db/ClientManagerPlayer.cpp:14-68` | `CreateItemTableFromRes` |
| result → game | `db/ClientManagerPlayer.cpp:938-959` | `RESULT_ITEM_LOAD` |
| safebox/mall load | `db/ClientManager.cpp:803-813` | `SELECT` 27 columns `... AND window='%s'` |
| safebox/mall save | `db/ClientManager.cpp:1516-1590` | `REPLACE INTO item` with all 27 columns |
| character save | `db/Cache.cpp:55-190` | `REPLACE INTO item` with a *conditional* column list |
| destroy | `db/ClientManager.cpp:1828-1848` | `DELETE FROM item WHERE id=%u` |
| destroy, cached path | `db/Cache.cpp:57-65` | `DELETE FROM item WHERE id=%u` |
| character delete | `db/ClientManagerPlayer.cpp:1529-1542` | `DELETE FROM item WHERE owner_id=%d AND window in (...)` |
| award | `db/ClientManager.cpp:997-1005` | `INSERT INTO item (...) VALUES(...)` |
| game-side placement | `game/input_db.cpp:1451-1567` | `CInputDB::ItemLoad` |

Every statement is built with `snprintf` and executed through
`CDBManager::ReturnQuery` / `DirectQuery` / `AsyncQuery`. There are no prepared
statements and no bound parameters anywhere in the DB server:

- positive control: the query really is issued through the MySQL C API, and
  `command grep -n -a 'mysql_real_query' server/server/db/DBManager.cpp` names the call site
- absence: `command grep -rn -a 'mysql_stmt\|MYSQL_STMT\|mysql_real_escape' server/server/db` → exactly one
  hit, `DBManager.cpp:163`, which is the body of `CDBManager::EscapeString`
  (declared `DBManager.h:54`).

So the escape primitive exists and is used in 24 places (`ClientManager.cpp:1105`,
`ClientManagerPlayer.cpp:232-242,1261-1280`, `Cache.cpp:355`, `game/guild.cpp:680,841,1065`,
`game/shop_manager.cpp:205`, …) — and in exactly one function it is applied to one
column and not to its neighbour. `CPrivateShopCache::OnFlush` escapes the title
and then interpolates the owner name raw:

```
Cache.cpp:354   char szEscapedTitle[TITLE_MAX_LEN * 2 + 1] { 0 };
Cache.cpp:355   CDBManager::instance().EscapeString(szEscapedTitle, m_data.szTitle, strlen(m_data.szTitle));
...
Cache.cpp:361       "owner_name = '%s', "
Cache.cpp:363       "title = '%s', "
...
Cache.cpp:379       m_data.szOwnerName,      <-- raw
Cache.cpp:381       szEscapedTitle,          <-- escaped
```

That is in the same file as the item cache, one function away from the item path,
and it is a clean demonstration that the codebase has the tool and does not always
use it. It is not on the item path; it is reported because it calibrates what an
unescaped interpolation costs in this codebase.

## 2. The load path

### 2.1 The query

```
db/ClientManagerPlayer.cpp:365-386
  "SELECT id," "window+0," "pos," "count," "vnum," "transmutation," "refine_element,"
  "socket0,"..."socket5,"
  "attrtype0," "attrvalue0," ... "attrtype6,attrvalue6"
  " FROM item%s WHERE owner_id=%d AND (window in
     ('INVENTORY','EQUIPMENT','DRAGON_SOUL_INVENTORY','ATTR67_ADD','SWITCHBOT','AURA_REFINE','BELT_INVENTORY'))"
```

27 columns: 5 base + `transmutation` + `refine_element` + 6 sockets + 7×(type, value).
`window+0` forces MySQL to hand back the enum's ordinal as text so
`str_to_number` can take it.

`GROUND` is deliberately **not** in the list, and neither is `SAFEBOX`/`MALL` —
those load through a different query. A ground item is therefore not restored by
character load; it is restored by the ground-item system, if at all.

### 2.2 The decode, and the `SELECT`/`#ifdef` skew

```
db/ClientManagerPlayer.cpp:37-62
    int cur = 0;
    str_to_number(item.id,   row[cur++]);   ... 5 unconditional
#ifdef __CHANGELOOK_SYSTEM__      str_to_number(item.transmutation,    row[cur++]);  #endif
#ifdef ENABLE_REFINE_ELEMENT      str_to_number(item.dwRefineElement, row[cur++]);  #endif
    str_to_number(item.alSockets[0..2], row[cur++]);                       #ifdef ENABLE_EXTENDED_SOCKETS
    str_to_number(item.alSockets[3..5], row[cur++]);                       #endif
    for (j = 0; j < ITEM_ATTRIBUTE_MAX_NUM; j++) { str_to_number(bType); str_to_number(sValue); }
```

The SQL at `:365-386` is **unconditional**. The decode is conditional on
`__CHANGELOOK_SYSTEM__` (`prodomodefines.h:16`), `ENABLE_REFINE_ELEMENT` (`:28`)
and `ENABLE_EXTENDED_SOCKETS` (`:76`) — all three live in this build. So today the
27 and the 27 agree. If any one of the three were ever turned off, the decode
would consume 27 values from a 27-column row and read the wrong fields, silently,
with no compile error and no runtime check. That is a latent Defect, not a live
one. It is the same latent skew that `CItemCache::OnFlush` carries more visibly
(§3.1).

`pVec->resize(rows)` at `:30` value-initialises each `TPlayerItem`, so a NULL or
absent column leaves the field at **0** rather than failing. `str_to_number` is
declared `bool str_to_number(T&, const char*)` (`common/utils.h:4-108`) and
`CreateItemTableFromRes` **discards every return value** (`:38-62`). A row whose
`count` came back as the empty string stores an item with `count = 0`, which
`CItem::SetCount` treats as a destruction request (`item.cpp:307-338`). Silent
corruption, not a reported error.

The row walk is `row[cur++]` with no comparison against `mysql_num_fields(res)`.
The only length guard in the function is the row count at `:24`. If the select
list and the decode ever disagreed, the walk runs off the end of the row. This is
the "column read past the end" class; it is latent here for the reason above.

### 2.3 The hop to the game thread

```
db/ClientManagerPlayer.cpp:940   static std::vector<TPlayerItem> s_items;   <-- function-level static
:942   CreateItemTableFromRes(pRes, &s_items, dwPID);
:943   DWORD dwCount = s_items.size();
:945   peer->EncodeHeader(HEADER_DG_ITEM_LOAD, dwHandle, sizeof(DWORD) + sizeof(TPlayerItem) * dwCount);
:946   peer->EncodeDWORD(dwCount);
:954   peer->Encode(&s_items[0], sizeof(TPlayerItem) * dwCount);
:957       PutItemCache(&s_items[i], true);
```

The count and the rows are correct here: the count is derived from the vector
that is about to be encoded, so they cannot disagree on this side. I checked
whether the `static` is a race, and it is not:

- positive control that the token is greppable in this tree:
  `command grep -rl -a -E 'pthread_create' server/extern/` → 4 files
- absence in the servers' own source:
  `command grep -rn -a -E 'pthread_create|std::thread' server/server/game/` → no output;
  the same sweep over `server/server/db/` → no output.
- `db/Main.cpp:162` runs one `thecore_init(heart_beat, emptybeat)` loop, and
  `CClientManager::Process()` (`db/ClientManager.cpp:3082-3103`) drains the async
  SQL results in that one loop. `Lock.h` exists but `command grep -c -a
  'CLock|lock()|unlock()' server/server/db/ClientManager.cpp` → `0`.

So the DB server is single-threaded for this purpose and the shared `s_items` is a
latent hazard, not a reachable defect. Recorded as latent, on the record.

The count *can* disagree on the receiving side. `CInputDB::Analyze` dispatches
`HEADER_DG_ITEM_LOAD` to `ItemLoad(d, c_pData)` (`game/input_db.cpp:1980-1982`)
with **no length argument at all**, and `ItemLoad` reads `dwCount` at `:1461` and
then walks `dwCount` `TPlayerItem`s by pointer increment (`:1468-1470`) with no
bound. A frame shorter than `4 + dwCount * sizeof(TPlayerItem)` is read past its
end. In the Rewrite this is a non-issue because there is no socket, and
ADR-0001 retires the DB peer. It is named here so the retired path's rule is not
mistaken for a rule the Rewrite needs.

### 2.4 Where the item is actually put

```
game/input_db.cpp:1470-1533
  for (DWORD i = 0; i < dwCount; ++i, ++p) {
      LPITEM item = ITEM_MANAGER::instance().CreateItem(p->vnum, p->count, p->id);
      if (!item) { /*sys_err(...)*/ continue; }                    // 1474-1478
      item->SetSockets(p->alSockets); item->SetAttributes(p->aAttr);
      item->SetLastOwnerPID(p->owner);
      ... switch (p->window) { INVENTORY / DRAGON_SOUL_INVENTORY / ATTR67_ADD / SWITCHBOT -> AddToCharacter
                                 EQUIPMENT -> CheckItemUseLevel ? EquipTo : v.push_back }  // no default
  }
  while (v non-empty) { pos = GetEmptyInventory(size);
      if (pos < 0) { AddToGround(...); SetOwnership(ch, 180); StartDestroyEvent(); }
      else AddToCharacter(ch, TItemPos(INVENTORY, pos)); }        // 1541-1561
```

Three findings here.

**(a) A row for an unknown vnum is dropped silently, forever.** `CreateItem`
returns NULL for a vnum that is not in the Game data, and `:1477` is a bare
`continue`. The diagnostic that would have named it is **commented out** at
`:1476`. The `item` row is never deleted and never re-saved, so the same silent
drop repeats on every login, and the row stays in the table forever. This is
precisely the "load silently drops an item, the row was never deleted" class, and
the only thing that makes it diagnosable is the log line someone commented out.
Reachability: the row's `vnum` is not client-controlled and the prototype table
is loaded before clients are accepted, so an honest client cannot trigger it. It
fires on a Game data mismatch — a proto that was removed or renamed, an
`item_proto.txt` newer than the table, or a failed proto load. Confirmed as a
Defect in the source, not client-reachable.

**(b) The placement `switch` has no `default`, and `AURA_REFINE` is in the select
but has no `case`.** The query lists `'AURA_REFINE'`
(`ClientManagerPlayer.cpp:386,522`) and `__AURA_SYSTEM__` is live
(`prodomodefines.h:30`), so `EWindows::AURA_REFINE` exists (`common/length.h:668-670`).
The `switch` at `input_db.cpp:1506-1532` handles `INVENTORY`,
`DRAGON_SOUL_INVENTORY`, `ATTR67_ADD`, `SWITCHBOT`, `EQUIPMENT` — and nothing
else. A row whose `window` ordinal is 7 is instantiated, `OnAfterCreatedItem()` is
called, `SetSkipSave(false)` is called, and the item is never added to the
character and never saved. It is the same silent drop as (a), with a different
trigger.

I then checked whether ordinal 7 can occur, and it cannot:

- the only `TItemPos(AURA_REFINE, ...)` in the tree is `game/char_aura.cpp:389`,
  and it fills the `AuraCell` field of a client packet, not a storage cell;
- `command grep -rn -a 'TItemPos(AURA|SetWindow(AURA' server/server` returns that
  one line and nothing else;
- `command grep -rn -a 'case AURA_REFINE' server/server` → no output, against the
  positive control `command grep -n 'case SWITCHBOT' server/server/game/input_db.cpp`
  → `1514: case SWITCHBOT:`.

Nothing ever stores an item in the `AURA_REFINE` window, so nothing ever writes
ordinal 7 to the column, so the missing `case` cannot be reached from the table.
**Latent, not reachable.** Recorded, not counted as a live Defect.

**(c) The enum order in the two definitions disagrees, but the numbers still round
trip.**

| ordinal | MySQL `enum` (`legacy/sql/schema/player.sql:354`) | C++ `EWindows` (`common/length.h:657-676`) |
|---|---|---|
| 6 | `ATTR67_ADD` | `ATTR67_ADD` |
| 7 | `SWITCHBOT` | `AURA_REFINE` |
| 8 | `AURA_REFINE` | `SWITCHBOT` |
| 9 | `BELT_INVENTORY` | `BELT_INVENTORY` |
| 10 | `GROUND` | `GROUND` |

The save writes `window=%d` with the C++ ordinal (`Cache.cpp:115`,
`ClientManager.cpp:1560`) and the load reads it back as `window+0` and
`str_to_number`s it into the C++ ordinal (`ClientManagerPlayer.cpp:39`). The
ordinal is therefore preserved across the round trip; only the *name* in the
column is wrong. A switchbot item is stored as `'AURA_REFINE'`, which is visible
to anyone reading the table and is wrong, and it is not data loss. This is a
naming Defect, and it is the one item finding where the task's framing
("a save that writes a window value the load cannot read") does **not** hold.

**(d) `BELT_INVENTORY` is re-homed on load, once.** With
`ENABLE_BELT_INVENTORY_EX` live (`prodomodefines.h:13`), `:1491-1496` rewrites
`BELT_INVENTORY` to `INVENTORY` with `pos + BELT_INVENTORY_SLOT_START`, and the
next save writes `window=1`. The `'BELT_INVENTORY'` member of the `in (...)` list
is therefore a one-shot: a belt row is read that value exactly once in its life.
That is consistent, and it is why the list names a window no item is stored in.

## 3. The save path

### 3.1 The character cache

`CItemCache::OnFlush` (`db/Cache.cpp:55-190`) writes a `REPLACE INTO item`
whose column list is assembled at run time:

```
Cache.cpp:70-81   bool isSocket = false, isAttr = false;
                  memset(alSockets, 0, ...); memset(aAttr, 0, ...);
                  if (memcmp(alSockets, p->alSockets, ...)) isSocket = true;
                  if (memcmp(aAttr,      p->aAttr,      ...)) isAttr  = true;
:87-112   id, owner_id, window, pos, count, vnum [, transmutation] [, refine_element]
:135-142  if (isSocket) -> socket0..socket5 appended      (ENABLE_EXTENDED_SOCKETS branch)
:145-175  if (isAttr)  -> attrtype0..6 / attrvalue0..6 appended
:178      snprintf(..., "REPLACE INTO item%s (%s) VALUES(%s)", ...);
```

**The conditional column list is present and is not, today, data loss.** The
socket group is omitted only when all six are zero, and the attribute group only
when all fourteen are zero; a `REPLACE` resets omitted columns to their column
default, which is `0` for all of them (`player.sql:360-391`). So an omission can
only ever drop zeros. The condition is what makes it safe, and the reason is
visible in the source. I am recording this as **latent**: the safety is a property
of the *condition*, not of the *statement*, and a one-character change to the
condition (for example "omit when unchanged" instead of "omit when zero") would
turn it into silent data loss on the next save of every socketed item.

The `memcmp` sizes are `sizeof(long) * 6` and `sizeof(TPlayerItemAttribute) * 7`.
That is 24 bytes only because the target is 32-bit x86
(`server/server/premake5.lua:12`), where `long` is 4 bytes. A 64-bit build would
compare 48 bytes against a 24-byte field and read past it. The build target makes
it correct; I did not build it (§10).

`iLen` is the length returned by `snprintf` and is then used as the *offset* into
the buffer (`:128`, `:137`, `:147`). `snprintf` returns the length it *would* have
written. With `szColumns[QUERY_MAX_LEN]` and a worst case of 27 column names this
never truncates, so the offsets stay correct. Latent, worth a check, not a Defect.

### 3.2 The safebox and mall save

`CClientManager::QUERY_ITEM_SAVE` (`db/ClientManager.cpp:1516-1590`) branches on
the window: `SAFEBOX`/`MALL` get an immediate full `REPLACE` with all 27 columns
(`:1545-1579`); everything else goes into the cache (`:1588`). So the
conditional-column behaviour is confined to the character's own items.

### 3.3 When a save actually happens

`CClientManager::PutItemCache` (`:1655-1714`) flushes the cache entry immediately
if the owner has no cache set (`:1712` `c->OnFlush()`), otherwise it leaves it
for `FlushItemCacheSet`. The game side also flushes on Warp, Channel change and
logout. That is the ADR-0003 "background, a few seconds late, always on logout /
Warp / Channel change / shutdown" shape, and the Rewrite's rule is already
right — what is *not* yet in the Rewrite is a flush trigger, and that is
out of scope for this survey.

## 4. The delete path

Three delete statements, and they differ in what they scope on.

| site | statement | scoped on |
|---|---|---|
| `db/ClientManager.cpp:1838` | `DELETE FROM item%s WHERE id=%u` | id only |
| `db/Cache.cpp:60` | `DELETE FROM item%s WHERE id=%u` | id only |
| `db/ClientManagerPlayer.cpp:1531` | `DELETE FROM item%s WHERE owner_id=%d AND window in (...)` | owner **and** window |
| `db/ClientManager.cpp:1531`(character delete, same line) | — | see below |

`CClientManager::QUERY_ITEM_DESTROY` (`db/ClientManager.cpp:1828-1848`) receives
both an id and an owner pid and uses the pid for **nothing but the log line and
the choice of execution path**:

```
:1830  DWORD dwID  = *(DWORD *) c_pData;
:1833  DWORD dwPID = *(DWORD *) c_pData;
:1835  if (!DeleteItemCache(dwID)) {
:1838      snprintf(szQuery, sizeof(szQuery), "DELETE FROM item%s WHERE id=%u", GetTableRange..., dwID);
:1841      if (g_log) sys_log(0, "HEADER_GD_ITEM_DESTROY: PID %u ID %u", dwPID, dwID);
:1843      if (dwPID == 0)  AsyncQuery(szQuery);        // pid 0 = "nobody owns this"
:1846      else              ReturnQuery(szQuery, QID_ITEM_DESTROY, ...);
      }
```

So: **a destroy carries an owner and does not use it in the `WHERE` clause.**
Two further observations make the class worse rather than better:

- The `WHERE` clause is keyed on the **global** item id, and the id space is
  shared by every channel, the safeboxes, the mall and the ground
  (`CItemIDRangeManager`). The `dwPID` guard that would make it safe is on the
  *branch*, not the *predicate*.
- Which statement runs is chosen by **cache membership**, not by ownership
  (`:1835`): if the id happens to be in the cache, `DeleteItemCache` marks it and
  the cache's own `OnFlush` issues the identical id-only delete
  (`db/Cache.cpp:57-61`). The `dwPID` argument has no influence on the delete in
  either branch.

I then checked the two ways the global id invariant can break, and both are closed
in the owner's snapshot:

- the allocator is monotonic within a block and the pool refuses a block whose
  usable tail is short (`db/ItemIDRangeManager.cpp:132-137`, `:188-189`), so a block
  is not handed out twice while it is live;
- `BuildRange` sets `dwUsableItemIDMin` to `MAX(id)+1` over the *whole* block
  (`db/ItemIDRangeManager.cpp:94,110`), so a re-used block starts above every id
  already in it, and it scans `item` **and** `private_shop_item` (`:94`, `:110`).

Reachability: the id is not client-supplied — the game sends `pItem->GetID()`
and items are addressed to the client by window and cell, not by id. So this is
**not client-triggerable**. It is a confirmed Defect in the statement (a
destructive predicate that ignores the ownership it is handed) whose trigger is
any violation of the global-id invariant — a restored backup, a hand-edited row,
a partially-migrated store. The Rewrite must not copy the id-only predicate
regardless of whether legacy's invariant holds today, and the Rewrite's own
`item` table makes the correct form expressible: `owner_id` is nullable
(`db/migrations/0005_items.sql:27`).

**The character-delete path, and where it really loses items.** It is issued in
this order (`db/ClientManagerPlayer.cpp:1529-1532`):

```
:1529  "DELETE FROM player%s WHERE id=%d"   -> DirectQuery, and its affected rows ARE checked (:1522)
:1531  "DELETE FROM item%s WHERE owner_id=%d AND (window in (...))"  -> DirectQuery, result discarded
```

The player row is deleted first and the item delete is a **second** `DirectQuery`
whose affected-row count is thrown away (`delete CDBManager::instance().DirectQuery(szQuery);`
— no result inspected). If that second statement fails, the character is gone and
its items are rows owned by a `player` that no longer exists. No load path can ever
find them again: the item load is `WHERE owner_id=%d` driven from a `player` row
that is gone. The items are gone, the delete was reported as a success to the
client (`:1552` `HEADER_DG_PLAYER_DELETE_SUCCESS`), and nothing is logged.

That is a confirmed silent item loss, in the exact class the brief names, and it
is reachable by any transient error on that one statement.

The same statement is also **window-scoped**, so two kinds of row survive a
character delete on purpose and by accident: `GROUND` items the player dropped,
and `SAFEBOX`/`MALL` rows. The safebox rows are arguably correct (a safebox
outlives the character); the `GROUND` rows are orphans with no owner row and no
path back. They are harmless to the id invariant (`BuildRange` skips the block)
and they never collide, because ids are never reused. They are a leak, not a loss.

## 5. New characters and starter items

**There is no starter-item code.** The character-create result handler
(`game/input_db.cpp:270-274`) declares one `TPlayerItem`, memsets it, and never
uses it. The comment above it is already corrupted in the frozen tree (it decodes
to U+FFFD replacement characters), so the intent is not recoverable from the text.

Two names from the brief do not exist in this tree at all:
`command grep -rn -a 'AwardCash' server/server` → no output;
`command grep -rn -a 'CHARACTER::AddItem' server/server` → no output. The real
spells are `CHARACTER::AddItem` → not present; the game's own item-adding entry
point is `CItem::AddToCharacter` / `ITEM_MANAGER`. So a port that looked for
`AwardCash` or `AddItem` would find nothing and might conclude the feature is
missing rather than that the names are wrong.

Items arrive at a new character by the safebox-award path instead (§6), and by
`ITEM_MANAGER::CreateItem` at whatever gameplay system makes them.

## 6. The award path

`CClientManager` writes an awarded item straight into the safebox or the mall
(`db/ClientManager.cpp:997-1013`):

```
:997  "INSERT INTO item%s (id, owner_id, window, pos, vnum, count, socket0, socket1, socket2) VALUES(%u, %u, '%s', %d, %u, %u, %u, %u, %u)"
:1001     GainItemID(),                                  <- the id
:1002     pi->account_id,                                <- an ACCOUNT id, in owner_id
:1003     pi->ip[0] == 0 ? "SAFEBOX" : "MALL",
:1005     pItemAward->dwVnum, pItemAward->dwCount, pItemAward->dwSocket0, pItemAward->dwSocket1, dwSocket2
:1008  DirectQuery(szQuery);
:1012  if (pRes->uiAffectedRows == 0 || pRes->uiInsertID == 0 || ...) break;
```

Three things to record:

1. **The award's id comes from the DB server's own counter**, not from the game's
   block. `GainItemID()` is `return m_itemRange.dwUsableItemIDMin++;`
   (`db/ClientManager.cpp:3413-3416`) over the block the DB server read from its
   own `ITEM_ID_RANGE` config key (`:3394-3402`; the owner's value is
   `100000000 200000000`, `legacy/config/db/conf.txt:8`). The two spaces are kept
   apart by one check — `CItemIDRangeManager::Build` skips any candidate block the
   DB server's own range fully contains (`db/ItemIDRangeManager.cpp:27-31`).
   In the owner's configuration the blocks are disjoint, so there is no collision.
   The check is the *only* thing keeping them apart, it is written in the
   "fully contains" direction, and it never consults the game server's ranges.
   In one process (ADR-0001, ADR-0002) the two counters collapse into one, which
   **deletes this Defect class outright**. That is an argument for doing it.
2. **The failure test is a disjunction with a dead term** (`:1012`):
   `uiAffectedRows == 0 || uiInsertID == 0 || uiAffectedRows == (uint32_t)-1`. The
   `uiAffectedRows == 0` term is the meaningful one and it is present, so the check
   is right. The `uiInsertID` term can never be true after a successful `INSERT` into
   a table with `AUTO_INCREMENT` (`player.sql:352`), so it is dead weight inside a
   correct check rather than the check's foundation. *Corrected by the parent unit,
   which re-read `:1012` and found the disjunction: the first draft of this
   paragraph called the whole test tautological, which would have overstated it.*
3. **The award row has no `refine_element`, no `transmutation` and no attributes**
   — the column list stops at `socket2`. The columns not named take their column
   defaults (`player.sql:358-359, 366-391`), all `0`, so the row is complete. The
   first save after the player moves the item rewrites it in full.

The award table itself is `item_award` (`legacy/sql/schema/player.sql:462`), read
by `ItemAwardManager`. It is the source of *which* items are awarded; the row
above is the result.

## 7. The boot-time item-ID range

### 7.1 The three constants

```
db/ItemIDRangeManager.h:8-10
    const static DWORD cs_dwMaxItemID           = 4290000000UL;
    const static DWORD cs_dwMinimumRange       =  10000000UL;
    const static DWORD cs_dwMinimumRemainCount =      10000UL;
```

`db/ItemIDRangeManager.cpp:13-38` (`Build`) slices `[1, 4290000000]` into
10,000,000-wide blocks, starting at `cs_dwMinimumRange*(i+1)+1` and ending at
`cs_dwMinimumRange*(i+2)`, breaking when `dwMax == cs_dwMaxItemID`. The loop
terminates here only because 4,290,000,000 is an exact multiple of 10,000,000
(429 × 10,000,000). The test is `==` on a `DWORD` product; a constant that was not
a multiple would run the product past `0xFFFFFFFF` and never match, looping
forever. Latent, and a good reason for the Rewrite not to copy the loop.

`BuildRange` (`:87-203`) resolves one block:

- `:94` `SELECT MAX(id) FROM item%s WHERE id >= %u and id <= %u`
- `:110` the same over `private_shop_item`
- `:132-137` refuses the block when `dwMax - dwUsableItemIDMin < cs_dwMinimumRemainCount`
- `:162` and `:189` refuse a block whose usable tail already holds a
  `private_shop_item` or an `item`; `:202` refuses an exhausted block

`GetRange` (`:60-85`) pops the FIFO and then checks every candidate against every
connected game server's live range (`Peer.cpp:180-184`, which logs an overlap and
returns false). `CPeer::OnClose` returns a peer's block to the pool
(`db/Peer.cpp:46-52`).

### 7.2 How the range reaches the game thread

```
db/ClientManager.cpp:588-597   TItemIDRangeTable itemRange      = GetRange();
                               TItemIDRangeTable itemRangeSpare = GetRange();
                               peer->Encode(&itemRange, ...); peer->Encode(&itemRangeSpare, ...);
                               peer->SetItemIDRange(itemRange); peer->SetSpareItemIDRange(itemRangeSpare);
game/item_manager_idrange.cpp:51-71   bool ITEM_MANAGER::SetMaxItemID / SetMaxSpareItemID
game/item_manager_idrange.cpp:20-49   DWORD ITEM_MANAGER::GetNewID()
```

`GetNewID` is `m_dwCurrentID++` with `if (m_dwCurrentID >= m_ItemIDRange.dwMax)`
switching to the spare and asking the DB server for a new one
(`HEADER_GD_REQ_SPARE_ITEM_ID_RANGE` at `:38`, answered by
`db/Peer.cpp:135-155`). So the handed-out set is `dwUsableItemIDMin .. dwMax-1`:
**`dwMax` is exclusive.**

On exhaustion, `GetNewID` logs ten times, `touch(".killscript")`,
`thecore_shutdown()` and **returns 0** (`:26-33`). Zero is the one value an item
id cannot take (`world/src/item.rs:88` `NO_ITEM: ItemId = 0`). That is the
sentinel the Rewrite's pool already replaces with `None` — the right call.

### 7.3 What the Rewrite has

`db/src/item_id_range.rs` reproduces the constants exactly
(`MAX_ITEM_ID = 4_290_000_000`, `MINIMUM_RANGE = 10_000_000`,
`MINIMUM_REMAIN_COUNT = 10_000`) and the usability rule
(`is_usable`, `max - usable_item_id_min >= 10_000`, with an explicit
`usable_item_id_min > max` guard so the subtraction cannot underflow). It also
documents, correctly, that it **performs no SQL** and that legacy's zero sentinel
is replaced by `None`.

Against the legacy it is missing three things, all of which are somebody's job:

1. **the `SELECT MAX(id)` step** — `ItemIDRangeManager.cpp:94` has no owner in
   the Rewrite. The pool takes already-resolved ranges. Something must run the
   query, for `item` and for `private_shop_item`.
2. **refill** — legacy asks for a new spare mid-life. The pool is a boot
   snapshot; after `next_range()` returns `None` the game thread has no way to ask
   for more. `take_active_and_spare()` is also single-consumer shaped: with one
   process there is one consumer, so the "active plus spare" pair is an artefact
   of the two-process topology and should collapse to one range.
3. **the "refuse a block whose tail already holds an item" rule**
   (`ItemIDRangeManager.cpp:188-189`) — a block with an item in
   `[dwUsableItemIDMin, dwMax]` is refused outright, not trimmed. `is_usable`
   implements the *arithmetic* of that rule but not the *query* that detects it.

And there are two types for one concept, which will not survive integration:

| | `db::item_id_range::ItemIdRange` | `world::item::ItemIdRange` |
|---|---|---|
| fields | `min`, `max`, `usable_item_id_min` | `first`, `last`, `first_usable` |
| interval | closed at `min`, **exclusive** at `max` | half-open `[first, last)` |
| validity | `is_usable()` | `new(...) -> Result<_, BadIdRange>`, also refuses `first_usable == 0` |
| allocator | none | `ItemIds::allocate() -> Result<ItemId, IdRangeExhausted>` |
| users | none outside its own tests | the world allocator |

They agree on the interval semantics, so this is not a bug yet. It is two
structures with the same meaning, and the one that has no consumer is the one
modelled on the legacy wire record. The store should produce
`world::item::ItemIdRange` and `db::item_id_range` should go, or the pool should
live where the range does and hold `world::item::ItemIdRange` values.

## 8. Defect register

"Confirmed" means the source proves the defect. "Reachable" means an honest
client can trigger it. Nothing in this table is reachable from a client except
where the row says so.

| # | class | finding | evidence | status |
|---|---|---|---|---|
| 1 | silent item loss | character delete issues `DELETE FROM player` and checks its result, then a second `DirectQuery` for the items whose result is discarded; a failure orphans every item and the client is told success | `db/ClientManagerPlayer.cpp:1522-1532,1552` | confirmed, not client-reachable |
| 2 | silent item loss | a row whose vnum is not in the Game data is dropped by a bare `continue` with the diagnostic **commented out**; the row is never deleted, so it repeats every login | `game/input_db.cpp:1474-1478` | confirmed, not client-reachable |
| 3 | destroy ignores the owner | `QUERY_ITEM_DESTROY` takes a `dwPID`, uses it for the log and the async/sync branch, and deletes `WHERE id=%u` only; the cached path issues the same id-only delete | `db/ClientManager.cpp:1833,1838,1841,1843-1846`; `db/Cache.cpp:57-61` | confirmed, not client-reachable |
| 4 | unescaped interpolation | `CPrivateShopCache::OnFlush` escapes `title` and interpolates `owner_name` raw, one line apart | `db/Cache.cpp:355,361,363,379,381` | confirmed, adjacent to the item path |
| 5 | row leak | a character delete's item delete is window-scoped and omits `GROUND`, leaving orphan rows | `db/ClientManagerPlayer.cpp:1531` | confirmed, harmless to the id invariant |
| 6 | dead term in a check | the award insert's failure test is a disjunction whose `uiInsertID` term can never be true after a successful `AUTO_INCREMENT` insert; the `uiAffectedRows == 0` term beside it is the meaningful one and is present | `db/ClientManager.cpp:1012` | confirmed, dead code rather than a wrong check |
| 7 | select/decode skew | the item `SELECT` is unconditional; the decode is behind `__CHANGELOOK_SYSTEM__`, `ENABLE_REFINE_ELEMENT`, `ENABLE_EXTENDED_SOCKETS` | `db/ClientManagerPlayer.cpp:365-386` vs `:43-56` | latent (all three live today) |
| 8 | unchecked column walk | `row[cur++]` with no comparison against `mysql_num_fields`; `str_to_number`'s `bool` return is discarded and a failed parse leaves 0 | `db/ClientManagerPlayer.cpp:30,37-62`; `common/utils.h:4-108` | latent |
| 9 | missing placement case | the `switch (p->window)` in the load has no `default` and no `case AURA_REFINE`, although the query selects it and `__AURA_SYSTEM__` is live | `game/input_db.cpp:1506-1532`; `:386`; `prodomodefines.h:30` | latent — nothing ever stores ordinal 7 |
| 10 | enum order disagrees | MySQL puts `SWITCHBOT` at 7 and `AURA_REFINE` at 8; C++ has them the other way round | `legacy/sql/schema/player.sql:354` vs `common/length.h:668-673` | confirmed, naming only — ordinals round trip |
| 11 | conditional save columns | the socket and attribute groups are omitted from the `REPLACE` when all values are zero, which `REPLACE` then resets to their (zero) defaults | `db/Cache.cpp:70-81,135-175,178` | latent, not data loss today |
| 12 | shared mutable state | `RESULT_ITEM_LOAD` decodes into a function-level `static std::vector` | `db/ClientManagerPlayer.cpp:940` | latent — the DB server is single-threaded (§2.3) |
| 13 | count with no length | the game is handed `dwCount` and a pointer and no byte count at all | `game/input_db.cpp:1461-1470,1980-1982` | latent; irrelevant once the peer is retired |
| 14 | loop termination | `Build` breaks on `dwMax == cs_dwMaxItemID`, a `DWORD` product that only terminates because the constant is an exact multiple of `cs_dwMinimumRange` | `db/ItemIDRangeManager.cpp:13-38` | latent |
| 15 | exhaustion returns a reserved id | `GetNewID` shuts down and returns 0, which is `NO_ITEM` | `game/item_manager_idrange.cpp:26-33` | confirmed, unreachable without an exhausted pool |
| 16 | width assumption in `memcmp` | the cache's zero-comparison sizes depend on `long` being 4 bytes | `db/Cache.cpp:77,80`; `premake5.lua:12` | correct for the frozen target |

Things the brief asked about that I looked for and did **not** find:

- **an out-of-range socket or attribute value on save.** `CItem::SetSocket`
  (`game/item.cpp:1643-1648`) asserts `i < ITEM_SOCKET_MAX_NUM` and nothing else
  — no lower bound, and an `assert` that is compiled out in a release build. But
  every caller I traced bounds the index before calling: the load's socket copy
  goes through `SetSockets(p->alSockets)` (an array copy, `input_db.cpp:1481`),
  the attribute copy through `SetAttributes(p->aAttr)` (`:1482`), and the
  per-slot callers use a loop counter that starts at 0 and runs
  `ITEM_SOCKET_MAX_NUM` times. I did not find a client-controlled index into
  either array. The missing lower bound is real; its reachability is not
  demonstrated.
- **a delete that removes another player's item by client action.** Item ids are
  not on the client wire; items are addressed by window and cell. Finding 3 is the
  real form of this class and it is not client-reachable.
- **a save that writes fewer attributes than the load read.** Finding 11 is the
  real form; it is safe today by its own condition, not by its statement.
- **starter items on character creation.** There is none. §5.
- **`CHARACTER::AwardCash`, `CHARACTER::AddItem`.** Neither exists in this tree.
  §5.

## 9. The Rewrite: current state and four measured defects

`db/migrations/0005_items.sql` already exists and is the right shape in outline:
it keeps the 6 + 7 column layout on purpose, makes `owner_id` nullable so
"nobody owns this" is expressible, adds a biconditional `CHECK ((window_type = 10)
= (owner_id IS NULL))`, a partial index for the load, and a unique index on
`(owner_id, window_type, pos)`. The two comments that argue for the design — that
`aLimits` belongs to the prototype and not the instance, and that `bSize` is
`TItemTable`'s and not `TItemData`'s — are the two right corrections to make.

It also has a class of defect I can measure rather than argue about. PostgreSQL
`integer` is signed 32-bit and `smallint` is signed 16-bit; several legacy fields
are wider than that, and the migration's `CHECK` bounds do not help because the
type refuses the value first.

I applied the migration to a scratch database on the PostgreSQL 18 container
already running on this machine (`podman exec prodomo-pg18`), inserted one
control row and then the boundary values, and dropped the database afterwards
(`SELECT datname FROM pg_database WHERE datname LIKE 'prodomo\_%'` → empty).

| column | migration type | legacy field | probe | PostgreSQL 18 said |
|---|---|---|---|---|
| `id` | `integer CHECK (id > 0 AND id <= 4290000000)` | `DWORD` (`tables.h:434`); pool ceiling `cs_dwMaxItemID = 4290000000` (`ItemIDRangeManager.h:8`) | 2 141 000 001 (a legal pool id) | **`integer out of range`** |
| `id` | " | " | 4 290 000 000 (the constant itself) | **`integer out of range`** |
| `id` | " | " | 2 147 483 647 | `INSERT 0 1` (positive control) |
| `vnum` | `integer CHECK (0..4294967295)` | `DWORD` (`tables.h:439`); legacy `int(11) unsigned` (`player.sql:357`) | 3 000 000 000 | **`integer out of range`** |
| `refine_element` | `integer CHECK (0..4294967295)` | `DWORD` (`tables.h:445`) | 2 147 483 648 | **`integer out of range`** |
| `pos` | `smallint CHECK (pos >= 0)` | `WORD`, i.e. **unsigned** 16-bit (`tables.h:436`) | 32 767 | `INSERT 0 1` (positive control) |
| `pos` | " | " | 32 768 and 65 535 | **`smallint out of range`** |
| `attrtype0` | `smallint CHECK (-128..127)` | `BYTE`, i.e. **unsigned** 8-bit (`tables.h:428`); legacy `tinyint` signed (`player.sql:366`) | 200 | `violates check constraint item_attrtype0_check` |
| `socket0` | `integer`, no `CHECK` | `long`, signed (`tables.h:440`) | −1 | `INSERT 0 1` — **the signed-socket decision is correct and verified** |
| `count` | `smallint CHECK (1..5000)` | `DWORD` (`tables.h:437`), game bound `ITEM_MAX_COUNT = 5000` (`item_length.h:19`) | 5 001 | `violates check constraint item_count_check` — correct and intended |

So four things need a decision, and all four are the same mistake: sizing a
PostgreSQL column from the *SQL* type rather than from the *C++ field* it stores.

1. `id` must be `bigint` (or the pool's ceiling must be capped at
   `i32::MAX`). `db/src/item_id_range.rs:21` types the ceiling as `u32` and
   `world/src/item.rs:85` types `ItemId = u32`, so the Rust side is right and the
   column is wrong. Note also that `MAX_ITEM_ID` does not fit a *signed*
   32-bit integer at all, so the ADR-0003 phrase "IDs the client sees stay
   32-bit integers" is satisfied only in the unsigned sense, exactly as the
   frozen 32-bit target satisfied it.
2. `pos` must be `integer`, not `smallint`. The field is `WORD`; a ground cell
   index above 32 767 is representable in the game and in the legacy column
   (`smallint(5) unsigned`, `player.sql:355`) and is not representable here.
3. `vnum`, `refine_element`, `transmutation`, `flags`, `anti_flags` are all
   `DWORD` and all need `bigint`. For `flags`/`anti_flags` a bit 31 set is the
   ordinary case, not an edge case.
4. `attrtypeN` is `BYTE` in the field and `tinyint` (signed) in the legacy
   column. The migration currently matches the legacy *column*, not the legacy
   *field*, and does not say so. That is a defensible choice — it keeps the table
   able to hold anything the legacy table could — but it should be stated, the
   way the `socket0` comment states its opposite. `attrvalueN` is `short`
   (`tables.h:429`) and `smallint` is exactly right.

The right fix for 1–4 is a new migration, not an edit of `0005`, per the
repository rule that an applied migration is never edited.

Beyond the widths, the Rewrite needs to decide, not copy:

- **the delete predicate.** `owner_id` being nullable makes the correct form
  writable. Finding 3 is a reason to write `WHERE id = ? AND owner_id = ?` and to
  refuse the delete when the owner does not match, which also removes the
  cache-membership-versus-ownership question of `db/ClientManager.cpp:1835`.
- **the load is a query, not a vector of records.** The `row[cur++]` walk and
  the `str_to_number` return values have no counterpart, because a row becomes a
  `row.as_item()?` that either decodes or errors. Finding 2's "silently drop and
  keep the row" has no counterpart either; the Rewrite must **fail loudly**,
  because that is the point of finding 2.
- **the `SELECT MAX(id)` and the refill** have no owner. §7.3.
- **`item_award`** has no Rewrite equivalent and needs one, since that is how a
  new character receives anything.
- **the `apply_path`/`apply_value`/`apply_type` triple for attributes 0..3**
  (`player.sql:367-378`) is absent from `0005` by design, and the migration says
  so and says where it goes. That is the right call and needs no change.

## 10. What I could not measure

- **`i686-linux-gnu-g++-12` is not installed on this machine**
  (`command -v i686-linux-gnu-g++-12` → nothing). I therefore made **no** measured
  packed-width claim. Every width in this report is read from the field
  declaration in the frozen source (`common/tables.h`, `common/length.h`), and the
  32-bit-target premise is cited to `server/server/premake5.lua:12` rather than to
  a compilation. Where I say a field is 4 bytes I mean the C++ declaration on a
  32-bit target, per the repository's own rule that the legacy target is 32-bit
  x86.
- **I did not build or run the legacy server**, so no claim here rests on observed
  legacy behaviour at run time. Every finding is a source reading.
- **I did not run the Rust gates.** No source file was changed, so there is no new
  test count to record. `rustfmt` and `cargo-clippy` are present on this machine;
  the `i686` width probe is not, which is the one gate that could not have run.
- The four migration findings in §9 are the only findings in this report that come
  from executing anything. They were produced against a scratch database on the
  existing `prodomo-pg18` container and the database was dropped; no repository
  file was touched and no file under `legacy/` was read as anything but bytes.

## 11. Control log

| purpose | command | result |
|---|---|---|
| positive control: a known token is found | `command grep -c -a 'ITEM' server/server/game/input_db.cpp` | `43` |
| negative control: an invented token is not found | `command grep -rc -a 'ZZZ_NOT_A_REAL_TOKEN_9f3a' server/server/game/input_db.cpp` | `0` |
| positive control: a QID identifier is found | `command grep -rn -a 'QID_ITEM_SAVE' server/server/db/QID.h` | `16: QID_ITEM_SAVE, // 10` |
| absence: `QUERY_LOAD_ITEM` | `command grep -rn -a 'QUERY_LOAD_ITEM' server/server` | no output |
| absence: `AwardCash` | `command grep -rn -a 'AwardCash' server/server` | no output |
| absence: `CHARACTER::AddItem` | `command grep -rn -a 'CHARACTER::AddItem' server/server` | no output |
| positive control: `mysql_real_escape` is findable | `command grep -rn -a 'mysql_real_escape' server/server/db` | `DBManager.cpp:163` (the `EscapeString` body) |
| absence: prepared statements in the DB server | `command grep -rn -a 'mysql_stmt\|MYSQL_STMT' server/server/db` | no output |
| positive control: `pthread_create` is greppable in the tree | `command grep -rl -a -E 'pthread_create' server/extern/` | 4 files |
| absence: threads created by the servers | `command grep -rn -a -E 'pthread_create\|std::thread' server/server/game/` and the same over `server/server/db/` | no output in both |
| positive control: the placement switch does have a `SWITCHBOT` arm | `command grep -n 'case SWITCHBOT' server/server/game/input_db.cpp` | `1514: case SWITCHBOT:` |
| absence: an `AURA_REFINE` arm | `command grep -rn -a 'case AURA_REFINE' server/server` | no output |
| positive control: locks are greppable | `command grep -rn -a 'lock_t' server/server/db/Lock.h` | `8: typedef pthread_mutex_t lock_t;` |
| absence: no lock in the client manager | `command grep -c -a 'CLock\|lock()\|unlock()' server/server/db/ClientManager.cpp` | `0` |
| no leftover scratch database | `SELECT datname FROM pg_database WHERE datname LIKE 'prodomo\_%'` | empty |

Tooling used: `command grep -a` throughout, with `--include=*.h --include=*.H`
for header sweeps; every non-UTF-8 file read through Python with
`errors="replace"`.
