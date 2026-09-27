# Survey 195: the character item slot storage, the accessors, and every Defect found

Scope: the frozen legacy build under `server/server/game/char_item.cpp`, read-only.
Nothing in the repository was modified. All probes were built in `/tmp`.

Every claim below is keyed to a `file:line` in the frozen tree. Every absence claim
carries a positive control and a negative control. Every Defect is quoted from the
source and, where the consequence is not obvious from reading, backed by a compiled
probe that reproduces it.

---

## 0. First, three corrections to the task's wording

The task named a signature that does not exist. What the frozen build actually has:

| task asked for | what the build has | citation |
|---|---|---|
| `CHARACTER::SetItem(TItemPos pos, TItemData data)` | `void SetItem(TItemPos Cell, LPITEM item, bool bHighlight = true)` | `char.h:1162`, `char_item.cpp:366` |
| `CHARACTER::RemoveItem` | **does not exist** | absence, controls in §7.2 |
| `CHARACTER::GetItemByVnum` | **does not exist** | absence, controls in §7.2 |
| `CHARACTER::GetItemCount` | **does not exist** | absence, controls in §7.2 |
| `CHARACTER::CountEmptyItemGrid` | **does not exist** | absence, controls in §7.2 |

There is no `TItemData`. The stored type is `LPITEM`, a pointer to a live `CItem`
instance owned by `ITEM_MANAGER`. `SetItem` is `void`, and that single fact is the
root of Defect D1.

Removal is expressed as **`SetItem(pos, NULL)`**. There is no remove function on
`CHARACTER`. `ITEM_MANAGER::RemoveItem` (`item_manager.cpp:566`) and
`CItem::RemoveFromCharacter` (`item.cpp:359`) are the removal paths, and both of
them are outside `char_item.cpp`.

---

## 1. The storage: four arrays, not seven inventories

`char.h:458-461`, the only item storage on a character:

```c
LPITEM  pItems[INVENTORY_AND_EQUIP_SLOT_MAX];   // char.h:458
WORD    bItemGrid[INVENTORY_AND_EQUIP_SLOT_MAX];// char.h:459
LPITEM  pDSItems[DRAGON_SOUL_INVENTORY_MAX_NUM];// char.h:460
WORD    wDSItemGrid[DRAGON_SOUL_INVENTORY_MAX_NUM]; // char.h:461
```

Sizes, from the compiler through the tree's own probe
(`.scratch/probe194/slot_space.cpp`, rebuilt in `/tmp/probe194rebuild`):

| array | extent | bytes on the 32-bit target |
|---|---|---|
| `pItems` | 1370 | 5480 |
| `bItemGrid` | 1370 | 2740 |
| `pDSItems` | 1152 | 4608 |
| `wDSItemGrid` | 1152 | 2304 |
| total | | **15132** |

`bItemGrid` is a **`WORD`**, not a `BYTE`, despite the `b` prefix. This is load-bearing
and was the right call: the grid stores `wCell + 1`, so cell 1369 stores 1370. A
`BYTE` would wrap that to 90, which is a live cell, and the grid would report the
wrong cell as occupied. The probe asserts both directions
(`slot_space.cpp:283-285`): `grid marker fits a WORD` expects true,
`grid marker would wrap a BYTE` expects false.

The element name is also a misnomer, not a type: it is a `WORD` holding a **one-based
anchor**, so `0` means empty and `anchorCell + 1` means "covered by the item anchored
here". The comment at `char_item.cpp:961-962` says this in Korean.

### The two bands

The window byte (`BYTE`, `EWindows`, `length.h:657-676`) selects the band. Measured
numbering, all three feature gates live in this snapshot:

| byte | member | band reached |
|---|---|---|
| 0 | `RESERVED_WINDOW` | refused |
| 1 | `INVENTORY` | `pItems` / `bItemGrid` |
| 2 | `EQUIPMENT` | `pItems` / `bItemGrid` |
| 3 | `SAFEBOX` | **neither array** — deferred |
| 4 | `MALL` | **neither array** — deferred |
| 5 | `DRAGON_SOUL_INVENTORY` | `pDSItems` / `wDSItemGrid` |
| 6 | `ATTR67_ADD` | a single scalar, `pAttr67AddItem` (`char.h:475`) |
| 7 | `AURA_REFINE` | refused |
| 8 | `SWITCHBOT` | `pSwitchbotItems[5]` (`char.h:478`) |
| 9 | `BELT_INVENTORY` | refused by `GetItem`; see §3 |
| 10 | `GROUND` | refused |

`INVENTORY_AND_EQUIP_SLOT_MAX` is 1370 and decomposes into six **contiguous** ranges,
which is why a `WORD` index with no per-window base works at all:

```
base inventory        0 .. 180
equipment           180 .. 244
dragon soul deck    244 .. 256
dragon soul reserve 256 .. 274
belt               274 .. 290
custom (6 x 180)   290 .. 1370
```

**Belt cells are stored as `INVENTORY`.** `CHARACTER::SetCell`
(`char_item.cpp:617-631`) relabels a held item `INVENTORY` for the base inventory,
the belt band, and a custom category, and `EQUIPMENT` for everything else.
`EWindows::BeltInventory` therefore names no cell that `GetItem` will ever find; it
survives only as the byte a belt item is *persisted* with before that relabelling
(`input_db.cpp:1491-1495` does the inverse on load).

### Sentinels

| store | empty marker | occupied marker |
|---|---|---|
| `pItems[]`, `pDSItems[]`, `pSwitchbotItems[]` | `NULL` | `LPITEM` |
| `bItemGrid[]`, `wDSItemGrid[]` | `0` | `anchorCell + 1` |

`pAttr67AddItem` has no grid at all; `ATTR67_ADD_SLOT_MAX` is 1
(`length.h:202`), so it is one item in one slot.

---

## 2. `SetItem` transcribed

Two overloads, selected by `__BL_ENABLE_PICKUP_ITEM_EFFECT__`, which **is** defined
in this snapshot (`prodomodefines.h`):

```c
char_item.cpp:366  void CHARACTER::SetItem(TItemPos Cell, LPITEM pItem, bool bHighlight)
char_item.cpp:368  void CHARACTER::SetItem(TItemPos Cell, LPITEM pItem)   // dead
```

The live one, in order:

```
370  {
371      WORD wCell = Cell.cell;
372      BYTE window_type = Cell.window_type;
373      if ((unsigned long)((CItem*)pItem) == 0xff || ... == 0xffffffff)
375          sys_err("!!! FATAL ERROR !!! item == 0xff ..."); core_dump(); return;
380      if (pItem && pItem->GetOwner()) { assert(!"GetOwner exist"); return; }
386      switch (window_type)
```

Three pre-switch guards, then a six-arm `switch` with a `default` at 562-564.
**The `switch` is fine; the arms are not.**

The `0xff` check at 373 is a width trap. `(unsigned long)` is 4 bytes on the legacy
target, so `0xffffffff` is a plausible corrupted pointer. The `0xff` comparison is
dead on the same target: a pointer is never `0xff`. Harmless, but it is the shape of
code that was written for a 16-bit assumption and never removed.

