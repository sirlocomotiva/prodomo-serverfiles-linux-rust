# Rewrite status

This page records where the Rewrite stands, what is left, and what is next. Rules live in
`AGENTS.md`, terms in `CONTEXT.md`, decisions in `docs/adr/`, and the change history in
`docs/REWRITE_LEDGER.md`. This page is not a changelog: a ledger section updates the numbers, the
next steps, and the Divergence and Defect registers here, and writes its story in the ledger. What
this page said section by section until ledger 678 is in `docs/STATUS_HISTORY.md`.

Last reviewed: 2026-11-15, the direction review (ledger 679); the numbers are ledger 685's.

## Where the Rewrite stands

Steps 1 to 3 are done. Step 4 (game systems) is in its tail. Step 5 (the owner's play test with
the Reference client) has not started, and by the owner's decision of 2026-11-15 it starts when
the tail is closed.

| measure | value at ledger 693 |
|---|---|
| Parity inventory | 2,331 rows: 1,547 `ported`, 256 `partial`, 491 `unused`, 37 `missing`; **293 items left** (the `partial` and `missing` rows, each note ending in `Left:`) |
| Scenarios | 530 tests in `prodomo/tests/parity.rs`, each against the real binary and a scratch store |
| Tests | 5,670 pass in 46 targets; the scenarios run only with `DATABASE_URL` (ledger 693 has the receipt) |
| Client records | client to game: 91 of 92 registered headers; game to client: 125 of 134 (`protocol/src/*_inventory.rs`) |
| Store | PostgreSQL 18, 45 migrations |
| Size | about 483,000 lines of Rust, tests included |
| Idle cost | release build, Channel 1 and the Shared Channel, 65,604 NPCs and monsters, no player: 0.05 ms of each 40 ms Pulse, 595 MB |

A client can do what the `ported` rows say: log in, choose and create characters, enter the
world, move, chat, fight monsters and players, die and restart, use and trade items, shop, store,
join parties and guilds, fight guild wars, run quests, and use the GM commands. What it cannot do
yet is what the 293 rows left still lack (next steps, package B).

## Direction review, 2026-11-15

The owner asked for a review of the whole effort. Fifteen read-only audits ran, one per concern;
each finding below was checked against the code or measured before it was written here. The
ledger section has the evidence.

**The direction is sound.** The decisions hold in the code:

- One game thread steps every Channel; no store or file I/O runs on it; its command queue is
  bounded; a panic on it ends the process with an error (ADR-0002).
- A Transfer is committed before the client is told (item moves, the guild fee and the private
  shop buy were traced); the safebox belongs to the account by a CHECK; passwords are argon2id by
  a CHECK (ADR-0003, ADR-0005).
- No quest source was edited: `legacy/` has one commit. The quest crate reaches the game through
  one bridged function, with no `unsafe`, `io`, `debug`, `load` or `loadfile` (ADR-0004, ADR-0006).
- Every gate is green, and sampled scenarios assert exact bytes and values.

**The execution has drifted** in the ways below. None needs a decision reversed; each has a next
step.

| drift | evidence | next step |
|---|---|---|
| Nothing bounds the tail. | `UseItemEx` has 296 `case` labels; ledger 640-678 port them one per section, and no list says how many are left. The 65 `partial` notes are ledger logs, and some are stale (`timer_based_on_wear_expire_event`, `sys.item.core`, `sys.mob.proto`). | A1 |
| Verification got shallower as the pace rose (about 300 sections in 7 days). | Mutation sweeps have been deferred since 2026-10-15, on the owner's word, for about 450 sections. Of the 180 gate receipts from section 500 on, one names its `DATABASE_URL` run. More than 100 sections record a scenario that failed in the full suite and "passes alone"; the review's own first full run lost 1 of 518 that way. | A2, C7 |
| Every scenario ran in the test-server mode, where a caller passes GM gates. | `test_server` defaulted to on, and gates disagreed on what it granted. | Decided: ADR-0007. C4 |
| A crash could lose minutes, not seconds. | The save event defaulted to legacy's compiled 120-second cycle, and the example said 180; ADR-0003 says a few seconds. | Decided and done: the default is 10 (ledger 679). |
| The game thread can be stalled. | No Lua instruction limit; unbounded client outboxes; no deadline before login; no Pulse duration recorded. Only the idle cost has ever been measured. | C1, C2, C3 |
| The steering documents went stale. | This page was 269 KB, with 690 lines of per-section narrative above its summary; the agents' goal and progress files stopped at ledger 229k; `AGENTS.md` miscounted ADRs and records; 76 owner questions sat in a folder named for one feature (`.scratch/equip/`, now `.scratch/migration/`); 321 KB of finished research notes and two finished issues still read as open work. | Done in ledgers 679 and 682; rule A3 |
| `unused` rows were closed without the owner. | 445 rows; the spec asks for the owner's confirmation. | Decided: accepted in bulk, provisionally (2026-11-15). |
| The store drifted from ADR-0003 in two columns. | `guild_comment.content` and `guild_grade.name` are `text`. | C5 |
| Rules live in the binary crate. | `prodomo/src/game_state/` has over 100 rule modules (`guild.rs` 13,232 lines); `main.rs` is 17,779 lines and holds every record handler; `parity.rs` is 63,051 lines. `AGENTS.md` put gameplay rules in `world`. | F |
| Test runs leak. | 15 `prodomo_test_*` databases, a scenario server three days old and 32,472 scenario directories in `/tmp` were found on the test machine. | Done in ledger 683 (C6); `AGENTS.md`, "TEST HYGIENE". |
| No width has an independent witness. | `i686-linux-gnu-g++-12` was not installed; every recent receipt says "No width was measured". | C8; the compiler is installed since ledger 680. |
| The play-test kit does not exist. | The README promised a `compose.yaml`; the README and the example disagreed on the store's port and password. | D; `compose.yaml` and the agreement landed in ledger 680. |

## Next steps

In this order. A package is done when its exit line holds, not when its sections are written.

### A. Make the tail countable

1. **One list for what is left.** Done in ledger 686. Every `partial` and `missing` note ends
   with `Left:` and the items still missing, each checkable against the legacy source. The large
   switches are generated tables (`.scratch/parity/tools/gen.py`): `item-use.md` (the `case`
   labels of `UseItemEx`, `G/char_item.cpp:2718-7164`, and its limit checks), `points.md` (the
   arms of `PointChange` and `ApplyPoint`) and `skill-arms.md` (the per-skill switches of
   `G/char_skill.cpp`). *Exit:* 612 items left at ledger 686; the number only goes down.
2. **A ledger section is a slice of a package, not one arm.** Port a family of arms together (the
   fireworks, the detectors, the scrolls of one kind), with one scenario per behaviour and one
   gate run for the section. The receipt names the run with `DATABASE_URL` and the run without
   it, and lists by name every test that failed on the first run. A scenario that fails in the
   full suite is fixed or gets an issue under `.scratch/`; "passes alone" does not close it.
   Read its failure against the legacy rule before calling it load: every scenario lost in
   ledgers 679 to 685 had its cause in the scenario (a draw, a wall-clock second, a window, a
   log line its server had already printed, a monster that wandered, a read cut short by five
   quiet seconds) or in an order the Rewrite does not fix (C11), and none in the load; seven
   of the 54 scenarios the receipts before 679 called load flakes had such a cause too (48
   runs of each under a full run's load found the last), and five full runs in a row then
   lost none. `AGENTS.md`, "SCENARIO TIMING", has the rules a new scenario follows; ledger
   685.7 has each loss.
3. **This page is not a changelog.** A section changes the table above, this list, and the
   registers below. It does not add a "Since N" paragraph anywhere.
4. **A new scenario runs in the shipped mode** (`test_server = false`), with a GM grant where it
   plays a game master (ADR-0007).

### B. Close the rows left

By what a player meets first. Each row is `ported` when its `Left:` list is empty and a scenario
proves the last item; a row that waits on the owner's data says so and moves to the owner's list.
A row of the generated arm tables (B9) closes with the family it belongs to (A2).

| package | rows |
|---|---|
| B1. Items (10) | `cg.game.item_use`, `cg.game.item_use_to_item`, `cg.game.item_drop`, `cg.game.item_drop2`, `cg.game.item_pickup`, `sys.item.core`, `sys.item.refine`, `data.item.special_group`, `table.item_attr`, `event.item.timer_based_on_wear_expire_event` |
| B2. Monsters, combat, drops (10) | `sys.combat.core`, `sys.mob.ai`, `sys.mob.proto`, `data.proto.mob`, `data.drop.mob`, `data.drop.common`, `data.drop.etc`, `data.drop.group`, `data.map.regen`, `sys.world.regen` |
| B3. Skills (6) | `sys.skill.core`, `sys.skill.extra`, `cg.game.add_fly_targeting`, `cg.game.fly_targeting`, `table.skill_proto`, `data.motion` |
| B4. Character and world (10) | `sys.char.points`, `sys.char.save`, `sys.char.pk`, `sys.char.multi_language`, `sys.world.map`, `sys.world.move`, `sys.world.warp`, `sys.world.objects`, `cg.game.on_click`, `sys.npc.safebox` |
| B5. Guild and the cross-Channel paths (4) | `sys.guild.core`, `sys.guild.war`, `sys.guild.dungeon`, `sys.bus` |
| B6. Quests, dungeons, events (11) | `sys.quest.runtime`, `questlib.questlib`, `sys.dungeon.core`, `sys.dungeon.ship_defense`, `sys.dungeon.dragon_lair`, `data.regen.dungeon`, `sys.event.manager`, `sys.event.arena`, `sys.event.xmas`, `sys.event.castle_monarch`, `sys.event.threeway_war` |
| B7. Life skills and custom systems (4) | `sys.life.fishing`, `sys.life.mining`, `sys.custom.dragon_soul`, `sys.custom.new_set_bonus` |
| B8. Login, logs, data (7) | `sys.login.by_key`, `sys.login.enter`, `sys.log`, `data.proto.item`, `data.locale.strings`, `data.locale.names`, `data.map.index` |
| B9. The arm tables (231) | `item-use.md`: 112 `partial`, 35 `missing`; `points.md`: 47 `partial`, 2 `missing`; `skill-arms.md`: 35 `partial`. Most `partial` arms have Rust code and lack only a scenario; the `missing` ones (`ITEM_UNIQUE`, `ITEM_HAIR`, `ITEM_DS`, `ITEM_FISH`, the no-op types, the seasonal `USE_SPECIAL` vnums, 70008, 70009, 70102, the marriage ring, the ramadan candy, `COSTUME_ATTR_BONUS`) have none |

*Exit:* no `partial` or `missing` row is left; each is `ported`, `unused`, or on the owner's list.

### C. Harden what the review found (alongside B; all of it before D)

1. **The Pulse budget.** Record each Pulse's duration in the game loop, log an overrun, and add a
   scripted-client load run (clients that walk and fight on Channel 1). *Exit:* the 99th
   percentile Pulse is under 20 ms with the load the owner expects; if not, the per-Pulse passes
   move onto one due-time queue (ADR-0002).
2. **The quest host's limit.** An instruction-count hook ends a runaway script as a script error
   (ADR-0006). *Exit:* a scenario with `while true do end` in a quest keeps the Pulse counting.
3. **Slow and silent clients.** A cap on a client's outbox closes it; a connection that has not
   finished the handshake and login within a deadline is closed; the frame cap before login is
   the largest record that phase accepts. *Exit:* a scenario for each.
4. **One meaning for the test-server mode** (ADR-0007): one function answers a character's GM
   level; the scenarios move to the shipped mode package by package; the harness default and
   `GameState`'s test default are removed with the last one.
5. **The two `text` columns** become `bytea` in a new migration (ADR-0003).
6. **The harness leaves nothing behind.** Done in ledger 683. A killed test process runs no
   `Drop`; the next test process now drops its databases (an advisory lock per process says
   whose owner is gone) and removes its directories and servers (`parity::scratch`), and
   `tools/gates.sh` fails when a run leaves a database, a server, a temporary file or a core.
7. **Mutation testing resumes by risk, not by section.** The 450 deferred per-section sweeps are
   not run again. One sweep each covers the code where a wrong line loses or duplicates value:
   the Transfers (trade, shop, private shop, safebox and mall, refine), the item move and stack
   rules, death and its penalties, and the damage formulas. Every survivor gets a named test.
   The owner confirmed on 2026-11-15 that this machine is the one to use, so nothing blocks it.
   The driver is `tools/mutate.py` (the one the sweeps up to ledger 229k used, rescued in ledger
   683); it runs on a copy of the tree, never in the working tree.
8. **Widths.** `i686-linux-gnu-g++-12` is installed (ledger 680: the packed two-`long` control
   reads 8, and `TPacketGCPhase`, `TPacketGCHandshake` and `TPacketGCLoginFailure` read 2, 13
   and 10, as their codecs do). Probe every game-to-client record the Rewrite sends, and
   reconcile `gc_inventory.rs` with the codecs (`REFINE_INFORMATION` and `SYNC_POSITION` are
   flagged missing but have codecs). *Exit:* a table of record, source struct, probed width and
   codec width with no difference. The probe goes in `.scratch/probes/`, beside the earlier
   ones.
9. **A quest step's send order.** A step's world moves run after its script, kind by kind
   (`apply_world_moves`, `prodomo/src/game_state/quests.rs`), so a record a legacy call writes
   as it is called can leave behind the step's page. Found in ledger 680: `pc.change_empire`'s
   re-show (`RestartAtSamePos`, `G/char_change_empire.cpp:119-123`) reaches the client after
   the gold change, the scroll's removal and the last page, where legacy writes it first.
   `the_exchange_scroll_moves_alpha_to_the_yellow_empire` pins today's order and names this
   item. List every queued kind against its legacy call's writes. *Exit:* each kind either
   sends in call order or is a Divergence row with its reason. This is a difference the
   Rewrite's design causes, so ADR-0008 does not make it an improvement.
10. **Legacy's mistakes still reproduced are fixed** (ADR-0008). Seven are reproduced today;
    each gets a test of the fixed behaviour, a Defect row and, where a player can tell, a
    Divergence row. A register row that calls a fix already made "provisional" loses the word
    when it is next touched: the fix is final. *Exit:* no register row says "reproduced".
    - D4: a group entry at one point spawns its group, not the mob that shares the group's
      number (`G/regen.cpp:346-353`). First count the entries in the owner's regen files that
      reach it, and put the count in the ledger.
    - D5: `AFF_SPAWN` is cleared after the first show, for monsters, stones and a morphed
      player (`G/char.cpp:1886-1887`, `:7701`).
    - A worn piece's attributes are written with its points taken off and put back, so no stat
      stays behind or goes negative (`/attr_full_set`, `/change_attr`, `/add_attr`; ledgers 343
      and 368).
    - A revive herb is spent only when it revives the giver's own dead horse
      (`G/char_item.cpp:9483-9499`, ledger 393).
    - `guild_war_bet.lua:39` formats three verbs with one argument, so no bet can complete on
      either server (ledger 408). Fix the source under ADR-0004's rule.
    - Two more change numbers every player meets, so they are fixed and also listed for the
      owner below: D1, a group-group weighs each member by the probability its file gives
      (`G/mob_manager.cpp:287-293`); and B5, a victim's evasion reads the victim's level, not
      the attacker's (`G/battle.cpp:282`).
11. **A Warp the game thread orders for several characters.** Legacy warps a party, a guild or
    an arena pair inside one call, member by member in player-ID order
    (`ForEachOnMapMember`, `G/party.h:295`, `CDungeon::JumpParty`, `G/dungeon.cpp:396-425`):
    a later member hears each earlier member's removal, and an earlier one never hears a
    later one's. The Rewrite hands each member's descriptor a `ClientOrder::Warp` and each
    descriptor leaves the world when it gets to it, so either member may go first and the
    first by player ID can hear a removal legacy never sends it. Found in ledger 685: two
    forest scenarios lost a full run on it; their readers (`read_own_removal`) now accept
    both orders and name this item. The sites are the dungeon jumps and exits
    (`game_state/dungeons.rs`), the guild war and land Warps (`guild.rs`) and the arena
    (`arena.rs`). *Exit:* the members leave the world in legacy's order on the game thread
    and the scenarios pin it, or the order is a Divergence row with its reason. This is a
    difference the Rewrite's design causes, so ADR-0008 does not make it an improvement.

### D. The play-test kit (opens step 5; the play test itself is out of scope for now)

Done in ledger 680: `compose.yaml` and `Dockerfile` for PostgreSQL 18 and `prodomo`, the
README's Compose section, and the README and the example agreeing on the store's address. The
image builds (101 MB), the stack starts with Podman, the four ports answer with the handshake
phase, `account create` and `gm grant` run through `compose run`, and `compose stop` ends the
server with exit 0 after its shutdown sequence. `compose restart` through Podman's
Docker-compatible API ended it without the SIGTERM; the kit says to stop and start instead.
Left, for when the owner opens the play test: what to change in the client's `serverinfo`; an
unknown or refused client frame logged with its phase, header and bytes, so a session the
Reference client loses can be read from the log; a written play-test script: the first
session's checks, starting with owner questions 7 and 27; and the same start under Docker,
which was not run. *Exit:* a second machine follows the README and a scenario client logs in
to it.

### E. The play test (the owner)

With the Reference client, following the script. Each finding becomes a ledger section; a row a
player reaches that was `unused` is reopened.

### F. Structure, when a package touches the code (never as a sweep of its own)

- A rule that needs only a character, a map and the Game data lives in `world`; `prodomo`'s
  `game_state` keeps what needs the clients, the store queue or several systems at once. New
  rule code follows this; old code moves when its package is open.
- `prodomo/src/main.rs` keeps the command line and the boot; the record handlers move to modules
  by phase. `prodomo/tests/parity.rs` splits into one module per inventory table.
- The Divergence and Defect registers below get an id per row, and code comments cite the id.

## Waiting on the owner

The questions are in `.scratch/migration/owner-questions.md`, with what each blocks. Decided on
2026-11-15: the play test comes last and is out of scope until the owner opens it; the
test-server mode ships off (ADR-0007); the save cycle is 10 seconds; the `unused` rows stand,
provisionally; ADR-0006 is accepted; this machine is the one for the mutation sweeps; the cross
compiler is installed; `change_empire.lua` is fixed (ADR-0004); the drop limit is 1000 ms; the
game logs to keep are the Rewrite's to choose (below); and legacy's mistakes are fixed without
a question (ADR-0008). Nothing the owner has yet to answer blocks packages A to C.

Fixes that change numbers every player meets (ADR-0008). They are made without waiting; the
owner may name one a Quirk, and it goes back:

| fix | legacy | what a player notices |
|---|---|---|
| D1. A group-group weighs its members (package C10; not yet made). | Every member weighs 1: the probability is parsed and dropped (`G/mob_manager.cpp:287-293`). | Which monster groups stand at a group-group spawn, in the mix the file gives. |
| B5. Evasion reads the victim's level (package C10; not yet made). | `int victim_lv = pkAttacker->GetLevel();` (`G/battle.cpp:282`). | A hit's damage shifts slightly: Alpha's punch on dog 101 takes 329 where legacy's takes 322 (ledger 229j). |

Deliberate legacy behaviour that looks odd and stays as a Quirk; changing it is a design
change and the owner's: no exp loss at a death at the top level (question 30,
`G/char.cpp:4113-4116`), and the horse's refill at every login, which the owner's build switches
on with `__FIX_NIVEL_CAL__` (question 35).

