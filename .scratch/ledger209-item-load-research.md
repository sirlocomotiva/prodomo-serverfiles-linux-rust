# Ledger 209 research: the legacy inventory load / relog path

**Status:** research only. No Rust was edited, no Cargo was run, and no width probe was
compiled. Every width below is summed from a packed legacy struct whose field list is quoted,
and the feature gates that select those fields are quoted with it. Where this document claims
something is absent, a positive control and a negative control for the same search are recorded
in section 9.

**Correction (ledger 210).** Two statements about the tail below are wrong; the body is kept as
written. `PointsPacket` sends `GC_CHARACTER_GOLD` (byte 224, 9 bytes) **before**
`GC_CHARACTER_POINTS`, under `ENABLE_REMOVE_LIMIT_GOLD` (`char.cpp:2078-2085`,
`prodomodefines.h:157`), so the tail is gold then points, not points alone (the
`GC_CHARACTER_POINTS` paragraph and step 4 of the send order). And `CheckMaximumPoints` is at `input_db.cpp:1563`, **before**
`PointsPacket` at `:1564`, so a byte-17 point change it sends comes before the gold and points
records, not after them. `docs/PROTOCOL_NOTES.md` section 210 has the corrected account.

**Read first, because it changes the shape of the task:** there is no `CG_ITEM_LOAD` packet in
this legacy build. The name belongs to a DB-peer record. The inventory load is driven by the
DB server as a side effect of character select, and it lands on the client between the loading
burst and `CG_ENTERGAME`. Section 1 proves the absence and section 2 gives the trigger that
does exist.

---

## 1. `CG_ITEM_LOAD` does not exist; `HEADER_DG_ITEM_LOAD` does

The record the task names is a **DB peer** record, not a client record.

`server/server/common/tables.h:196`

```
	HEADER_DG_ITEM_LOAD			= 42,
```

`server/server/game/input_db.cpp:1451` is the handler, and `CInputDB` is the DB-server
descriptor, not the client one.

**Absence claim, with controls.** There is no `CG_ITEM_LOAD`, no `HEADER_CG_ITEM_LOAD`, and no
client-sent record that asks the server to load inventory. Measured with `command grep -a`
(both `*.h` and `*.H` swept) over `server/server/game/packet.h`:

| search | result | role |
|---|---|---|
| `HEADER_CG_ITEM` | 11 | positive control: the search does reach the inventory CG headers |
| `HEADER_CG_ZZZ_NO_SUCH_NAME` | 0 | negative control: a name that cannot exist matches nothing |
| `ITEM_LOAD` | 0 | the claim |
| `CG_ITEM` | 11 | positive control for the second spelling |

`server/server/game/packet.h` therefore registers no client-sent item-load record. The
`HEADER_CG_ITEM*` block that does exist is the ordinary inventory CG set
(`CG_ITEM_MOVE`, `CG_ITEM_USE`, `CG_ITEM_DROP`, and so on).

Two further independent confirmations of the same absence:

- `server/server/game/packet_info.cpp:117-161` is the inbound CG registration table. It
  registers the inventory CG records and contains no item-load entry. Positive control: the
  same range registers 11 `CG_ITEM*` handlers.
- `protocol/src/cg_inventory.rs` is the Rewrite's machine-readable CG inventory. It has no
  load row either, and its coverage test would fail if one were added without a codec.

The task's brief therefore needs its trigger replaced. Section 2 gives the real one.

---
## 2. The real trigger: `CG_CHARACTER_SELECT` (byte 6)

The client asks for a character's data, and the inventory comes back as part of that answer.
Nothing in the client protocol names it "item load".

`server/server/game/packet.h:17`

```
	HEADER_CG_CHARACTER_SELECT					= 6,
```

`server/server/game/input_login.cpp:1185-1187` dispatches it:

```
		case HEADER_CG_CHARACTER_SELECT:
			CharacterSelect(d, c_pData);
			break;
```

`CInputLogin::CharacterSelect` answers with a DB request, not with game data
(`server/server/game/input_login.cpp:298-304`):

```
	TPlayerLoadPacket player_load_packet;

	player_load_packet.account_id	= c_r.id;
	player_load_packet.player_id	= c_r.players[pinfo->index].dwID;
	player_load_packet.account_index	= pinfo->index;

	db_clientdesc->DBPacket(HEADER_GD_PLAYER_LOAD, d->GetHandle(), &player_load_packet, sizeof(TPlayerLoadPacket));
```

The DB server replies with the player row and then the item rows, and the game server consumes
the item rows while the descriptor is still in its loading phase. The phase is set on the way
in, before any item arrives (`server/server/game/input_db.cpp:414-416`):

```
	d->SetPhase(PHASE_LOADING);
	SECTREE_MANAGER::Instance().SendEntity(ch);
	ch->MainCharacterPacket();
```

`PHASE_LOADING` is handled by the **login** input processor, not the game one
(`server/server/game/desc.cpp:515-532`):

```
		case PHASE_SELECT:
			// ... (comment elided)
		case PHASE_LOGIN:
		case PHASE_LOADING:
#ifndef _IMPROVED_PACKET_ENCRYPTION_
			m_bEncrypted = true;
#endif
			m_pInputProcessor = &m_inputLogin;
			break;

		case PHASE_GAME:
		case PHASE_DEAD:
#ifndef _IMPROVED_PACKET_ENCRYPTION_
			m_bEncrypted = true;
#endif
			m_pInputProcessor = &m_inputMain;
			break;
```

So the ordering constraint is exact and it is a phase constraint, not a packet trigger:

```
CG_LOGON / CG_LOGIN2
  -> CG_CHARACTER_SELECT  (byte 6)
     -> GD_PLAYER_LOAD -> PHASE_LOADING
        -> main character packet, skills, points      (the loading burst)
        -> DG_ITEM_LOAD  -> one GC_ITEM_SET per item  (this path)
     -> GC_MAIN_CHARACTER / map setup completes
  -> CG_ENTERGAME  (byte 10) -> PHASE_GAME
```

`CG_ENTERGAME` is byte 10 (`server/server/game/packet.h:20`), dispatched at
`server/server/game/input_login.cpp:1197-1199`:

```
		case HEADER_CG_ENTERGAME:
			Entergame(d, c_pData);
			break;
```

and `CInputLogin::Entergame` (`server/server/game/input_login.cpp:562`) closes the loading
phase at `server/server/game/input_login.cpp:608`:

```
	d->SetPhase(PHASE_GAME);
```

**Consequence for the port:** the inventory records go out **after** the loading burst and
**before** `CG_ENTERGAME`. There is no client request to hang the load off, and there is no
window in which the client may ask for items. The Rewrite has to send them from the
select-character step, which is where section 6 shows the hook already exists.

---
## 3. `CInputDB::ItemLoad`, verbatim

`server/server/game/input_db.cpp:1451-1567`, quoted in full. Nothing in this function is
elided; the only non-ASCII line in the range is line 1476, which is already commented out.

```
1451: void CInputDB::ItemLoad(LPDESC d, const char * c_pData)
1452: {
1453: 	LPCHARACTER ch;
1454:
1455: 	if (!d || !(ch = d->GetCharacter()))
1456: 		return;
1457:
1458: 	if (ch->IsItemLoaded())
1459: 		return;
1460:
1461: 	DWORD dwCount = decode_4bytes(c_pData);
1462: 	c_pData += sizeof(DWORD);
1463:
1464: 	sys_log(0, "ITEM_LOAD: COUNT %s %u", ch->GetName(), dwCount);
1465:
1466: 	std::vector<LPITEM> v;
1467:
1468: 	TPlayerItem * p = (TPlayerItem *) c_pData;
1469:
1470: 	for (DWORD i = 0; i < dwCount; ++i, ++p)
1471: 	{
1472: 		LPITEM item = ITEM_MANAGER::instance().CreateItem(p->vnum, p->count, p->id);
1473:
1474: 		if (!item)
1475: 		{
1476: 			//sys_err("cannot create item by vnum %u (name %s id %u)", p->vnum, ch->GetName(), p->id);
1477: 			continue;
1478: 		}
1479:
1480: 		item->SetSkipSave(true);
1481: 		item->SetSockets(p->alSockets);
1482: 		item->SetAttributes(p->aAttr);
1483: 		item->SetLastOwnerPID(p->owner);
1484: #ifdef ENABLE_REFINE_ELEMENT
1485: 		item->SetRefineElement(p->dwRefineElement);
1486: #endif
1487: #ifdef __CHANGELOOK_SYSTEM__
1488: 		item->SetTransmutation(p->transmutation);
1489: #endif
1490: #ifdef ENABLE_BELT_INVENTORY_EX
1491: 		if (p->window == BELT_INVENTORY)
1492: 		{
1493: 			p->window = INVENTORY;
1494: 			p->pos = p->pos + BELT_INVENTORY_SLOT_START;
1495: 		}
1496: #endif
1497:
1498: 		if ((p->window == INVENTORY && ch->GetInventoryItem(p->pos)) ||
1499: 				(p->window == EQUIPMENT && ch->GetWear(p->pos)))
1500: 		{
1501: 			sys_log(0, "ITEM_RESTORE: %s %s", ch->GetName(), item->GetName());
1502: 			v.push_back(item);
1503: 		}
1504: 		else
1505: 		{
1506: 			switch (p->window)
1507: 			{
1508: 				case INVENTORY:
1509: 				case DRAGON_SOUL_INVENTORY:
1510: #if defined(__ATTR_6TH_7TH__)
1511: 				case ATTR67_ADD:
1512: #endif
1513: #ifdef ENABLE_SWITCHBOT
1514: 				case SWITCHBOT:
1515: #endif
1516: 					item->AddToCharacter(ch, TItemPos(p->window, p->pos));
1517: 					break;
1518:
1519: 				case EQUIPMENT:
1520: 					if (item->CheckItemUseLevel(ch->GetLevel()) == true )
1521: 					{
1522: 						if (item->EquipTo(ch, p->pos) == false )
1523: 						{
1524: 							v.push_back(item);
1525: 						}
1526: 					}
1527: 					else
1528: 					{
1529: 						v.push_back(item);
1530: 					}
1531: 					break;
1532: 			}
1533: 		}
1534:
1535: 		if (false == item->OnAfterCreatedItem())
1536: 			sys_err("Failed to call ITEM::OnAfterCreatedItem (vnum: %d, id: %d)", item->GetVnum(), item->GetID());
1537:
1538: 		item->SetSkipSave(false);
1539: 	}
1540:
1541: 	itertype(v) it = v.begin();
1542:
1543: 	while (it != v.end())
1544: 	{
1545: 		LPITEM item = *(it++);
1546:
1547: 		int pos = ch->GetEmptyInventory(item->GetSize());
1548:
1549: 		if (pos < 0)
1550: 		{
1551: 			PIXEL_POSITION coord;
1552: 			coord.x = ch->GetX();
1553: 			coord.y = ch->GetY();
1554:
1555: 			item->AddToGround(ch->GetMapIndex(), coord);
1556: 			item->SetOwnership(ch, 180);
1557: 			item->StartDestroyEvent();
1558: 		}
1559: 		else
1560: 			item->AddToCharacter(ch, TItemPos(INVENTORY, pos));
1561: 	}
1562:
1563: 	ch->CheckMaximumPoints();
1564: 	ch->PointsPacket();
1565:
1566: 	ch->SetItemLoaded();
1567: }
```

