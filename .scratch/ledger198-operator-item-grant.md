# Ledger 198: the Operator item grant, and the first `GC_ITEM_SET`

Status: needs-triage

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

## Progress

This issue is still `ready-for-agent`, because none of the gates above is met. Three units have
landed against it and the remaining work is smaller each time.

- **Ledger 199** (committed `396fdc95`): the placement half. `world::character::grant` ports the
  `do_item` placement rule, and a live `Character` owns its four item windows.
- **Ledger 200** (this tree): the whole grant as one transport-free reducer,
  `prodomo::item_grant::grant_item`. It resolves the name, reads the prototype, allocates an id,
  places the item, and returns the `ItemRow` and the `GcItemSet` together. Not called by anything.

Three of the decisions above are now answered by the code rather than by argument:

- **Targeting online characters only, by exact name.** Confirmed necessary, not just chosen: the
  name index is keyed on the lower-cased name, so a case-sensitive lookup would be free to pick a
  different character than the operator typed. The grant goes through
  `CharacterManager::find_player_mut`, and a test grants to `"SHAMAN"` after creating `"Shaman"`.
- **No `dwStackMax` in this fork.** The clamp decision above assumed a per-prototype maximum. The
  search for it returns nothing while the same search for `bSize` returns two files, so the tree is
  being searched and the constant is absent. The only ceiling is the global `ITEM_MAX_COUNT`, and
  the reducer clamps to that.
- **The log line is a warning, not a dependency.** The reducer logs a warning when the allocator
  repeats an id, but the refusal itself is a returned `Err`, never a log line a caller has to read.

**Ledger 204-207 landed after this paragraph was written**, and the list below is updated
in place. For the record, what each of them closed:

- Ledger 204: the store half, including the refusal repair.
- Ledger 205: a live client's character in the real world, entered from `enter_game` and
  left before the final save, with the world writing to the client through a clone of the
  descriptor's outbox.
- Ledger 206: the order, as one function. `grant_and_deliver` is world, row, client, and
  it names all three outcomes.
- Ledger 207: the Operator interface, decided from the ADRs rather than deferred. The open
  question at the end of this file is answered in ledger 207.1: the console runs inside
  `serve`, because the world is only that process's memory (ADR-0002) and a second process
  would need the protocol class ADR-0001 retired. Legacy has no console at all, so this is
  new, not a port.

Still to do, in the order the gates imply:

1. ~~A game-thread command that carries a `GrantRequest` from an async task to the thread that
   owns the world.~~ **Done at ledger 202.** `GameCommand::GrantItem` carries the request and a
   `oneshot` answer, the loop delegates it to `GameState::apply`, and
   `GameLoopController::request_grant` is the one call a caller makes. `GameCommand` is no longer
   `Copy` or `Clone`, which is the cost of a per-request reply and is recorded at 202.1.
   **Still to do in this step:** install the store's item-id range through the same channel,
   because that is the one input `serve` cannot have when the loop is spawned (ledger 201
   explains the ordering). The state is built with a range today, so the startup path still
   passes `|_| {}` and nothing reaches it.
2. **Write the row before the record reaches the client.** The order is deliberate and is
   the opposite of the intuitive one: persist `GrantOutcome::row` first, in its own
   transaction, and only then send `GrantOutcome::record`. A client that sees an item it
   does not have on a relog is a divergence; a stored item nobody has seen yet is not.
   Legacy sends `GC_ITEM_SET` from `SetItem` and the save is a background flush, so this
   is a recorded improvement rather than parity. The same unit also does the write to the
   descriptor, and the answer names the real reason rather than always blaming the
   inventory.
3. ~~The `GC_ITEM_SET` write to the descriptor, and the refusal replies.~~ **Done at ledger
   206 and 207.**
4. ~~The Operator interface itself.~~ **Done at ledger 207**, and it is new rather than a
   port. The survey settled that legacy has no adminpage item-give path; ledger 207.1
   establishes with controls that legacy has no console either, and the ADRs pick the
   in-process reader.
5. ~~**The only thing left.** The create-and-destroy round trip as a scripted-client
   scenario.~~ **Done at ledger 208, in four scenarios** (see below).

## Ledger 208: the round trip, and what it proved and did not

Four scenarios in `prodomo/tests/parity.rs`, all against a real `prodomo serve` and a real
FIFO. The status is now `partial`, not `ported` — see the two gaps at the end.

| scenario | what it proves |
|---|---|
| `an_operator_gives_an_online_character_an_item_and_the_client_is_told` | `item give` over the pipe reaches the client's own socket as a 72-byte `GC_ITEM_SET`, with the vnum at bytes 4..=8 and the `ItemPos` at 1..=3, and that vnum is in the owner's `item_proto.txt` |
| `a_granted_item_is_still_in_the_store_after_the_client_disconnects` | the row outlives the descriptor — a disconnect is not a rollback |
| `an_operator_destroy_takes_the_item_out_of_the_world_and_deletes_its_row` | the world releases the cell, the row is deleted, and a *refused* second destroy does not touch the row or another character |
| `a_destroy_sends_nothing_to_the_client_and_says_so` | the console's answer tells the Operator the cell stays drawn, instead of implying a clean window |