## Game logs the Rewrite keeps

Legacy's `LogManager` writes its rows to a MySQL log database (`G/log.cpp`). No log table comes
back (ADR-0001). The owner left the choice to the Rewrite on 2026-11-15 (question 73): a kind is
kept when an operator needs it to answer who cheated, who used GM power, where an item or gold
went between owners, or what happened to an account or a character. Nothing is logged per
monster kill, per pickup or per consumable. Each kept kind is one `tracing` event on the target
`gamelog`, named as legacy names it and carrying legacy's columns as fields, so an operator can
route the target to its own file. Porting the sites is `sys.log` (package B8).

| group | legacy methods | which sites |
|---|---|---|
| Security | `HackLog`, `SpeedHackLog`, `GMCommandLog` | All. |
| Sessions | `LoginLog`, `DetailLoginLog`, `BootLog`, `ChangeNameLog` | All. |
| Character | `LevelLog`, `CharLog`, `QuestRewardLog`, `TopPlayerAffLog` | All. |
| Value | `ItemLog`, `MoneyLog`, `GoldBarLog`, `RefineLog`, `SashLog` | Where an item or gold changes owner, is made by something other than a monster's drop, or is destroyed by a system: trade, shop, private shop, safebox and mall, guild, refine, sash, cube, quest gives and takes, the item shop, GM and Operator grants. Of `MoneyLog`'s types: `SHOP`, `REFINE`, `QUEST`, `GUILD` and `MISC`. |
| Events | `DragonSlayLog`, `MeleyLog`, `OkayEventLog`, `ShoutLog` | All. |
| Not kept | `FishLog`; `ItemLog` at a drop, a pickup or a use; `MoneyLog`'s `MONSTER`, `MONSTER_KILL` and `DROP` | One row per action of the ordinary loop, and none answers a question above. |
| Dead in legacy | `HackCRCLog`, `PCBangLoginLog`, `CubeLog`, `ChatLog`, `InvalidServerLog` | No caller (ledger 566). |

## Build order

| step | scope | state |
|---|---|---|
| 1. Restructure | One `prodomo` binary, TOML configuration, the PostgreSQL store, the Operator commands. | Done (ledger 177-179). |
| 2. Parity inventory | Every legacy system and handler in `.scratch/parity/`, and the scripted client. | Done (181). |
| 3. Vertical slice | Handshake to logout with save, with a Warp between maps. | Done (228). |
| 4. Game systems | Items, NPCs and Transfers, quests, monsters and combat, skills, party and guild, dungeons and events, the Prodomo custom systems, GM commands. | In its tail: 65 `partial` rows (packages A to C above). |
| 5. Game data and play test | The play-test kit, then the owner's play test with the Reference client. | Not started (packages D and E). |

### Acceptance

A Parity inventory item is ported only when its scripted-client scenario passes. The scripted client
is the test-only `parity` crate (181): `parity::Server` starts the real `prodomo` binary on ports the
operating system picks, and `parity::Client` plays raw client bytes over TCP, built with the
`protocol` codecs (and, from step 3, `DescriptorCrypto` in the client-key role). The scenarios are
the tests in `prodomo/tests/parity.rs`, because only a `prodomo` test is told where the binary is;
`inventory_rows_keep_the_rules` checks that every `ported` row names one of them. Each scenario asserts an expected server-to-client
sequence taken from the legacy source. Transfers and cross-Channel features get scenarios with
several clients.

### Play test setup

The owner runs the play test on a Linux machine or VM with the Reference client, after the tail is
closed (owner, 2026-11-15). The kit is package D above; none of it exists yet beyond the README's
account and GM commands (179). Ports default to the legacy ones.

## Topology

Taken from `legacy/config` (see `legacy/README.md` for the full table):

| Channel | legacy ports | notes |
|---|---|---|
| auth | 30001 | Not a Channel. |
| 1 | 30003, 30005 | Hosts 8 maps that Channels 2-4 lack. The owner runs Channel 1 alone (2026-10-14). |
| 2, 3, 4 | 30007/30009, 30011/30013, 30015/30017 | Off in the owner's deployment (2026-10-14). |
| 99 (Shared Channel) | 30019 | Lists 29 map ids, of which 10 have a line in the map index (72, 73, 81, 103, 105, 110, 111, 113, 133, 207). The other 19 get no region, so no route enters them. Its maps exist once for everyone. Never picked at login. Stays on (owner, 2026-10-14). |

In legacy each port belongs to one Core, and a Warp to a map on the other Core of the same Channel
reconnects the client to that Core's port. The Rewrite has no Cores: each Channel listens on all of
its ports, and any of them admits the Channel's players.

The owner decided on 2026-10-14 to run Channel 1 with Channels 2-4 off. They also decided that the
Shared Channel stays hosted if Channel 1's players need it, and they do: in legacy, Channel 1 players
reach Shared Channel maps through the OX event and through two kinds of war.

- **OX event.** NPC 20011 stands on maps 1, 21, 41, 112 and 373. While `oxevent_status` is 1, it
  sends players to map 113 (`event/ox_event.quest:28-39`).
- **Guild battle and flag wars.** The war observer NPCs 11001, 11003 and 11005 stand on maps 1, 3,
  21, 23, 41 and 43. During a battle or flag war they send players to maps 110 and 111
  (`guild_war_observer.lua`, `questlua_pc.cpp:406-441`).
- **Kingdom and siege wars.** The war buttons send players to map 103 (`kingdom_war_event.start`,
  `siege_war_event.start`).

`pc.warp` is ported (ledger 274) and the OX event's route runs (`sys.event.ox` is `ported`). The war
routes are behind the `partial` rows `sys.guild.war`, `sys.event.castle_monarch` and
`sys.event.threeway_war` (package B). The Shared Channel stays on for them (ledger 229f.5).
`config/prodomo.toml.example` runs Channel 1 and the Shared Channel. Channels 2-4 stay in it as
commented-out tables with their legacy maps, so each can be turned on again by uncommenting its table.

Hosting only these two Channels bounds the one game thread's load (ADR-0002). Measured on the
release build at ledger 679: 65,604 NPCs and monsters stand, clients are accepted 2.4 s after the
process starts, it holds 595 MB, and with no player in the world the game thread uses 0.05 ms of
each 40 ms Pulse. (At ledger 229f, before monsters acted: 65,632 characters and 291 MB.) The cost
with players in the world has not been measured (package C1).

Each Channel in `config/prodomo.toml.example` keeps its legacy map set. The binary
refuses a topology it cannot run before binding anything (ledger 178.3). Two legacy settings the
owner should know about (178.5):

- `test_server` defaults to on in legacy and none of the owner's `CONFIG` files turns it off, so the
  legacy deployment ran in test-server mode, where legacy gives **every** character IMPLEMENTOR
  authority (`G/gm.cpp:53`) and, with it, the GM flag `AFF_YMIR` and the protect PK mode at load
  (`G/char.cpp:2231`, `:2362-2375`, `:6354-6358`). The Rewrite ships with the mode off (owner,
  2026-11-15, ADR-0007): game masters come only from `prodomo gm`, and the restart's wait and
  `IsHack` run as production (229k, V18i).
- `BLOCK_LOGIN` refuses accounts created on or after its date. Its legacy default is `30000705`; an
  empty value would refuse every account.

## Legacy flow references

`G/` is `server/server/game/` and `D/` is `server/server/db/`.

- **Handshake.** The default key is `1234abcd5678efgh` (`G/desc.cpp:225-248`). `PHASE` (253) is
  written before TEA is enabled (`G/desc.cpp:494-540`). The echo is at `G/input.cpp:130-157`, and the
  key switch is at `G/input_login.cpp:208`.
- **Auth.** `LOGIN3` (66 bytes) is handled at `G/input_auth.cpp:67-195`, with SQL at `:170-190`. The
  login data is registered at `D/ClientManager.cpp:1998-2050`, and `G/input_db.cpp:1706-1737` sends
  `0x96`. The client then reconnects to a Channel port.
- **Login by key.** `LOGIN2` is handled at `G/input_login.cpp:153-219`. The login checks are at
  `D/ClientManagerLogin.cpp:82-150`. `G/input_db.cpp:109-213` sends `EMPIRE` (90), the 357-byte
  `0x20` record, and `PHASE(SELECT)`.
- **Select.** `G/input_login.cpp:265-304`. `G/input_db.cpp:328-457` sends `PHASE(LOADING)`, then
  249, 15, 28, 16, and 76, then the items (21) via `:1451`.
- **Enter game.** `G/input_login.cpp:562` onward sends 1, 136, and 19 (`G/char.cpp:1060-1293`),
  `PHASE(GAME)`, 106, 121, and `CHAT` (4).
- **What is loaded at start-up.** `D/ClientManagerBoot.cpp` lists every table the legacy DB loaded,
  and `G/input_db.cpp:459-990` shows how the game consumed them. `PROTO_FROM_DB = 0` in
  `legacy/config/db/conf.txt`, so the protos come from the text files.

## Divergences decided so far