The `assert` at 382 is **compiled out in Release**, which
`premake5.lua:54,59` confirms: both `Release` and `Fast-Release` `defines { "NDEBUG" }`.
So in a shipping build line 383's `return` is the only thing that runs, and the
`sys_err` that would have named the item is not there either. The condition is
detected, silently swallowed, and the item is not stored.

### Arm `INVENTORY` / `EQUIPMENT` (388-470) — **checks first, correctly**

```
391      if (wCell >= INVENTORY_AND_EQUIP_SLOT_MAX) { sys_err(...); return; }   <-- FIRST
...
410      LPITEM pOld = m_pointsInstant.pItems[wCell];                            <-- then read
```

This arm is the one that is right. The bound check at 391 precedes every array
subscript. It is the template the other two arms should have followed.

Grid maintenance, both directions, lines 412-440 (clear) and 442-466 (set):

```
422      int p = wCell + (i * 5);       // stride 5 = INVENTORY_WIDTH, a bare literal
425      if (p >= wCategoryEndIndex) continue;      // active build (ENABLE_CUSTOM_INVENTORY)
435      m_pointsInstant.bItemGrid[p] = 0;          // clear
461      m_pointsInstant.bItemGrid[p] = wCell + 1; // set
468      m_pointsInstant.pItems[wCell] = pItem;
```

The stride is the literal `5`. `INVENTORY_WIDTH` is also 5 (`length.h:285`), so it
is correct today, but nothing in `char_item.cpp` says so. The 194 probe compares
them (`slot_space.cpp:268`) and that comparison is one of its live controls.

A page is `INVENTORY_WIDTH` 5 by `INVENTORY_HEIGHT` 9 = 45 cells
(`length.h:17-19`), laid out row-major, so `cell = row*5 + col`. A stride of 5 is
one **row** down. See D4 for what that costs.

### Arm `DRAGON_SOUL_INVENTORY` (471-521) — **reads before it checks**

```c
471  case DRAGON_SOUL_INVENTORY:
472      {
473          LPITEM pOld = m_pointsInstant.pDSItems[wCell];      // <-- UNCHECKED READ
474
475          if (pOld)
476          {
477              if (wCell < DRAGON_SOUL_INVENTORY_MAX_NUM)      // check AFTER the read
...
492              else
493                  m_pointsInstant.wDSItemGrid[wCell] = 0;     // <-- UNCHECKED WRITE
494          }
495
496          if (pItem)
497          {
498              if (wCell >= DRAGON_SOUL_INVENTORY_MAX_NUM)     // the ONLY bound check
500                  sys_err("CHARACTER::SetItem: invalid DS item cell %d", wCell);
501                  return;
...
516              else
517                  m_pointsInstant.wDSItemGrid[wCell] = wCell + 1;
518          }
519          m_pointsInstant.pDSItems[wCell] = pItem;            // <-- UNCHECKED WRITE
520      }
521      break;
```

The single bound check is at 498, and it lives **inside `if (pItem)`**. For a
removal (`pItem == NULL`) it is never reached. Line 473 has already read
`pDSItems[wCell]`, and line 519 goes on to write it, with `wCell` unbounded up to
65535 against a 1152-element array.

Verified by compiling the arm verbatim with an instrumented extent
(`/tmp/dsprobe/dsprobe3.c`):

```
declared extent: pDSItems[1152], wDSItemGrid[1152] -> valid indices 0..1151
CASE 1 remove(cell=2000)         : oob_read=1 oob_write=1 max_index=2000  sys_err=0
CASE 2 insert(cell=5000)         : oob_read=1 oob_write=0 max_index=5000  sys_err=1
CASE 3 remove(cell=1155,stale)    : oob_read=1 oob_write=1 max_index=1155  sys_err=0
CASE 4 remove(cell=65535)         : oob_read=1 oob_write=1 max_index=65535 sys_err=0
```

Read the `sys_err` column. Cases 1, 3 and 4 are **silent**: an out-of-bounds read
*and* write with no log line at all. Case 2 logs, but only after the out-of-bounds
read has already happened.

This is a defect a client can plausibly reach, because the only guard upstream,
`CItem::AddToCharacter` (`item.cpp:455-461`), is itself wrong — see D2.

### Arm `ATTR67_ADD` (523-532) — correct

```
525      if (wCell >= ATTR67_ADD_SLOT_MAX) { sys_err(...); return; }
530      m_pointsInstant.pAttr67AddItem = pItem;
```

Bound first, scalar store, no grid. The one clean arm besides INVENTORY.

### Arm `SWITCHBOT` (535-560) — **reads before it checks**

```c
535  case SWITCHBOT:
536  {
537      LPITEM pOld = m_pointsInstant.pSwitchbotItems[wCell];  // <-- UNCHECKED READ
538      if (pItem && pOld)
539      {
540          return;                                            // early return, no check
541      }
542
543      if (wCell >= SWITCHBOT_SLOT_COUNT)                      // the ONLY bound check
545          sys_err(...); return;
...
558      m_pointsInstant.pSwitchbotItems[wCell] = pItem;
```

`pSwitchbotItems` is 5 elements (`char.h:478`, `length.h:937`).
Verified (`/tmp/dsprobe/sbprobe.c`):

```
declared extent: pSwitchbotItems[5] -> valid indices 0..4
insert(cell=9)                              : oob_read=1 oob_write=0 max=9     sys_err=1
insert(cell=60000)                          : oob_read=1 oob_write=0 max=60000 sys_err=1
insert(cell=5,stale non-NULL, early return) : oob_read=1 oob_write=0 max=5     sys_err=0
```

The third line is the interesting one: the early return at 540 fires **before** the
bound check at 543, so an out-of-range cell whose neighbouring slot happens to be
non-`NULL` produces a silent out-of-bounds read and nothing else.

### `default` (562-564) — present

```c
562  default:
563      sys_err ("Invalid Inventory type %d", window_type);
564      return;
```

`SetItem`'s `switch` **has** a `default`. This is one of the two requested "missing
`default`" claims, and it is **false** for `SetItem`.

### The tail (567-620) — the output boundary

Only reached when the `switch` did not `return`. It builds `TPacketGCItemSet` on
insert, `TPacketGCItemDelDeprecated` on removal, and sends. The highlight field is
computed from the build switch at 585-589.

Because every arm that fails validation `return`s early, **a failed insertion sends
the client nothing at all.** The client keeps rendering the item it believes it has.
See D1.

---

## 3. `GetItem` and every accessor, transcribed

### `GetItem` — `char_item.cpp:254-305`

