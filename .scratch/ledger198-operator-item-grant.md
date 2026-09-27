# Ledger 198: the Operator item grant, and the first `GC_ITEM_SET`

Status: ready-for-agent

## Why this unit

`sys.item.core` is at `codec`. Ledger 196 gave items a table, 197 gave them a query path, and
nothing yet creates one or shows one to a client. The Parity inventory says a system is ported when
its row has a passing scripted-client scenario, so the next step is the smallest end-to-end path
that creates an item through the store and sends it to a client. That is the Operator grant.

Legacy has **no adminpage item-give path in this tree** (survey section 0: the only "adminpage"
hits are `game/config.cpp` keys, `game/input.cpp` accessors, and nothing else). The only grant that
exists is the in-game chat command `ACMD(do_item)`, `game/cmd_gm.cpp:467-519`, registered at
`game/cmd.cpp:282`. So this is a port of `do_item` plus a new Operator interface around it, and the
two must not be confused when the ledger records what was decided.

## The `do_item` path, in order

1. `arg1` is the vnum, resolved numerically or by a **case-insensitive prefix** locale-name match
   (`item_manager.cpp:722-738`), first row wins.
2. `iCount` defaults to 1; the `MINMAX(1, iCount, g_bItemCountLimit)` clamp runs **only when a
   second argument is given** (`cmd_gm.cpp:480-484`). `g_bItemCountLimit` is 5000.
3. `ITEM_MANAGER::CreateItem(dwVnum, iCount, 0, true)` — `id = 0` means new, **`bTryMagic = true`**.
4. `ch->GetEmptyInventoryEx(item)` picks a cell, then `AddToCharacter` -> `SetItem` -> `GC_ITEM_SET`.
5. On no cell: `M2_DESTROY_ITEM`, `ChatPacket("Not enough inventory space.")`, no log.

## Decisions this unit makes

Each one is a real choice, not a lookup, so each gets a reason here.

| # | Question | Decision | Why |
|---|---|---|---|
| D1 | Which cell search? | `GetEmptyInventoryEx`: the six custom banks first, then base cells 0..179 | This is the pickup-shaped path a client scenario will observe. The other overload (`GetEmptyInventory(BYTE size)`) is what shop, quests, gifts and the battle pass use. They differ. |
| D2 | Page locks? | Honour the unlocked size, as `GetEmptyInventory(BYTE size)` does | `GetEmptyInventoryEx` scans all 180 cells and can drop an item into a page the player has not unlocked, which the client cannot show. That is a legacy Defect (an honest client can trigger it) and is not reproduced. Divergence, recorded. |
| D3 | Custom-category membership? | Port it (`gamedata::item_custom_category`, already written) | The lists are hard-coded in `item.cpp:3004-3032`, so not porting them means every grant falls through to the 0..179 scan, which is a Divergence. Measured against the owner's 7,305 rows. |
| D4 | Stacking? | **No stacking.** Always a new item in a new cell, as `do_item` does | The Operator is a deliberate "give me exactly this", not a drop. `PickupItem` and `AutoGiveItem` do stack, so this is a difference from those paths and not from the one being ported. |
| D5 | `bTryMagic`? | **No.** Refine, sockets and attributes are zero | It needs the proto pct columns and the RNG. An Operator command that silently hands out a random enchanted item is a surprise, and the percentages are not yet ported. Recorded as a deliberate difference. |
| D6 | Persistence timing? | **Write through in one transaction** | Legacy is ~5.1 s (game) plus up to 5 min (DB cache). ADR-0003 says "a few seconds"; five minutes is not covered, and an Operator action whose result can be lost is wrong. Divergence, recorded. |
| D7 | Target? | An **online** character, by Name, through the game thread | `do_item` grants only to the caller. The Operator is a new interface, so this is a new decision, not a port. An offline target would need a world the character is not in. |
| D8 | Money log? | **None.** | `do_item` writes no `MONEY_LOG_DROP`; only `AutoGiveItem` does. Matching the path being ported. |
| D9 | "Inventory full"? | `"Not enough inventory space."` on stdout, and a real log line | `do_item`'s exact string. Its silence in the server log is a Defect, so the Rewrite logs. The Rewrite has no `[LS;445]` locale table either way. |
| D10 | Item log row? | **No dependency.** The grant succeeds or fails on the item write alone | Legacy's `INSERT DELAYED` is gated on a log level and is not guaranteed. |
| D11 | Id allocation? | One Postgres sequence, seeded above `MAX(id)`, per startup band | Legacy's banded range came from the retired DB peer. The first free id differs, but ids are not on the wire, so this is a note, not a Divergence — after confirming no id is exposed. |
| D12 | `window` column? | `smallint` keyed on `EWindows` | Legacy writes the C++ enum value into a MySQL `ENUM` whose ordinals disagree for `AURA_REFINE` and `SWITCHBOT`. Never reproduced. |
| D13 | `highlight` byte? | `1` | A fresh grant: `m_dwLastOwnerPID` is still 0 when `AddToCharacter` evaluates it, so the comparison is true. |
| D14 | Non-stackable count? | Force 1, as `CreateItem` does for a non-stackable vnum | `item_manager.cpp:261-262`. |

## Out of scope, named so it is not silently dropped

- `REWARD_MISSION_FIRST_ITEM`, which `AddToCharacter` fires for vnum 319 only, reads an absolute
  path outside the repository, and can re-enter itself. Not ported. If vnum 319 is ever granted,
  that needs an explicit decision.
- `LIMIT_REAL_TIME` and `LIMIT_TIMER_BASED_ON_WEAR`, which `CreateItem` starts as live timers. The
  item-limit *rules* are not ported yet.
- The `prefix` name lookup as an Operator argument. Accepted: exact vnum only. A prefix match is
  ambiguous for an Operator and the locale table is not loaded in that process.
- `bRefine`. There is no such field; refine state is `dwRefineElement` only, and a grant is 0.

## Gates

The unit is done when: `prodomo item give <name> <vnum> [count]` creates a persisted item in an
online character's first legal cell, the client receives `GC_ITEM_SET`, and every gate in AGENTS.md
is green.