| Divergence | legacy behaviour |
|---|---|
| Leaving a Shared Channel map returns the player to the Channel they came from. | Channel 1, because channel 99 learns only the Channel 1 and 99 map locations (`D/ClientManager.cpp:1298-1400`). |
| Every Transfer commits in one transaction when it happens. | The DB cache flushes on a timer; a crash can lose or duplicate items. |
| Passwords are argon2id. | MySQL `PASSWORD()` (`G/input_auth.cpp:125-127`). |
| The adminpage has no default password. | `SHOWMETHEMONEY` (`G/config.cpp:110`). |
| Clients are refused until the store's schema is migrated (179) and, from step 3, until the world has loaded its Game data. While the store is unreachable the ports stay bound and each connection is closed at once. | Clients are accepted before the DB boot completes (`G/main.cpp:691-695`). |
| Logins are 2 to 30 ASCII letters and digits, stored in lowercase. | `account.login` is `varchar(16)` (`legacy/sql/schema/account.sql`), although its column comment says `LOGIN_MAX_LEN=30`. |
| A GM grant is one character Name of one account. Names are unique regardless of case, and a Name held by another account must be revoked before it is granted again. | `gmlist` rows are looked up by exact Name in a `std::map` (`G/gm.cpp:55`); nothing stops two rows whose Names differ only in case. |
| The GM host check and its `gmhost`, `mContactIP`, and `mServerIP` columns are not ported. | Used only when `gm_host_check` is set (`G/gm.cpp:62-105`, `G/config.cpp:45` and `:1212`); none of the owner's `CONFIG` files sets it. |
| An unknown or malformed client frame closes the descriptor. | Logged and consumed, or ignored (ledger 160.5). |
| A store or hashing error during auth closes the connection (ledger 184). | The query failure is logged and the client waits with no answer. |
| An auth login key is drawn only once the login succeeds, and the panama and hybrid-crypt records are not sent (ledger 184). | The key is drawn before the query (`G/input_auth.cpp:113`); `SendPanamaList` and the crypt keys follow the success, from data absent from `legacy/`. |
| An `index` line in the map folder with a map index but no name stops the server at start-up (ledger 185). | `sscanf` leaves the name buffer as it was: the previous line's name, or uninitialised bytes on the first line (`G/sectree_manager.cpp:691-775`). |
| A character Name must end with a NUL inside its 25-byte field (ledger 186). | Read as a C string, running past the field when it has no NUL (`G/input_login.cpp`). |
| The race of a new character is checked as the whole 16-bit `job` word (ledger 186). | Truncated to a byte first, so job 256 creates a warrior (`NewPlayerTable2` takes a `BYTE`). |
| A character is created only for an account with an empire, and choosing empire 0 closes the connection (ledger 186). | A character of an account without an empire is placed near (0, 0); empire 0 is stored and moves the account's characters to (0, 0). |
| A rename naming a slot past 3 closes the connection at once (ledger 186). | Closed 5 seconds later (`DelayedDisconnect(5)`). |
| A text proto line of 2,048 bytes or more, a quoted field still open at the end of the file, and a `mob_names.txt` or `mob_proto.txt` data row with one column stop the server at start-up (ledger 186). | `getline` fails and the rest of the file is silently skipped; the unfinished row is dropped; `std::vector::at` throws. |
| The Channel status list is computed when it is asked for, with the ports in ascending order (ledger 183). | Each Core reports to the DB server at boot and then every five minutes, so a status can be five minutes old; the list is in `unordered_map` order (`G/desc_client.cpp:292-313`, `D/ClientManager.cpp:4455-4466`). |
| An apply type named by an `attr` group row is resolved with the short names of `c_aApplyTypeNames`, which have no `APPLY_` prefix (ledger 192). | Kept, so a row writing `APPLY_MAX_HP` is refused exactly as legacy refuses it. |
| A special-item-group row vnum is read unsigned and a group vnum signed, and every `str_to_number` keeps the low 32 bits of the saturated `strtol` (ledger 192). | Kept, because the C++ types are `DWORD` and `int` and the cast is what legacy does. |
| `MAX_APPLY_NUM` is 130, measured by compiling `EApplyTypes`, and the `// NN` comments in the enum are two too high for 14 members from `APPLY_COSTUME_ATTR_BONUS` up (ledger 192). | The comments are the only place a reader can find these numbers, and they are wrong; the shipped values are right and are not changed. |
| A stored item the load cannot place is refused with a warning that names it, and its row is kept (ledger 210): an unknown vnum, a cell outside its window, or no free cell for an item set aside. A window this build does not load yet is refused the same way, which is a gap and not a Divergence. | An unknown vnum is skipped with its diagnostic commented out (`G/input_db.cpp:1474-1478`); an item set aside with no room is dropped on the ground with a 180-second ownership and a destroy timer (`:1547-1558`). |
| The load reads a character's rows ordered by window, cell and item ID, so which of two rows claiming one cell is set aside does not depend on the store (ledger 210). | Rows arrive in whatever order MySQL returns them: the query has no `ORDER BY` (`D/ClientManagerPlayer.cpp:386` and `:522`). |
| A cell is cleared with a byte-21 `GC_ITEM_SET` whose vnum and every other field is 0 (ledger 211, which supersedes ledger 208's "send nothing"). How the Reference client draws it is calibrated in the play test. | `SetItem(pos, NULL)` sends byte 20 in the 62-byte `TPacketGCItemDelDeprecated`, which the client frames at its 72-byte item set and drops (`G/char_item.cpp:596-611`, ledger 193). |
| An item move's rows are stored in one transaction before its records are sent, and a failed write closes the connection (ledger 211). | The records are sent at once and `ITEM_MANAGER`'s delayed save writes the items later. |
| `game.item_count_limit` is refused at start-up unless it is 1 to 5000 (ledger 211). | Any `WORD` is read (`G/config.cpp:976`): 0 makes every merge move nothing, and a limit above 5000 builds stacks the store refuses. |
| The auto-find's search of the base inventory stops at the character's usable cells (ledger 211). | `GetEmptyInventory` searches to `INVENTORY_MAX_NUM` and leaves a locked page to the grid check; the cell found is the same. |
| The point changes `ComputePoints` writes inside `SetPlayerProto` are not sent (ledger 212). The loading burst's points record sets every slot they set, with the values the load ends on; what the client's select-phase reader does with them is calibrated in the play test. | `BindDesc` comes before `SetPlayerProto` (`G/input_db.cpp:385-386`), so each is written while the descriptor is still in the select phase. |
| A stored `EQUIPMENT` cell is read whole, so a cell at or past `WEAR_MAX_NUM` is set aside into the inventory (ledger 213). | `GetWear` and `EquipTo` take a `BYTE`, so a row at cell 256 is worn at cell 0 (`G/input_db.cpp:1498-1531`). |
| A stored worn item that would start a system not ported yet is refused with a warning and its row kept (ledger 213). This shrinks as each system is ported. | It is worn and starts the system (the dragon soul deck, the aura, mount and pet costumes, the unique and timer events, the accessory stone expiry). |
| `OnAfterCreatedItem` runs for no loaded item (ledger 213). | It locks a blend item and starts its expiry, loads a toggle item, and starts a first-use item's real-time timer (`G/item.cpp:2757-2784`); none of those systems is ported. |
| A language's `locale_string.txt` that cannot be read stops the server at start-up (ledger 221). | The language is skipped and its lines go out untranslated (`G/locale.cpp:356-359`). |
| A shout is heard only by clients in the game phase (ledger 222). | Every descriptor with a character hears it, so a client still loading does too (`G/input_p2p.cpp:227`). |
| `game.shout_limit_level` below 1 stops the server at start-up (ledger 222). | Skipped, so the compiled-in 15 stays (`G/config.cpp:1113-1121`). |
| An NPC's VID counts up from 2^31, so no NPC shares a number with a player (ledger 223); since ledger 229f every monster, stone and ore vein's VID does too. | One counter numbers every character (`CHARACTER_MANAGER::AllocVID`, `G/char_manager.cpp:91-95`). |
| The boot characters are spawned once. No regen event fires and nothing respawns (ledger 223). Since ledger 229f each entry keeps its regen, its count and its first fire's `number(0, 16)`, drawn where legacy draws it, for the event to use. | Every entry with a regen time schedules a `regen_event` (`G/regen.cpp:684-688`), which spawns again whatever of the entry has died. |
| An NPC never moves and has no state machine (ledger 223), and since ledger 229f neither does a monster, a stone or an ore vein. The view calls the hook where legacy starts it (`G/entity_view.cpp:110-117`, `G/sectree.cpp:152-155`); `sys.mob.ai` fills it. | A PC coming into view starts the state machine of every non-PC that is not a warp or goto (`G/entity_view.cpp:110-117`). |
| Every warp and goto NPC's event fires on the Pulses that are a multiple of 12 (ledger 228). | Each NPC's event fires 12 Pulses after that NPC was created and every 12 Pulses after that (`StartWarpNPCEvent`, `G/char.cpp:8017-8030`). |
| V1. The sectrees around a point are walked in legacy's neighbour order. The entities of one sectree and the entries of a view are walked in key order: players by VID, then NPCs by VID, then ground items by ground VID; the moving players of each Pulse are walked by VID. A warp NPC judges the players around it in that order, and a player one NPC has warped is judged by no other NPC on that Pulse (ledger 229b). Items whose destroy event falls on one Pulse go in ground-VID order, before the Pulse's moves (ledger 229c). The NPCs of the state set update in VID order, after the Pulse's player moves (ledger 229i). | Hash order: `m_set_entity` and `m_map_view` by pointer (`G/entity.h:9`, `:62`), and the players' `UpdateCharacter`, which runs their `StateMove`, over `m_map_pkPCChr`, a hash map by name (`G/char_manager.h:22`, `:122`, walked at `G/char_manager.cpp:689-708`; `:716-725` walks the NPC state machines of `m_set_pkChrState`), so the order is unspecified; `WarpSet` takes a warped player out of its sectree at once (`G/char.cpp:6749-6755`, `:7940-7974`, `:8011`). |
| A warp or goto NPC's name is parsed once, when the NPC stands up, and a name that does not parse, or whose target overflows 32 bits, is logged once; that NPC then sends nobody (ledger 228). | The name is parsed on every fire, a failure is logged when `number(1, 100)` draws below 5, and an overflowing target wraps (`G/char.cpp:7909-7922`). No name of a placed warp or goto NPC in the owner's data fails or overflows; four unplaced GOTO protos (10814, 10817, 10818 and 20039) have names that do not parse. |
| The warp NPC's `IsHack` has no `[TestOnly]` line, and the refine, personal shop, cube and aura checks and `CanHandleItem(false, true)` refuse nobody (ledger 228): none of those systems exists yet. This shrinks as each is ported. | `IsHack` adds `[TestOnly]Pulse %d LoadTime %d PASS %d` under `test_server`, and each of those windows and timers refuses the player (`G/char.cpp:8226-8293`). |
| `GC_WARP` names the port the client is connected to when its Channel hosts the target map, and the Shared Channel's first port otherwise (ledger 228). Any port of a Channel admits the Channel's players. | The port of the Core that hosts the map (`CMapLocation::Get`, `G/map_location.cpp:10-41`), so the 14 warp NPCs of each of Channels 1-4 whose target is on the other Core name the other port (the Shared Channel runs one Core). |
| A Warp from a Shared Channel map to a map the Shared Channel does not host is refused as a position no map serves, until the return of the first row of this table is built (ledger 228). No warp NPC leaves a Shared Channel map for another Channel. | The Shared Channel's Core knows Channel 1's maps, so the player goes to Channel 1 (`D/ClientManager.cpp:1343-1369`). |
| `WarpSet` saves the row with the destination before `GC_WARP` leaves, and a failed save closes the connection without it (ledger 228). | `Save` only queues the character (`G/char.cpp:1472-1476`, `:6747`); the queued `SaveReal` runs after `m_posWarp` is set, at the next drain of the queue (every 29 Pulses, `G/main.cpp:276-277`) or at the close's `FlushDelayedSave` (`G/char.cpp:1786-1789`), whichever comes first, and writes the destination (`:1551-1557`). `GC_WARP` leaves without waiting for the store, so no failed write holds it back. |
| A save banks the minutes played at the save cycle, a Warp and the logout, and only the save cycle's bank reaches the world's `POINT_PLAYTIME` while the character stays, so `pc.get_playtime` reads the minutes the last save cycle banked: up to one cycle (10 seconds by default since ledger 679) of play late (ledger 261). The minutes a bank adds do not move the alignment yet. | `CreatePlayerProto` banks at every `SaveReal` (`G/char.cpp:1519-1545`), which the delayed-save queue runs within 29 Pulses of each `CHARACTER::Save()` (`G/char.cpp:1472-1476`, `G/main.cpp:276-277`): a level step (`:4001`, `:4241`), a quest reward (`G/questpc.cpp:332-337`), a skill change (`G/char_skill.cpp:716`, `:902`), a trade, a shop and more, as well as the save event. Each bank adds 5 alignment a minute, or 60 or 120 for a negative alignment, outside a `BANPK` cell (`:1529-1539`). |
| `pc.give_item2` gives only on a dialog step or a click, whose descriptor stores the rows before it sends; on a letter, the login, a level, a kill or a target's event it gives nothing and answers 0 (ledger 262). `pc.remove_item` takes items on the same steps only (ledger 263); `pc.count_item` counts on every step. Its count stops at 65535, and a given potion placed in a bank cell past 255 fills no quickslot. The rewards log and the gold-bar log are not written. | `AutoGiveItem` and `RemoveSpecifyItem` run on every event (`G/questlua_pc.cpp:540-587`, `:801-860`). The count reaches it as a `WORD`, which wraps; the quickslot names the cell as a `BYTE`, which names the wrong base cell (`G/char_item.cpp:9189-9200`); and for 80003 to 80007 the gold-bar log reads the item before it checks it for `NULL` (`G/questlua_pc.cpp:584-589`). All three are Defects. |
| `WarpSet` releases the login as it sends `GC_WARP`, so the client's login by key at the new address is never answered `ALREADY` (ledger 228). | The login is held until the old descriptor is destroyed (`HEADER_GD_LOGOUT`, `G/desc.cpp:133-143`), and a login by key that arrives first is answered `ALREADY` (`D/ClientManagerLogin.cpp:98-105`). |
| After `GC_WARP` the descriptor sends nothing more and drops every frame until the client closes it, a `CG_WARP` included (ledger 228). | The descriptor stays in the game phase with the character out of the world, so its frames are handled; a `CG_WARP` there runs `WarpEnd` on the pending target (`G/char.cpp:6795-6846`). |
| `CG_WARP` in the game phase is answered with nothing and keeps the connection (ledger 228). Whether the Reference client sends it, and on which connection, is confirmed in the play test. | The same on every descriptor whose character holds no pending target: `WarpEnd` returns at once (`G/input_main.cpp:2271-2274`, `G/char.cpp:6800-6801`). |
| A Warp keeps no private map: the row has no `map_index` column, so a character is stored by its position alone (ledger 228). No warp NPC names one. | `WarpSet` stores `lPrivateMapIndex` as the warp map and the save writes it (`G/char.cpp:6727-6737`, `:6757`). |
| `WarpSet` sends no Supplementary Data Block, has no proxy address, no switchbot hand-over and no `CharLog` row; the `WARP` line goes to the log (ledger 228). | It sends the SDB when the map changes, replaces `lAddr` with `g_stProxyIP` when one is set, marks the switchbot warping, and writes a `WARP` `CharLog` row (`G/char.cpp:6709-6725`, `:6769-6783`, `:6787-6789`). |
| A refused `WarpSet` of a warp NPC (a position no map serves) is logged at debug level (ledger 228). | `sys_err` on every fire, so a player standing by one of the ten such NPCs of the owner's data logs an error every half second (`G/char.cpp:6703-6707`). |
| A `server_attr` legacy would misread stops the server at start-up (ledger 229a): a short file, a block longer than 69,699 bytes, a block the decoder fails or that decodes to other than 65,536 bytes, a header wider or taller than the map's sectrees, a header with a negative width or height, a map whose sectrees cannot be placed (a negative base or size, or sectrees past the 16 bits `SECTREEID` keeps of each axis), and a missing file. Bytes after the last block are ignored, as legacy ignores them. The owner's 59 regions and 69 files hit none of these. | `LoadAttribute` checks no `fread`, ignores the decoder's result, stops at a block of the wrong length and leaves every later sectree without attributes, `abort()`s on a header too wide or tall, reads no block for a negative header, and logs a missing file; `Build` ignores its result and goes on (`G/sectree_manager.cpp:392-491`, `:752-753`). `BuildSectreeFromSetting` builds a map with a negative base or far sectrees under truncated 16-bit ids, which `LoadAttribute` computes the same way, and a map with a negative size gets no sectree (`:224-262`, `:424-427`). |
| A sectree past its map's `server_attr` header reads every cell as 0 (ledger 229a, **provisional**, an owner question). The owner's map 216 builds 32 x 32 sectrees and its file covers 24 x 24, so world `x >= 460800` or `y >= 1356800` there. | The sectree keeps no attribute, and the first `GetAttribute` or `IsAttr` there dereferences NULL, an assert in the Debug build (`G/sectree.cpp:202-206`): a random spawn, a player's save or a PvP check on those 448 sectrees ends the process. |
| Only the sectrees inside a map are built (ledger 229a, **provisional**, an owner question). A map whose width or height is not a whole number of sectrees keeps no sectree for its partial column and row, as in legacy; the owner's maps 12, 13 and 226 are such maps. | `BuildSectreeFromSetting` also builds up to two edge trees outside the map, one a column past the partial one and one at a row taken from the base's x (a typo, `G/sectree_manager.cpp:245-259`); neither gets an attribute, and only a forged position reaches them. |
| An entering character whose saved position has no movable point near it is warned about, not logged as an error (ledger 229a). | `sys_err("!GetMovablePosition ...")` (`G/input_login.cpp:579-583`). |
| A regen group stands its members with the first as their leader, and the leader's party answers with it (ledger 229t): a hit or stunned member sends `PM_ATTACKED_BY`, the aggro shares past `PM_AGGRO_INCREASE`, `Return` scatters past `PM_RETURN`, and a follower past its bound walks back to the leader. The party is derived from `Npc::leader`; no party record is kept. | The first member is the master of a new `CParty`, the others join it and follow it, and the leader holds the regen (`G/char_manager.cpp:638-653`). |
| An ore vein's lifetime is drawn and kept, but nothing destroys it, so a vein stands until the process stops (ledger 229f). | `kill_ore_load_event` destroys it `number(420, 900)` seconds after `SetProto` (`G/char.cpp:2563-2570`). |
| An anywhere spawn sends no `test_server` notice (ledger 229f, with owner question 5). The owner's files have no anywhere entry that is not event-gated, so none would. | Under `test_server` it sends every client a notice naming the vnum and the point (`G/char_manager.cpp:375-376`). |
| A click on a character runs its quests (ledger 227) and then answers nothing unless it is a shop keeper, whose window opens (ledgers 223 and 224). This shrinks as the other click triggers are ported. | `CHARACTER::OnClick` runs the quest click and then the NPC's click trigger (`G/char.cpp:6181-6352`). |
| A mob vnum listed twice in `mob_proto.txt` resolves to its first row in file order (ledger 223). | `std::sort` is not stable, so the row the game keeps depends on the standard library (`D/ClientManagerBoot.cpp:220-297`, `G/mob_manager.cpp:57-110`). |
| A missing `country/en/mob_names.txt`, or a regen file that exists but cannot be read, stops the server at start-up (ledger 223). | Logged; every mob is named `NoName`, or the file is skipped. |
| A stranger pays the tripled price it is shown (ledger 224). | `CShop::AddGuest` shows a keeper of another empire each price tripled, and `CShop::Buy` charges it untripled (`G/shop.cpp:658-659`, `:863-873`). |
| A click on a second keeper while a window is open changes nothing (ledger 224). | `AddGuest` refuses it, but `StartShopping` makes the new keeper the shop owner anyway, so the next buy measures the distance to one keeper and buys from the other's shop (`G/shop_manager.cpp:157-158`). |
| A shop item `CreateItem` would make by a path the Rewrite does not keep (gold as an item, a unique, dragon soul, blend or aura item, a timed item, an addon or always-magic item, an auto potion or a skill book) is answered `SOLD_OUT` (ledger 224). No owner shop sells one. | The item is made (`G/item_manager.cpp:160-455`). |
| A bought item is numbered once it has a cell (ledger 224). | The id is taken before the cell is looked for, so a full inventory uses one up. |
| Of the other-window check (`[LS;876]`) only an open safebox refuses a shop (ledger 226): a trade never reaches the check, because since ledger 225 `OnClick` ignores the click of a character that trades (`G/char.cpp:6210-6217`), and the personal shop, the cube and the aura window do not exist yet. The inventory protection (`IsSecured`), `CanHandleItem`, the locked item and the buy-and-sell throttle never refuse a shop step (ledger 224): none of their states or settings exists yet. | Each refuses when its system is in that state (`G/shop_manager.cpp:123-133`, `:385-594`); the throttle is off until a quest sets `g_BuySellTimeLimitValue`. |
| The world settles a trade and the store writes both sides after. When the write fails, the character whose accept completed the trade is disconnected, and its partner keeps the world's side of the trade until it relogs (ledger 225). | `CExchange::Done` changes both characters at once and the delayed saves write them later, so no store refuses a trade (`G/exchange.cpp`). |
| The quest check at `START` (`GiveItemToPC`), the spectator check, `IsSecured`, the exchange-block mode, the item lock, and the personal shop, cube and aura windows never refuse a trade (ledger 225): none of them exists yet. `GiveItemToPC` runs only a quest's `target` click, and no quest can set a target while `target.*` is not ported (ledger 227). A script that waits for either side's client refuses the trade when both accept (ledger 227), an open safebox refuses one, and a safebox load or close holds back every step of its character for 10 seconds (ledger 226). | Each refuses a trade when its system is in that state (`G/input_main.cpp:1367-1527`, `G/exchange.cpp:77-152`). |
| A dragon soul stone, an item outside the `INVENTORY` window and an item with no prototype cannot be offered in a trade: the offer is refused silently after every legacy check (ledger 225). | A stone goes to a cell of the other side's dragon soul inventory (`G/exchange.cpp:524-547`). |
| The save does not write the gold. Each Transfer adds its change to the stored gold in the transaction that stores its items (ledger 225). | `CreatePlayerProto` puts the gold in the table every save writes (`G/char.cpp:1498`). |
| The safebox and the mall belong to the account in a column of their own (`item.account_id`), and the `safebox` table holds only an argon2id hash of the password (ADR-0005, ledger 226). The size and gold columns are not carried over. | The items carry the account id in `owner_id`, the column that holds a character id for every other window, and the password is kept in plain text next to a size and a gold column (`D/ClientManager.cpp:706-814`). |
| A password change creates the account's `safebox` row when it is missing, and the old password must match exactly (ADR-0005, ledger 226). | Only a size change creates the row, so an account without one can never change its password, and the old password is compared without case (`strcasecmp`, `D/ClientManager.cpp:1078-1130`, `:1101`). |
| A character whose safebox never loaded opens it and trades at once (ledger 226). | `m_iSafeboxLoadTime` starts at 0 (`G/char.cpp:278`) and is compared with the Pulse, so for the first 10 seconds after the process starts every safebox load waits and every trade step is refused. |
| The safebox opens one page of nine rows and the mall 27 rows for every account, and the mall opens with what the account holds in it (ledger 226). The premium and large-safebox pages, dragon soul items in either window, the mall's item awards, the inventory lock, `IsSecured`, `CanHandleItem` and the item log are not ported. | A premium account or a worn large-safebox item opens six pages (`G/input_db.cpp:1144-1150`), although neither exists in the owner's build, and the DB process puts each pending `item_award` in the mall as it loads (`D/ClientManager.cpp:726-900`). |
| A quest's flags and state live in the store with the character (ledger 244), so a flag outlives the session that set it. `set_state` moves its quest when the script ends, whichever step called it and however the script ends (ledger 257), `pc.learn_grand_master_skill` writes its read count, and the server writes its own flags: the guild's withdraw time, the war's member flag and `/setblockmode`'s seven `game_option.block_*` flags (ledger 256). `pc.getqf` reads a flag (ledger 257) and `pc.setqf` writes one (ledger 259); the rest are "not ported". A state change runs no `leave`, `enter` or `letter` script, and until the script ends the quest's flag holds the old state, so a character that leaves while the script waits keeps the old one (ledger 257). The load brings the flags with the character, so `Loading quest. Please wait a moment.` is never said, and a save writes a character's whole set of flags at once instead of the flags that changed. A call of an API the Rewrite does not back ends the script silently, so the client sees no `[DONE]` where a script error sends one. The event flags stay in memory (ADR-0006). | The `quest` table is loaded with the character and saved with it (`D/ClientManagerPlayer.cpp:352-394`, `D/ClientManager.cpp:692-698`), a quest waits, with that line, until the load arrives (`G/questmanager.cpp:920-931`), and a save writes only the flags that changed (`quest::PC::Save`, `G/questpc.cpp:519-590`). `quest_setstate` writes the state and its flag at once (`G/questlua_quest.cpp:112-146`), and `EndRunning` runs the old state's `leave`, the new one's `enter` and its `letter` (`G/questpc.cpp:352-371`). |
| `_basic/change_empire.lua` loads, as quest 22, with the stray `end` of its line 132 removed: the owner's one edit to a quest source (2026-11-15, ADR-0004, ledger 680). Every later quest's number is one higher than it would be without it. | Legacy's `qc` aborts on the file and it never reached `object/`, so the quest never ran. |
| Only an NPC's `click` and `chat` scripts run (ledger 227), the `kill` and `levelup` events reach the no-target quests (ledger 244), a war's `letter` and the letter's `button` run (ledger 239), and the `login` event runs when a character enters the game or comes back from a Warp (ledger 257). The login runs at once, behind the enter's records and the offline lines it read. The `logout`, `timer`, `target` and every other event is raised by no ported system, and the hide-and-seek reward of `OnClick` is not ported. | Each system raises its event through `CQuestManager`, and `CHARACTER::OnClick` gives the hide-and-seek reward while that event runs (`G/char.cpp:6299-6330`). When the quest load lands before the game phase, `quest_login_event` runs the login at the first one-second tick that finds the character in the game (`G/input_db.cpp:1005-1048`, `:1104-1115`). |
| A quest API name whose system is not ported raises a "not ported" error, which ends the script silently: the client sees no `[DONE]`, where a Lua error sends one (ADR-0006, ledger 227, ledger 244). Of the 699 names 92 are backed (ledger 267); `pc.warp`, `game.set_safebox_level`, `q.set_title2` and `target.pos` are among the refused. | Every name runs. |
| `find_npc_by_vnum` answers the first NPC of the race that spawned on the character's map (ledger 259). | It walks the race's `unordered_set` (`G/typedef.h:33`, `G/questlua_global.cpp:792-822`), in no defined order, so a map with two NPCs of the race may answer either. |
| A quest target's VID is read as a `DWORD` (ledger 259). | `target_vid` casts the number to `int` (`G/questlua_target.cpp:58-67`). Legacy's VIDs fit, but the Rewrite numbers NPCs from 0x8000_0000, which would not. |
| A click on a marked character while the clicker's script waits is taken and runs nothing; the waiting script keeps its place (ledger 260). | `NPC::OnTarget` runs the target script through `ExecuteQuestScript`, whose `PC::SetQuest` overwrites the running state (`G/questnpc.cpp:206-264`, `G/questpc.cpp`), a Defect. |
| A quest mark's event looks for the marked VID on the character's own Channel (ledger 260). | `target_event` asks `CHARACTER_MANAGER::Find` across the process (`G/target.cpp:57-132`). |
| A quest state change nests at most 16 deep; a deeper change moves the quest without running its scripts and logs an error (ledger 259). | `EndRunning` recurses with no limit (`G/questpc.cpp:352-371`), so two states whose `enter` scripts move the quest to each other recurse until the stack runs out. |
| A quest cannot open `io` or `debug`, `os` holds only `date`, `time`, `clock` and `difftime`, and the base library has no `loadfile`, `load` or `require` (ADR-0006, ledger 227). No loaded file calls a removed name: only the loggers of `luaFunctions.lua`, which no script calls, open a file, and only the unloaded `questing.lua` runs `os.execute`. | `io` and `debug` are opened (`G/questlua.cpp:652-654`, marked `TEMP`), and `luaopen_io` opens all of `os` (`liblua/liolib.c:752`). Lua 5.0's base library holds `loadfile` and `require` (`liblua/lbaselib.c:532-535`). |
| A quest's number is its place in `quest_list` and then in the other scripts sorted by path, and an event's files run in byte order (ADR-0006, ledger 227). | The `readdir` order of `object/state/` and of each event directory, which the file system chooses. |
| A `when` condition or a `begin_condition` that raises an error is false, and a chat menu label that raises one is empty (ledger 227). | The error message is read as the result, which makes a failing condition true (`IsScriptTrue`, `G/questlua.cpp:173-189`). |
| `confirm` and `select_item` are not ported: a script that suspends on either ends (ledger 227). | Each sends its window and waits for the answer. |
| `questnpc.txt` is required, and a sign before a number ends it as any other non-digit does (ledger 227). | A missing file is logged and loads no NPC, and `>>` reads a sign into the unsigned number, so `-1` names NPC 4294967295 (`G/questmanager.cpp:114-158`). |
| A running script's text, skin and error flag live for one call into the manager, and a quest's errors go to the log, not to the player (ledger 227). | They are process-wide, so text a script added and never sent went out with the next character's dialog (`G/questmanager.cpp:1113-1151`). |
| The quest commands of `cmd_info` (`/qf`, `/setqf`, `/getqf`, `/delqf`, `/set_state`, `/clear_quest`, `/eventflag`, `/reload q` and the rest) answer nothing, as every GM command does for a `GM_PLAYER` (ledger 227). | Each runs for a GM of its level. |
| A second drop within a second is refused (ledger 217). Ledger 227 found that the only file setting `item_drop_limit_time`, `questlib_extra.lua`, is never loaded, so legacy's limit is the stored event flag, which the snapshot lacks. The owner left the value to the Rewrite on 2026-11-15: it stays the 1000 ms that one setter names, as a constant (ledger 680). | `g_ItemDropTimeLimitValue` is 0, no limit, unless the event flag is stored (`G/questmanager.cpp:35`, `:1603-1606`). |
| `/click_safebox` is never refused on a dungeon or war map (ledger 226): no map of the Rewrite is one yet. | `do_click_safebox` refuses a character that is not a GM there (`G/cmd_general.cpp:3494-3498`). |
| A command this build does not port answers nothing, every character is `GM_PLAYER`, and a character is either standing or sitting (ledger 226). The GM audit gives the levels, and each system's commands arrive with it. | `interpret_command` runs every entry of `cmd_info` for a character of a high enough GM level, and refuses by the entry's position among eight (`G/cmd.cpp:610-717`, `:666-679`). |
| A checkin of an item offered in a trade is refused silently (ledger 226). | Unreachable: an open safebox refuses a trade, and an open trade refuses the safebox's load (`G/input_db.cpp:1140`). |
| The `LOCALE_YMIR` table is not loaded (ledger 221). | Loaded from the English folder (`G/locale_service.cpp:437-438`) and never read, because `locale_find` answers the text itself for `LOCALE_YMIR` (`G/locale.cpp:73-74`). |
| A dropped gold pile is laid unnumbered, and its count is `u16` (ledger 229o). A pile past 65535 gold keeps its gold on the drop and pays it on the pick-up; the count is never sent. | `CreateItem(1, gold)` numbers the pile from the shared id counter and keeps the count in a `DWORD` (`G/item_manager.cpp:160-410`, `G/item.h:384`). |
| The common-drop table sorts stably by level end (ledger 229o). It shows only with tied level ends; the owner's file is empty. | `std::sort`, unstable on ties (`G/item_manager_read_tables.cpp:102-104`). |
| V2. A player's walk or run is interpolated at the speed of its race's `.msa` clip for its weapon's stance and walk state (ledger 229e). Not ported: the mount arm and the polymorph arm of that choice, and the two recomputes of the move duration, on a POINT_MOV_SPEED change mid-walk and after a forced Goto of a victim, so a walk keeps the duration its Goto set (V2). No monster `.msa` clip is loaded, so every non-player moves at the fall-back 300 units a second (ledger 229i). Every monster's `MOTION_NORMAL_ATTACK` clip is loaded, and a swing holds it `(DWORD)(seconds * 25)`, else 50 (`G/char_state.cpp:1179-1181`; ledger 230c). | From the `.msa` motion of its race, weapon and walk state, else 300 (`G/char.cpp:3555-3578`, `G/motion.cpp:264-356`). |
| V3. ~~A running player's stamina is not consumed and a walking player's is not restored in `StateMove`. No player is set walking or reset (`SetNowWalking`, `ResetWalking`, `/set_walk_mode`, `/set_run_mode`), and a player at stamina 0 or below is walking for `GC_WALK_MODE` (ledger 229b). The affect event's stamina refill is ported (ledger 229d); the walking flags, `PointChange`'s switch, `StateMove`'s restore and consumption, and the two commands are not (parts A, B, D and E of the stamina work).~~ Closed in ledger 569: `StateMove` spends a running player's stamina a point a step in the fight window and restores a walking player's past five seconds, `PointChange`'s switch walks the flag at 0 and restores the mode line's point off it, the refill runs through the switch, the two commands set the lines (ledger 295), and the consume gauge starts and stops with its lines. The uninitialized walk stamp is a recorded Defect; the dead-pool gate stays a pre-existing 229d gap (legacy's arm returns on `IsDead`). | `StateMove` consumes and restores stamina and sets walking at 0 (`G/char_state.cpp:812-851`); `PointChange(POINT_STAMINA)` sets and resets walking (`G/char.cpp:4287-4295`), and `SetNowWalking` relays `GC_WALK_MODE` (`G/char.cpp:7345`, `:7364`); the two commands set it (`G/cmd_general.cpp:2016-2026`). |
| V4. A player entering at a point no sectree holds is shown to itself, sees nobody and is seen by nobody; a later move to a point a sectree holds places it and fills its view (ledger 229b, provisional). | `Show` returns before placing it and `Sync` refuses every move (`G/char.cpp:1847-1854`, `:3386-3387`), so it stays unplaced. |
| V6. The view never holds an observer or an object, and an insert sends no guild name, shop sign or ownership (ledger 229b). | `G/entity_view.cpp:100-104`, `:140-210`, `G/char.cpp:1070`, `:1238-1247`. |
| V7. The loading `GC_ENTITY` list is empty (ledger 187; recorded in 229b). | It lists every player on the Core, the loading one included (`G/sectree_manager.cpp:1697-1746`, `G/input_db.cpp:415`). |
| V9. The revive-invisible affect differs from legacy in five ways (ledger 229d). `LoadAffect`'s @fixme118 records after entry are not sent (owner question 18, provisional). A stopped body's stamina wait starts at its `EnterWorld` Pulse, not at the stop time legacy keeps from the load (owner question 19). The wait counts 75 Pulses, not 3000 ms of legacy's clock (owner question 19). `GD_REMOVE_AFFECT` is not sent to the database, which the client cannot see (owner question 20). The update's alignment is 0, not the character's (owner question 21). A town restart's 50 hit points are healed by the recovery event; nothing refills them at entry (owner question 18; ledgers 229k, 229l). | `LoadAffect` runs 1 to 25 Pulses after entry and refills HP and SP (`G/char_affect.cpp:535-554`, `:660-665`); the stop time is set by the constructor (`G/char.cpp:353`, `:7416`) and reset on each stop (`G/input_main.cpp:1833`, `:1872`), and the refill divides `get_dword_time()` minus it by 3000 (`G/char_affect.cpp:219`); `SendAffectRemovePacket` sends `GD_REMOVE_AFFECT` on expiry and removal (`G/char_affect.cpp:95-99`); `UpdatePacket` sends `m_iAlignment / 10` (`G/char.cpp:1314`). |
| V10. A client's next frame is read only once the world has run its move, pose or talking line, at the world's next Pulse, so every record keeps the order of the frames that caused it; a read holding N such frames is applied over up to N Pulses, as one holding N item steps or syncs already was (ledger 229b). | `DESC::ProcessInput` runs every record of a read in one pass (`G/desc.cpp:252-349`), on the one thread that also runs the Pulse (`G/main.cpp:787`). |
| V11. A warped player's viewers are told of its removal when its connection leaves the world, a few milliseconds after the warp is decided (ledger 229b). | `WarpSet` removes it from its sectree and its viewers at once (`G/char.cpp:6749-6755`). |
| V12. A drop's ground add and a whole pick-up's removal reach the other players in the item's view as the world runs the step, before the store writes its rows. A drop never waits, and its own records, its copy of the add among them, wait for its write. A gold drop and a gold pick-up follow the same rule (ledger 229o); a pile has no row, so the pick-up's wait starts satisfied. A pick-up of an item a drop laid waits for the drop's rows before it writes its own, and its records, its copy of the removal among them, wait with them (ledger 229c, **provisional**, an owner question). A drop whose write fails closes the dropper, and a pick-up waiting on that drop closes with its step unwritten, the store keeping the item where it was; the others saw the item fall, or go, as the world ran the steps. | Every record goes out at once, and `ITEM_MANAGER`'s delayed save writes the items later (`G/item.cpp:533-547`, `:549-584`). |
| V13. A ground item's VID is drawn when it is dropped, from a counter of its own that starts at 1 (ledger 217, recorded in 229c). A client sees only ground VIDs, so it cannot tell the two apart. | `ITEM_MANAGER::CreateItem` numbers every item it creates, held or not (`++m_dwVIDCount`, `G/item_manager.cpp:264`), so the first item dropped carries the count of every item made before it. |
| V15. The target differs from legacy in three ways (ledger 229g). The characters targeting one are walked by VID (V1). A horse has no rider, so it shows 100 for each value. No character is a GM, so nobody targets anything on a castle map (181 to 183), and none is polymorphed, which would matter only for one polymorphed into a horse. `CheckTarget` runs on the player recovery event every 75 Pulses, and a death clears its killer's target (ledger 229k). | `m_set_pkChrTargetedBy` is walked by pointer (`G/char.cpp:6419-6456`, `:6600-6661`); the player `recovery_event` runs `CheckTarget` every three seconds and drops a target 4800 or more away (`G/char.cpp:3250`, `:6663-6669`); a kill clears the killer's target (`G/char_battle.cpp:1275-1276`); a horse shows its rider's horse health (`G/char.cpp:6510-6541`), a player polymorphed into a horse's race does too, and a GM targets on a castle map (`G/char.cpp:6463`). |
| V14. A pick-up finds an item only on the picker's Channel and map (ledger 217, recorded in 229c). | `FindByVID` searches every item of the Core, and `DistanceValid` compares x and y alone (`G/char_item.cpp:7974-7982`, `G/item.cpp:586-597`), so a picker within 300 of the same coordinates on another map of the Core, such as a private copy of the map, picks it up. |
| V16. The regen events, ore-vein ends and the fight's stun, death and recovery events due on one Pulse run by due Pulse, in the order ore-vein end, regen, stun, death, recovery (ledger 229j), then the player's stun, death, recovery and poison (the poison in 229v), then by VID or by map and entry (V1). A map keeps its non-players in a list, and a destroyed one's place is taken by the last one; only the Rewrite's own index sees the list (ledger 229h). A regen run whose VIDs are exhausted logs a warning and stands nothing. A monster's update runs after every event of its Pulse, so a stun event due on the Pulse of a swing kills first (ledger 229l, provisional). | The event heap's order (`G/regen.cpp:582-599`, `:679-689`, `G/char.cpp:2447-2462`); `m_map_pkChrByVID`, a hash map by VID (`G/char_manager.h:120`). |
| V17. A player's first fight differs from legacy in ten ways (ledger 229j; (k) closed in 229k; (b), (g) and (i) narrowed in 229l; (a) narrowed in 229x; (d) closed in 229z; (e) closed in 230a; (h) closed in 230b; (f) narrowed in 230d re PC skills, narrowed in 230e re the arrow/magic skills and SPLASH, narrowed in 230f re skill-up, the self buffs and the grandmaster wire, narrowed in 230g re the party buffs' solo arm, the books and the GRAND training, narrowed in 230h re the combo toggle, the charge pair and the EUNHYUNG hide, narrowed in 230i re the party walk's foundation, narrowed in 230j re roles, bonuses, the parameter, narrowed in 230k re heal, warp and exp share, narrowed in 230l re the leader-unlink bonus clear and the leader-side party_kill, narrowed in 230m re the party chat fan). (a) Only a monster or a stone on the attacker's Channel and map is hit (229x: a stone is struck, the same-empire refusal with `DAMAGE_BLOCK`); a player, a door, a building, a horse and race 5001 are refused with nothing sent, and a player victim waits for PvP. (b) A hit monster fights back (229l), every battle type (229s), with the aggro (229r). (c) `Dead` pays `Reward`'s exp half (229n): the damage map, the split, the alignment, the quest `kill` and the kill recovery; and its gold half (229p): the chance, the multiplier and the branch draws, the piles at the monster's scatter with no owner. No `STACK_ATTR`, and no boss or stone log; the battle pass, the reward missions and the duel, arena and siege hooks hold nothing for a monster a player kills. A hitting drop row creates its item (229q, D5), but no hosted spawn drops on the owner's data, so every hosted kill pays gold or nothing. A gold steal draws and adds no gold (229o); a hitting row's steal pays the purse (230a). (d) A hit lands its poison, stun and slow on a monster victim (229z) and on a player victim (229v). (e) A gold steal pays the purse (230a). (f) The PC skills cast (230d: the 10 melee rows; 230e: the SPLASH walk, the RANGE/MAGIC attr arms, `CalcArrowDamage`, the RANGE/MAGIC/NORMAL_RANGE pipes, the plain bow shot, CRUSH, and the POISON/STUN/`FIRE_CONT` functor arms): `CG_USE_SKILL`'s entry gates, SP charge and cooldown stamp, a `CG_ATTACK` of a skill's type through `CheckSkillHitCount`, the 38-Pulse term gate and the damage arm, and the fly/shoot records (`CG_FLY_TARGETING`/`CG_ADD_FLY_TARGETING` store, `CG_SHOOT` shoots); skill-up, the self buffs (3 4 19 34 49 63 64 65 79) and the heal 109, and the grandmaster wire (the ladder, the grand-add cost, the master-bonus swap, the stored slots) land in 230f; the party buffs compute on the caster alone (no party ported), the books read (`ITEM_SKILLBOOK` + 17 `ITEM_USE` arms, `LearnSkillByBook`, the training count, the horse book) and the GRAND training runs inline (`pc.learn_grand_master_skill`, `pc.get_skill_level`) in 230g; the party walk's foundation lands in 230i (membership, invite, the Update's nears, the fan — dead while no hosted row is PARTY-flagged), the `EUNHYUNG` hide and the combo/MUYEONG/charge specials in 230h, and roles/bonuses/parameter land in 230j, and heal/warp and exp share in 230k. (g) A player's victim is not held; nothing ported reads it. A player's `POS_FIGHTING` is held (229l): its own hit within reach and a monster's first landed hit set it, and its kill, its death and its restart clear it. A monster's first hit (`BeginFight` on the player) takes a running player out of `StateMove`, so the server stops stepping it at its last point until its next move, and the chase does not lead it. Its other readers are not ported: the 10-second `StartMoveChannel` wait (no `cmd.move_channel`) and the `mount` command's position gate. (h) A dead monster with a living reviver in its party stands back up (230b); a hit or stunned member sends `PM_ATTACKED_BY` (229t). (i) A monster's hit points reach its targeters as the hit lands, and so do a player's under a monster's hit (229l); a player's other changes reach them at the end of the command or Pulse. (j) The `__DUNGEON_INFO__` best hit of six dungeon bosses waits for the dungeons; no regen line spawns them. | `G/battle.cpp:99-152`, `:172-210`, `:489-497`; `G/char_battle.cpp:402-548`, `:1238-1696`, `:1990-2433`, `:3342-3371`; `G/input_main.cpp:1966-2009`, `:2010-2160`; `G/char.cpp:653`, `:3172-3240`, `:5469-5575`. |
| V18. A player's death and restart differ from legacy in eleven ways (ledger 229k). (a) The town restart and the automatic restart write their hit points, penalty and `CloseRestartWindow` records before the Warp's `GC_CHARACTER_DEL` and `GC_WARP`, because the Rewrite's Warp saves the row first. (b) The player's recovery event runs `CheckTarget`, the heal and `DistributeSP` (229l), with the poison arm (229v: a poisoned player heals nothing); `UpdateKillerMode` has no killer mode to end. (c) No pet, mount or horse: `CheckPet`, `CheckMount` and `StopRiding` are not run. (d) No guild war, three-way war, dungeon, duel, arena or siege branch and no battle pass kill mission: a death is never forked, and `restart_now`'s war-map test and `do_restart`'s war arms are not ported. (e) `ClearAffect(true)` clears the hit affects (229v: the poison, stun and slow); the revive invisibility, the silence and the 500-600 no-clear band (281: the skill-book delays, the dragon-soul qualification) survive a death; no affect 511 exists, so `DeathPenalty`'s `AFFECT_NO_DEATH_PENALTY` arm is not reached; the cube, sash, change-look and aura refine windows and the premium shop's panel, search and viewers have nothing to close. (f) The item drop penalty is ported (235, V21), and the quest `die` trigger is not run (no owner quest has one). (g) The descriptor learns of the death when it writes `GC_PHASE`, so a command line it reads before that write runs with the standing position (the V10 window). (h) `m_dwLastDeadTime` is not kept; only `CanSummon` reads it. (i) The production arms run: `restart_here`'s wait (allowed from 226 Pulses, 9.04 seconds) and its `IsHack`, and `restart_now`'s `IsHack`, which legacy's default `test_server` skips; the exp line and the `[TestOnly]` line are not sent (owner question 5, provisional). (j) The automatic restart takes the town penalty, which costs nothing, where legacy takes the here penalty (owner question 29, provisional). (k) `DeathPenalty` reads the flag before it rolls or speaks, so a death that owes nothing hears nothing, and a town restart neither rolls nor sends a record (owner question 32, provisional). | `G/char_battle.cpp:330-400`, `:402-548`, `:1238-1696`; `G/cmd_general.cpp:471-706`; `G/char.cpp:1779-1783`, `:3172-3284`, `:6663-6670`; `G/char_affect.cpp:247-337`, `:704-711`; `G/input_main.cpp:1804-1812`. |
| V19. A monster's fight differs from legacy in twelve ways (ledger 229l; (a), (b), (c) and (d) closed in 229s, 229r, 229t and 229w, mob skills out of (e) in 229u, the affect half of (i) in 229v, (l) in 229x, (i) closed in 229y, (f) in 230c). (a) RANGE and MAGIC fight (229s): a hit one shoots (`FlyTarget`, `Shoot`'s two arms), and an aggressive one searches and chases to eight tenths of its range. (b) The aggro is ported (229r): the damage map rows carry `iAggro`, a hit feeds it through `UpdateAggrPointEx`, the chase bleeds it, and `ChangeVictimByAggro` switches past the 3-second window; the party `PM_AGGRO_INCREASE` share is ported (229t), and only `DAMAGE_TYPE_NORMAL` reaches the multipliers (skills and bows wait). (c) The party fights with it (229t): a hit or stunned member sends `PM_ATTACKED_BY`, the aggro shares past `PM_AGGRO_INCREASE`, `Return` scatters past `PM_RETURN`, and the protector follows the leader in Idle and at the dead end. (d) A guard looks for the nearest monster past `noguard`, battles it on the NPC slot, and its swing stuns with `NO_REWARD` and no hit-point change (229w). (e) A fighting skill monster rolls its slots before its swing and casts (229u: the 162 self-heal, the hit events, `ComputeSkillAtPosition`'s splash-damage arm, CRUSH's slide, 263's slow, 270's refusal — proved in-memory, since no hosted spawn names a skill row); no summoner's call or god speed is used, and race 2191's jump, race 2493's battle and the Meley re-search are not ported. (f) A swing holds the monster its clip's seconds (230c, V2). (g) `FindVictim` walks the 3x3 sectrees in `around_of` order, so of two players at one distance the first met is taken (V1). No player is a building or holds `AFF_TERROR`, `AFF_EUNHYUNG` or `AFF_INVISIBILITY`. (h) The `test_server` lines are not sent, and a chaser does not switch to running (owner question 5). (i) A monster's hit lands its poison, stun and slow (229v: the level arm, the 10-tick poison event with its HP-1 floor, the heal refusal, the stun's stop and attack gates, the slow's −30), and the victim's reflect answers (229y: the pre-bonus share, thirded past `IMMUNE_REFLECT`) with the SPECIAL tail and the rewarded `NO_REWARD` split; the marriage, monarch and `mob_dam` terms take their defaults. (j) A player stunned by a monster dies by the monster's next swing, with the monster as killer. When the monster stops first, `StunEvent` kills it with no killer, which costs no exp and drops nothing (legacy, provisional). (k) A stun event due on a swing's Pulse runs first (V16, provisional). (l) A stone spawns its sons from the ten-arm Idle ladder (229x, closed): the runtime `SpawnGroup`, the son's Idle, and the half-exp feed; a 0-MaxHP stone reads 0% (legacy SIGFPEs), and a dead or missing stone reads as none in the protege, nearest-victim and feed readers (legacy reads freed memory) — both Divergences. | `G/char_state.cpp:626-1183`; `G/char.cpp:3471-3510`, `:3555-3583`, `:6848-7034`, `:7459-7471`; `G/trigger.cpp:59-174`; `G/battle.cpp:154-817`; `G/char_battle.cpp:220-328`, `:1744-2436`, `:3411-3428`, `:3835-3897`; `G/party.cpp:875-950`. |