```c
254  LPITEM CHARACTER::GetItem(TItemPos Cell) const
255  {
256      if (!IsValidItemPosition(Cell))
257          return NULL;
258      WORD wCell = Cell.cell;
259      BYTE window_type = Cell.window_type;
260      switch (window_type)
261      {
262      case INVENTORY:
263      case EQUIPMENT:
264          if (wCell >= INVENTORY_AND_EQUIP_SLOT_MAX)
266              sys_err(...); return NULL;
269          return m_pointsInstant.pItems[wCell];
270      case DRAGON_SOUL_INVENTORY:
271          if (wCell >= DRAGON_SOUL_INVENTORY_MAX_NUM)
273              sys_err(...); return NULL;
276          return m_pointsInstant.pDSItems[wCell];
278  #if defined(__ATTR_6TH_7TH__)
279      case ATTR67_ADD:
281          if (wCell >= ATTR67_ADD_SLOT_MAX)
283              sys_err(...); return NULL;
287          return m_pointsInstant.pAttr67AddItem;
289  #endif
291  #ifdef ENABLE_SWITCHBOT
292      case SWITCHBOT:
293          if (wCell >= SWITCHBOT_SLOT_COUNT)
295              sys_err(...); return NULL;
298          return m_pointsInstant.pSwitchbotItems[wCell];
299  #endif
301      default:
302          return NULL;
303      }
304      return NULL;
305  }
```

Line 256 calls **`CHARACTER::IsValidItemPosition`** (`char_item.cpp:10010`), *not*
`SItemPos::IsValidItemPosition` (`length.h:973`). The two disagree on three windows,
and this is the only retrieval path in the server, so the disagreement is live.

**`GetItem` cannot distinguish an invalid position from an empty cell.** Both return
`NULL`. Line 256 returns `NULL`; line 269 returns `NULL` because the slot is empty.
Nothing downstream can tell them apart except by calling `IsValidItemPosition`
separately, and most callers do not.

**No `case SAFEBOX` and no `case MALL`.** Those windows pass validation at
10027-10037, which defers to `m_pkSafebox->IsValidPosition(cell)` / the mall's. With
a safebox open, a safebox position **is** valid, passes line 256, and then falls
through the whole `switch` to `default: return NULL` at 301-302. A position that its
own validator admitted retrieves nothing. This is ledger 194.5 and it is correct.

**`BELT_INVENTORY` is refused.** `CHARACTER::IsValidItemPosition`'s `switch` has no
belt arm, so it reaches `default: return false` at 10046-10047 and `GetItem` returns
`NULL` at 257. `SItemPos::IsValidItemPosition` *does* have the arm
(`length.h:981-982`). So a `{BELT_INVENTORY, 280}` passes the struct-level check used
by `exchange.cpp` and `DragonSoul.cpp:1163`, and is then refused by `GetItem`.

**The missing `break;` at 276-278 and 298-299 is not a real fall-through.** Both
cases end in `return`, so control cannot reach the next case. Verified by compiling
the body with `-Wimplicit-fallthrough=5` and a fall-through witness
(`/tmp/dsprobe/getitem2.c`):

```
DRAGON_SOUL_INV         100 -> 0x1e84e4  pDSItems[100]
ATTR67_ADD                0 -> 0x309     pAttr67AddItem
SWITCHBOT                 4 -> 0x2dc6c4  pSwitchbotItems[4]
fall-through test: DS cell 100 does NOT answer pAttr67AddItem, and
                   SWITCHBOT 4 does NOT answer pItems or NULL.
```

The compiler emitted no fall-through warning, and the answers identify the right
arrays. The missing `break;` is a readability Defect, not a behavioural one. **This
corrects the task's premise that there is a "real fall-through" in `GetItem`.**

`default:` at 301 **is** present. **The task's "missing `default`" claim is false for
`GetItem` too.** What is missing is `case SAFEBOX` / `case MALL`, which is a
different thing and is a real Defect.

Line 304 is unreachable: every `switch` path returns. Dead code, not a defect.

### The other accessors

| accessor | citation | bounds check | notes |
|---|---|---|---|
| `GetInventoryItem(WORD)` | 249-252 | none itself; delegates | `return GetItem(TItemPos(INVENTORY, wCell));` — so it inherits every `GetItem` behaviour, including the safebox hole if a caller ever passes a `SAFEBOX` window (it cannot; it hardcodes `INVENTORY`) |
| `GetWear(BYTE bCell)` | 647-656 | `bCell >= WEAR_MAX_NUM + DRAGON_SOUL_DECK_MAX_NUM * DS_SLOT_MAX` (64 + 12 = 76) | returns `pItems[INVENTORY_MAX_NUM + bCell]`, i.e. 180..255 |
| `GetCustomInventoryItem(BYTE cat, WORD cell)` | 309-317 | `cat >= CUSTOM_INVENTORY_CATEGORY_NUM` (6) | **correctly checks the category**, unlike `GetInventoryPageByPos` (194.6) |
| `GetAttr67AddItem(BYTE)` | `char.h:2510` | via `GetItem` | the scalar |
| `GetEmptyCustomInventory(BYTE cat, BYTE size)` | 319-332 | `cat >= 6` | returns an **absolute** cell, `CUSTOM_INVENTORY_SLOT_START + cat*180 + i` |
| `GetEmptyInventory(LPITEM, BYTE bSearchInventory)` | 1202-1237 | via `IsEmptyItemGrid` | see D6 |
| `GetEmptyInventory(BYTE size)` | 1239-1267 | via `IsEmptyItemGrid` | base inventory only |
| `GetEmptyInventoryEx(LPITEM)` | `char.h:1223` | via `IsEmptyItemGrid` | |
| `GetEmptyDragonSoulInventory(LPITEM)` | 1270-1289 | `IsEmptyItemGrid(DS, i + wBaseCell, bSize)` | `wBaseCell` from `DSManager::GetBasePosition`; `WORD_MAX` refused |
| `GetEmptyDragonSoulInventoryWithExceptions` | 1291-1309 | same | |
| `CountEmptyInventory()` | 1331-1347 | `Inventory_Size()` | returns `Inventory_Size() - count`, where `count` is the sum of **`GetSize()`**, not `GetCount()`. It counts *cells*, not items. |
| `CountSpecifyItem(DWORD vnum)` | 8791-8846 | `Inventory_Size()`, then `CUSTOM_INVENTORY_SLOT_START..END` | counts `GetCount()`, i.e. a stack count |
| `CountSpecifyTypeItem(BYTE type)` | 8957-8983 | `Inventory_Size()` | **counts the base band only.** Unlike `CountSpecifyItem` it has no custom-inventory second loop. |
| `FindSpecifyItem(DWORD vnum)` | 8724-8743 | `Inventory_Size()`, then custom | first match, ascending |
| `FindItemByID(DWORD id)` | 8763-8790 | `Inventory_Size()`, belt, custom | |
| `CopyDragonSoulItemGrid` | 1313-1318 | `resize(1152)` + `std::copy` | reads all 1152 `WORD`s |

`CountEmptyInventory` and `CountSpecifyItem` therefore mean different units (cells vs
stacks), and `CountSpecifyTypeItem` silently covers less of the inventory than
`CountSpecifyItem` does with the same signature shape. Neither is named so a reader
would know.

**`GetItemByVnum` does not exist.** `FindSpecifyItem(DWORD vnum)` is the function the
task's name corresponds to, and it returns the *first* match, not a count.

---

## 4. Removal

`CHARACTER::RemoveItem` **does not exist**. Removal is `SetItem(pos, NULL)`.

The two real removal paths, both outside `char_item.cpp`:

**`CItem::RemoveFromCharacter`** — `item.cpp:359-435`. Branches on item kind:

```c
386   if (IsDragonSoul())
388       if (m_wCell >= DRAGON_SOUL_INVENTORY_MAX_NUM) sys_err(...);   // LOG ONLY, no return
390       else pOwner->SetItem(TItemPos(m_bWindow, m_wCell), NULL);
394   else if (m_bWindow == SWITCHBOT)
396       if (m_wCell >= SWITCHBOT_SLOT_COUNT) sys_err(...);            // LOG ONLY, no return
402       else pOwner->SetItem(TItemPos(SWITCHBOT, m_wCell), NULL);
407   else {
408       TItemPos cell(INVENTORY, m_wCell);
411       if (false == IsDefaultInventoryPosition() && false == IsBeltInventoryPosition()
411                                       && false == IsCustomInventoryPosition())
416           sys_err("CItem::RemoveFromCharacter: Invalid Item Position");
420       else pOwner->SetItem(cell, NULL);
```

Both the DS arm (388) and the SWITCHBOT arm (396) **log and fall through**. Unlike
`AddToCharacter`, they have no `return`, so on an out-of-range cell they log and then
execute lines 428-433 unconditionally:

```c
428   m_pOwner = NULL;
429   m_wCell = 0;
431   SetWindow(RESERVED_WINDOW);
432   Save();
```

The item is detached from its owner and marked `RESERVED_WINDOW` **without ever being
removed from the array**. The `LPITEM` in `pDSItems[]` or `pSwitchbotItems[]` is now
owned by nobody. A later `Save()` has nothing to write it to, and the next load
restores it. This is a dangling-owner Defect, distinct from the out-of-bounds one,
and it needs the same out-of-range cell.

Note also line 380-384: SAFEBOX, MALL and ATTR67_ADD items skip the whole block,
because `SetItem` has no arm for them either.

**`ITEM_MANAGER::RemoveItem`** — `item_manager.cpp:566-595`. The SAFEBOX/MALL split at
577-586 is the compatibility shim; the `else` at 587-591 does
`o->SyncQuickslot(...)` then `item->RemoveFromCharacter()`. Then
`M2_DESTROY_ITEM(item)` at 594, unconditionally. A safe refusal in
`RemoveFromCharacter` therefore does not save the item.

---

## 5. Grid maintenance: `IsEmptyItemGrid` and the empty-cell helpers

### `IsEmptyItemGrid` — `char_item.cpp:739-1036`

Signature `bool IsEmptyItemGrid(TItemPos Cell, BYTE size, int iExceptionCell = -1) const`
(`char.h:1168`). Under `ENABLE_EXTEND_INVEN_SYSTEM` it is a `switch` on
`Cell.window_type` with arms `INVENTORY` (742), `DRAGON_SOUL_INVENTORY` (955),
`SWITCHBOT` (1017) and a closing `return false` at 1035.

The `iExceptionCell` protocol: a caller that is *moving* an item passes the source
cell so the grid is asked "is this cell free **ignoring the item I am moving from**".
`++iExceptionCell` at 762 and 963 then compares the stored marker against the
increment, so a grid value equal to `sourceCell + 1` is treated as empty. It is a
one-shot memo and it is not reused across a loop; `MoveItem` passes `Cell.cell`
(`char_item.cpp:7807`).

**`INVENTORY` arm, the belt sub-path (764-778)** — checks the belt item exists, then
`CBeltInventoryHelper::IsAvailableCell`, then the single grid cell. A belt item is
1x1, so there is no walk. Correct.

**`INVENTORY` arm, the walk (787-859)** — two nearly identical blocks:

```c
801   WORD p = bCell + (5 * j);
803   if (p >= wCategoryEndIndex)  return false;          // index guard
806   if (GetInventoryPageByPos(cat, p) != bPage) return false;   // PAGE guard
818   if (m_pointsInstant.bItemGrid[p])
819       if (m_pointsInstant.bItemGrid[p] != iExceptionCell) return false;
```
and the same again at 839-856. **The `INVENTORY` arm asks the right array twice in a
row.** Both the guard and the occupancy test use `bItemGrid`. Correct.

**`DRAGON_SOUL_INVENTORY` arm (955-1013)** — the exception block at 974-985 is
correct:

```c
976   int p = wCell + (DRAGON_SOUL_BOX_COLUMN_NUM * j);
978   if (p >= DRAGON_SOUL_INVENTORY_MAX_NUM) return false;
981   if (m_pointsInstant.wDSItemGrid[p])
982       if (m_pointsInstant.wDSItemGrid[p] != iExceptionCell) return false;
```

The empty-cell block at 996-1011 is **not**:

```c
1005                  if (m_pointsInstant.bItemGrid[p])        // <-- FLAT grid
1006                      if (m_pointsInstant.wDSItemGrid[p] != iExceptionCell)  // <-- DS grid
```

The guard asks `bItemGrid`; the comparison asks `wDSItemGrid`. These are two
independent arrays that nothing ever keeps in sync for DS traffic — `SetItem`'s DS
arm only touches `wDSItemGrid`. This is a copy-paste Defect and the two arms now
answer different questions. **The identical pair of lines appears at 1186-1187**, in
`IsEmptyItemGridSpecial`.

Verified (`/tmp/dsprobe/gridprobe2.c`), with a size-2 dragon soul anchored at 108
(writing `wDSItemGrid[108] = 109`, `wDSItemGrid[116] = 109`) and the flat grid all
zero, which is the state of any character whose DS box is in use and whose inventory
is empty:

```
state: wDSItemGrid[100]=0 (free)  wDSItemGrid[108]=109  wDSItemGrid[116]=109
       bItemGrid[108]=0 bItemGrid[116]=0 (flat grid never touched by DS)
ask: may a size-2 dragon soul be placed at DS cell 100?  iExceptionCell=1
  char_item.cpp:1005 form -> FREE (placed)
  char_item.cpp: 981 form -> OCCUPIED (refused)

second consequence: a stale flat-grid entry vetoes a genuinely free DS cell
  wDSItemGrid[200]=0 bItemGrid[208]=209 -> 1005 form says OCCUPIED
```

Two consequences, both real:

1. **It admits an overlap.** Cell 100 is free; 108 and 116 are not. The 1005 form
   says place, because `bItemGrid[108]` is 0. The 981 form says refuse. Legacy places
   a second dragon soul on cells another one already covers.
2. **It manufactures a false refusal.** `bItemGrid` is never cleared for DS traffic,
   so any flat-grid entry at `p` vetoes a DS cell the DS grid calls free. A player who
   has any item at flat cell 208 cannot place a multi-cell dragon soul at DS cell 200.

Neither is a crash. Both are silent, and both are on the dragon-soul path, which is
the item type with the strictest placement rules in the game.

### The `default` on `IsEmptyItemGrid`'s switch

There is **no `default`**, but there is a trailing `return false` at 1035. For a
`switch` whose arms all return, that is equivalent to `default: return false`. **So
this is a style difference, not a defect.** The third "missing `default`" claim in
the task is false.