### 3.1 Signature and guards

- Signature: `void CInputDB::ItemLoad(LPDESC d, const char * c_pData)`. There is **no length
  parameter**. The function cannot validate how many bytes it is being handed.
- `server/server/game/input_db.cpp:1455-1456` returns if the descriptor or the character is
  missing.
- `server/server/game/input_db.cpp:1458-1459` returns if `ch->IsItemLoaded()`. This is the only
  re-entrancy guard, and it is what makes a duplicate `DG_ITEM_LOAD` a no-op.

### 3.2 Item lookup

- `server/server/game/input_db.cpp:1461-1462` reads the count and advances the pointer:

```
	DWORD dwCount = decode_4bytes(c_pData);
	c_pData += sizeof(DWORD);
```

  `decode_4bytes` is a bare pointer cast (`server/server/game/protocol.h:35-38`):

```
inline INT decode_4bytes(const void *a)
{
	return (*((INT *) a));
}
```

  No byte swap, no alignment guard, no bound.

- `server/server/game/input_db.cpp:1468` casts the remainder to an array of `TPlayerItem` with
  no length check:

```
	TPlayerItem * p = (TPlayerItem *) c_pData;
```

- `server/server/game/input_db.cpp:1470` iterates exactly `dwCount` records and advances a
  pointer per iteration, so the number of bytes read is `4 + 72 * dwCount` regardless of what
  the frame actually carried.
- `server/server/game/input_db.cpp:1472` is the lookup. It is keyed on **vnum and count**, and
  it is handed the stored id:

```
		LPITEM item = ITEM_MANAGER::instance().CreateItem(p->vnum, p->count, p->id);
```

  So a row is looked up by `(vnum, count)` and the row's own `id` is only a *request* for that
  id. Section 7.3 explains what happens when two rows share a vnum.
- `server/server/game/input_db.cpp:1474-1478` drops a failed creation with the diagnostic
  **commented out**:

```
		if (!item)
		{
			//sys_err("cannot create item by vnum %u (name %s id %u)", p->vnum, ch->GetName(), p->id);
			continue;
		}
```

  A row that cannot be created vanishes with no log line at all.
- The payload struct, `server/server/common/tables.h:432-450`:

```
typedef struct SPlayerItem
{
	DWORD	id;
	BYTE	window;
	WORD	pos;
	DWORD	count;

	DWORD	vnum;
	long	alSockets[ITEM_SOCKET_MAX_NUM];	// ...

	TPlayerItemAttribute    aAttr[ITEM_ATTRIBUTE_MAX_NUM];
	DWORD	owner;
#ifdef ENABLE_REFINE_ELEMENT
	DWORD	dwRefineElement;
#endif
#ifdef __CHANGELOOK_SYSTEM__
	DWORD	transmutation;
#endif
} TPlayerItem;
```

  `TPlayerItemAttribute` is `BYTE bType; short sValue;` = 3 bytes
  (`server/server/common/tables.h:426-430`). `ITEM_SOCKET_MAX_NUM` is 6 and
  `ITEM_ATTRIBUTE_MAX_NUM` is 7, both live:
  `server/server/common/item_length.h:13-14` gives 6 behind `ENABLE_EXTENDED_SOCKETS`, and
  `server/server/common/item_length.h:21-30` gives
  `ITEM_ATTRIBUTE_MAX_NUM = ITEM_ATTRIBUTE_RARE_END, // 7`. The legacy target is 32-bit
  (`server/server/premake5.lua:12`), so `long` is 4 and the packed record is
  `4 + 1 + 2 + 4 + 4 + 24 + 21 + 4 + 4 + 4 = 72` bytes.
- This 72-byte record is a **DB-peer** record. It is retired by ADR-0001 and has no place in
  `protocol/`.

### 3.3 State restored per item, and where it is restored

`server/server/game/input_db.cpp:1480-1496` restores state before any placement decision.
`SetSkipSave(true)` at 1480 is the hinge: it suppresses every `Save()` until line 1538 clears
it, which is what keeps a half-placed item out of the store.

```
		item->SetSkipSave(true);
		item->SetSockets(p->alSockets);
		item->SetAttributes(p->aAttr);
		item->SetLastOwnerPID(p->owner);
#ifdef ENABLE_REFINE_ELEMENT
		item->SetRefineElement(p->dwRefineElement);
#endif
#ifdef __CHANGELOOK_SYSTEM__
		item->SetTransmutation(p->transmutation);
#endif
#ifdef ENABLE_BELT_INVENTORY_EX
		if (p->window == BELT_INVENTORY)
		{
			p->window = INVENTORY;
			p->pos = p->pos + BELT_INVENTORY_SLOT_START;
		}
#endif
```

All three gates are on: `ENABLE_REFINE_ELEMENT` (`server/server/common/prodomodefines.h:28`),
`__CHANGELOOK_SYSTEM__` (`server/server/common/prodomodefines.h:16`) and
`ENABLE_BELT_INVENTORY_EX` (`server/server/common/prodomodefines.h:13`).

The belt remap at 1491-1495 rewrites `window` 9 to window 1 and adds 274 to the cell, so a belt
row is loaded as an ordinary `INVENTORY` cell in the belt range. **The window byte the client
sees for a belt item is therefore 1, not 9.** This is the one case where the stored window byte
and the wire window byte differ.

### 3.4 Placement, and the three collections the item can end in

- `server/server/game/input_db.cpp:1498-1503` is the collision check. A row whose cell is
  already occupied is pushed onto the deferred vector `v` and logged as `ITEM_RESTORE`.
- `server/server/game/input_db.cpp:1506-1532` is the placement switch, and its set of cases is
  `{INVENTORY, DRAGON_SOUL_INVENTORY, ATTR67_ADD, SWITCHBOT}` for the "put it where the row
  says" branch and `{EQUIPMENT}` for the wear branch. See section 7.2 for the missing
  `default:`.
- `server/server/game/input_db.cpp:1516` places at the stored cell.
- `server/server/game/input_db.cpp:1519-1531` is the equipment branch, and it has a
  **level gate** that silently unequips:

```
				case EQUIPMENT:
					if (item->CheckItemUseLevel(ch->GetLevel()) == true )
					{
						if (item->EquipTo(ch, p->pos) == false )
						{
							v.push_back(item);
						}
					}
					else
					{
						v.push_back(item);
					}
					break;
```

  A stored equip the character is too low a level for lands in the base inventory instead of
  its wear slot. Nothing is logged.
- `server/server/game/input_db.cpp:1535-1536` runs `OnAfterCreatedItem` and only logs a
  failure.
- `server/server/game/input_db.cpp:1538` re-enables saving.
- `server/server/game/input_db.cpp:1541-1561` drains the deferred vector **after** the main
  loop, so a deferred item's record is sent after every directly-placed item's record, not at
  the position it held in the row set.
- `server/server/game/input_db.cpp:1547` picks the deferred item's cell with
  `ch->GetEmptyInventory(item->GetSize())`, the first free base cell in ascending order.
- `server/server/game/input_db.cpp:1555-1557` is the overflow case: the item is dropped on the
  ground at the character's own coordinates, given 180 s of ownership, and scheduled for
  destruction. The `bool` return of `AddToGround` is discarded.
- `server/server/game/input_db.cpp:1563-1566` is the tail:

```
	ch->CheckMaximumPoints();
	ch->PointsPacket();

	ch->SetItemLoaded();
```

  `CheckMaximumPoints` (`server/server/game/char.cpp:3863-3870`) only acts when the stored HP or
  SP exceeds the recomputed maximum, and it acts by calling `PointChange`, which ends in
  `UpdatePointsPacket` (`server/server/game/char.cpp:4823`, definition at
  `server/server/game/char.cpp:2089`). So on a normal login it sends nothing, and on a login
  where the character's stored HP is above its new maximum it sends a point-change record. This
  is a **second** `GC_CHARACTER_POINTS`-family record after the one the loading burst already
  sent.

### 3.5 Records emitted per loaded item

**`GC_ITEM_SET`, server byte 21, 72 bytes, exactly one per placed item.** There is no other
inventory-bearing record on this path.

`server/server/game/item.cpp:522-526` is the placement call site:

```
#if defined(__BL_ENABLE_PICKUP_ITEM_EFFECT__)
	ch->SetItem(TItemPos(window_type, pos), this, bHighlight);
#else
	ch->SetItem(TItemPos(window_type, pos), this);
#endif
```

`server/server/game/char_item.cpp:567-593` is the send:

```
	if (GetDesc())
	{
		if (pItem)
		{
			TPacketGCItemSet pack;
			pack.header = HEADER_GC_ITEM_SET;
			pack.Cell = Cell;

			pack.count = pItem->GetCount();
#ifdef ENABLE_REFINE_ELEMENT
			pack.dwRefineElement = pItem->GetRefineElement();
#endif
#ifdef __CHANGELOOK_SYSTEM__
			pack.transmutation = pItem->GetTransmutation();
#endif
			pack.vnum = pItem->GetVnum();
			pack.flags = pItem->GetFlag();
			pack.anti_flags	= pItem->GetAntiFlag();
#if defined(__BL_ENABLE_PICKUP_ITEM_EFFECT__)
			pack.highlight = bHighlight;
#else
			pack.highlight = (Cell.window_type == DRAGON_SOUL_INVENTORY);
#endif

			thecore_memcpy(pack.alSockets, pItem->GetSockets(), sizeof(pack.alSockets));
			thecore_memcpy(pack.aAttr, pItem->GetAttributes(), sizeof(pack.aAttr));
			GetDesc()->Packet(&pack, sizeof(TPacketGCItemSet));
		}
```