| V20. A guild symbol upload is judged as if the guild held land (ledger 230v). | `test_server` opens the land gate (`G/input_login.cpp:1230`); the owner's files run `test_server` 1 and real guilds hold land, while no land system exists yet, so a literal port would close every symbol upload. |

| V21. The death's item drop penalty differs from legacy in six ways (ledger 235). (a) The shuffle uses the game's one generator, where legacy uses a fresh `std::mt19937` seeded from `std::random_device` (`G/char_battle.cpp:1085-1087`), so which items a penalty drops is not legacy's set. (b) The dropped items' rows go at once, where legacy's go in the delayed-save queue's flush, up to 127 Pulses (about 5.1 seconds) after the drop, and they name the victim until then. (c) `AddToGround` finding no tree for the point loses the item in legacy (`G/item.cpp:569-574`, a Defect); the Rewrite keeps it. (d) The GM arm is narrower: legacy's `GetGMLevel() == GM_PLAYER || test_server` runs the penalty for a GM when `test_server` is on, and it is on in the owner's configuration; the Rewrite spares the GM. (e) Legacy's quickslot sync for a worn item names the wear index, which is an inventory cell, and deletes the wrong quickslot (`G/char_battle.cpp:1150`); the Rewrite syncs the flat cell and deletes nothing. (f) The private shop, the protected inventory, the agreed PvP, the arena duel and the forked road are not ported, so their refusals and their flags cannot happen. | `G/char_battle.cpp:1016-1199`; `G/item_manager.cpp:478-499`, `:546-564`; `G/main.cpp:284-288`; `G/item.cpp:569-574`; `G/unique_item.h:29`. |
| V22. A guild war's member is sent to the war map before the quest that asked it closes, where legacy sends it after (ledger 239). The quest's end (`[DONE]`) is written first and the warp follows, because the quest step answers the descriptor and the warp goes through the order queue. | The `GC_WARP` is sent while the script runs, so the load comes first and the quest's end after it (`CHARACTER::WarpSet`, `G/char.cpp:6694-6792`). The two records are the same; only their order differs. |
| V23. A guild war's map sends its members back to the place they came from, where legacy sends them back onto the war map (ledger 240). The exit's warp names the town the member came from, and the member stays connected; legacy warps to the pending warp, which the warp to the war map overwrote with the war map's own start position, so the member is sent back onto the war map and is disconnected with it a few seconds later. | The rule is `CHARACTER::ExitToSavedLocation` (`G/char.cpp:6687-6692`), which warps to `m_posWarp` and `m_lWarpMapIndex` (`G/char.cpp:6690`); the place the member came from is `m_posExit` and `m_lExitMapIndex` (`G/char.cpp:6679-6683`), which legacy forgets without reading. The end state is the same: the member is back in the town, by its next login in legacy and by the warp in the Rewrite. |
| V24. The flag war's flags take and score are run by the war map's tick, where legacy runs them by the flag and base characters' own states (ledgers 242 and 243). The wire is the same: the nearest player that may take a flag carries it, the base scores the other team's flag, and the flags reset and come back ten seconds later. | The flag mob's state (`CHARACTER::StateFlag`, `G/char_state.cpp:1185-1216`) and the base mob's (`CHARACTER::StateFlagBase`, `:1218-1222`) run through the character state machine, which the Rewrite's characters do not have; the war map's tick runs the same two rules (`prodomo/src/game_state/guild.rs`). |
| V26. A quest reads the character's stored GM level even in the test-server mode (ledger 272). `pc.is_gm` and `pc.get_gm_level` answer the points' level, which rode the admission; the sash quest's `not pc.is_gm()` gate opens for players and no GM panel letter lands at login, as the death penalty's and the restart's production arms already judge. | `GetGMLevel` answers `GM_IMPLEMENTOR` for every character while the mode is on (`G/char.cpp:6355-6356`), and the owner's `CONFIG` files never turn it off, so on the deployment every character was a GM, the sash quest never fired, and every login lettered the GM event panel. The `test_server` audit (owner question 5) settles the silence. |
| `/reload`'s `t` and `m` arms read their files before they swap, so a bad file keeps the live tables (ledger 358.1). | `do_reload` destroys the live maps and groups first (`G/cmd_gm.cpp:2683-2715`), so a bad `special_item_group.txt` or `mob_drop_item.txt` empties them until the next good reload. The caller hears the same Romanian miss lines either way. |
| V27. A `/shutdown` after the bar restarts nothing, and a `/shutdown` mid-countdown restarts it at 10 (ledger 360). The game thread must not kill the process, so the operator stops it with SIGTERM, which the graceful path already serves; the bar never clears. | `Shutdown()` past the bar calls `thecore_shutdown()` (`G/cmd_general.cpp:246-249`), exiting the process at once; mid-countdown it starts a second concurrent event, whose notices double the first's (`:250-261`). |
| A login's loaded affects lay their points at the next computation, not while they load, and a saved mount sends no `MountVnum` re-show (ledger 399). The adds, and the mount's and its bonus's removes, ride the burst as legacy's do. | `LoadAffect` runs `ComputeAffect` on each row it loads (`G/char_affect.cpp:633`), so a saved mount re-shows the character mounted and `RemoveAffect(AFFECT_MOUNT)` re-shows it unmounted (`:644-645`); the Rewrite's restore runs before the burst, where point records and a re-show would reach the client ahead of the character it describes. |
| A quest's `command` line never trips the anti-flood, and a click's dialog records beat the world's notices to the socket (ledger 413). The flood is fresh on every quest line, and the step's records go straight to the socket while the broadcasts travel through the outbox. | `interpret_command` counts quest lines against the character's flood and disconnects past 10 a second (`G/cmd.cpp`), and the notices go out while the script runs with the dialog at its end; the world has no delayed-disconnect path, and the step's records bypass the outbox. |
| The daily-gift cash is loaded at the admission and spent from memory (ledger 441): an Operator grant mid-session shows at the next login, and the spent delta saves with the row. | `GetCash` queries `account.account` on every open (`G/char_daily_gifts.cpp:321-345`). |
| A sash material cannot be moved onto, and a sash material cannot be given (ledger 442): both refuse silently with `Locked`. | A move buries the held item (no destination check, `G/char_item.cpp:7636-7637`); `GiveItem` runs no `isLocked` check. |
| The `USE_DETACHMENT` sash clean skips the `ITEM_FLAG_REFINEABLE` check (ledger 442): the flag bit is not ported. | `IS_SET(pkTarget->GetFlag(), ITEM_FLAG_REFINEABLE)` gates the arm (`G/char_item.cpp:1996`). |
| The Gaya shelf and slot unlocks live in memory per login: the unlocks are kept (ledger 444). | `<Name>_gaya_system.txt` per character (`G/char_gaya.cpp:175-273`); the path is `GetName` unsanitised, and `fopen` failures NULL-deref. |
| A cube category past 100 bytes is truncated to the wire, and `/reload c` reads before it swaps (ledger 448): a bad or missing file keeps the live recipes, silently. | `memcpy` of the whole string into `char[100]` (`G/cuberenewal.cpp:559`); `Cube_init` destroys the proto first, so a miss leaves no recipes (`G/cmd_gm.cpp:2716-2719`). |