The consequence is worth stating anyway: `IsEmptyItemGrid` **refuses**
`EQUIPMENT`, `SAFEBOX`, `MALL`, `ATTR67_ADD`, `AURA_REFINE`, `BELT_INVENTORY` and
`RESERVED_WINDOW` with no log line at all. `BELT_INVENTORY` is handled *inside* the
`INVENTORY` arm, not as a window byte, because belt cells are stored as `INVENTORY`.

### `GetEmptyCustomInventory` and `GetEmptyInventory`

`GetEmptyCustomInventory(cat, size)` (319-332) is correct: checks the category,
computes absolute bounds, calls `IsEmptyItemGrid`, returns the absolute cell.

`GetEmptyInventory(LPITEM pItem, BYTE bSearchInventory)` (1202-1237) has a Defect at
1214:

```c
1214   for(int catIndex = 0; catIndex < CUSTOM_INVENTORY_MAX_NUM; catIndex++)
1215   {
1216       if(pItem->IsCustomCategory(catIndex) && !bFoundCategory)
1217       {
1218           iEmptyPos = GetEmptyCustomInventory(catIndex, pItem->GetSize());
```

The loop bound is `CUSTOM_INVENTORY_MAX_NUM` = **180**, the number of *cells in one
category*. The number of *categories* is `CUSTOM_INVENTORY_CATEGORY_NUM` = **6**
(`length.h:31`). The loop therefore runs 180 times over 6 categories. It is not a
memory error — `GetEmptyCustomInventory` re-checks `bCategory >= 6` and returns `-1`,
and `IsCustomCategory` is called with a `BYTE` so 180 fits — but the loop is 30x longer
than it needs to be, and `GetEmptyCustomInventory(catIndex, ...)` takes a `BYTE` while
`catIndex` is an `int`, so the conversion happens silently. The intent was almost
certainly `CUSTOM_INVENTORY_CATEGORY_NUM`. This is a copy-paste Defect, same class as
the `bItemGrid`/`wDSItemGrid` mix, and it is why the first-match category rule
documented at ledger 194.6 is implemented twice: once in
`CItem::GetItemCategory` (`item.cpp:3149-3158`, correctly bounded by
`CUSTOM_INVENTORY_CATEGORY_NUM`) and once here.

Note also the `!bFoundCategory` at 1216: after a category is found and fills, the
remaining 179 iterations still call `IsCustomCategory` on every index. Dead work.

---

## 6. Defects, in severity order

### D1 — `SetItem` cannot report failure, so a lost item is silent

`SetItem` is `void`. Three arms `return` early on a bad cell, and each of those
returns happens **before** the packet tail at 567. So a rejected insert sends the
client no `GC_ITEM_SET`, and the client keeps drawing an item the server does not
have. `CItem::AddToCharacter` (`item.cpp:523-530`) does not know:

```c
523   ch->SetItem(TItemPos(window_type, pos), this, bHighlight);
527   m_pOwner = ch;
529   Save();
530   return true;                 // ALWAYS true
```

It sets the owner, saves, and returns `true` regardless of what `SetItem` decided.
Every caller that checks the return value is therefore checking a constant.

This is the single most consequential finding, because it converts each of D2, D3 and
D4 from "a log line" into "an item the client believes in and the server does not
have".

### D2 — `AddToCharacter` range-checks the wrong variable

`item.cpp:444-472`:

```c
444   WORD pos = Cell.cell;                  // the DESTINATION
...
455   else if (DRAGON_SOUL_INVENTORY == window_type)
456   {
457       if (m_wCell >= DRAGON_SOUL_INVENTORY_MAX_NUM)      // the item's STALE cell
459           sys_err("CItem::AddToCharacter: cell overflow: ... %d", m_wCell);
460           return false;
461   }
...
466       if (m_wCell >= SWITCHBOT_SLOT_COUNT)              // again m_wCell
```

`pos` is never consulted by any of the three guards. `pos` is first used at 523, when
it is handed to `SetItem`. A freshly created item has `m_wCell == 0`, so **every**
destination passes all three guards.

Verified (`/tmp/dsprobe/addprobe.c`):

```
  m_wCell   pos    guard  then SetItem receives
        0   5000    PASS    TItemPos(DRAGON_SOUL_INVENTORY, 5000) -> reads pDSItems[5000]
        0   1152    PASS
        0  65535    PASS
       99   2000    PASS
```

The `INVENTORY` arm at 449 has the same shape:
`if (m_wCell >= INVENTORY_MAX_NUM && BELT_INVENTORY_SLOT_START > m_wCell)` — a
compound condition that is true only for `m_wCell` in `[180, 273]`, which is the
equipment and DS-deck bands. It is a `m_wCell` check, not a `pos` check, and its
intent (belt cells are 274..289) does not match what it tests.

Combined with the D3 arm, this is the path by which a client-supplied DS or SWITCHBOT
cell above the extent reaches an out-of-bounds read. The two defects are one bug
between them.

### D3 — out-of-bounds read and write in the `DRAGON_SOUL_INVENTORY` and `SWITCHBOT` arms of `SetItem`

Detailed in §2. `char_item.cpp:473` reads `pDSItems[wCell]` and `:519` writes it with
no bound check; `:493` writes `wDSItemGrid[wCell]` with no bound check; and the only
check (`:498`) is inside `if (pItem)`, so removals skip it. `char_item.cpp:537` reads
`pSwitchbotItems[wCell]` before the check at `:543`, and the early return at `:540`
can skip that check too.

The 32-bit target means `pDSItems` is 4608 bytes of pointer array and
`wDSItemGrid` 2308 bytes of `WORD`, immediately followed in `TPointsInstant` by
whatever comes next, so a large `wCell` reads and writes unrelated character state.

### D4 — `SetItem`'s grid walk has no page guard; `IsEmptyItemGrid`'s does

`SetItem` marks the grid with a stride of 5 (one row down) and a single guard
(`p >= wCategoryEndIndex`, line 455). `IsEmptyItemGrid` walks with the same stride
but **also** refuses a walk that leaves the page (lines 806, 844). A page is
5 x 9 = 45 cells. Cell 40 is row 8, col 0 — the last cell of page 0 — so a size-2
item there needs cell 45, which is row 0, col 0 of **page 1**.

Verified (`/tmp/dsprobe/pageprobe.c`), scanning every cell 0..1369 and every size
2..9 against the active `ENABLE_CUSTOM_INVENTORY` arm:

```
size=2 anchor= 40 (page 0)  IsEmptyItemGrid REFUSES, but SetItem would mark: 40 45*
size=2 anchor= 41 (page 0)  IsEmptyItemGrid REFUSES, but SetItem would mark: 41 46*
...
* marks a cell outside the anchor's own 45-cell page.
```

And the page guard is definitely live, not vacuous (`/tmp/dsprobe/pageguard2.c`):

```
arm                    line 841 (index guard)   line 844 (page guard)
base inventory 0..179     180 (first cell 140)     540 (first cell   5)
custom 290..1369         1080 (first cell 430)    3240 (first cell 295)
```

720 of the guards that fire are the **page** guard. So `IsEmptyItemGrid` refuses
these placements, but `SetItem` will mark across the page boundary if it is called
directly. And it is called directly from about 70 sites (the `AddToCharacter` sweep
in §8), several of which skip `IsEmptyItemGrid` entirely.