`__BL_ENABLE_PICKUP_ITEM_EFFECT__` **is** defined
(`server/server/common/prodomodefines.h:23`), so the live `highlight` byte is the
`AddToCharacter` parameter and not the dragon-soul expression. `AddToCharacter` then rewrites
it (`server/server/game/item.cpp:475-476`):

```
	if (bHighlight)
		bHighlight = this->GetLastOwnerPID() != ch->GetPlayerID();
```

and the load path always passes the default (`server/server/game/item.h:123`):

```
	bool	AddToCharacter(LPCHARACTER ch, TItemPos Cell, bool bHighlight = true);
```

so for a loaded item the highlight byte is **1 exactly when the row's `owner` column differs
from the loading character's PID**.

The 62-byte `GC_ITEM_DEL` branch is the `else` at
`server/server/game/char_item.cpp:595-610` and requires `pItem == NULL`. On this path it is
never taken. Confirmed by the shape of the calls: every `SetItem` on the load chain passes
`this` (item.cpp:523/525) or a non-null `item` (`SetWear`), whereas every NULL-item call site
in the tree is a removal path — `server/server/game/item.cpp:391`, `:402`, `:420`, `:426` —
and none of them is on the load chain. Positive control: those four lines are found by the same
search that finds the load chain; negative control: a bogus call name matches zero.

The same argument retires `GC_ITEM_UPDATE` (byte 25, 59 bytes) and
`GC_ITEM_GROUND_ADD` (byte 26, 21 bytes) from the *inventory* path:

- `CItem::UpdatePacket` (`server/server/game/item.cpp:249-275`) is called only from
  `SetCount`/`SetAttribute`-style setters and from `CreateSocket`/`SetSocket`
  (`server/server/game/item.cpp:291`, `:344`, `:354`, `:1632`, `:1647`). `AddToCharacter`,
  `EquipTo` and `SetWear` do not call it. `EquipTo` calls
  `m_pOwner->UpdatePacket()` at `server/server/game/item.cpp:1495` and `:1499`, which is
  `CHARACTER::UpdatePacket`, a character record, not an item record.
- `CItem::EncodeInsertPacket` (`server/server/game/item.cpp:163-182`) sends
  `HEADER_GC_ITEM_GROUND_ADD` and is reached only from the view update at
  `server/server/game/entity_view.cpp:42` and `:63`, i.e. only for an item that actually lands
  in a sectree.

**`GC_ITEM_GROUND_ADD` is therefore a real, but conditional, second record on this path**: if
`AddToGround` succeeds for an overflow item, every player in view, including the one logging
in, receives a 21-byte ground-add for it. The 21 bytes are
`server/server/game/item.cpp:172-182`:

```
	struct packet_item_ground_add pack;

	pack.bHeader	= HEADER_GC_ITEM_GROUND_ADD;
	pack.x		= c_pos.x;
	pack.y		= c_pos.y;
	pack.z		= c_pos.z;
	pack.dwVnum		= GetVnum();
	pack.dwVID		= m_dwVID;
	//pack.count	= m_dwCount;

	d->Packet(&pack, sizeof(pack));
```

summed as `1 + 4 + 4 + 4 + 4 + 4 = 21`. The `count` field is commented out, so a ground
announcement carries no stack count.

---
## 4. The record on the wire: header, width, field order, acceptance

### 4.1 `TPacketGCItemSet` — server byte 21, 72 bytes

`server/server/game/packet.h:1427-1444`, verbatim:

```
typedef struct packet_item_set
{
	BYTE	header;
	TItemPos Cell;
	DWORD	vnum;
	WORD	count;
#ifdef ENABLE_REFINE_ELEMENT
	DWORD	dwRefineElement;
#endif
#ifdef __CHANGELOOK_SYSTEM__
	DWORD	transmutation;
#endif
	DWORD	flags;
	DWORD	anti_flags;
	bool	highlight;
	long	alSockets[ITEM_SOCKET_MAX_NUM];
	TPlayerItemAttribute aAttr[ITEM_ATTRIBUTE_MAX_NUM];
} TPacketGCItemSet;
```

`TItemPos` is packed (`server/server/common/length.h:956-960`):

```
#pragma pack(push, 1)
typedef struct SItemPos
{
	BYTE window_type;
	WORD cell;
```

so it is 3 bytes. `TPlayerItemAttribute` is `BYTE bType; short sValue;` = 3 bytes
(`server/server/common/tables.h:426-430`). The legacy target is 32-bit
(`server/server/premake5.lua:12`), so `bool` is 1 and `long` is 4. With the live gates
(`ENABLE_EXTENDED_SOCKETS`, `ENABLE_REFINE_ELEMENT`, `__CHANGELOOK_SYSTEM__`,
`__ATTR_6TH_7TH__`, `ENABLE_SWITCHBOT`, `ENABLE_CUSTOM_INVENTORY`, `__AURA_SYSTEM__`,
`ENABLE_BELT_INVENTORY_EX`, `__BL_ENABLE_PICKUP_ITEM_EFFECT__`, `ENABLE_SWITCHBOT`,
`__SASH_SYSTEM__`, `ENABLE_REWARD_SYSTEM`, `__NEW_SET_BONUS__`,
`ENABLE_MOUNT_COSTUME_SYSTEM`, `ENABLE_PET_COSTUME_SYSTEM`, `ENABLE_AFFECT_RENEWAL` — all
present in `server/server/common/prodomodefines.h` at lines 13, 15, 16, 23, 28, 29, 30, 31, 33,
76, 158, 162, 191, 200, 202), the field order and byte offsets are:

| offset | bytes | field | source type |
|---|---|---|---|
| 0 | 1 | `header` = 21 | `BYTE` |
| 1 | 1 | `Cell.window_type` | `BYTE` |
| 2 | 2 | `Cell.cell` | `WORD`, little-endian |
| 4 | 4 | `vnum` | `DWORD` |
| 8 | 2 | `count` | `WORD` |
| 10 | 4 | `dwRefineElement` | `DWORD`, behind `ENABLE_REFINE_ELEMENT` |
| 14 | 4 | `transmutation` | `DWORD`, behind `__CHANGELOOK_SYSTEM__` |
| 18 | 4 | `flags` | `DWORD` |
| 22 | 4 | `anti_flags` | `DWORD` |
| 26 | 1 | `highlight` | `bool`, one byte under `pack(1)` |
| 27 | 24 | `alSockets[6]` | six `long`, 4 each |
| 51 | 21 | `aAttr[7]` | seven of `BYTE` + `short` |

Total: `1 + 3 + 4 + 2 + 4 + 4 + 4 + 4 + 1 + 24 + 21 = 72`. Header byte is included.

The header values are two different names in the two directions, and the Rust side already
transcribes the split. `protocol/src/gc_item_window.rs:9-18` states it, and
`protocol/src/gc_inventory.rs:494-513` carries the same table as two rows, `client_name`
against `server_name`. The practical facts:

- **Wire byte 21** is the one this path uses, and the client accepts it. Legacy calls it
  `HEADER_GC_ITEM_SET`; the client calls the same byte `HEADER_GC_ITEM_SET2` and reads a
  72-byte `TPacketGCItemSet2`, which is why 72 is the right width.
- **Wire byte 20** is *not* usable. Legacy calls it `HEADER_GC_ITEM_DEL` and writes 62 bytes
  (`server/server/game/packet.h:1085-1099`, `1 + 3 + 4 + 1 + 4 + 4 + 24 + 21 = 62`), but the
  client registered that byte as `HEADER_GC_ITEM_SET` with a 72-byte `TPacketGCItemSet`, so its
  `CheckPacket` drops it. `docs/PROTOCOL_NOTES.md:1193-1211` records this. It is a documented
  finding, not an open question, and it is not on the load path in any case (section 3.5).

The Rewrite's codec agrees byte for byte, and its decode offsets are the table above:
`protocol/src/gc_item_window.rs:449-460` reads `cell` at 1, `vnum` at 4, `count` at 8,
`refine_element` at 10, `transmutation` at 14, `flags` at 18, `anti_flags` at 22, `highlight` at
26, sockets at 27, attributes at 51.

### 4.2 The equipment case uses the same record with a translated cell

An equip is not sent with window 2. `CItem::EquipTo` calls
`ch->SetWear(bWearCell, this)` (`server/server/game/item.cpp:1437`), and `SetWear` translates
the window (`server/server/game/char_item.cpp:658-667`):

```
void CHARACTER::SetWear(BYTE bCell, LPITEM item)
{
	if (bCell >= WEAR_MAX_NUM + DRAGON_SOUL_DECK_MAX_NUM * DS_SLOT_MAX)
	{
		sys_err("CHARACTER::SetItem: invalid item cell %d", bCell);
		return;
	}

#if defined(__BL_ENABLE_PICKUP_ITEM_EFFECT__)
	SetItem(TItemPos(INVENTORY, INVENTORY_MAX_NUM + bCell), item, false);
```

So the client receives, for every worn item, **window byte 1** and cell
`INVENTORY_MAX_NUM + wearCell`, i.e. 180 for `WEAR_BELT` through 243 for the last wear slot.
`INVENTORY_MAX_NUM` is 180 and `WEAR_MAX_NUM` is 64, both pinned in the Rewrite at
`common/src/item_slots.rs:199` and `common/src/item_slots.rs:296`; in legacy they are
`server/server/common/length.h:25` (`INVENTORY_PAGE_SIZE*INVENTORY_PAGE_COUNT` with
`INVENTORY_PAGE_COUNT = 4` behind `ENABLE_EXTEND_INVEN_SYSTEM`, so `5 * 9 * 4`) and
`server/server/common/length.h:89`. `EquipTo` also records the same cell on the item itself
(`server/server/game/item.cpp:1441`):

```
	m_wCell	= INVENTORY_MAX_NUM + bWearCell;
```