| The `envanter` column refuses unlocks past 18 at the store (ledger 449): a hand-written row cannot name cells past the base inventory. | Legacy's `player.envanter` is an unchecked `INT` (`player.sql:866`), copied straight into `m_points.envanter` (`G/char.cpp:2346`). |
| Every `/dungeon_info rank` line reads the store (ledger 463): the 5-minute `m_mapDungeonRank` cache is not ported. | `SendDungeonRank` caches each rank for 5 minutes (`G/char_manager.cpp:1340-1380`). |
| A duel's players are held by player id across the process, so a Warp to an arena map another Channel hosts keeps the duel, and `EndDuel` sends each duelist back to the Channel it started on, Channel 1 for one that started on the Shared Channel (ledger 467). | The arena manager is per process: a duelist warped to another core leaves the duel's process, and the next arena event ends the duel as "opponent disappeared" (`G/arena.cpp:224-245`). |
| A nonmember woken on an arena map is ejected to its empire start on its own Channel when that Channel serves the target, else on the lowest-numbered Channel that does (ledger 469, owner question 60). | Legacy's `WarpSet` resolves the target's core through the P2P map table (`G/char.cpp:6694-6795`). |
| The test-server mode ships off: `[game] test_server` defaults to `false`, so no character is a game master without a grant from `prodomo gm` (owner, 2026-11-15, ADR-0007, ledger 679). | `test_server` is compiled on (`G/config.cpp:72`) and the owner's `CONFIG` files never turn it off, so `GetGMLevel()` answers `GM_IMPLEMENTOR` for everyone (`G/gm.cpp:53`). |
| A character is saved every 10 seconds by default (owner, 2026-11-15, ADR-0003, ledger 679). | `save_event_second_cycle` is compiled as 120 seconds (`G/config.cpp:31`) and the owner's `CONFIG` files set 180, behind the DB cache's own flush. |

## Legacy defects not to reproduce

- `CShop::SetShopItems` checks the display slot against the 45-cell grid but writes it into the 40-entry window vector, so a `display_pos` of 40-44 writes past `m_itemVector` (`G/shop.cpp:139-161`). The Rewrite refuses the table, as the slot has no window cell (`myshop_slots`, ledger 491).
- When a character enters the game with no movable point within 1000 of its saved position,
  legacy calls `Show` at its empire's recall position without testing that the map has a
  sectree there, and at an uninitialised `PIXEL_POSITION` when the map has no recall position,
  because `GetRecallPositionByEmpire`'s result is ignored (`G/input_login.cpp:572-590`). Where
  no sectree of the map holds that point, `Show` returns before placing the character
  (`G/char.cpp:1847-1854`): it stays at its saved position and its client never gets its own
  insert. The owner's map 235 (`metin2_map_n_flame_dungeon_01`) recalls every empire to
  (614200, 706800), a point of map 62. The Rewrite keeps the saved position, shows the
  character there and warns (`prodomo::loading_phase::entering_position`, ledger 229a). The view
  treats such a character as the V4 Divergence says (ledger 229b). A town restart or the automatic
  restart on map 235 warps to that point of map 62, as legacy's does; the Rewrite keeps it as data
  (ledger 229k).
- A drop from a point no sectree of the map holds takes the item, or the part of a stack, from
  its cell before `AddToGround` fails, and the item is lost with no line
  (`G/char_item.cpp:7503-7541`, `G/item.cpp:569-574`). The Rewrite refuses the drop after every
  check legacy makes, so each line legacy sends first is still sent, and the item stays in its
  cell (`MoveRefused::NoSectree`, ledger 229c, **provisional**, an owner question). Only a
  character the V4 Divergence keeps outside every sectree can drop from such a point.
- `LoadAttribute` reads each block's `uiSize` bytes into a 69,699-byte stack array without
  bounding it (`G/sectree_manager.cpp:413`, `:461-462`). The Rewrite refuses a longer block at
  start-up (`gamedata::server_attr`, ledger 229a).