**Two gaps, both recorded rather than closed:**

1. **A relog does not put the item back in the client's window.** The inventory load is not
   handled. The scenario proves the *row* survives; it does not prove the item is drawn
   again, and it should not be read that way. Legacy does not send items on enter-game
   either — `CInputDB::PlayerLoad` (`input_db.cpp:328`) sends no items — and the send it
   does make runs inside the loading phase that `CG_CHARACTER_SELECT` starts
   (`CInputDB::ItemLoad`, `input_db.cpp:1451`). Ledger 209.1 corrected an earlier draft of
   this item, which named a `CG_ITEM_LOAD` that **does not exist on the client wire**: the
   name is the retired DB-peer `HEADER_DG_ITEM_LOAD`. The hook is `select_character`
   (`prodomo/src/main.rs:1467`), not `enter_world`. `db::items::load_owner_items` has no
   caller, exactly as `grant_and_deliver` had none at 206. **This is the next step-4 item**,
   and it is the half of the round trip this unit did not do.
2. **A destroy sends no client record, and cannot.** The only record legacy has that clears
   a window cell is `GC_ITEM_DEL`, and the two trees disagree about it: legacy writes 62
   bytes into wire byte 20 while the client reads byte 20 as `HEADER_GC_ITEM_SET` and sizes
   it at 72, so the stock client drops it. The Rewrite does not reproduce a Defect and does
   not invent a record. Ledger 208.2 lays out the two options for the owner; the choice is
   the play test's, not a code guess.

**What the round trip found that no test could have.** The first `drain_game` decrypted
per 8-byte unit and looked for a 72-byte record, which can never match; it was dropping the
unit that began with the header byte and reporting a *missing* record for one that had in
fact arrived. The failure message was right and the diagnosis was wrong. The harness now
decrypts the window and walks records, each declaring its own width. Separately, a
`read_to_string` on `item_proto.txt` failed outright — the proto name column is Korean in a
legacy code page — so the existence check now goes through `ItemProtos::load`, the same
reader the server uses, with a negative control in the same table.

**Open question for the owner, recorded rather than assumed. ANSWERED at ledger 207.1.**
The text as it stood, kept because the answer is easier to check against it:**

**Open question for the owner, recorded rather than assumed.** Legacy's
`ACMD(do_item)` is reachable from the game-server console (`cmd.cpp:282`) and from a GM
chat line, and the survey found no adminpage path for it. The `ChatEffect::Command`
interpreter is still a stub, so the legacy route is unreachable too. Step 4 therefore
picks an interface, and the choice is the owner's: a new `prodomo item grant` subcommand
is the smallest thing that works, but the Operator CLI is a separate process and there is
no channel from it to a running `prodomo serve`. Either the grant runs *inside* the serve
process (an in-process console reader, closest to legacy's console) or a control channel
is built. This is called out rather than decided because both are real work and the
legacy source does not point at one.

## Ledger 209: the load trigger, and a console answer that never arrived

Two findings, both recorded in `docs/REWRITE_LEDGER.md` section 209.

**The reload trigger was named wrong, twice in this document.** There is no `CG_ITEM_LOAD` on
the client wire. `ITEM_LOAD` against `server/server/game/packet.h` is **0** where `CG_ITEM` is
**11** and a bogus name is **0**, and the name belongs to the retired DB peer
(`HEADER_DG_ITEM_LOAD = 42`, `tables.h:196`). The real trigger is `CG_CHARACTER_SELECT` (byte
6), and `CInputDB::ItemLoad` runs inside the loading phase `PlayerLoad` sets — so the
inventory is the server's send, with no client request to answer. The hook is
`select_character` (`prodomo/src/main.rs:1467`). This is now stated correctly above.

**The console's answer never reaches its pipe, and the log says it did.** `run` documents
that an answer goes to the log *and back to the pipe*. Measured on a real server with a
`0600` pipe and one command: **1** answer line in the log, **0** echo-failure warnings, and
**0** bytes at a second reader holding the pipe open. The write at
`prodomo/src/operator_console.rs:286` goes through `reader.get_mut()`, and `reader` comes from
`tokio::fs::File::open` (`:331`), which is `O_RDONLY` — a write that the kernel rejects with
`EBADF`. Tokio's `File::poll_write` returns the count it **copied into its own buffer** before
the blocking `write(2)` runs, so `write_all` is satisfied by the copy and the error is never
observed by this code path.

So the console is log-only in practice. Two honest fixes exist and **the owner picks one**:
delete the echo and document log-only, or open the pipe read-write and skip exactly the bytes
the console wrote, so it does not read its own answer back as the next command. Ledger 209.2
has both, and this unit did not pick. A test pinning the measurement is in
`prodomo/src/operator_console.rs`. The first draft hung (a read-only `open` of a FIFO waits
for a writer); ledger 209.4 rewrote it, and it runs and passes.

**The gates in this document's own table are ledger 208's.** They apply to commit `b41f1223`.
The working tree carries two modified files after it, so the numbers must not be cited for
the tree.