This is the second place where the stored window byte and the wire window byte differ, and it
is the common case: **every equipped item is reported in window 1.** A port that writes the
stored `window` column straight into the record's window byte will get equipment wrong in a way
that a vnum-only test will not catch.

`TItemPos::IsEquipPosition` (`server/server/common/length.h:1002-1005`) is the legacy statement
of the same rule:

```
	bool IsEquipPosition() const
	{
		return ((INVENTORY == window_type || EQUIPMENT == window_type) && cell >= INVENTORY_MAX_NUM && cell < INVENTORY_MAX_NUM + WEAR_MAX_NUM)
			|| IsDragonSoulEquipPosition();
	}
```

### 4.3 The two conditional extra records

| record | byte | width | when |
|---|---|---|---|
| `GC_ITEM_GROUND_ADD` | 26 | 21 | an overflow item is dropped on the map (`input_db.cpp:1555`) and the sectree view update reaches the loading client (`entity_view.cpp:42`, `:63`) |
| `GC_CHARACTER_POINT_CHANGE` | 17 | 25 | `CheckMaximumPoints` finds stored HP or SP above the recomputed maximum (`input_db.cpp:1563` -> `char.cpp:3865-3869` -> `char.cpp:4823`) |

`GC_CHARACTER_POINT_CHANGE` is 25 bytes, and its header field is **four bytes wide**, not one
(`server/server/game/packet.h:1064-1071`):

```
typedef struct packet_point_change
{
	int		header;
	DWORD	dwVID;
	BYTE	type;
	long long	amount;
	long long	value;
} TPacketGCPointChange;
```

`1 + 4 + 1 + 8 + 8` would be 22, and the record is **25** because the header is an `int`:
`4 + 4 + 1 + 8 + 8 = 25`. The send site writes a byte value into it
(`server/server/game/char.cpp:2105`):

```
		pack.header = HEADER_GC_CHARACTER_POINT_CHANGE;
```

so the first four wire bytes are `11 00 00 00`. The Rewrite already has this right
(`protocol/src/gc_nested.rs:411-422` keeps `header: i32` and
`protocol/src/gc_nested.rs:426-432` writes four bytes), and the comment at
`protocol/src/gc_nested.rs:439-441` says why there is no single-byte header check. This is a
legacy shape quirk that the Rewrite transcribed rather than "corrected"; nothing needs to
change, but a port that assumes a one-byte header will emit a 22-byte record and desynchronise
the client.

`GC_CHARACTER_POINTS`, sent unconditionally by `PointsPacket`
(`server/server/game/input_db.cpp:1564`), is the third record on this path's tail. It is byte
16 (`server/server/game/packet.h:116`) and its width comes from
`server/server/game/packet.h:1052-1056`:

```
typedef struct packet_points
{
	BYTE	header;
	long long		points[POINT_MAX_NUM];
} TPacketGCPoints;
```

with `POINT_MAX_NUM = 255` (`server/server/common/length.h:82`), so `1 + 8 * 255 = 2041`
bytes. `long long` is 8 on i686, so this record is 2041 bytes and not 1021. It is not
inventory-bearing; it is listed because it is part of the load tail and because its width is
easy to get wrong by a factor of two.

### 4.4 What the stock client accepts, on this path

The only inventory-bearing record on this path is byte 21 at 72 bytes, and the client accepts
it. Three independent witnesses:

1. `protocol/src/gc_item_window.rs:9-18` records the client-side registration: wire 21 is the
   client's `HEADER_GC_ITEM_SET2` with a 72-byte `TPacketGCItemSet2`.
2. `prodomo/tests/parity.rs:2668-2674` is a passing scripted-client scenario against the real
   binary, and it is written against the same record:

```
/// A 72-byte `GC_ITEM_SET` and its header byte, as the client sees them.
///
/// Byte 21 is the 72-byte record. Byte 20 is the byte legacy calls `GC_ITEM_DEL` and
/// the client calls `HEADER_GC_ITEM_SET`, which is why a destroy cannot clear a window
/// slot and does not try. `protocol::gc_item_window` has the full rename and the widths.
const ITEM_SET_LEN: usize = 72;
const ITEM_SET: u8 = 21;
```

3. `prodomo/tests/parity.rs:2810-2833` asserts the offsets of that record as the client saw
   it, including the window byte:

```
    let (records, state) = alpha.drain_game(Duration::from_millis(500), an_item_window_record);
    let record = the_item_set(&records)
        .unwrap_or_else(|| panic!("the client must receive a GC_ITEM_SET, got {records:02x?}"));
```

   with, at `:2829-2833`:

```
    assert_eq!(
        record[1],
        common::item_slots::EWindows::Inventory as u8,
        "a grant goes into the base inventory"
    );
```

So a bare 72-byte byte-21 record carrying its own window byte is accepted with nothing before
it. Section 5 follows from that.

---
## 5. Ordering, and the window question

### 5.1 There is no window-open record

**Claim:** the load path opens no window, because no such record exists in this build.

| search over `server/server/game/packet.h` | result | role |
|---|---|---|
| `WINDOW` | 0 | the claim |
| `GC_ITEM` | 10 | positive control: the search reaches the item records |
| `GC_ZZZ_NO_SUCH_NAME` | 0 | negative control |

`EWindows` (`server/server/common/length.h:657-676`) is an enumeration of **storage window
ids**, not of records:

```
enum EWindows
{
	RESERVED_WINDOW,
	INVENTORY,
	EQUIPMENT,
	SAFEBOX,
	MALL,
	DRAGON_SOUL_INVENTORY,
#if defined(__ATTR_6TH_7TH__)
	ATTR67_ADD,
#endif
#ifdef __AURA_SYSTEM__
	AURA_REFINE,
#endif
#ifdef ENABLE_SWITCHBOT
	SWITCHBOT,
#endif
	BELT_INVENTORY,
	GROUND
};
```

So the window byte is a **field of the item record**, not something a separate record announces.
The only window-bearing inventory records on this path are the byte-21 set (which carries
`Cell.window_type` and `Cell.cell` inline) and the byte-26 ground add (which carries no window
at all, because a ground item has no window).

The inventory and equipment windows are client-side UI over a flat cell space that the Rewrite
already models exactly. `common/src/item_slots.rs:25-33` gives the layout the load path writes
into:

```
//! | base inventory | `INVENTORY_MAX_NUM` | 0 | 180 |
//! | equipment | `WEAR_MAX_NUM` | 180 | 244 |
//! | belt inventory | `BELT_INVENTORY_SLOT_START..END` | 274 | 290 |
//! | custom inventory | `CUSTOM_INVENTORY_SLOT_START..END` | 290 | 1370 |
```

and `common/src/item_slots.rs:109` states the window byte these all share:

```
    /// `INVENTORY` 1. The base inventory, the equipment, the dragon soul and belt
    /// ranges, *and* the custom inventory all use this byte.
    Inventory = 1,
```

This is the second reason equipment and belt both travel as window 1: they are not separate
windows to the client, they are separate *ranges* of window 1.

### 5.2 The order the items arrive in is neither window-, slot- nor id-ordered, and is not deterministic

This is the most consequential finding for a port, so it is worth being exact about where the
order comes from. There are two code paths on the DB side, and they order differently and
non-deterministically.

**Cache hit.** The container is a hash set of pointers
(`server/server/db/ClientManager.h:47`):

```
	typedef std::unordered_set<CItemCache *, std::hash<CItemCache*> > TItemCacheSet;
```

and it is walked in iteration order (`server/server/db/ClientManagerPlayer.cpp:331-341`):

```
		DWORD dwCount = 0;
		TItemCacheSet::iterator it = pSet->begin();

		while (it != pSet->end())
		{
			CItemCache * c = *it++;
			TPlayerItem * p = c->Get();

			if (p->vnum)
				thecore_memcpy(&s_items[dwCount++], p, sizeof(TPlayerItem));
		}
```

`std::hash<CItemCache*>` is the identity hash of the pointer, and the bucket layout is chosen by
the allocator. **The order on a cache hit is the heap layout of the cache objects.** It is
stable within a process run and meaningless across runs and machines.

**Cache miss.** The query has no `ORDER BY`
(`server/server/db/ClientManagerPlayer.cpp:364-387`, repeated for the non-player-cache path at
`:501-523`):

```
		snprintf(szQuery, sizeof(szQuery),
				"SELECT id,"
				"window+0,"
				"pos,"
				...
				" FROM item%s WHERE owner_id=%d AND (window in ('INVENTORY','EQUIPMENT','DRAGON_SOUL_INVENTORY','ATTR67_ADD','SWITCHBOT','AURA_REFINE','BELT_INVENTORY'))",
				GetTablePostfix(), pTab->id);
```

The whole `SELECT` is quoted because the absence of an `ORDER BY` is the point: **the order is
whatever MySQL's plan produces**, which for this table is in practice primary-key order on
`id`, and is not guaranteed by anything.

The rows are then converted and sent in result order
(`server/server/db/ClientManagerPlayer.cpp:938-958`):

```
void CClientManager::RESULT_ITEM_LOAD(CPeer * peer, MYSQL_RES * pRes, DWORD dwHandle, DWORD dwPID)
{
	static std::vector<TPlayerItem> s_items;

	CreateItemTableFromRes(pRes, &s_items, dwPID);
	DWORD dwCount = s_items.size();

	peer->EncodeHeader(HEADER_DG_ITEM_LOAD, dwHandle, sizeof(DWORD) + sizeof(TPlayerItem) * dwCount);
	peer->EncodeDWORD(dwCount);
```

`CreateItemTableFromRes` (`server/server/db/ClientManagerPlayer.cpp:14-68`) appends in
`mysql_fetch_row` order, so the DB order is preserved end to end.

**What the client actually sees, in order:**

1. Every directly-placed item, in DB/cache order, one byte-21 set each.
2. Then every deferred item — the collisions (`input_db.cpp:1502`), the level-gated and
   failed equips (`input_db.cpp:1524`, `:1529`) and the switch-dropped rows (section 7.2) — in
   the order they were pushed onto `v`, which is itself DB/cache order, placed by
   `GetEmptyInventory` into the lowest free base cells (`input_db.cpp:1547`, `:1560`).
3. Then, if a deferred item found no free cell, a byte-26 ground add in the same order
   (`input_db.cpp:1555`).
