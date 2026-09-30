# Parity inventory

Status: ready-for-agent

Everything the legacy server does that a client can observe, one row per item, each with a
porting status. Build step 2 (`docs/STATUS.md`). The Rewrite reaches Parity when every row is
`ported` or `unused` and the owner's play test agrees.

## Tables

| file | rows | kept by | what one row is |
|---|---|---|---|
| `client-packets.md` | 111 | generated | A client header handled in one phase, plus the two framing rules (`cg.any.*`) |
| `sub-headers.md` | 113 | generated | One sub-command a game-phase handler dispatches on |
| `server-records.md` | 134 | generated | A record the client decodes (from the client's registration table) |
| `commands.md` | 267 | generated | A chat command in the live `cmd_info` table |
| `quest-api.md` | 699 | generated | A Lua function the quest runtime registers |
| `timed-events.md` | 99 | generated | A timed event function |
| `quests.md` | 62 | generated | A quest script (51) or a quest library (11) |
| `systems.md` | 115 | by hand | A system: the behaviour behind the entry points above |
| `gamedata.md` | 45 | by hand | A Game data file or table and its reader |
| **total** | **1,645** | | |

The generated rows come from the **live** legacy source only: `tools/active.py` runs the C
preprocessor over each file with the legacy build's defines (FreeBSD, clang, i386, `NDEBUG`,
`prodomodefines.h`, and every non-guard define in the headers), so an arm behind an undefined
switch or `#if 0` is not listed. Its controls: `#if 0` and `__WIN32__` blocks are dropped, and
`__SASH_SYSTEM__`, `__FreeBSD__`, and `ENABLE_QUEST_DIE_EVENT` blocks are kept.

## Regenerating

```bash
python3 .scratch/parity/tools/gen.py
```

The generator keeps the `status`, `scenario`, and `note` cells of every ID it finds again, and
names every ID that disappeared from the source. Only those three columns may be edited in a
generated table. Run it with `PYTHONDONTWRITEBYTECODE=1`, or delete `tools/__pycache__` after.

## Statuses

| status | meaning |
|---|---|
| `missing` | Nothing in the Rewrite. |
| `codec` | The wire codec exists and is golden-byte tested; nothing sends or handles it. |
| `partial` | Some of the behaviour exists (a reducer, a table rule); no scenario proves it. |
| `ported` | A scenario in `prodomo/tests/parity.rs` passes. The `scenario` cell names its test function. |
| `unused` | Legacy never reaches it. The note gives the evidence, with its controls; the owner confirms before it is dropped. |

A `ported` row must name a scenario, and only a `ported` row may. The test
`inventory_rows_keep_the_rules` enforces that, that IDs are unique across all tables, and that
each named scenario exists; it runs in every `cargo test`.

## Acceptance

A row is `ported` when its scenario starts the real `prodomo` binary (`parity::Server`), plays
the client's bytes over TCP (`parity::Client`), and checks the answers against golden bytes taken
from the legacy source's field order. A scenario proves the row it is named in; a system row is
ported when its scenario covers the system's behaviour, not only one of its entry points. The
scenarios need a store and run only when `DATABASE_URL` is set, like every database test.

## Findings for the owner

- **The adminpage is unreachable in legacy.** `HEADER_CG_TEXT` (64) is registered
  (`G/packet_info.cpp:98`), but no phase analyzer handles it; `IsAdminPage` and
  `IsEmptyAdminPage` (`G/input.cpp:24-37`) have no callers; the password is only set and logged
  in `G/config.cpp`. Row `sys.adminpage`. If the owner agrees, it is `unused`, and the decision
  that it stays off until configured is moot.
- **`BlueDragon.lua` and `monkey_dungeon.lua` are never loaded,** so `G/BlueDragon_Binder.cpp`
  always runs on its fallbacks. `MapProperty.txt`, `charset.txt`, and `pet_skill_names.txt`
  have no reader either. Rows `data.lua.unread`, `data.map.property`, `data.locale.charset`,
  `data.locale.names`.
- **`questlib_extra.lua` and `questing.lua` are never loaded, and only `qc` reads
  `quest_functions`** (ledger 227). Rows `questlib.questlib_extra`, `questlib.questing`,
  `questlib.quest_functions`. `questlib_extra.lua` holds the only setter of
  `item_drop_limit_time`, so ledger 217's one-second drop limit waits for the owner's value.
- **`HEADER_CG_STATE_CHECKER` (0xce) is served in the handshake phase**, before auth: it is the
  Channel status list on the login screen. Row `sys.net.channel_status`.
- **`ENVANTER_BLACK` (0xe2)** is a client header without the `HEADER_CG_` prefix
  (inventory expansion). The first extraction missed it; the generator now matches every name in
  `protocol/src/cg_inventory.rs`.