- `CAttribute::Get` tests `x > width || y > height`, so on a block with cells of more than one
  value a column of 128 reads the next row's first cell, or one past the cells on row 127, and
  row 128 follows a row pointer read one past the block's row table; a block of one value
  answers it (`server/server/libgame/attribute.cpp:44-78`, `:220-235`). No legacy caller
  passes one; `CellBlock::get` answers 0 there (ledger 229a).
- `SECTREEID` keeps 16 bits of each sectree axis (`G/sectree.h:13-23`), so a position at or
  past 419,430,400 aliases a sectree near the origin. The Rewrite finds no sectree there
  (`SectreeGrid::sectree_at`, ledger 229a). The view uses the grid, which does not alias (ledger
  229b).
- Three spawn mistakes are still reproduced and are fixed by package C10 (ADR-0008; ledger
  229f, owner questions 22 to 24). **D1:** a group-group picks its group uniformly:
  `LoadGroupGroup` parses each member's probability and then calls `AddMember` without it
  (`G/mob_manager.cpp:287-293`), so every member weighs the default 1 and `GetMember`'s
  `number(1, total)` (`G/mob_manager.h:58-83`) is one uniform draw. **D4:** a group or group-group entry whose box
  is one point goes to `SpawnMob` with its group vnum as a mob vnum (`G/regen.cpp:346-353`), so it
  spawns the mob of that number, if one exists, instead of the group. **D5:** `Show` with the spawn
  motion sets `AFF_SPAWN` and nothing resets it (`G/char.cpp:1886-1887`), so every boxed stone and
  every spawn-motion monster carries it in each later insert. `SetPolymorph` sets it on a player too
  (`G/char.cpp:7701`), so a player who used `/polymorph` carries it in each later insert and
  update, even after the clear (ledger 364).
- For the first 10 seconds after the process starts, `IsHack` refuses every player a warp NPC
  finds with `[LS;850;10]`, because `m_iSafeboxLoadTime` starts at 0 and is compared with the
  Pulse (`G/char.cpp:278`, `:8234-8241`). The Rewrite refuses nothing for a timer that was never
  set (ledger 228, as ledger 226 did for the safebox).
- The login's map refusal sends a character of empire 0 home with `EMPIRE_START_MAP(0)` and
  `EMPIRE_START_X(0)`, which is `(0, 0)` and so not a pending Warp: the save writes the position
  unchanged and every later login refuses the same map again (`G/input_db.cpp:424-435`,
  `G/char.cpp:1551-1565`). The Rewrite stores nothing, the same row, and does not loop the
  client itself (`prodomo::warp::home_warp_location`, ledger 189, recorded in 228).
- `WarpEnd` folds a private map with `index > 10000` (`G/char.cpp:6805`), while `WarpSet` 78
  lines above and every sibling test fold with `>= 10000` (`:6727`), so map 10000 is
  checked as itself. The Rewrite folds with `>=` (`prodomo::warp::warp_map_for_check`, ledger
  189, recorded in 228).
- `SetWarpLocation` multiplies both axes by 100 in a 32-bit `long` with no overflow check
  (`G/char.cpp:6672-6677`), so a large coordinate wraps to the wrong side of the world. The
  Rewrite refuses it (`prodomo::warp::set_warp_location_checked`, ledger 189, recorded in 228).
- The select index is used before its bound check (`G/input_login.cpp:281` before `:288`). The
  create path forwards an unchecked index and uses `strncpy`.
- Auth SQL is built by string formatting (`G/input_auth.cpp:133-152`), and `TABLE_POSTFIX` is
  interpolated into SQL.
- The proxy `lAddr` is written after the `memcpy` that sends it (`G/desc.cpp:874-907`).
- A use-after-free at `G/char_item.cpp:7438-7439` on every successful item destroy.
- The item-window delete record is 62 bytes on a byte the client reads as a 72- or 60-byte
  item set, so a stock client drops it (ledger 193, `G/char_item.cpp:597-610`). The codec
  records the legacy send; what the Rewrite should send instead is left open.
- `GetInventoryPageByPos` takes a signed category and never range-checks it, so category `-1`
  reports a page for an equipment cell (ledger 194, `G/char_item.cpp:314`).
- `RefineInformation` carries its cell in a `BYTE`, so a custom-inventory cell above 255 cannot be
  named in that record (ledger 194, `item_length.h:196`).
- `CHARACTER::GetItem` validates with a check that admits a safebox or mall position, then has no
  case for either and returns `NULL` (ledger 194, `G/char_item.cpp:256`).
- `CItem::SetCount` destroys the item from inside its own setter when the count reaches zero
  (ledger 195, `G/item.cpp:307-338`), so the caller keeps a freed pointer.
- `CHARACTER::SetItem` writes the new item's grid over a neighbour's marks with no conflict test
  (ledger 195, `G/char_item.cpp:462`), and the neighbour's later `RemoveItem` then zeroes them.
- `CHARACTER::AddToCharacter` guards an item it already owns with an `assert` that compiles out in
  Release, so the item is stored twice and nothing is reported (ledger 195, `G/char_item.cpp:380-383`).
- `CHARACTER::RemoveItem` reads `pSwitchbotItems[wCell]` six lines before checking `wCell`
  (ledger 195, `G/char_item.cpp:537` and `:543`).
- A stack whose walk leaves its category is stored with a grid that records fewer cells than the
  item claims (ledger 195, `G/char_item.cpp:426`).
- A private shop persists only sockets 0..2, so a shop round trip zeroes socket 5, which is
  `ITEM_SOCKET_UNIQUE_REMAIN_TIME` (ledger 195, `prodomodefines.h:189`).
- `USE_CHANGE_ATTRIBUTE2` consumes its scroll and reports success on 27% of uses, because level 6
  is rejected after the attributes are already cleared (ledger 195, `G/char_item.cpp:6434`).
- `AddRareAttribute` never tests `nAttrSet == -1`, so an attribute-67 client record on an arrow
  reads `bMaxLevelBySet[-1]` and the quest still reports success (ledger 195,
  `G/item_attribute.cpp:439`).