4. Then `GC_CHARACTER_POINTS` (2041 bytes, byte 16), and only on a stale-HP/SP row a byte-17
   point change (`input_db.cpp:1563-1564`).

**No legacy ordering is observable to the client**, because the record carries an explicit
window and cell and the client places by cell, not by arrival. A port may therefore choose any
order. The Rewrite's choice is already the better one: `load_owner_items` sorts explicitly
(section 6.1), which makes a scenario deterministic and is worth recording as a **Divergence**
rather than as parity, because legacy has no defined order to be parity with.

### 5.3 Which cells may collide, and what happens then

Two items in the same cell do not both go to the client. The second one is caught at
`input_db.cpp:1498-1503` and deferred, so the client receives one set for the cell and a second
set for whatever free cell the deferred item is given. A multi-cell item does **not** reserve
its neighbours: the check at 1498 is a single-cell `GetInventoryItem(p->pos)`, and
`AddToCharacter` does no grid occupancy test at all, so a 2x2 item can be placed on top of
another 2x2 item's cells and the client will render the overlap. `GetEmptyInventory` *does*
respect size (`IsEmptyItemGrid`, `char_item.cpp:737`), so the deferred path is safe and the
direct path is not. Section 7.5 records that as a Defect.

### 5.4 The `owner` field the load compares against is not the stored owner

`CreateItemTableFromRes` never reads an `owner` column. The query does not select one, and the
converter assigns the requested PID (`server/server/db/ClientManagerPlayer.cpp:64`):

```
		item.owner		= dwPID;
```

The save path writes that same field back as `owner_id`
(`server/server/db/ClientManager.cpp:1545-1559`), so `owner` tracks **current** ownership, not
last ownership. Three consequences on the load path, all of them provable from the lines above:

- `input_db.cpp:1483` sets `m_dwLastOwnerPID` to the loading character's own PID.
- `item.cpp:476` compares it against `ch->GetPlayerID()`, which is the same value, so
  **the highlight byte is always 0** on a loaded item. The `__BL_ENABLE_PICKUP_ITEM_EFFECT__`
  feature is dead on this path.
- `item.cpp:480-481`, the `ENABLE_REWARD_SYSTEM` first-item mission reward, can therefore never
  fire from a load.

A port that reads a real last-owner column and sets the highlight byte would diverge from legacy
here. The Rewrite should send 0 and record why.

---
## 6. What the Rewrite has today

### 6.1 `db/src/items.rs`: the loader exists and is correct, and has no caller

`db/src/items.rs:568-584`, quoted in full:

```
pub async fn load_owner_items(store: &Store, owner_id: u32) -> Result<Vec<ItemRow>, ItemError> {
    // `owner_id = $1` is the only filter, and that is deliberate: the migration's
    // biconditional makes a row with an owner never a ground row, so adding
    // `AND window_type <> 10` would be a second copy of a rule that already holds.
    //
    // The order is window, then cell, then id, so the load is deterministic: a world
    // that hands the client its items in load order needs exactly this order.
    let sql = format!(
        "{} WHERE owner_id = $1 ORDER BY window_type, pos, id",
        select_sql()
    );
    let rows = sqlx::query(&sql)
        .bind(i64::from(owner_id))
        .fetch_all(store.pool())
        .await?;
    rows.iter().map(row_from).collect()
}
```

`ItemRow` is `db/src/items.rs:113-138` and carries every field the 72-byte set needs: `id`,
`owner_id`, `window_type`, `pos`, `vnum`, `count`, `refine_element`, `transmutation`, `flags`,
`anti_flags`, `sockets: [i32; SOCKETS]`, `attributes: [Attribute; ATTRS]`.

Three things are already better than legacy and should be kept:

1. **The order is explicit** (`ORDER BY window_type, pos, id`), which is the determinism
   legacy lacks (section 5.2).
2. **Every value is a bound parameter.** Legacy formats the query with `snprintf` and
   interpolates `GetTablePostfix()` (`ClientManagerPlayer.cpp:386-387`).
3. **No window filter.** The Rewrite loads every owned row and lets the caller place it, rather
   than dropping a row whose window it does not recognise. This is already a recorded
   divergence from legacy's silent drop and it is the right one.

**Absence claim: `load_owner_items` has no production caller.** Whole-workspace
`command grep -a` over `*.rs`:

| search | result | role |
|---|---|---|
| `load_owner_items` | 7: the definition at `db/src/items.rs:568` and 6 call sites in `db/tests/items.rs` | positive control: the search reaches test code and the definition |
| `load_owner_items_ZZZ` | 0 | negative control |
| `load_item` in `db/src` and `prodomo/src` | 3, all in `db/src/items.rs` | positive control that the search reaches the production crates at all |

So the loader is written and tested, and nothing in `prodomo/` calls it. **The inventory load is
not ported.** The `sys.item.core` unit (the Operator grant) is ported; this unit is not.

**One documentation defect to fix in the same change.** The doc comment at
`db/src/items.rs:551-558` says legacy loads "the same set (`db/ClientManagerPlayer.cpp:386`,
the six `INVENTORY` windows)". The citation is right; the count is wrong:

```
/// Legacy loads the same set (`db/ClientManagerPlayer.cpp:386`, the six `INVENTORY`
/// windows). It filters in the query (`row[0] = 0` and the switch) and **silently drops**
/// a row it does not recognise, so a character that owns an item in a window this build
/// does not have loses it at the next login. This function does not repeat that: the only
/// filter is the owner, and a row this build cannot decode is an [`ItemError::Corrupt`]
/// rather than a missing item.
```

Legacy's query names **seven** windows, not six: `INVENTORY`, `EQUIPMENT`,
`DRAGON_SOUL_INVENTORY`, `ATTR67_ADD`, `SWITCHBOT`, `AURA_REFINE`, `BELT_INVENTORY`
(`server/server/db/ClientManagerPlayer.cpp:386`). The sentence also cites "the switch", which
is the *game*-side switch at `input_db.cpp:1506` and not a DB-side filter, so the two halves of
the sentence are about different processes. Both halves should be corrected when the unit lands,
and the seven-window list belongs in `docs/PROTOCOL_NOTES.md` because it is the source of the
`AURA_REFINE` leak in section 7.2.

### 6.2 `prodomo/src/game_state.rs`: `enter_world` does not touch items

`prodomo/src/game_state.rs:584-610`, quoted in full:

```
    pub fn enter_world(
        &mut self,
        vid: common::vid::Vid,
        player_id: u32,
        name: &str,
        outbox: ClientOutbox,
    ) -> Result<(), EnterWorldRefused> {
        if vid.is_null() {
            return Err(EnterWorldRefused::NullVid);
        }
        // Checked before the create, because `CharacterManager::create_player` reports
        // a duplicate player id or name but allocates a **fresh** VID, so a duplicate
        // VID would otherwise slip past it and consume a counter value silently.
        if self.characters.find_by_vid(vid).is_ok() {
            return Err(EnterWorldRefused::DuplicateVid { vid });
        }
        // The manager is the authority on the indexes it owns. A clone of the outbox
        // is taken only after both sides agree the character is new, so a refusal
        // cannot leave a sender for a character that was never admitted.
        match self.characters.create_player_with_vid(player_id, name, vid) {
            Ok(()) => {
                let _previous = self.outboxes.insert(vid, outbox);
                Ok(())
            }
            Err(error) => Err(EnterWorldRefused::from_manager(error, name)),
        }
    }
```

It rejects a null or duplicate VID, creates the character, and installs the outbox. It does
**not** read the store, does not create items, and does not write a record. Its only production
caller is `prodomo/src/main.rs:1791-1794`:

```
    let entered = context
        .game
        .enter_world(world_vid, character.id, character.name.clone(), outbox)
        .await;
```

### 6.3 The correct hook already exists: `select_character`

`prodomo/src/main.rs:1467` is the Rewrite's mirror of the legacy trigger, and its own doc
comment already says so (`prodomo/src/main.rs:1465-1466`):

```
/// The load itself is a store read here, where legacy asked its DB process. The two are the
/// same step in the same order; the Rewrite has no server-to-server protocol (ADR-0001).
```

The delivery closure at `prodomo/src/main.rs:1533-1554` is where the item load belongs:

```
    let delivery = async {
        // `SetPhase(PHASE_LOADING)` writes `GC_PHASE` and then installs the loading phase's
        // input boundary, before any other loading record.
        session.set_phase(PostHandshakePhase::Loading).await?;
        send_all(session, &burst.before_map_test).await?;
        // The map test. `PlayerLoad` folds an instance map onto its base map first.
        let map_index = context.atlas.index_at(character.x, character.y);
        let allowed = map_index.is_some_and(|index| {
            map_is_allowed(
                public_map_index(index),
                context
                    .routes
                    .channel_maps
                    .get(&seat.number)
                    .map_or(&[][..], Vec::as_slice),
            )
        });
        if !allowed {
            return refuse_map(session, addr, context, seat, &character, account, map_index).await;
        }
        send_all(session, &burst.after_map_test).await
    };
```

Legacy's order inside the loading phase is `PlayerLoad` records, then `ItemLoad` records, and
`ItemLoad` is a DB answer that can only arrive after the player row. The Rewrite's equivalent
point is therefore **after `send_all(session, &burst.after_map_test)` and before the function
returns `true` at `main.rs:1563`** — that is, still inside the loading phase, still before
`CG_ENTER_GAME` reaches `enter_game` (`prodomo/src/main.rs:1648`). Placing the load after
`enter_world` instead would put it after `GC_PHASE` to the game phase and after
`CG_ENTER_GAME`, which is a different order from legacy and is observable, because the client's
inventory window is built during the loading phase.

### 6.4 The pieces the load needs are all in place

| need | already exists | where |
|---|---|---|
| the 72-byte byte-21 record | `GcItemSet` + `encode` | `protocol/src/gc_item_window.rs:368` and `:426` |
| build a record from a world item | `Item::gc_item_set(pos, highlight)` | `world/src/item.rs:258` |
| the cell space and window bytes | `EWindows`, `ItemPos`, the 1370-cell layout | `common/src/item_slots.rs:104` and `:25-33` |
| place an item at a cell | `CharacterItems::set(pos, item)` | `world/src/character/items.rs:799` |
| read a cell | `CharacterItems::get(pos)` | `world/src/character/items.rs:633` |
| is a cell taken | `CharacterItems::occupied()` | `world/src/character/items.rs:334` |
| lowest free base cell for a size | `find_free_inventory_cell(usable_cells, size)` | `world/src/character/items.rs:451` |
| the trailing points record | `GcPoints`, 2041 bytes | `protocol/src/gc_fields.rs:902` and `:154` |
| the conditional point-change record | `GcPointChange`, 25 bytes, 4-byte header | `protocol/src/gc_nested.rs:411` |