This is not a memory error: the flat array is contiguous, so cell 45 is a legal
index. It is a **grid-truth** error. `bItemGrid[45]` is set to 41, so cell 45 on
page 1 now reports occupied by an item the client drew on page 0. Page 1 loses a
cell, and the item's own footprint is wrong. The reverse also happens: the item's
real cells on page 0 are marked, but the client draws the item clipped.

### D5 — `RemoveFromCharacter` logs and continues, leaving a dangling owner

`item.cpp:388-389` and `396-399`, detailed in §4. Both DS and SWITCHBOT arms log a
bad cell and then fall through to 428-433, which clear the owner and the window
without clearing the array. The `LPITEM` is left in `pDSItems[]` /
`pSwitchbotItems[]` owned by nobody. `ITEM_MANAGER::RemoveItem` then destroys the
item at `item_manager.cpp:594` regardless. This is a use-after-free in the making:
the array holds a pointer to a destroyed `CItem`, and the next `GetItem` on that cell
returns it.

This one is **worse** than the out-of-bounds write, because it produces a dangling
pointer that is read back through a normal, in-bounds `GetItem`.

### D6 — `bItemGrid` where `wDSItemGrid` belongs

`char_item.cpp:1005-1006` and `1186-1187`. Detailed in §5. Silent, and it both admits
an overlap and manufactures a false refusal on the dragon-soul path.

### D7 — `GetEmptyInventory`'s category loop is bounded by the wrong constant

`char_item.cpp:1214`: `catIndex < CUSTOM_INVENTORY_MAX_NUM` (180) where
`CUSTOM_INVENTORY_CATEGORY_NUM` (6) is meant. 30x dead work, plus a silent `int` ->
`BYTE` narrowing at 1218. Not a memory error.

### D8 — `GetItem` admits a safebox or mall position and then retrieves nothing

`char_item.cpp:256` admits it, `301-302` returns `NULL`. Ledger 194.5. The retrieval
side has `default:` but no `case SAFEBOX` / `case MALL`.

### D9 — `GetItem` cannot distinguish an invalid cell from an empty cell

Both return `NULL`. Not a memory error, but every caller that wants to tell them
apart must call `IsValidItemPosition` itself, and most do not.

### D10 — `assert(!"GetOwner exist")` compiles out in Release

`char_item.cpp:380-384`. `premake5.lua:54,59` defines `NDEBUG` in `Release` and
`Fast-Release`. A double-owned item is therefore dropped with **no** diagnostic
whatsoever, and by D1 with no packet either.

### D11 — `RemoveFromCharacter`'s own `sys_err` and the destroy are not connected

`item_manager.cpp:594` calls `M2_DESTROY_ITEM(item)` unconditionally, outside the
`if ((o = item->GetOwner()))` block. So any failure inside `RemoveFromCharacter` is
followed by destruction regardless.

### D12 — the `0xff` pointer test is dead on the 32-bit target

`char_item.cpp:373`. `(unsigned long)((CItem*)pItem) == 0xff` can never be true for a
4-byte pointer. Harmless; recorded because it is the residue of a 16-bit assumption
and it sits immediately above a real bug.

### D13 — `CountSpecifyTypeItem` covers less than `CountSpecifyItem`

`char_item.cpp:8957-8983` has no custom-inventory second loop, where
`CountSpecifyItem` (8824-8843) has one. Two functions with the same shape and
different coverage.

### D14 — `IsEmptyItemGrid`'s `switch` has no `default`

Only a style issue: the trailing `return false` at 1035 covers every remaining byte.
Not a behavioural Defect. Recorded because the task asked, and because the *absence*
of `default` is what lets `EQUIPMENT`, `SAFEBOX` and `ATTR67_ADD` be refused
silently.

---

## 7. Negative claims, with controls

### 7.1 The three storage-size claims in ledger 194

194.1 claims `pItems[1370]`, `bItemGrid[1370]`, `pDSItems[1152]`, `wDSItemGrid[1152]`
and 15,132 bytes total. **All five confirmed** by building the tree's own committed
probe (`git show HEAD:.scratch/probe194/slot_space.cpp`):

```
char.h  pItems[1370] bItemGrid[1370] pDSItems[1152] wDSItemGrid[1152]
i686 bytes  pItems=5480 bItemGrid=2740 pDSItems=4608 wDSItemGrid=2304 total=15132
```

The six ranges are also confirmed contiguous and ending at 1370.

**The 15,132 is arithmetic, not a measurement.** The probe prints the line with
literal `4 *` and `2 *` multipliers, so the same output appears from a 64-bit build.
The figure is correct for the legacy 32-bit target, where `LPITEM` is 4 bytes and
`WORD` is 2, but nothing in the probe proves it. See §7.3.

194.3's `EWindows` table is confirmed: all eleven values match the compiler.

### 7.2 The four absent functions

Each absence was checked with a positive control (a term that *does* exist, so the
sweep demonstrably can find things) and a negative control (a term that does not
exist, so a silent sweep is distinguishable from a sweep that ran).

```
=== NEGATIVE: GetItemByVnum ===          (no output, exit 1)
=== POSITIVE control: FindSpecifyItem ===
  char.h:1231            LPITEM FindSpecifyItem(DWORD vnum) const;
  item.cpp:318           LPITEM pItem = pOwner->FindSpecifyItem(GetVnum());
  char_item.cpp:6874     LPITEM pSource1 = FindSpecifyItem(item->GetValue(1));
  ... (5 hits)

=== NEGATIVE: CHARACTER::RemoveItem ===  (no output, exit 1)
=== POSITIVE control: ITEM_MANAGER::RemoveItem ===
  item_manager.cpp:566   void ITEM_MANAGER::RemoveItem(LPITEM item, const char * c_pszReason)

=== NEGATIVE: CountEmptyItemGrid ===     (no output, exit 1)
=== NEGATIVE: CHARACTER::GetItemCount ===(no output, exit 1)
=== POSITIVE control: GetItemCount DOES exist elsewhere ===
  private_shop.h:20      WORD GetItemCount() { return m_map_shopItem.size(); }
  shop.h:92              int  GetItemCount();
=== POSITIVE: the helpers that DO exist ===
  char.h:1168  bool IsEmptyItemGrid(TItemPos Cell, BYTE size, int iExceptionCell = -1) const;
  char.h:1177  int  GetEmptyInventory(LPITEM pItem, BYTE bSearchInventory = 0) const;
  char.h:1227  int  CountEmptyInventory() const;
```

So: `GetItemByVnum`, `CHARACTER::RemoveItem`, `CountEmptyItemGrid` and
`CHARACTER::GetItemCount` are **not found**. The controls confirm the sweeps ran.

### 7.3 The 194 probe silently transcribes a legacy constant wrong

**First, a correction to my own earlier reading.** I initially rebuilt the *working
tree* copy of `.scratch/probe194/slot_space.cpp` and it reported four control
failures with exit 1, which would have made ledger 194.13's "all controls passed
(0 control failures)" false. That was wrong. The working tree carries an
**uncommitted** edit (`git diff` shows +67 lines, mtime 14:51 today) that is not mine.
Building the **committed** source with `git show HEAD:`:

```
char.h  pItems[1370] bItemGrid[1370] pDSItems[1152] wDSItemGrid[1152]
i686 bytes  pItems=5480 bItemGrid=2740 pDSItems=4608 wDSItemGrid=2304 total=15132
all controls passed (0 control failures)
exit=0
```

**Ledger 194.13 is accurate for the committed probe.** The four failures belong
entirely to the uncommitted edit. Recorded so the next session does not re-derive it.

The finding that survives is narrower and still real. Both the committed copy
(line 119) and the working copy (line 129) read:

```c
enum E8 { DRAGON_SOUL_BOX_SIZE = 32, DRAGON_SOUL_BOX_COLUMN_NUM = 6,
          DRAGON_SOUL_BOX_ROW_NUM = DRAGON_SOUL_BOX_SIZE / DRAGON_SOUL_BOX_COLUMN_NUM };
```

`length.h:84` says **`DRAGON_SOUL_BOX_COLUMN_NUM = 8`**. **No control in the
committed probe checks this value**, so the probe passes while enshrining a
constant that disagrees with the frozen source. The uncommitted edit adds
`check("DRAGON_SOUL_BOX_COLUMN_NUM", DRAGON_SOUL_BOX_COLUMN_NUM, 8)` at its line 261,
and that check fails — which is how the discrepancy was found. The same edit adds a
corrected `check("safe rows", DRAGON_SOUL_BOX_ROW_NUM, 32 / 8)` at its line 262 while
leaving the stale `check("safe rows", DRAGON_SOUL_BOX_ROW_NUM, 32 / 6)` at line 241, so
it now contains two contradictory row checks.

**This matters for the storage, not just the probe.** `DRAGON_SOUL_BOX_COLUMN_NUM` is
the DS grid stride at `char_item.cpp:481, 508, 976, 1000, 1156, 1181` and at
`exchange.cpp:433, 442` and `private_shop.cpp:414, 425`. The frozen source says 8, and
`DSManager::GetBasePosition` (`DragonSoul.cpp:130-148`) confirms the geometry: a box
occupies `col_type * 6 * 32 + row_type * 32` cells, i.e. 32 cells arranged **8 wide**
(`DRAGON_SOUL_BOX_SIZE / DRAGON_SOUL_BOX_COLUMN_NUM = 4` rows). A stride of 8 is one
row down inside an 8-column box. With the probe's 6, `DRAGON_SOUL_BOX_ROW_NUM` becomes
5 and the box no longer divides evenly — 32 is not a multiple of 6. **The frozen
source is self-consistent at 8; the probe is not.** The Rust `common::item_slots`
constants must be re-checked against `length.h:84` before the item-instance unit
builds on them, because a probe that never checks this value will not catch a port
that copies the 6.

**And the "i686 bytes" line is not a measurement.** The working copy declares a
`CItemStorage` struct holding all nine item members (its lines 191-201) and a
`static_assert(sizeof(WORD) == 2)` at 184, but **`sizeof(CItemStorage)` is never
evaluated.** The reported total comes from literals:

```c
printf("i686 bytes  pItems=%d ... total=%d\n",
       4 * INVENTORY_AND_EQUIP_SLOT_MAX, 2 * INVENTORY_AND_EQUIP_SLOT_MAX, ...);
```

The `4` and the `2` are hand-written for the legacy 32-bit target. A 64-bit build
prints the same 15132, which is the proof it is not measuring anything. **The figure
in ledger 194.1 is right by arithmetic, not by `sizeof`,** and the struct added by the
uncommitted edit does not change that.

**Two of the four failures are the edit's own bad controls.** Its lines 273-280:

```c
int worst_flat = (INVENTORY_AND_EQUIP_SLOT_MAX - 1) + ((int)INVENTORY_MAX_NUM - 1) * 5;
if (worst_flat >= INVENTORY_AND_EQUIP_SLOT_MAX) { ...fail... }
int worst_ds = (DRAGON_SOUL_INVENTORY_MAX_NUM - 1) + ((int)DRAGON_SOUL_BOX_SIZE - 1) * DRAGON_SOUL_BOX_COLUMN_NUM;
if (worst_ds >= DRAGON_SOUL_INVENTORY_MAX_NUM) { ...fail... }
```

These compute a worst case with **no guard modelled**, then assert it stays in
bounds. It cannot: `1369 + 179*5 = 2264` and `1151 + 31*6 = 1337`. But the frozen
source *does* guard every step — `char_item.cpp:425-426` (`if (p >=
wCategoryEndIndex) continue;`) and `:510-511` (`if (p >= DRAGON_SOUL_INVENTORY_MAX_NUM)
continue;`). The control asserts a property of arithmetic the code never performs.
Those are falsified controls, not discovered defects.

Recommended before ledger 195: keep the new `DRAGON_SOUL_BOX_COLUMN_NUM` check and
delete or guard the two stack checks, and either evaluate `sizeof(CItemStorage)`
under a 32-bit target or stop labelling the literal sum "i686 bytes".

---

## 8. Every `SetItem` and removal caller

### In `char_item.cpp`

| line | call | result |
|---|---|---|
| 366, 368 | the two definitions | — |
| 667, 669 | `SetWear`: `SetItem(TItemPos(INVENTORY, INVENTORY_MAX_NUM + bCell), item, false)` | `SetWear` checks `bCell < 76` first (660). `180 + 75 = 255 < 1370`. Safe. Note it uses window `INVENTORY` for an equipment cell, which is why `GetItem`'s `INVENTORY`/`EQUIPMENT` cases share an arm. |
| 7817, 7819 | `MoveItem`: `item->RemoveFromCharacter(); SetItem(DestCell, item, false);` | `MoveItem` validates both ends (7612, 7672) and `IsEmptyItemGrid(DestCell, ...)` (7807). The `SetItem` return is `void` and nothing is checked. |
| 10587 | `SetItem(TItemPos(ATTR67_ADD, 0), pkRegistItem)` | `ATTR67_ADD_SLOT_MAX` is 1 and the cell is the literal 0. Safe. |

**`SetItem` is called exactly three times in `char_item.cpp`**, plus the two
definitions. There is **no `SetItem` call in `input_main.cpp`** (sweep returns
nothing; positive control: `SetItem` does match 7 times in `char_item.cpp`).

### In `input_main.cpp`

`input_main.cpp` never calls `SetItem`. It reaches storage through
`AddToCharacter` and `RemoveFromCharacter`, and **discards the result in all three
places**:

| line | handler | call | result discarded |
|---|---|---|---|
| 2358 | `SafeboxCheckin` (2276) | `pkItem->RemoveFromCharacter();` | returns `LPITEM`, never `NULL` in practice |
| 2361 | `SafeboxCheckin` | `pkSafebox->Add(p->bSafePos, pkItem);` | `CSafebox::Add` returns `bool` (`safebox.cpp:56`) and returns `false` on an invalid position (58-62). **Discarded.** The item was already removed from the character at 2358, so on a `false` it is in no window at all. |
| 2428 | `SafeboxCheckout` (2368) | `pkItem->AddToCharacter(ch, DestPos);` | `bool`, always `true` (D1). Discarded. |
| 2446 | `SafeboxCheckout` | `pkItem->AddToCharacter(ch, p->ItemPos);` | same. Discarded. |