- A 17-byte desync in `PrivateShopItemCheckin` (ledger 148).
- An out-of-bounds write at `D/ClientManagerBoot.cpp:357-362`.
- Never send the Panama packet (151).
- A second `LOGIN3` on one auth descriptor with a different login leaves the first login marked as
  connected (`ConnectAccount` overwrites the descriptor's login without releasing the first).
- A use-after-free on every successful auth: `G/db.cpp:456-457` logs `pinfo->login` after
  `M2_DELETE(pinfo)`.
- `QUERY_LOGIN_BY_KEY` checks for a logged-on account (`ALREADY`) before it compares the login and the
  client key, and the game then kicks the descriptor holding the login the client *typed*
  (`D/ClientManagerLogin.cpp:82-149`, `G/input_db.cpp` `LoginAlready`). Any live login key could
  disconnect any logged-on account by name. The Rewrite judges the key first (ledger 185).
- The DB server's player-cache branch of `CreateAccountPlayerDataFromRes` forces `bChangeName = 0`,
  so a pending forced rename disappears from the list once the character is cached. The Rewrite
  always sends the stored flag (ledger 185).
- A creation refused for its Name or shape is answered with a zeroed 10-byte
  `TPacketGCLoginFailure` under `HEADER_GC_CHARACTER_CREATE_FAILURE`, whose client record is 2
  bytes (`G/input_login.cpp:463-481`). The Rewrite sends the 2-byte record, type 0 (ledger 186).
- Choosing an empire moves only the characters in the first three of the four slots to the
  empire's start (`D/ClientManager.cpp:1217-1281`). The Rewrite moves every slot, in the same
  transaction as the check (ledger 186).
- Three client Python wrappers send uninitialized stack data; the server must not trust those
  bytes.
- The client-version check is on by default against a hard-coded `"1215955205"` and neither config
  key is set, so every client with another version gets a notice and `DelayedDisconnect(0)`
  (`G/input_login.cpp:743-771`). The `if (!d->GetClientVersion())` arm above it is dead, because
  `GetClientVersion()` returns `c_str()`. The Rewrite lets the client in and records the version
  it saw (ledger 187).
- `CHARACTER::PointsPacket` sends 16 bytes of uninitialized stack in point slots 0 (`POINT_NONE`)
  and 2 (`POINT_VOICE`) (`G/char.cpp:2033-2086`). The Rewrite writes all 255 slots (ledger 187).
- After a failed `GetMovablePosition`, legacy takes the recall position's `x`, `y` and `z`,
  and `LoadMapRegion` never sets that `z` (`G/sectree_manager.cpp:347-386`), so an
  indeterminate height reaches the insert packet and the save (`G/input_login.cpp:572-590`,
  `G/char.cpp:1081`, `:1563`). The Rewrite sends and saves 0 (ledgers 187 and 229a).
- A `group` line whose name holds a space calls `exit(1)` (`G/text_file_loader.cpp:75-81`), and a
  `Bind` on a file ending in a high byte appends one byte past the buffer
  (`G/file_loader.cpp:99-102`). The Rewrite reports the group name and stops at the buffer
  (ledger 192).
- A text-file key with no value ends its group with a `sys_err` and a `break`, so the rest of the
  group is silently dropped (`G/text_file_loader.cpp:133-140`). The Rewrite returns an error
  (ledger 192).
- A special-item-group row with fewer than three fields reads `pTok->at(1)` and `pTok->at(2)` past
  the end of the vector, and an `attr` row with one field reads `pTok->at(1)`
  (`G/item_manager_read_tables.cpp:264`, `:266`, `:190`). The Rewrite refuses the file (ledger 192).
- `char buf[4]` truncates the decimal row key to three characters, and the row loops run to
  `k < 1024`, so the last 24 iterations read the rows written for keys 100, 101 and 102 again and
  add their items a second time (`G/item_manager_read_tables.cpp:171-174`, `:214-217`). The Rewrite
  builds the key as a decimal string, so key `1000` means `1000` (ledger 192).
- `GetAttrVnum` returns a `DWORD` from the row's signed `int count`, so a negative count becomes a
  huge attribute vnum rather than 0 (`G/item_manager.h:206-218`). The Rewrite keeps the bits, as
  the type demands, and says so (ledger 192).
- A character delete runs `DELETE FROM player` and checks its result, then a **second** statement
  for the items whose result is discarded, so a failure on the second orphans every item while the
  client is told the delete succeeded (`D/ClientManagerPlayer.cpp:1522-1532,1552`). The Rewrite
  gets `ON DELETE CASCADE` from the foreign key, so the two are one statement (ledger 196).
- A stored item whose vnum is not in the Game data is dropped by a bare `continue` with the
  diagnostic **commented out**, and the row is never deleted, so the same silent drop repeats on
  every login (`G/input_db.cpp:1474-1478`). The Rewrite refuses that one row with a warning that
  names it, keeps the row, and loads the rest (ledger 210; ledger 196 said the load fails, which
  would lock a player out over one row an Operator can repair).
- `QUERY_ITEM_DESTROY` is handed the owning pid, uses it for the log line and to choose the async
  or sync branch, and then deletes on the item id alone -- as does the cached path -- even though
  the id space is global across every character, safebox, the mall and the ground
  (`D/ClientManager.cpp:1833-1846`; `D/Cache.cpp:57-61`). The item id is never client-supplied, so
  this is not client-reachable. The Rewrite deletes on `id` **and** `owner_id` (ledger 196).
- The item award path writes an **account** id into `player.item.owner_id` for a safebox or mall
  row and a character id for everything else, with nothing in the schema telling the two apart
  (`D/ClientManager.cpp:997-1008`). The Rewrite makes `owner_id` a typed reference to `player`, so
  an awarded item has to belong to a safebox table instead (ledger 196).
- The award insert's failure test is a disjunction, `uiAffectedRows == 0 || uiInsertID == 0 ||
  uiAffectedRows == (uint32_t)-1`, and its `uiInsertID` term can never be true for a successful
  insert into an `AUTO_INCREMENT` table, so it is dead weight inside a check that is otherwise
  right (`D/ClientManager.cpp:1012`). The Rewrite tests the affected-row count alone (ledger 196).
- `player.attrtypeN` is a MySQL `tinyint`, so the legacy table can hold a negative attribute type
  that legacy's own `TPlayerItemAttribute::bType`, a `BYTE`, cannot; the load wraps it
  (`D/ClientManagerPlayer.cpp:60`; `C/tables.h:428`). The Rewrite's `CHECK` is 0..255, so a corrupt
  row is an error rather than a wrap (ledger 196).
- `MoveItem`'s auto-find stores `GetEmptyInventory`'s -1 in a `WORD` and then tests it against -1,
  which is always unequal, so neither "no room" notice is ever sent: a full inventory writes 65535
  into the destination, which fails `IsValidItemPosition` in silence (`G/char_item.cpp:7644-7645`,
  `:7658-7659`). The Rewrite reads the search's answer and sends the notice (ledger 211).
- An item that belongs to two custom banks is placed in the first with room
  (`G/char_item.cpp:1214-1225`), but a move into a bank is checked against `GetItemCategory`, which
  answers only the first bank the item belongs to (`G/item.cpp:3149-3158`), so an item placed in
  its second bank cannot be moved inside it. Five of the owner's items are in two banks. The
  Rewrite accepts any bank the item belongs to (ledger 211). A checkout from the safebox into a
  bank has the same check (`G/input_main.cpp:2392-2396`), and the Rewrite accepts any bank there
  too (ledger 226).
- A split passes the source cell to the grid check as its exception (`G/char_item.cpp:7807`), so a
  split can land on the stack it came from. The Rewrite checks a split a second time without the
  exception (ledger 211).
- `ITEM_FLAG_IRREMOVABLE` is tested only when the source names window 1 (`G/char_item.cpp:7625`),
  so the same flat cell named through window 2 moves an irremovable item out of the equipment or
  belt cells. The Rewrite tests both windows (ledger 211).
- `PointChange` writes its record (`G/char.cpp:4803-4820`) and then `UpdatePointsPacket` writes it
  again (`:2089-2118`); the second copy halves `POINT_MOV_SPEED` under the map's movement will, so a
  weak conqueror is first told a speed it does not have. The Rewrite writes one record, with the
  value legacy's second copy carries (ledger 212).
- `ComputePoints` stores the base maximum hit and spell points only when the new base differs from
  the old *total* (`G/char.cpp:3050-3062`), so an old total that equals the new base keeps the
  previous base and every bonus is added to it. The Rewrite always stores the new base (ledger
  212).
- The point arithmetic sums in `int` and wraps. The Rewrite sums in 64 bits and saturates; no value
  a client can reach comes near either bound (ledger 212).
- `ApplyPoint` keeps a pool's share of a maximum a worn item raises (`G/char.cpp:4876-4893`), but
  at load the stored pool was saved with the item worn and the share is taken against the maximum
  without it, so a relog heals: 600 of 1500 comes back as 900 of 1500. The Rewrite keeps the
  stored pools and clamps them (ledger 213).
- `aApplyInfo` gives `APPLY_ATTBONUS_BOSS` (90) the stone bonus point and `APPLY_ATTBONUS_METIN`
  (91) the boss bonus point (`G/constants.cpp:820-821`), so a bonus against bosses counts against
  stones. The Rewrite maps each type to the point of its own name (ledger 213).
- `ComputePoints` ORs a worn item's proto immunity bits into the character's (`G/char.cpp:3078`),
  but the two use different bit orders (`ProtoReader.cpp` against `length.h:713-722`), so an item
  immune to stun makes its wearer immune to falling. No wearable row of the owner's proto sets
  the column; the Rewrite refuses a worn item that does (ledger 213).
- An armour's rarity in socket 4 indexes the three-entry table `{0, 20, 50}` without a bound
  (`G/item.cpp:938-944`, `1146-1155`). The Rewrite adds nothing for a value outside it (ledger
  213).
- A weapon's refine element applies `APPLY_ENCHANT_ELECT + (type - 1)` (`G/item.cpp:912`, `1142`),
  so a type past 6 names an unrelated apply. The Rewrite grants nothing for it (ledger 213).
- The item bonus arithmetic multiplies in 32-bit `long` and wraps. The Rewrite computes in 64 or
  128 bits and saturates (ledger 213).
- `ChatPacket` sends as many bytes as `vsnprintf` says the whole line needs, so a line past
  `CHAT_MAX_LEN` reads past its 513-byte buffer (`G/char.cpp:5160-5183`). The Rewrite sends the
  512 bytes written (ledger 221).
- A translated format with more conversions than its caller passes makes `vsnprintf` read past
  the arguments. The Rewrite writes such a conversion as it is (ledger 221).
- `locale_find` sends a language past the last to English only when it is above
  `LOCALE_MAX_NUM`, so language 12 reads past the table array (`G/locale.cpp:76-79`). The auth
  login refuses 12, and the Rewrite reads English for it (ledger 221).
- Every `CHARACTER` starts with a last shout Pulse of 0 (`G/char.cpp:252`), and the cooldown
  compares it with the Core's Pulse (`G/input_main.cpp:893`), so nobody can shout in the first
  fifteen seconds after a Core starts. The Rewrite starts a character with no last shout
  (ledger 222).
- `SECTREE_MAP::Find` keys a sectree by 16 bits of `x / 6400` and `y / 6400` taken from the
  coordinate as a `DWORD` (`G/sectree_manager.cpp:71-77`), so a point far outside a map, or a
  negative one, can land in one of its sectrees and spawn there. The Rewrite spawns a point only
  inside its map's region (ledger 223).
- `regen_load` reads a word into `szTmp[256]` with no bound, adds and multiplies in a C `int`,
  and spawns an entry with a negative count 2^32 minus that many times (`G/regen.cpp:27-247`,
  `:325-383`). The Rewrite refuses such a file at start-up (ledger 223).
- A mob flag named twice in one `mob_proto.txt` column is summed twice, which carries into the
  next flag's bit (`D/ProtoReader.cpp:680-753`). The Rewrite sets the bit once (ledger 223).
- `CHARACTER_MANAGER::Find` finds a VID on any map, and `DISTANCE_APPROX` ignores the map, so a
  modified client naming a keeper on another map at the same coordinates opens its shop. The
  Rewrite looks only on the character's own Channel and map (ledger 224).
- `CShopManager::Sell` removes the item before `ChangeGold` refuses gold that would reach
  `GOLD_MAX_MAX`, so the item is lost unpaid. The Rewrite refuses the sale first (ledger 224).
- The sale tax is a `DWORD` (`G/shop_manager.cpp:544-552`), so a sale worth more than about 143
  billion pays too much, and `SetShopItems` multiplies a price in 32 bits
  (`G/shop.cpp:176-188`). The Rewrite computes both in 64 bits (ledger 224).
- `SetShopItems` answers an item it cannot place with `continue` and never advances, so every
  later item of the shop is lost. It also writes an item of size 0 over slot 0, and an item
  whose cell is past slot 39 past the end of its slot vector. The Rewrite leaves out only the
  item and logs it (ledger 224).
- `CExchange::Done` skips an item it finds no cell for (`continue`, `G/exchange.cpp:534-539`),
  so the trade completes and the giver keeps the item. `CheckSpace` fills the receiver's cells
  as they are before either side's items leave, so a first-fit placement can come out
  differently in `Done`. The Rewrite places every item on copies of both characters, and an
  item with no cell refuses the whole trade (ledger 225).
- `ChangeGold` refuses gold that would reach `GOLD_MAX_MAX` after the giver's gold has been
  taken (`G/char.cpp:3823-3850`), so a trade loses the gold. The Rewrite checks both receivers
  first and refuses the trade (ledger 225).
- The same `CHARACTER_MANAGER::Find` lets a modified client start a trade with a player on
  another map at the same coordinates, and `CInputMain::Exchange` refuses a dead `arg1` for
  every subheader, although only `START` names a character. The Rewrite looks only on the
  character's own Channel and map, and only for `START` (ledger 225).
- `MoveItem` tests only the source's `IsExchanging` before a merge (`G/char_item.cpp:7786`), so
  a player can grow a stack the other side has already seen offered. The Rewrite refuses a
  merge into an offered stack silently, as the use paths do (ledger 225).
- `CSafebox::MoveItem` takes the item out of the safebox before it subtracts the merged count
  (`G/safebox.cpp:216-220`), so a partial merge loses the part that did not fit and a merge onto a
  full stack loses the whole source; its count is a `BYTE`, although the client sends a `WORD`.
  The Rewrite destroys the source only when every item moved, leaves the rest in its cell, changes
  nothing when none fits, and counts the whole `WORD` (ledger 226).
- `CSafebox::Add` ignores the answer of `Put` (`G/safebox.cpp:69`), so a loaded row whose cells
  are taken or run past the last row shares a cell with another item. The Rewrite skips the row
  and logs it (ledger 226).
- `CSafebox::Add` never writes the highlight of the set record, so the client reads whatever the
  stack held. The Rewrite sends 0 (ledger 226).
- `CInputMain::Exchange`'s lookup of `arg1` for every subheader also tells the trade-wait line to
  whatever character an `arg1` of another subheader names, an amount of gold or a cell that
  happens to be a VID, when that character's safebox loaded or closed within 10 seconds
  (`G/input_main.cpp:1377-1383`). The Rewrite checks the wait of the player a `START` asks alone
  (ledger 226).
- `SendScript` sends a dialog longer than a `WORD` with sizes that have wrapped around
  (`G/questmanager.cpp:1113-1151`), so the client reads a cut record and then garbage. The
  Rewrite logs a dialog longer than 65529 bytes and does not send it (ledger 227).
- A script whose first result is not text reaches `strcmp(NULL)`, and a `select` whose last result
  is not a table reaches an unprotected `luaL_getn` (`G/questlua.cpp:970-1056`). The Rewrite ends
  the script as a Lua error (ledger 227).
- `getnpcid` with no name builds a `std::string` from `NULL`. The Rewrite answers 0 (ledger 227).
- `SendQuestInfoPakcet` copies `packet_quest_info` into its buffer before it adds each part's
  length to `qi.size`, so every `GC_QUEST_INFO` says 8 bytes however many parts follow, and it
  writes each name as `N` bytes from `c_str()`, which sends whatever the string's buffer held
  past the terminating zero, and no zero at all for a name of `N` bytes or more
  (`G/questpc.cpp:231-308`). The Rewrite writes the whole record's size, zero-fills each name
  field and keeps `N - 1` bytes of a name (`protocol::gc_quest_info`, ledger 258).
- `ReadQuestCategoryToDict` reads `data[0]` and `data[1]` of every non-empty line of
  `questcategory.txt` without checking that the line has two words, so a line of blanks or of
  one word indexes past its vector (`G/questmanager.cpp:320-365`). The Rewrite warns and skips
  the line (ledger 258). The owner's file is empty.
- `CTargetManager::CreateTarget` compares the stored name, cut to 32 bytes, with the whole name
  the script passed (`strcmp`, `G/target.cpp:134-238`), so a mark named with 33 bytes or more
  never matches its own earlier mark: each call adds another mark and sends another
  `GC_TARGET_CREATE`. The Rewrite compares both names cut to 32 bytes (ledger 259).
- `SendTargetCreatePacket` builds `TPacketGCTargetCreate` on the stack and `strlcpy`s the name
  into it (`G/target.cpp:15-28`), so the bytes of `szName` past the terminator are whatever the
  stack held. The Rewrite zero-fills them (ledger 259).
- A click on a marked character while the clicker's script waits for an answer runs the mark's
  `target.click` script through `ExecuteQuestScript`, whose `PC::SetQuest` puts the new state in
  place of the waiting one (`G/questnpc.cpp:206-264`). The waiting coroutine is never closed and
  its dialog's answer reaches the wrong script. The Rewrite takes the click and runs nothing
  (ledger 260).
- D1. `StateMove` with a zero duration on its start millisecond divides 0 by 0, and `(int)NaN` is
  `INT_MIN`, so `Sync` logs a hack and keeps the spot (`G/char_state.cpp:784-793`). The Rewrite
  skips the Pulse's move and arrives on the next (ledger 229b).
- D2. `EncodeInsertPacket` sends a MOVE whenever `iDur` is not 0, so an arrival the Pulse has not
  applied yet sends a duration near 2^32 (`G/char.cpp:1097-1108`, `:1211-1223`). The Rewrite sends
  MOVE and its `GC_WALK_MODE` only when `iDur > 0` (ledger 229b).
- D4. `int dwViewRange(VIEW_RANGE + VIEW_BONUS_RANGE)` overflows for a `VIEW_RANGE` near `INT_MAX`
  (`G/entity_view.cpp:85-95`). The Rewrite adds in 64 bits (ledger 229b).
- `CInputMain::Target` finds the VID with `CHARACTER_MANAGER::Find`, which searches every map, so a
  modified client can target a character on another map of the Core and follow its hit points
  (`G/input_main.cpp:2268`). The Rewrite finds a player or an NPC on the clicker's own Channel and
  map, as its click, shop and trade do (ledger 229g).
- `ClearTarget` and `BroadcastTargetPacket` never set `bElement`, so the byte is whatever the stack
  held (`G/char.cpp:6419-6456`, `:6600-6661`). The Rewrite sends 0 (ledger 229g).
- `SetTarget` and `BroadcastTargetPacket` multiply the hit points by 100 in an `int`, which wraps past
  21,474,836 hit points (`G/char.cpp:6493`, `:6555`, `:6624`). The Rewrite multiplies in 64 bits; the
  owner's largest maximum is 12,958,000 (ledger 229g).
- `ComputePoints` reads a monster's `DWORD dwMaxHP` into an `int`, so a maximum past `INT_MAX` is
  negative (`G/char.cpp:3017`). The Rewrite holds it at `i32::MAX`; none of the owner's is above
  12,958,000 (ledger 229g).
- `CInputMain::Attack` finds the victim with `CHARACTER_MANAGER::Find`, which searches every map, and
  `battle_melee_attack` measures the reach by x and y alone (`G/input_main.cpp:1975`,
  `G/battle.cpp:172-210`), so a modified client can hit a monster at the same coordinates on another
  map of the Core. The Rewrite finds the victim on the attacker's own Channel and map (ledger 229j).
- `Damage` adds every victim a player hits to the world boss's per-VID damage map
  (`SetAccumulateDamageByVID`, `G/char_battle.cpp:2398-2405`, `G/char.cpp:11602-11613`), which grows
  without bound until the player leaves. The Rewrite keeps no such map; the world boss is not ported
  (ledger 229j).
- The non-player `recovery_event` multiplies the maximum hit points by the regen percentage in an
  `int` (`G/char.cpp:3191-3240`), which wraps once the product passes `INT_MAX`. The Rewrite
  multiplies in 64 bits (ledger 229j).
- `Sync` kills a non-player whose step lands where no sectree stands (`Dead()`,
  `G/char.cpp:3403-3407`). The Rewrite leaves it where it stood (ledger 229i). An idle walk cannot
  reach this, as its point and its half-way point are movable and the sectrees form one rectangle.
- B5, a backdoor. `Disconnect` saves a dead player with its hit points at 0 or below, and the
  relog's @fixme118 refill stands it up where it died with full hit points, no penalty and no wait
  (`G/char.cpp:1779-1783`, `:4250`; `G/char_affect.cpp:660-665`). The Rewrite pays the here
  penalty, stands the player and stores 50 hit points, so the relog equals `/restart_here`
  (ledger 229k).
- B25, a backdoor. `Position` has no dead guard, so a dead player that sits leaves `POS_DEAD` and
  then stands up revived with no penalty and no wait (`G/input_main.cpp:1530-1548`;
  `G/char.cpp:3335-3349`, `:1438-1444`; `G/char_battle.cpp:550-556`). The Rewrite refuses the
  pose while the player is dead (ledger 229k).
- B8. `restart_now` stands before `restart_here` in the command table, so the prefix walk gives it
  every word from `/rest` to `/restart_n`, and a typo pays 300000 gold (`G/cmd.cpp:298`, `:656`).
  The Rewrite matches `restart_now` by its whole name only (ledger 229k).
- B1. `restart_now`'s `IsHack` refusal says the player paid 300000 gold and charges nothing
  (`G/cmd_general.cpp:500`). The Rewrite sends `IsHack`'s own line only (ledger 229k).
- B3. `restart_now` tests the gold before the dead event (`G/cmd_general.cpp:480`, `:486`). The
  Rewrite tests the dead event first (ledger 229k).
- B24. `do_restart`'s `IsHack` refusal is followed by `[LS;516;%d]` with a wait unrelated to its
  cause, 0 or below after 170 seconds (`G/cmd_general.cpp:548`, `:567`). The Rewrite sends
  `IsHack`'s line only (ledger 229k).
- B20. `restart_now` tests an always-true inner condition beside `IsHack`
  (`G/cmd_general.cpp:497-498`). The Rewrite drops it (ledger 229k).
- B11 and D2, provisional (owner question 32). `DeathPenalty` rolls and sends `[LS;901]` before it
  tests the flag, a lucky roll leaves the flag set for a later death, and a town restart consumes
  the roll and sends a zero exp record (`G/char_battle.cpp:347-363`, `:378-390`). The Rewrite
  tests the flag first and clears it on every path, and a town restart only clears it (V18k,
  ledger 229k).
- B12 and B13. A town restart or an automatic restart whose recall Warp fails leaves the player
  standing where it died, unseen by its view and with no revive invisibility, because
  `WarpSet`'s result is ignored (`G/cmd_general.cpp:687-690`, `G/char_battle.cpp:510-516`). The
  Rewrite restarts it in place when no Channel serves the recall point (ledger 229k).
- B15 and D3, provisional (owner question 29). The automatic restart warps to town but takes the
  here penalty, and spends affect 511, which the developers' own `NO_DEATH_PENALTY_BUG_FIX` spares
  a town restart (`G/char_battle.cpp:365-376`, `:518-520`). The Rewrite takes the town penalty
  (V18j, ledger 229k).
- D5. A forked kill neither sets nor clears the penalty flag (`G/char_battle.cpp:1374-1381`). The
  Rewrite always assigns it (ledger 229k).
- D6. Standing keeps `m_dwKillerPID` (`G/char.cpp:1438-1444`). The Rewrite clears it when the
  player stands (ledger 229k).
- D7. `UseItem` tests only `IsStun`, so a dead player's potion is used up and wasted
  (`G/char_item.cpp:7227`). The Rewrite refuses any use while the player is stunned or dead
  (ledger 229k).
- D9. A stunned player at 0 hit points can walk for three seconds (`G/char.cpp:3362-3381`). The
  Rewrite refuses its move silently, as `CanMove` refuses a move (ledger 229k).
- D10. A dead player can pick up, trade and click NPCs; `G/input_main.cpp:1385` tests only the
  trade's target. The Rewrite refuses them while the player is dead (ledger 229k).
- B26, provisional. `SetVictim(NULL)` stands a stunned monster up: `battle_end` sets
  `POS_STANDING` on it, so it never dies of its stun (`G/char_battle.cpp:3624-3642`;
  `G/battle.cpp:231-240`; `G/party.cpp:892`). Reachable through `ForgetMyAttacker` and
  `PM_RETURN`. `forget_my_attacker` skips a stunned or dead monster; `PM_RETURN` gets the same
  rule in the group work (ledger 229l).
- B27, provisional. The element switch on a bitmask works against 16 of 470 element monsters
  (`G/battle.cpp:417-456`). The Rewrite runs the `#else` chain's first matching flag (ELEC,
  FIRE, ICE, WIND, EARTH, DARK), each with its `RESIST_*` and `RESIST_*_PCT` point, in the ELEC
  case's rounding (ledger 229l).
- B28. A monster's hit can heal: the victim's `NORMAL_HIT_DEFEND_BONUS` plus `RESIST_COMBAT`
  above 100 at 90 % hit points, or `MALL_DEFBONUS` above 100
  (`G/char_battle.cpp:2124-2138`, `:2181-2185`, `:2375-2387`). The Rewrite holds the damage at 0
  after the bonus steps, before the down check (ledger 229l).
- B29. `KILL_AND_GO` turns the monster toward the absolute target point measured from the map
  origin, so its `FUNC_WAIT` carries a meaningless heading (`G/char_state.cpp:1000-1022`). The
  Rewrite turns toward the target from the monster, with the truncated `(long)` point
  (ledger 229l).
- B30, provisional. A monster's empire tag spares the other empire: the proto text puts
  `NOATTCHUNJO` at bit 4 and `NOATTJINNO` at bit 5, but the enum and the accessors read bit 4
  as Jinno and bit 5 as Chunjo, so each tag spares the wrong empire (`common/length.h:691-693`;
  `server/server/db/ProtoReader.cpp:676`; `G/char_state.cpp:331-349`; `G/trigger.cpp:114-131`).
  It reaches 12 protos, none spawned on Channel 1 or 99, so it is dormant. The Rewrite takes
  the proto text's meaning: bit 4 spares empire 2, bit 5 empire 3 (ledger 229l).
- The quest item is a raw pointer (`m_pQuestItem`, `G/char.cpp:7620-7633`) that `UseItem` sets
  and nothing clears when the item is destroyed, so an `item.get_vnum` after the item is used
  up, sold or removed reads freed memory (`G/questlua_item.cpp:138-149`). The Rewrite keeps the
  item's ID and answers 0 once the character no longer holds it (ledger 264).