**One record is already on the select path that must now be sent twice.**
`prodomo/src/loading_phase.rs:511-521` sends `GcPoints` inside `after_map_test`:

```
    let after_map_test = vec![
        encoded(&mut GcGold::new(gold(character)), "the gold record"),
        encoded(&mut GcPoints::new(points(character)), "the points record"),
```

and `input_db.cpp:1564` sends it again at the end of the item load. Legacy therefore puts two
2041-byte points records on the wire between `CG_CHARACTER_SELECT` and `CG_ENTER_GAME`. A port
that appends the items and stops will send one, which is a smaller wire difference than it looks
but is a difference, and a scripted-client scenario that counts records will see it.

---
## 7. Defects and quirks on this path

Ordered by how much they can change what a port does. Every one is a Defect under the project's
own rule ("a bug an honest client can trigger is a Defect unless the owner names it a Quirk")
except where marked otherwise, and none of them may be reproduced.

### 7.1 The frame length is never checked against the record count — Defect

`CInputDB::ItemLoad` has no length parameter (section 3.1) and trusts `dwCount` completely.
The enclosing frame loop bounds the frame by its **declared** size, not by what
`ItemLoad` will read (`server/server/game/input_db.cpp:2257-2281`):

```
	for (m_iBufferLeft = bytes; m_iBufferLeft > 0;)
	{
		if (m_iBufferLeft < 9)
			return true;

		bHeader		= *((BYTE *) (c_pData));	// 1
		m_dwHandle	= *((DWORD *) (c_pData + 1));	// 4
		iSize		= *((DWORD *) (c_pData + 5));	// 4

		sys_log(1, "DBCLIENT: header %d handle %d size %d bytes %d", bHeader, m_dwHandle, iSize, bytes);

		if (m_iBufferLeft - 9 < iSize)
			return true;

		const char * pRealData = (c_pData + 9);

		if (Analyze(d, bHeader, pRealData) < 0)
```

`iSize` is a signed `int` read straight from the wire, and the test at 2268 is
`m_iBufferLeft - 9 < iSize`. Two consequences:

- A frame that declares 4 bytes and a count of 4096 makes `ItemLoad` read
  `4 + 72 * 4096` bytes past the frame.
- A **negative** `iSize` makes 2268 false, so the loop proceeds and then executes
  `c_pData += 9 + iSize;` (line 2279) and `m_iBufferLeft -= 9 + iSize;` (line 2280), both of
  which move *backwards*. The loop does not terminate on its own.

This is only reachable from the DB peer, which ADR-0001 retires, so it is a Divergence by
removal rather than work. It is recorded because the Rewrite must not reintroduce the shape: a
record count and a buffer length are two different facts, and the port needs the length.

### 7.2 The placement switch has no `default:` — Defect, and live today

`server/server/game/input_db.cpp:1506-1532` has cases for `INVENTORY`,
`DRAGON_SOUL_INVENTORY`, `ATTR67_ADD`, `SWITCHBOT` and `EQUIPMENT`, and **no `default:`**. The
DB query selects seven windows (`server/server/db/ClientManagerPlayer.cpp:386`), and
`__AURA_SYSTEM__` **is** defined (`server/server/common/prodomodefines.h:30`):

```
30:#define __AURA_SYSTEM__ // __GF_Aura__
```

so window 7 (`AURA_REFINE`) is in the result set and has no case. A row in that window falls
through the whole switch, is never placed and is never pushed onto `v`. The item object is
still alive: `CreateItem` registered it in the item manager's maps, `SetSkipSave(false)` at
line 1538 re-arms its save, and nothing ever owns it or gives it a cell. That is a leaked
`CItem` per `AURA_REFINE` row, per login, with the row still in the store.

`BELT_INVENTORY` (window 9) is also in the result set, but it is remapped to `INVENTORY` at
`input_db.cpp:1491-1495` under `ENABLE_BELT_INVENTORY_EX`, which is defined
(`server/server/common/prodomodefines.h:13`), so window 9 cannot reach the switch.

The Rewrite's answer is already the right one and is already written: `load_owner_items` does
not filter by window, so an `AURA_REFINE` row reaches the port, and the port must then refuse
it **loudly** rather than drop it. `world::character::items::set` returning `Rejected` is the
right shape. Record the refusal.

### 7.3 A duplicate item id makes the row vanish with no log line — Defect

`server/server/game/item_manager.cpp:178-185`, with the comment on 178 elided (see the note
below on the legacy comments):

```
	if (m_map_pkItemByID.find(id) != m_map_pkItemByID.end())
	{
		item = m_map_pkItemByID[id];
		LPCHARACTER owner = item->GetOwner();
		sys_err("ITEM_ID_DUP: %u %s owner %p", id, item->GetName(), get_pointer(owner));
		return NULL;
	}
```

Note on the legacy comments: the non-ASCII comments in the checked-in legacy source are **not
recoverable**. `server/server/game/item_manager.cpp:178` holds the bytes
`ef bf bd` (`U+FFFD REPLACEMENT CHARACTER`) where Korean text was, so the file is already
lossy on disk. This report therefore quotes only executable lines, and the one place where a
comment matters (`input_db.cpp:1476`) is ASCII. Do not transcode these files to read them.

`CreateItem` returns `NULL`, and `input_db.cpp:1474-1478` continues with its own diagnostic
commented out. So on a duplicate id the row is not loaded, not placed, and not reported by the
load path; the only trace is one `ITEM_ID_DUP` line from the item manager. The Rewrite's
`save_item` already refuses an occupied cell and refuses an id clash rather than replacing
(`db/src/items.rs:601-613`), so the port should surface the refusal as an error and name the id.

### 7.4 `AddToCharacter` range-checks the wrong variable — Defect, harmless in practice

`server/server/game/item.cpp:443-453`:

```
	WORD pos = Cell.cell;
	BYTE window_type = Cell.window_type;

	if (INVENTORY == window_type)
	{
		if (m_wCell >= INVENTORY_MAX_NUM && BELT_INVENTORY_SLOT_START > m_wCell)
		{
			sys_err("CItem::AddToCharacter: cell overflow: %s to %s cell %d", m_pProto->szName, ch->GetName(), m_wCell);
			return false;
		}
	}
```

The requested cell is in `pos`; the check reads `m_wCell`, the item's **current** cell. A freshly
created item has `m_wCell == 0` — the constructor initialises it at
`server/server/game/item.cpp:37` and `Initialize` resets it at `server/server/game/item.cpp:69` —
so `0 >= 180` is false and the check can never fire on the load path. The same is true of the
`DRAGON_SOUL_INVENTORY` test at `item.cpp:457` and the `SWITCHBOT` test at `item.cpp:466`.

The requested cell is range-checked afterwards by
`CHARACTER::SetItem` (`server/server/game/char_item.cpp:391-395`), so the wrong-variable check
is dead code rather than a hole. But it has a consequence worth naming: `SetItem` **returns
early** on a bad cell, and `AddToCharacter` does not notice. It goes on to set `m_pOwner = ch`
(`item.cpp:527`), call `Save()` (`:529`) and `return true` (`:530`), and `ItemLoad` discards that
return value at `input_db.cpp:1516`. The net result for a corrupt out-of-range cell is an item
with an owner, a saved row, no grid entry, and no record to the client — reproduced on every
login, and invisible to the load path. The Rewrite must check the *target* cell and treat a
refusal as a refusal.

### 7.5 A multi-cell item overwrites the grid marks of whatever is under it — Defect

The removal path in `SetItem` tests occupancy before clearing a grid mark
(`server/server/game/char_item.cpp:432-433`):

```
						if (m_pointsInstant.pItems[p] && m_pointsInstant.pItems[p] != pOld)
							continue;
```

and the placement path five lines later has no equivalent test
(`server/server/game/char_item.cpp:461`):

```
					m_pointsInstant.bItemGrid[p] = wCell + 1;
```

So a 2x2 item loaded onto cells that already hold another 2x2 item overwrites the occupant's
anchor marks with `wCell + 1` while leaving the occupant in `pItems[p]`. The occupant is now an
item with no grid mark: invisible to every size-aware lookup, and still owned. The load path
makes this reachable because its collision check is single-cell
(`input_db.cpp:1498-1499`) and `AddToCharacter` does no grid test at all.

A related narrow case: for a belt cell, `SetItem` takes the `else` at
`server/server/game/char_item.cpp:464-465` and marks only the anchor, so a multi-cell item in
the belt range gets no neighbour marks. The belt helper in `IsEmptyItemGrid`
(`server/server/game/char_item.cpp:764-778`) returns `false` for a multi-cell belt cell, so the
deferred path cannot produce it, but the direct path at `input_db.cpp:1516` with a stored belt
cell can.

### 7.6 The `pos` field narrows to a `BYTE` for equipment — Defect

`TPlayerItem::pos` is a `WORD` (`server/server/common/tables.h:436`), and
`CItem::EquipTo` takes a `BYTE` (`server/server/game/item.h:134`):

```
	bool		EquipTo(LPCHARACTER ch, BYTE bWearCell);
```

`input_db.cpp:1522` passes the `WORD` straight through:

```
						if (item->EquipTo(ch, p->pos) == false )
```

A stored `EQUIPMENT` row with `pos > 255` is truncated, so `EquipTo` range-checks a *different*
cell than the row names and places the item there. The Rewrite already stores `pos` as `u32`
(`db/src/items.rs:121`) and `ItemPos::cell` as a `u16`, so it can refuse this; it must.

`EquipTo`'s own range checks are correct and are not a finding. For the record, the
dragon-soul arm (`server/server/game/item.cpp:1411-1418`) rejects below `WEAR_MAX_NUM` or at or
above `WEAR_MAX_NUM + DRAGON_SOUL_DECK_MAX_NUM * DS_SLOT_MAX`, and the ordinary arm
(`:1419-1426`) rejects at or above `WEAR_MAX_NUM`, which is exactly the valid range.