`SafeboxCheckout` does call `ch->IsEmptyItemGrid(p->ItemPos, pkItem->GetSize())` at
2399 before line 2446, so the cell is grid-checked. It does **not** call
`AddToCharacter`'s result, and per D1 that result is a constant anyway.

Line 2428's `DestPos` is a server-computed `GetEmptyDragonSoulInventory` result when
`IsValidCellForThisItem` fails (`DragonSoul.cpp:501-513` shape, mirrored at
`input_main.cpp:2415-2423`), so it is bounded. The `2446` path uses the client's
`p->ItemPos` directly — that is the client-supplied cell, and for a
`DRAGON_SOUL_INVENTORY` window it is grid-checked by `IsEmptyItemGrid` (which refuses
`wCell >= 1152` at 958) before `AddToCharacter` runs. So D2 is **not** reachable from
`SafeboxCheckout`.

`SafeboxCheckin` line 2361 is the reachable one for the safebox, and it is a
lost-item path independent of D1: `RemoveFromCharacter` succeeds, `Add` refuses, and
nothing is logged at the call site.

### The wider `AddToCharacter` surface

A sweep for `AddToCharacter` across `server/server/game/` returns about 70 call
sites. Of these, the ones that take a **client- or shop-supplied** cell rather than a
server-computed empty cell are the ones where D2 can bite:

| site | cell source |
|---|---|
| `input_main.cpp:2428, 2446` | client `ItemPos` (safebox checkout) — grid-checked at 2399 |
| `private_shop_manager.cpp:478, 608` | shop `TDstPos` / `iPos` — **the return IS checked** (`if (!pItem->AddToCharacter(...))`) |
| `private_shop.cpp:469` | shop position — return checked |
| `char.cpp:9497, 9692` | `shop->pos` — **return discarded** |
| `input_db.cpp:1516` | `p->window`, `p->pos` from the store — **return discarded** |
| `exchange.cpp:547, 549` | `GetEmptyDragonSoulInventory` / `GetEmptyInventory` — server-computed, return discarded |
| `DragonSoul.cpp:533, 563` | `DestCell`, defaulted to a server-computed cell — return discarded |
| `shop.cpp`, `shopEx.cpp`, `cuberenewal.cpp`, `cmd_general.cpp`, `mining.cpp`, `fishing.cpp`, `questlua_*.cpp` | all `GetEmptyInventory` / `GetEmptyDragonSoulInventory` — server-computed, return discarded |

So the D2 exposure is: any path where a cell is not first produced by
`GetEmptyInventory`/`GetEmptyDragonSoulInventory`, **and** the return is discarded.
`input_db.cpp:1516` is the notable one: it restores `p->window` and `p->pos`
verbatim from the store, with no `IsEmptyItemGrid` and no returned check. A
`DRAGON_SOUL_INVENTORY` row with `pos >= 1152` in the legacy database reaches
`SetItem`'s line 473. Per AGENTS.md the Rewrite uses a fresh PostgreSQL 18 store
(ADR-0003), and `legacy/sql` is reference-only, so the row would have to be written
by hand — but nothing in the code prevents it, and the old database is exactly the
data a migration would carry over.

---

## 9. What this unit does not claim

- **No `SetItem` call exists in `input_main.cpp`.** Verified with a positive control
  (§8). Its storage writes go through `AddToCharacter` / `RemoveFromCharacter` /
  `CSafebox::Add`.
- **No Parity claim.** Nothing here moves a `sys.item.core` row.
- **The `BELT_INVENTORY` window byte is not a storage location.** Belt cells live in
  `pItems` under `INVENTORY`; `EWindows::BeltInventory` names only the persisted byte.
- **The `EQUIPMENT` window and the equipment band are the same cells.** `SetWear`
  passes `TItemPos(INVENTORY, 180 + bCell)`, never `TItemPos(EQUIPMENT, ...)`; the
  `EQUIPMENT` arm in `GetItem` exists for the persisted byte and shares the array.
- **The safebox is a different structure.** `CSafebox` has its own
  `m_pkItems[SAFEBOX_MAX_NUM]` (`45 * 6 = 270`, `safebox.cpp:37`) and its own
  `CItemGrid`. It is not a window into `pItems`.
- **The DS geometry is 8 columns.** Confirmed from `length.h:84` and
  `DSManager::GetBasePosition`, and **not** from the 194 probe, which transcribes 6
  (§7.3).
- **The i686 cross compiler was not used.** `AGENTS.md` warns it is not installed
  everywhere. Every width here comes from `WORD`/`LPITEM` in a counted array, and the
  15,132-byte total was reproduced by the tree's own probe rather than by
  cross-compiling. The `4 *` and `2 *` factors in that probe's own `printf` are
  hand-written multipliers, not `sizeof` results — so 15,132 is arithmetic that
  happens to be right, not a measurement.

---

## 10. Ledger 194 verification summary

| 194 claim | verdict |
|---|---|
| `pItems[1370]`, `bItemGrid[1370]`, `pDSItems[1152]`, `wDSItemGrid[1152]` | **confirmed** |
| 15,132 bytes on the 32-bit target | **confirmed** by arithmetic, not by `sizeof` |
| six contiguous ranges ending at 1370 | **confirmed** |
| all eleven `EWindows` values | **confirmed** |
| `bItemGrid` is a `WORD` (so `wCell + 1` for cell 1369 does not wrap) | **confirmed**, and it is `char.h:459` |
| belt cells stored as `INVENTORY` | **confirmed**, `SetCell` at 617-631 and `input_db.cpp:1491-1495` |
| `GetItem` has no `SAFEBOX`/`MALL` case (194.5) | **confirmed**, and it is Defect D8 |
| `GetInventoryPageByPos` never range-checks its category (194.6) | **confirmed**, `char_item.cpp:334-347` |
| `Inventory_Size()` is a stat, not a bound (194.7) | **confirmed**, and `CountEmptyInventory` at 1331 depends on the conflation |
| probe prints "all controls passed" (194.9, 194.13) | **CONFIRMED for the committed probe.** exit 0, 0 failures. The 4 failures I first saw belong to an uncommitted edit (§7.3) |
| `DRAGON_SOUL_BOX_COLUMN_NUM` | probe says 6, **`length.h:84` says 8**. Source is right, and **no committed control checks it** (§7.3) |
| 15132 measured by `sizeof` | **FALSE.** The probe prints literal `4 *` and `2 *` multipliers; a 64-bit build prints the same number (§7.3) |

194.10 says "No item instance and no item storage. `SetItem`, `GetItem`, `RemoveItem`,
and the safebox are not touched." That is accurate as a statement about the Rust
tree, and this survey is the floor's tenant, as the entry anticipated. Nothing in
194 contradicts this report; the two disagreements are the probe's failing controls
and the `6`-versus-`8` transcription.