- `do_goto`'s named map tests the wrong flag: `if (!bFind)` prints `There is no map named ...
  Cannot find map %s` whenever the name IS found (`G/cmd_gm.cpp:357-361`), so every honest
  `/goto <name>` reads a failure line for a warp that landed. The Rewrite warps and stays
  silent (ledger 312).
- `xmas::ProcessEventFlag`'s `xmas_santa` 1 arm tests the wrong way: its comment says it
  makes a Santa when there is none, but it calls `SpawnMobRandomPosition` only when
  `GetCharactersByRaceNum` finds one already standing (`G/xmas_event.cpp:75-85`), so the
  first Santa never comes. The Rewrite stands one when none stands (ledger 365).
- `CItem::AddSocket` writes `m_alSockets` with neither `UpdatePacket` nor `Save`
  (`G/item.cpp:1786-1793`), so `/add_socket`'s socket is invisible to the GM's client and is
  lost at logout unless an unrelated later save carries the item. The Rewrite sends
  `GC_ITEM_UPDATE` and stores the row, as `SetSocket` does (ledger 367).
- `/set_socket` hands any `int` to `CItem::SetSocket`, whose `assert` is the only bound
  before `m_alSockets[i]` is written (`G/cmd_gm.cpp:4698-4713`, `G/item.cpp:1643-1648`), so an
  index past 5 or below 0 writes outside the item. The Rewrite writes nothing (ledger 367).
- `CItem::AddRareAttribute` indexes `bMaxLevelBySet` with `GetAttributeSetIndex()`, which is
  -1 for a rod or a pick in the weapon slot, and picks `avail[number(0, avail.size() - 1)]`
  even when no row is left, which indexes out of the vector (`G/item_attribute.cpp:422-464`).
  The Rewrite adds nothing in both cases (ledger 369).
- The `POINT_CONQUEROR_EXP` arm of `PointChange` answers a loss of the whole champion
  experience with `GetExp()`, the plain experience, as the record's value
  (`G/char.cpp:3908-3915`), and prints a smaller loss's `int` amount with `%lu`, so `/set`
  shows a wrapped number. The Rewrite writes the 0 it stored and the signed amount
  (ledger 371).
- `SortInventoryItems` gives a plain stackable back through `AutoGiveItem(vnum, count)`,
  whose top-up raises a `MAKECOUNT` stack to the proto's `value1`, and which destroys the
  lifted item and makes a fresh one with the proto's sockets; an item whose vnum has no proto
  is lifted and never given back (`G/char.cpp:10790-10866`). The Rewrite gives back the count
  it lifted, gives an item with its own sockets or attributes back whole, and gives a
  proto-less item back whole (ledger 372).
- `AutoGiveItemNew`, when the stacks take part of a drop and no cell takes the rest, names
  the whole count in its `Ai primit:` line (`G/char_item.cpp:10815-10819`). The Rewrite names
  what the stacks took (ledger 373).
- `SaveAttributeToImage`, for a sectree the attribute file does not cover, paints whatever
  its line buffer still holds from the last covered sectree of the row — or, on a row whose
  first sectree is uncovered, whatever the fresh buffer held — because the `!pSec` arm only
  skips the `CopyRow` (`G/sectree_manager.cpp:1412-1440`). A map whose file is narrower than
  its sectrees therefore smears its last covered column right, and the paint depends on the
  heap. The Rewrite paints an uncovered sectree black (ledger 378).
- The 10-second grace for a missing client version never fires: the arm tests
  `!GetClientVersion()`, and a `std::string`'s `c_str()` is never null
  (`G/input_login.cpp:744-771`). Every versionless client is answered with the conflict notice
  and closed, as a wrong version is (ledger 380).
- `RequestEventQuest` builds its `std::string` over the whole 65-byte field, reading past
  its end when no NUL is stored. The Rewrite stops at the first NUL or the field's end
  (ledger 382).
- `WhisperDetails` reads the asked name as a C string that may run past its 25-byte field,
  and answers with a stack record whose 25th name byte it never sets, since `strncpy` copies
  at most 24 (`G/input_main.cpp:3603-3638`). The Rewrite reads the name within the field and
  writes 0 there. Its `P2P_MANAGER::Find` arm answers the language another core's character
  logged in with, because `ChangeLanguage` never tells the other cores; the Rewrite answers
  the current one (ledger 383).
- `TargetInfoLoad` makes a real item for each entry of the drop list, each with a new item
  ID and, for a timed item, a running expiry event, and never destroys them, so every click
  on a target leaks (`G/item_manager.cpp:915-1095`). A row whose percent is 0 sends the
  item's `dwRarity`, which `CreateItem` sets only for a nonzero percent and the constructor
  never initialises. The Rewrite makes no item and sends 0 (ledger 384).
- A gift to a smith passes the item's cell to `RefineInformation` as an inventory cell
  whatever its window (`G/char_item.cpp:9452-9481`), and `RefineInformation` accepts the
  custom bank cells (290 and up) but writes the cell into the window's one-byte `pos`
  (`:1865-1893`), so the client's refine names a different cell.
  `DoRefine` places the refined item with `AddToCharacter` at the old cell without checking
  that it fits, so a taller result overlaps the cells below. The Rewrite opens no window for
  such a cell and refuses a result that would not fit before anything changes (ledger 385).
- `CHARACTER::RefineInformation` stores the scroll cell in `m_iRefineAdditionalCell`, which
  no constructor or `Initialize` sets and `ClearRefineMode` never resets
  (`G/char_item.cpp:9563-9573`), so a scroll refine can read a cell left from an earlier
  window or an indeterminate one. The Rewrite starts with no scroll cell, keeps the last one
  as legacy keeps it, and forgets it when the character leaves (ledger 386).
- `SendEventInfo` counts its record into a `WORD wSize`, so more than 2978 events truncate the
  size and the client misframes (`G/event_manager.cpp:613-646`). The Rewrite refuses a calendar
  that does not fit (ledger 387).
- `SendEventInfo` compares the start's month but not its year, so an event of the same month in
  another year is listed, and a December one is never a "previous month" in January (`localtime`'s
  `tm_mon`). The Rewrite lists the same months, in UTC rather than the server's zone (ledger 387).
- `confirm_timeout_event` is never cancelled when the confirm is answered
  (`G/questlua.cpp:881-904`), so a script that confirms again within the first timeout hears
  the stale timer's `CONFIRM_TIMEOUT` early. The Rewrite disarms the timer when an answer is
  heard and a new confirm replaces it (ledger 388).
- `CHARACTER::OreRefine` takes the 100 ore before it checks the gold (`G/mining.cpp:443-451`),
  so a purse short of `ComputeRefineFee(cost)` loses the ore and is refused; the script checks
  only `GetOreRefineCost`, which is 10% lower. The Rewrite checks the gold before it spends
  anything (ledger 388).
- `GC_FISHING` leaves `dir` uninitialised on every sub-header but `START` (`G/fishing.cpp:403-428`,
  `498-503`, `651-655`, `733-737`), so the client hears a stack byte. The Rewrite sends 0
  (ledger 389).
- `CHARACTER::fishing` dereferences the sectree under the fisher without checking it exists
  (`G/char.cpp:5263-5267`), so a cast outside every sectree crashes the core. The Rewrite reads
  the map's attributes and refuses nothing extra (ledger 389).
- `fishing::DetermineFish` takes `lower_bound` over the cumulative odds without checking the
  end (`G/fishing.cpp:389-390`), so a roll past the last entry reads `fish_info[MAX_FISH]`. The
  Rewrite treats it as the empty catch: `FISH` names vnum 0 and nothing lands (ledger 389).
- `pc_mount_bonus` indexes `aApplyInfo` with the script's apply byte unchecked
  (`G/questlua_pc.cpp:1475`), so a type at or above `MAX_APPLY_NUM` (130) reads past the table and
  lays the bonus on whatever point byte it finds. The Rewrite refuses such a bonus (ledger 399).
- `d.new_jump` dereferences the current character without checking it
  (`G/questlua_dungeon.cpp:342-345`), so a reader-less call (a server timer's step) crashes the
  core. The Rewrite lets the create stand and jumps nobody (ledger 404).
- `CHARACTER::ChangeEmpire` keys its account update on the old empire and the account's pids
  and its count insert swaps the columns (`VALUES(account_id, count)` into
  `(change_count, account_id)`, `G/char_change_empire.cpp:105-185`); a second account at the
  same pids or empire would be moved and counted wrongly. The Rewrite keys on the account and
  counts up (ledger 405).
- The guild-war bet quest formats `_80_say`'s three verbs (`%s`, `%s`, `%d`) with one argument
  (`guild_war_bet.lua:39`), so `string.format` errors (`luaL_argerror`, 5.1's `lstrlib.c:772-773`;
  5.0.3 errors the same way) and the dialog ends after the title: no bet on any reservation can
  complete through the owner quest, on legacy either. The Rewrite runs the source as-is
  (ADR-0004) and errors at the same pick (ledger 408).
- `CHARACTER::CardsRestore` and `RestoreField` write the restored card to
  `cards_in_hand[GetEmptySpaceInHand()]` without checking for -1
  (`G/char_cards.cpp:193-205`, `:301-312`), so a restore with a full hand writes past the
  hand. The Rewrite runs nothing when no hand slot is free (ledger 411).
- `FishEventGeneralInfo`'s shape guard compares the held shape to the special box's vnum
  (`lastAttachedShape > ITEM_FISH_EVENT_BOX_SPECIAL`, `G/char.cpp:11086`), so every byte
  passes it. The Rewrite compares to shape 7, the highest the tables name (ledger 440).
- `CollectDailyGift` indexes the week's gift vector with the client's slot byte
  unchecked (`CDailyGiftItemGroup::GetVnum(bSlotIndex)`, `G/item_manager.h:46-58`), so a slot
  at or above 7 reads past the week's seven gifts. The Rewrite is silent on such a slot
  (ledger 441).
- `CleanSashAttr` refuses the target only when its type is not `ITEM_COSTUME` *and* its
  subtype is not `COSTUME_SASH` (`G/char.cpp:10486`), so every costume that is no sash passes
  the gate. The Rewrite keeps the `&&` (ledger 442); only the missing absorbed socket refuses
  such a target.
- The biologist's selective arm reads `byIndex` uninitialized for a choice past 2
  (`G/Biologist.cpp:237-245`), and hands whatever byte to `arr_bonus[state][byIndex]`.
  The Rewrite ends a choice past 2 silently (ledger 445).
- The biologist's time arm indexes the two flowers unchecked (`G/Biologist.cpp:299-331`),
  so an index past 1 reads past the table. The Rewrite ends an index past 1 silently
  (ledger 445).
- The biologist's open `memcpy`s 64 bytes from a title shorter than 64
  (`G/Biologist.cpp:88`), reading past the literal. The Rewrite pads the title with
  zeroes (ledger 445).
- `Cube_load` dereferences the open recipe for every token outside a `section`
  (`G/cuberenewal.cpp:193-229`), so a token before the first `section` or after an
  `end` writes through a stale or null pointer. The Rewrite skips such a token
  (ledger 448).
- `SendDateCubeRenewalPackets` never writes the `npc_vnum` or `index` words
  (`G/cuberenewal.cpp:532-560`), so every date on the wire carries 0 for both. The
  Rewrite sends the same zeroes (ledger 448).
- The biologist's close record leaves `dwLevelLimit` uninitialized
  (`SendBiologistClosePacket`, `G/Biologist.cpp:334-352`, sets every field but it).
  The Rewrite sends 0 (ledger 445).
- Mission 6 names `POINT_RESIST_*` (59-62) where `aApplyInfo` wants apply types, so its
  affect lands on `POINT_ATTBONUS_*` (54-57) instead (`constants.cpp:784-787`). The
  Rewrite keeps the landed points: the honest client shows what the server did
  (ledger 445).
- The biologist's `DropObjects` has no caller and its state guard can never fire
  (`state < 0 && state > 9` on a `DWORD`). The Rewrite ports no drop (ledger 445).
- The password change checks the digits of the two first bytes, not the two passwords
  (`has_only_digits(strFindFirstForNow)` at `G/char.cpp:11941`, where the activate passes
  the whole password at `:11878`). The Rewrite checks the wholes (ledger 446).
- The premium activation creates the voucher item to read its name and leaks it
   (`ITEM_MANAGER::instance().CreateItem(itemVnum)` at `G/char.cpp:11542` with no
   destroy). The Rewrite reads the proto's locale name instead (ledger 447).
- `/temp`'s send guard takes the empire as a `BYTE` but passes the full `int` on
  (`G/monarch.cpp:118-132`), and the DB server indexes `m_MonarchInfo.money` with it
  unchecked (`D/Monarch.cpp:124`). The Rewrite refuses an empire past 3 (ledger 459).
- `do_monarch_warpto` stamps `MI_WARP` again when its 60-second `FuncCheckWarp` fires
  (`G/cmd_general.cpp:672-682`), so an offline target re-arms the cooldown, but no
  client observes the second stamp. The Rewrite stamps once (ledger 460).
- `GetEventSungZiMapInfo` and `GetEventPassMapInfo` index their vectors with the
  `threeway_war_*_idx` flags unchecked (`G/threeway_war.cpp:201-213`), so a negative or
  too-large flag reads past the list. The Rewrite reads map 0 there (ledger 465).
- `duel_time_out` sends its empty duel-start record twice to player A and never to B
  (`G/arena.cpp:447-448`), so B's client keeps its opponent list. The Rewrite clears both,
  once each (ledger 467).
- `do_duel` and `CArena::StartDuel` accept one character as both duelists and a character
  already in another arena (`G/cmd_gm.cpp:4013-4052`, `G/arena.cpp:466-495`), so two arenas
  claim one player. The Rewrite refuses both with the start-failure line (ledger 467).
- `GiveItemFromSpecialItemGroup` returns an uninitialised `bSuccess` when the draw is
  empty (`G/char_item.cpp:9586-9589`, `:9689`). The Rewrite's world-boss reward refuses
  an empty group (ledger 472).
- `BattlePassAction`'s tier loop indexes `rewardsFree[i]` and `rewardsPremium[i]`
  with the crossed levels unchecked (`G/char.cpp:12893-12899`), so a level past the
  lists reads past them. The Rewrite grants only the tiers the table holds (ledger 473).
- `dungeon_set_fastest` guards argument 1 twice instead of argument 2 and passes
  `(int)lua_isnumber(L, 2)` — a 0/1 boolean — as the fastest time
  (`G/questlua_dungeon.cpp:1755-1771`). No quest calls it, so the function is `unused`
  (ledger 401) and the Rewrite ports nothing of it (ledger 512).
- `GetBuffLevel` and `SetBuffLevel` chat `TEST:` lines with every read and write
  (`G/ShamanSystem.cpp:341-348`, `:351-356`), and `USE_UNLOCK_SHAMAN` chats the success
  line while its training arm is commented out (`G/char_item.cpp:6944-6964`), so the
  premium flag is never set by any ported path. The Rewrite keeps the line (it is the
  arm's whole behaviour) but not the TEST chats (ledger 520).
- The guild re-entry tool reads `u_disbandTime`, which nothing assigns (the line above
  assigns `u_withdrawTime` twice, `guild_manage.lua:17-18`), so the `nil > 0` errors and
  the retry math, `get_time` and `is_test_server` never run — in Lua 5.0
  (`luaV_lessthan` -> `luaG_ordererror`, `server/server/liblua/lvm.c:258-268`) and 5.1
  alike. The Rewrite runs the script to the same error end (`GotoEndState`, ledger 525).
- `CHARACTER::CHARACTER` never initializes `m_dwWalkStartTime` (`G/char.cpp:280-312` sets
  every neighbour but it), while `StateMove`'s walk arm reads it whenever a body walks
  (`G/char_state.cpp:813-820`). A body walking on an empty stamina pool — `IsWalking` holds
  through `GetStamina() <= 0` without the mode line ever flipping — restores its pool from a
  garbage clock on its first steps. The Rewrite stamps the first walking step instead
  (`Body::walk_start_ms`, ledger 569).


## Environment

- PostgreSQL 18 runs in Podman (`postgres:18`, PostgreSQL 18.6), pulled once in section 179 under
  the owner's approval (planning Q23). On the current machine `host.docker.internal` does not
  resolve, so the container publishes its port on `127.0.0.1`; `AGENTS.md` has the commands.
- The one online `cargo fetch` the owner approved (planning Q23) ran in section 177. The offline
  gates build from the local cache.
- The toolchain is Rust 1.100.0 since ledger 684, installed by rustup from `rust-toolchain.toml`
  with rustfmt 1.11.0 and Clippy 0.1.100. The distribution's 1.85.1 in `/usr/bin` is still on the
  machine and is not used; the rustup `stable` toolchain (1.98.1) on it is a damaged install.
- `i686-linux-gnu-g++-12` 12.4.0, used by the width probe, is installed on the current machine
  since ledger 680 (`apt install g++-12-i686-linux-gnu`, with the owner's yes).
- `docker.io/library/rust:1.100.0-bookworm` (ledger 684) and `debian:bookworm-slim` (ledger 680)
  were pulled for the `compose.yaml` image build, which fetches the locked crates inside its
  build stage.
- The **database gate runs on the current machine** as of section 187: `postgres:18` (18.6) is
  already pulled, so a Podman container on `127.0.0.1:55432` and an exported `DATABASE_URL` are
  enough to exercise every store-backed test. Section 187 ran it and left no scratch database
  behind. `DATABASE_URL` is unset by default, so every gate stays green without a database.

## Code-quality backlog

Take these on when a step touches the code. The counts were measured again at ledger 683, with
`command grep -a` over the nine crates; the count of ledger 210 is in brackets where it moved.

- **Duplication.** 172 `pub enum ...Error` types (62), 41 `check_exact` functions (42), and 18
  little-endian reader functions by name (37, by a pattern the ledger does not give). `TItemPos`
  is defined twice (`protocol/src/lib.rs:608`, `common/src/tables.rs:393`) beside `ItemPos`
  (`protocol/src/item_pos.rs:143`), and `TSimplePlayer`, `TQuickslot` and `TPlayerSkill` twice
  each. There are 4 phase enums, all in `prodomo`; the duplicate `ServerState` is gone. A shared
  `wire` reader and writer, one `CodecError`, a `FixedRecord` trait, and a `CgPacket` enum for
  dispatch would remove most of it.
- **Stale slot constants in `common/src/constants.rs`** (found in ledger 210, still there).
  `INVENTORY_MAX_NUM` is 90 there, `DRAGON_SOUL_EQUIP_SLOT_START` 154,
  `BELT_INVENTORY_SLOT_START` 184 and `INVENTORY_AND_EQUIP_SLOT_MAX` 200: the values without
  `ENABLE_EXTEND_INVEN_SYSTEM` and `ENABLE_CUSTOM_INVENTORY`. The measured values are in
  `common::item_slots` (180, 244, 274 and 1370), which is what every user imports; nothing reads
  the stale copies. Remove them.
- **`common/src/tables.rs`** declares 72 `repr(C, packed)` structs. Do not size wire records from
  them.
- **Silent data loss:** `bytes_to_str` (`protocol/src/lib.rs:647`) returns `""` on invalid UTF-8.
  Legacy names are raw bytes, so this hides data rather than rejecting it.
- **Panic points in non-test code.** `common/src/logging.rs:64` panics if the log directory cannot be
  created. The rest are invariant panics that cannot fire today: 8 `.expect` calls in
  `protocol/src/cg_account.rs`, and one each at `protocol/src/cg_attack.rs:134` and
  `prodomo/src/sync_position.rs:303`.
- **`#[allow]` is banned** (`AGENTS.md`: "Fix Clippy findings by refactoring, never by
  `#[allow]`"), and **132 of them are checked in** (133 until ledger 684 removed a doubled one);
  there were 59 at ledger 210, so 74 were added after the rule. By lint: 38
  `clippy::too_many_arguments`, 27 `clippy::cast_possible_truncation`, 14 `dead_code`, 12
  `clippy::too_many_lines`. By file: 14 each in `protocol/src/gc_actors.rs` and
  `gamedata/src/records.rs`, 12 in `gamedata/src/renewal_shop.rs`, 10 in
  `prodomo/src/game_state/quests.rs`, 9 in `world/src/character/drop_item.rs`. Most of the
  argument counts take a wire struct field by field, so a builder that takes the struct would
  remove them and the field-by-field constructor duplication of the first bullet at once. A new
  one is a violation: the count in a section's receipt may not rise. Remove the old ones when a
  step already touches the file, not as a sweep of their own.