### 7.7 A sash's grade is initialised on load and the write is lost — Defect

`__SASH_SYSTEM__` is defined (`server/server/common/prodomodefines.h:15`), so
`server/server/game/item.cpp:488-515` runs on every load:

```
	if ((GetType() == ITEM_COSTUME) && (GetSubType() == COSTUME_SASH) && (GetSocket(SASH_ABSORPTION_SOCKET) == 0))
	{
```

and it ends at `item.cpp:515`:

```
		SetSocket(SASH_ABSORPTION_SOCKET, lVal);
```

`CItem::SetSocket` (`server/server/game/item.cpp:1643-1648`) is:

```
void CItem::SetSocket(int i, long v, bool bLog)
{
	assert(i < ITEM_SOCKET_MAX_NUM);
	m_alSockets[i] = v;
	UpdatePacket();
	Save();
```

Both calls are dropped on the load path, for two independent reasons:

- `UpdatePacket` (`server/server/game/item.cpp:249-252`) returns at once, because
  `m_pOwner` is still null — it is not assigned until `item.cpp:527`, which is after line 515:

```
void CItem::UpdatePacket()
{
	if (!m_pOwner || !m_pOwner->GetDesc())
		return;
```

- `Save` (`server/server/game/item.cpp:1609-1615`) returns at once, because `m_bSkipSave` is
  still true — `ItemLoad` set it at `input_db.cpp:1480` and clears it at `input_db.cpp:1538`,
  after `AddToCharacter` returns:

```
void CItem::Save()
{
	if (m_bSkipSave)
		return;
	
	ITEM_MANAGER::instance().DelayedSave(this);
}
```

So the grade is written into the in-memory socket, is never sent to the client, and is never
persisted by the load. The client shows the stored (zero) socket until some later change
rewrites the row. The Rewrite should treat the load's socket array as read-only and, if it
wants the sash grade initialised, do it as an explicit post-load write rather than relying on a
setter that silently no-ops.

### 7.8 An equip the character is too low a level for is silently unequipped — Defect, and the port must reproduce it

`input_db.cpp:1520-1530` (quoted in section 3.4) routes an equip that fails
`CheckItemUseLevel` onto the deferred vector, where it lands in the base inventory at
`GetEmptyInventory`. Nothing is logged and no record says why. A level-gated reward weapon
therefore comes back in the bag rather than on the character, at the same cell every time, and
the client's equipment view silently loses a slot. This is behaviour a client can observe, so
under the project rule it is a Defect in legacy. What matters for the port is that the
behaviour is **deterministic and must be reproduced**: gate on the character's level, place in
the base inventory at the lowest free cell.

### 7.9 The overflow item is dropped on the map and its row is never rewritten — Defect

`input_db.cpp:1549-1558`:

```
		if (pos < 0)
		{
			PIXEL_POSITION coord;
			coord.x = ch->GetX();
			coord.y = ch->GetY();

			item->AddToGround(ch->GetMapIndex(), coord);
			item->SetOwnership(ch, 180);
			item->StartDestroyEvent();
		}
```

- `AddToGround`'s `bool` is **discarded**. `CItem::AddToGround`
  (`server/server/game/item.cpp:578-582`) only reaches its `Save()` on the success path:

```
	SetWindow(GROUND);
	SetXYZ(pos.x, pos.y, pos.z);
	tree->InsertEntity(this);
	UpdateSectree();
	Save();
```

  A failure (no map, or a map index of 0, rejected at `item.cpp:551-555`) leaves an item with no
  owner, no cell and no sectree — and `StartDestroyEvent`
  (`server/server/game/item.cpp:152-161`) then schedules its destruction anyway.
- On **success**, the item is in the world with `window = GROUND` (10) and the ownership event
  is armed, but the **stored row is never updated**: the save that would rewrite `window` to 10
  and `pos` to a ground handle does not happen, because the item was loaded with `SetSkipSave`
  already cleared but its last `Save()` was the one inside `AddToGround`, which writes the
  ground state, and the ground state is not what the row said. The practical outcome is that
  the row still names the old cell, so at the next login the same item loads into the old cell
  while a destroyed instance sits on the map.

`StartDestroyEvent()` with no argument uses the default expiry
(`server/server/game/item.cpp:152`), so this is a 180-second window, not a permanent drop.

The Rewrite has no ground system yet (`sys.item.ground` is a later unit, per
`db/src/items.rs:560-562`), so this is a scope decision rather than immediate work. The decision
to record: when the base inventory is full, legacy drops the item, and the port must decide
whether to reproduce the drop or refuse the row. Refusing is safer and is the recommended
Divergence.

### 7.10 The load is silent about every row it loses — Defect

Three separate `continue`/`break`/fall-through paths in `ItemLoad` discard a row with no
diagnostic: a failed `CreateItem` (`input_db.cpp:1474-1477`, commented out), the missing switch
`default:` (`:1506-1532`, section 7.2), and a discarded `AddToGround` result (`:1555`). Only
the collision path logs, and only as `ITEM_RESTORE` (`:1501`). An operator has no way to learn
that a character lost items at login. The Rewrite's `load_owner_items` already returns
`ItemError::Corrupt` rather than a short vector (`db/src/items.rs:556-558`), which is the right
instinct; the port should keep it and log every refusal with the item id and the cell.

### 7.11 Legacy shape quirks the port must not "fix"

These are not bugs in the sense above. They are shapes that look like bugs and are not, and a
port that "corrects" them breaks the wire.

- **`GC_CHARACTER_POINT_CHANGE` has a 4-byte header** (`server/server/game/packet.h:1064-1071`),
  so the record is 25 bytes, not 22. The Rewrite already transcribes it
  (`protocol/src/gc_nested.rs:411-422`).
- **`GC_ITEM_SET` (byte 21) and `GC_ITEM_DEL` (byte 20) are named backwards between the two
  directions** (`protocol/src/gc_item_window.rs:9-18`). Byte 21 is the load's record.
- **The load sends two points records**, one in the loading burst and one at
  `input_db.cpp:1564`.
- **Equipment travels as window 1**, not window 2 (section 4.2), and so does the belt range and
  the custom-inventory range.
- **`GC_ITEM_GROUND_ADD` carries no count**; the field is commented out at
  `server/server/game/item.cpp:180`.
- **`SetItem` treats a pointer value of `0xff` or `0xffffffff` as fatal**
  (`server/server/game/char_item.cpp:373-378`) and calls `core_dump()`. It is a pointer-value
  check that can never be true for a real `CItem`, so it is dead, and the Rewrite must not
  reproduce a check on a sentinel pointer value.
- **`CHARACTER::SetItem` asserts against a double-owned item** and, in a release build where
  `assert` is compiled out, proceeds (`server/server/game/char_item.cpp:380-384`):

```
	if (pItem && pItem->GetOwner())
	{
		assert(!"GetOwner exist");
		return;
	}
```

  A double-owned item is a real state; refusing it without a debug build is right, and the
  Rewrite should do the same.

### 7.12 A field the load reads is set from a value that is never bounded — Defect with a bounded consequence

`CHARACTER::Set_Inventory_Point` (`server/server/game/char.h:1286`) is a bare assignment, and
the player load sets it from the stored row (`server/server/game/char.cpp:2345-2347`):

```
#ifdef ENABLE_EXTEND_INVEN_SYSTEM
	Set_Inventory_Point(t->envanter);
#endif
```

`Inventory_Size()` (`server/server/game/char.h:1285`) is then
`INVENTORY_OPEN_PAGE_SIZE + (INVENTORY_WIDTH*Inven_Point())`, and the `POINT_INVEN` growth path
*does* bound it (`server/server/game/char.cpp:4356-4363`) at `INVENTORY_LOCK_COVER_COUNT`. A row
whose `envanter` exceeds that bound makes `GetEmptyInventory` loop further than the intended 180
cells. It is **not** an out-of-bounds read, because `IsEmptyItemGrid`'s custom-inventory arm
returns `false` for any cell outside the base range or the custom range
(`server/server/game/char_item.cpp:780-785`). It is a range-check gap with a bounded
consequence. The Rewrite stores this as a bounded column and should keep it bounded.

---
## 8. Recommended port shape

Not code — this section is the specification the implementing change should be written against.

**Where the load runs.** Inside the loading phase, in `select_character`
(`prodomo/src/main.rs:1467`), after `send_all(session, &burst.after_map_test)` at
`prodomo/src/main.rs:1553` and before the `true` at `prodomo/src/main.rs:1563`. Not in
`enter_world` (section 6.2) and not in `enter_game` (`prodomo/src/main.rs:1648`). Legacy's
descriptor is in `PHASE_LOADING` for the whole of it (`desc.cpp:515-524`).

**What it does, in legacy order.**

1. `load_owner_items(&context.store, character.id)`. One store read; it already returns
   `ORDER BY window_type, pos, id`, so the send order is deterministic and needs no sort in the
   port.
2. For each row, in order:
   1. Build a `world::Item` from the row. **`owner_id` is not a last-owner field** (section
      5.4), so the highlight byte is **0** for every item.
   2. Translate the window the way legacy does, in this order:
      - stored window 9 (`BELT_INVENTORY`) -> window 1, cell `+ 274`;
      - stored window 2 (`EQUIPMENT`) -> window 1, cell `180 + pos`, **and only if the
        character's level passes the use-level gate**, otherwise it becomes a base-cell item;
      - stored windows 1, 5, 6, 8 pass through unchanged.
   3. If the target cell is occupied, do not place it there: put it in the lowest free base cell
      for its size (`CharacterItems::find_free_inventory_cell`), which is legacy's deferred
      path. `CharacterItems::get(pos)` answers the occupancy test.
   4. `CharacterItems::set(pos, &item)`. **Do not discard the `Result`.** A refusal is the
      signal legacy loses (sections 7.2, 7.4, 7.6): log the id and the cell, and keep going.
      Refusing loudly is the recorded Divergence.
   5. `item.gc_item_set(pos, 0).encode()` and queue it on the outbox.
3. After every row, and after the deferred placements, send a second `GcPoints`
   (`protocol/src/gc_fields.rs:902`, 2041 bytes) to mirror `input_db.cpp:1564`.
4. Send `GcPointChange` (25 bytes) **only** when the stored HP or SP exceeds the recomputed
   maximum, mirroring `input_db.cpp:1563`. This path is DB-gated, so it needs no new store read.

**What it must not do.**

- No `GcItemDelDeprecated` (byte 20, 62 bytes). The stock client drops it
  (`docs/PROTOCOL_NOTES.md:1193-1211`) and legacy does not send it here anyway.
- No `GcItemUpdate` (byte 25). Nothing on this path changes an item after its set.
- No window-open record. There is none (section 5.1).
- No `GHEADER_DG_ITEM_LOAD` codec, and no `TPlayerItem` in `protocol/`. The 72-byte DB-peer
  record is retired by ADR-0001; the Rewrite reads the store directly.
- No last-owner highlight, and no mission reward from the load (section 5.4, 7.7).

**Divergences to record** (each needs a `docs/REWRITE_LEDGER.md` section, an
`AGENTS.md`-style note in `docs/STATUS.md`, and a line in `.scratch/parity/`):

| divergence | legacy | Rewrite | why |
|---|---|---|---|
| send order | hash-set order or un-`ORDER`ed SQL result | `ORDER BY window_type, pos, id` | legacy has no defined order; determinism is worth more than an undefined match |
| unrecognised window | silently dropped (`input_db.cpp:1506`) | refused with a log line, row kept | legacy loses the item and says nothing |
| duplicate id | row vanishes, no log (`input_db.cpp:1474`) | error naming the id | legacy loses the item silently |
| out-of-range cell | item owned, saved, invisible, every login | refused, row kept | legacy produces an invisible owned item |
| full base inventory | dropped on the map, row never rewritten | refuse the row (no ground system yet) | legacy's drop corrupts the row; the ground system is a later unit |
| two points records | yes | yes | **not** a divergence; legacy sends two and the port must too |

**Tests the change needs.** The gate is a scripted-client scenario in
`prodomo/tests/parity.rs`, per the project's own rule ("a system is ported when its Parity
inventory item has a passing scripted-client scenario"). The minimum:

- a character with an item in the base inventory, one worn, one in the belt range and one in
  the custom range, granted through the existing Operator console, then relogged;
- assert the four records arrive after the loading burst's points record and before
  `CG_ENTER_GAME`, each 72 bytes with header 21;
- assert the **window byte is 1 for all four**, which is the assertion that catches the
  equipment and belt translations (sections 4.2, 5.1);
- assert the cell of each, using the offsets in section 4.1 (cell at bytes 1..4, vnum at 4..8,
  count at 8..10, highlight byte 26 == 0);
- assert exactly **two** points records on the wire between select and enter-game;
- a negative control: a row in a window this build does not have is refused and logged, and the
  client is sent no record for it. Without this, a scenario that sent nothing for that row
  would satisfy every assertion above.

**A pure-Rust unit is not enough here.** The existing `db/tests/items.rs` coverage proves
`load_owner_items` returns rows; it says nothing about what the client does with them. The
interesting failures in this path are all ordering-and-window failures, which are only visible
on the wire.

---

## 9. Controls for every absence claim

Each row is one claim, the search that measured it, a positive control that proves the search
would have found it, and a negative control that proves the search is not matching everything.

| # | claim | search and result | positive control | negative control |
|---|---|---|---|---|
| 1 | no `CG_ITEM_LOAD` in the legacy build | `command grep -ac 'ITEM_LOAD' server/server/game/packet.h` -> 0 | `'CG_ITEM'` -> 11 | `'HEADER_CG_ZZZ_NO_SUCH_NAME'` -> 0 |
| 2 | no item-load entry in the inbound CG table | `'ITEM_LOAD'` over `sed -n '117,161p' server/server/game/packet_info.cpp` -> 0 | `'CG_ITEM'` -> 8 and `'HEADER_CG'` -> 44 in the same 45 lines | `'ZZZ_NO_SUCH'` -> 0 |
| 3 | no client-sent load in the Rewrite's CG inventory | `command grep -ac 'CG_ITEM_LOAD' protocol/src/cg_inventory.rs` -> 0 | `'CG_ITEM'` -> 28 and `'ITEM'` -> 34 in the same file | `protocol/tests/cg_wiring.rs:75` (`every_item_family_codec_is_reachable_through_the_public_api`) fails on a row whose `implemented` flag is set with no codec, so a load row could not be added unnoticed |
| 4 | the load path sends no `GC_ITEM_DEL` | every `SetItem` on the chain passes `this` or a non-null `item`; the `else` at `char_item.cpp:595-610` needs `pItem == NULL` | the four NULL-item call sites `item.cpp:391`, `:402`, `:420`, `:426` are found by the same search | `'ZZZ_NO_SUCH_CALL'` -> 0 |
| 5 | the load path sends no `GC_ITEM_UPDATE` | `CItem::UpdatePacket`'s five callers are `item.cpp:291`, `:344`, `:354`, `:1632`, `:1647`, none on the chain | the definition is found at `item.cpp:249` | `EquipTo`'s `m_pOwner->UpdatePacket()` at `:1495`/`:1499` resolves to `CHARACTER::UpdatePacket`, checked by name |
| 6 | no window-open record exists | `command grep -c 'WINDOW' server/server/game/packet.h` -> 0 | `'GC_ITEM'` -> 10 | `'GC_ZZZ_NO_SUCH_NAME'` -> 0 |
| 7 | `EWindows` is storage ids, not records | `server/server/common/length.h:657-676` read in full; it is an `enum` with no record fields | the same enum is what `TItemPos::window_type` is typed as (`length.h:959`) | `IsValidItemPosition` (`length.h:973-1000`) returns `false` for 5 of the 11 enumerators (`RESERVED_WINDOW`, `SAFEBOX`, `MALL`, `AURA_REFINE`, `GROUND`), which is only meaningful for storage ids and not for records |
| 8 | `load_owner_items` has no production caller | whole-workspace `command grep -a 'load_owner_items' --include=*.rs` -> 7 hits, all in `db/src/items.rs` (the definition) or `db/tests/items.rs` | those 7 hits, and `'load_item'` -> 3 in `db/src/items.rs`, prove the search reaches the production crates | `'load_owner_items_ZZZ'` -> 0 |
| 9 | `enter_world` does not touch items | `prodomo/src/game_state.rs:584-610` read in full; it makes no store call and no record | its two `Outbox` and `CharacterManager` calls are both present in the quoted body | `main.rs:1791-1794` is its only production caller and passes no store handle |
| 10 | the SQL has no `ORDER BY` | the whole `SELECT` at `ClientManagerPlayer.cpp:364-387` and `:501-523` is quoted | `GetTablePostfix()` and `pTab->id` are both visible in the quote, so the search reached the format arguments | the neighbouring `SELECT` at `:496` is quoted too, and neither has an `ORDER BY` |
| 11 | the cache-hit order is unspecified | `ClientManager.h:47` names `std::unordered_set<CItemCache *, std::hash<CItemCache*>>` and `:331-341` walks it with `begin()`/`++` | the `while` loop and the `thecore_memcpy` are quoted, so the walk is the one being read | the type spelling `std::unordered_set` is the claim; `std::vector` appears in the same function at `:328` and is a different type |
| 12 | no codec is needed for the DB-peer record | `server/server/common/tables.h:432-450` defines it and ADR-0001 retires the protocol | `HEADER_DG_ITEM_LOAD = 42` at `tables.h:196` proves the name is a DG record | `'CG_ITEM_LOAD'` -> 0, control `'CG_ITEM'` -> 11 |

---

## 10. What this changes outside the code

- **`docs/PROTOCOL_NOTES.md` needs the seven-window filter** and the `AURA_REFINE` gap. The
  seven names are at `server/server/db/ClientManagerPlayer.cpp:386`; the missing case is at
  `server/server/game/input_db.cpp:1506-1532` against `server/server/common/prodomodefines.h:30`.
  This belongs in the notes because it is the reason a character can silently lose rows, and
  because `db/src/items.rs:553-555` currently describes it as "six".
- **`docs/PROTOCOL_NOTES.md` should also record the two window translations** (belt 9 -> 1 at
  `input_db.cpp:1491-1495`, equipment 2 -> 1 at `char_item.cpp:667`), because they are the two
  places where the stored window byte and the wire window byte differ, and both are silent.
- **`docs/STATUS.md`** should list the item load as a porting unit with its scenario name once
  it lands, and the Parity inventory row should move to `ported` in the same change.
- **A note for whoever reads the legacy tree next:** the non-ASCII comments in the checked-in
  C++ are `U+FFFD` on disk (`server/server/game/item_manager.cpp:178` is one example, read as
  bytes `ef bf bd`). A Korean comment quoted from this checkout is a guess, not evidence. Every
  quote in this report is therefore an executable line.

---

## 11. One-paragraph answer

There is no `CG_ITEM_LOAD`. The client asks for a character with `CG_CHARACTER_SELECT`
(byte 6), the server asks its DB process, and the DB process answers with the item rows, which
`CInputDB::ItemLoad` (`server/server/game/input_db.cpp:1451-1567`) turns into exactly one
72-byte byte-21 `GC_ITEM_SET` per item, between the loading burst and `CG_ENTERGAME` (byte 10).
The record is `header, TItemPos, vnum, count, refine_element, transmutation, flags, anti_flags,
highlight, 6 sockets, 7 attributes`; the window byte is **1 for equipment and for the belt
range**, because legacy translates both, and the order is the DB's, which is neither
`ORDER`ed nor deterministic on a cache hit. The Rewrite already has the loader
(`db/src/items.rs:568`, deterministic, no window filter), the record
(`protocol/src/gc_item_window.rs:368`), the world item
(`world/src/item.rs:258`) and the placement APIs (`world/src/character/items.rs:451`, `:799`),
and `prodomo/src/game_state.rs:584` does not use any of them, so nothing in `prodomo/` calls
`load_owner_items` and the inventory load is not ported. The hook is `select_character`
(`prodomo/src/main.rs:1467`), after `after_map_test`. The load's own defects that a port must
*not* reproduce are the missing `default:` on the placement switch, which leaks an `AURA_REFINE`
row per login, the wrong-variable range check in `AddToCharacter`, the silent loss of a
duplicate-id row, the un-clobbered grid marks of a multi-cell neighbour, and the `WORD`-to-`BYTE`
narrowing of an equip cell.
