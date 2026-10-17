# Questions for the owner

Each question names what waits on it. A Parity row that waits on a question here is blocked on
the owner. The Rewrite keeps legacy's behaviour, or the provisional choice named below, until the
owner answers.

## Standing decisions

- **2026-10-14, in chat.** An in-memory (world or unit) test is enough to prove a Parity row
  `ported` when a binary scenario is impractical (the ore vein's 7 to 15 minute life). Parity with
  the original matters, but the goal is a ported server to build on, so the work is done smartly
  rather than literally. Bugs and backdoors found in the original are fixed directly and recorded
  as Defects, not carried over. A question below that only asks whether to keep a legacy Defect
  can be closed under this decision when its section is next touched.

## Open

1. **ADR-0006 is proposed.** `docs/adr/0006-quest-dialect-and-host.md` (the quest dialect and the
   host, ledger 227) waits for the owner's acceptance. `AGENTS.md` lists it only once the owner
   accepts it. Waits on it: nothing blocks today; the quest runtime runs as the ADR proposes.

2. **`_basic/change_empire.lua` does not compile.** The `end` on line 132 closes the `if` before
   `elseif ret == 4` on line 133, so the script is a syntax error in legacy and in the Rewrite.
   The owner either fixes the source, or the script stays unloaded as it is today (a test pins
   that it is the only one). Waits on it: the `quests` row for `change_empire`.

3. **The drop limit.** Only `questlib_extra.lua`, which is never loaded, sets
   `item_drop_limit_time`. Until the owner picks a value, the Rewrite keeps 1000 ms (a STATUS
   Divergence row, ledgers 217 and 227). Waits on it: the drop limit's value.

4. **Two libraries are never loaded:** `questlib_extra.lua` and `questing.lua`, in legacy and in
   the Rewrite (rows `questlib.questlib_extra`, `questlib.questing`). The owner decides whether
   either should be loaded. Waits on it: those two rows, and question 3.

5. **Earlier findings.** `.scratch/parity/spec.md` ("Findings for the owner") lists:
   - the adminpage that legacy never reaches (row `sys.adminpage`);
   - `BlueDragon.lua` and `monkey_dungeon.lua`, which are never loaded, and the data files with
     no reader (`MapProperty.txt`, `charset.txt`, `pet_skill_names.txt`);
   - `HEADER_CG_STATE_CHECKER` served before auth;
   - the `ENVANTER_BLACK` header.

   `docs/STATUS.md` ("Topology") has the `test_server` note: legacy defaults it on, none of the
   owner's `CONFIG` files turns it off, and under it every character is IMPLEMENTOR. The owner
   decides when `test_server` is audited. Under `test_server` the warp NPC's `IsHack` also adds
   `[TestOnly]Pulse %d LoadTime %d PASS %d` to the safebox refusal (`G/char.cpp:8239-8240`); the
   Rewrite leaves that line out until the audit (ledger 228). Under `test_server`
   `GetGMLevel()` is `GM_IMPLEMENTOR` (`G/char.cpp:6354-6358`), so `SetPlayerProto` also gives
   every character `AFF_YMIR` and `PK_MODE_PROTECT` at load (`:2362-2375`;
   `ENABLE_GM_FLAG_IF_TEST_SERVER` at `:2231` compiles that rule in either way). The Rewrite
   keeps every character at `GM_PLAYER`, so only a character below level 15 is in the protect
   mode (ledger 229b). Waits on it: the rows those findings name.

   Addendum (ledger 229k): the restart toggles. Under `test_server` legacy skips
   `restart_here`'s wait and its `IsHack` check, and `restart_now`'s `IsHack`
   (`G/cmd_general.cpp:493`, `:542`), so on the owner's deployment a restart here was instant.
   The Rewrite runs the production arms (V18i, provisional): `/restart_here` is allowed 9.04
   seconds after the death, and a recent trade or storage holds both back. The exp line and
   the `[TestOnly]` line that `test_server` adds are not sent, and `/reset` (B23) stays a GM
   command.

6. **The first 10 seconds after boot (provisional, ledger 228).** Legacy starts every
   character's safebox timer at Pulse 0 (`m_iSafeboxLoadTime = 0`, `G/char.cpp:278`), so for
   the first 250 Pulses of the process `IsHack` refuses every player at a warp NPC with
   `[LS;850;10]` (`G/char.cpp:8234-8241`). The Rewrite records this as a Defect not reproduced: a
   timer that was never set refuses nothing. If the owner calls it a Quirk, the Rewrite adds the
   window back. Waits on it: nothing blocks; the choice stays provisional in STATUS ("Defects
   not reproduced").

7. **Checks for the play test (ledger 228).** Two things only the Reference client can show:
   - It reconnects to the address and port `GC_WARP` names. The Rewrite names the port the
     client is connected to when its Channel hosts the target, and the Shared Channel's first
     port otherwise; legacy names the port of the Core that hosts the map, so the 14 warp NPCs
     of each Channel whose target is on the other Core name the other port (STATUS row
     "`GC_WARP` names the port ...").
   - Whether it sends `CG_WARP`, and on which connection. The Rewrite answers it with nothing
     and keeps the connection (STATUS row "`CG_WARP` in the game phase ...").
   Waits on it: the owner's play test (step 5).

8. **Ten warp NPCs send nowhere (a data finding, ledger 228).** Of the 55 warp NPCs placed in
   the owner's data, 10 name a target that no map in the map index covers, so `WarpSet` refuses
   every player that stands by one, in legacy and in the Rewrite:

   | map | vnum | name numbers | target |
   | --- | --- | --- | --- |
   | 3 `metin2_map_a3` | 10067 | 7752 4477 | (775200, 447700) |
   | 23 `metin2_map_b3` | 10069 | 8520 4477 | (852000, 447700) |
   | 43 `metin2_map_c3` | 10071 | 9288 4477 | (928800, 447700) |
   | 62 `metin2_map_n_flame_01` | 10061 | 4608 10496 | (460800, 1049600) |
   | 63 `metin2_map_n_desert_01` | 10074 | 1352 6532 | (135200, 653200) |
   | 63 `metin2_map_n_desert_01` | 10076 | 1352 7300 | (135200, 730000) |
   | 65 `metin2_map_milgyo` | 10014 | 3326 7468 | (332600, 746800) |
   | 67 `metin2_map_trent` | 10065 | 7754 10553 | (775400, 1055300) |
   | 68 `metin2_map_trent02` | 10066 | 9915 11195 | (991500, 1119500) |
   | 72 `metin2_map_skipia_dungeon_01` | 10078 | 2842 8106 | (284200, 810600) |

   Legacy logs `sys_err` every 12 Pulses while a player stands by one (`G/char.cpp:6703-6707`);
   the Rewrite logs it at debug level (a STATUS Divergence row). The owner either adds the maps
   these name, fixes the names, or leaves them. Waits on it: nothing; no Parity row depends on
   it.

9. **Map 216's sectrees without attributes (provisional, ledger 229a).** `metin2_map_devilcatacomb`
   has `MapSize 8 8`, so legacy builds 32 x 32 sectrees, but its `server_attr` covers 24 x 24.
   The other 448 sectrees (world `x >= 460800` or `y >= 1356800`) keep no attribute, and the
   first read there dereferences NULL (`G/sectree.cpp:202-206`): a random spawn (`/mob`, a regen
   "anywhere" row, the world boss, Santa), a player's save or a PvP check there ends the process.
   About 44% of `SpawnMobRandomPosition`'s draws on that map land there. The Rewrite reads those
   cells as 0 (walkable, no flags) for now. The owner chooses: 0, `ATTR_BLOCK`, or clip the map
   to 24 x 24 (which also shrinks its random spawns). Waits on it: nothing blocks; the choice stays
   provisional in STATUS ("A sectree past its map's `server_attr` header ...").

10. **The partial column and row of maps 12, 13 and 226 (provisional, ledger 229a).**
    `moon_cave`, `demon_dungeon` and `santhia_kouzelny_dul` have a base that is not a multiple of
    6400, so the last column and row they reach are partial sectrees. Legacy's
    `BuildSectreeFromSetting` builds none for them: its two edge trees land one column past the
    partial one and at a row taken from the base's x, a typo (`G/sectree_manager.cpp:245-259`).
    A player cannot step into those strips (`Sync` refuses a position with no sectree), and all
    three `server_attr` files are uniform 0. The Rewrite keeps the strips without a sectree, as
    legacy does, and builds no edge tree (a provisional Divergence; only a forged position
    reaches one). The owner decides whether the strips should be walkable. Waits on it: nothing blocks; the choice stays provisional in STATUS
    ("Only the sectrees inside a map are built").

11. **The game-thread queue (ledger 229b).** Since 229b every move, sync, pose and relayed record
    passes the world's command queue, which keeps its 1024 slots and drains at most 512 commands
    a Pulse (`prodomo/src/game_loop_messages.rs`, `DEFAULT_COMMAND_CAPACITY` and
    `DEFAULT_MAX_COMMANDS_PER_PULSE`). A full queue makes the sender wait; it drops nothing. The
    plan is to raise both if the play test shows the queue filling. Waits on it: nothing blocks;
    the play test (step 5) measures it.

12. **Key order for legacy's hash order (V1, ledger 229b).** Legacy walks a sectree's entities, a
    view and the moving players in pointer-hash order, which no client can rely on. The Rewrite
    walks players by VID, then NPCs by VID. The warp NPCs of a map run in the order they were
    stood up, so when two could both send a player, the one stood up first sends it and the other
    no longer sees that player on the same Pulse (STATUS Divergence "The sectrees around a point
    are walked in legacy's neighbour order ..."). The Rewrite treats this as acceptable. Waits on
    it: nothing blocks.

13. **A character entering where no sectree is (V4, provisional, ledger 229b).** Legacy's `Show`
    returns before placing it and `Sync` refuses every move, so it stays invisible and frozen.
    The Rewrite shows it to itself and places it on its first move into a sectree (STATUS
    Divergence "A player entering at a point no sectree holds ..."). The other choice is to refuse
    it at entry. Only a stored position outside every map reaches this. Waits on it: nothing
    blocks; the choice stays provisional in STATUS.

14. **One move speed until the motions load (V2, ledger 229b).** Every player moves at 300 units
    a second, legacy's speed when a motion file is missing. 229e loads the `.msa` motions, so a
    warrior runs at 450 and the scenario walks shorten (5333 ms to 3555 ms for the same 1600
    units). Waits on it: nothing blocks; 229e replaces it.

    **Resolved in 229e.** The `.msa` motions load. A warrior runs at 450 units a second and walks
    at 221.075 (the walk clip's `Accumulation` y is -176.86 over a `MotionDuration` of 0.8); a
    shaman runs at 450 and walks at 155.74. The long-walk scenario (`LONG_WALK_MS`) runs 1600
    units at the general run clip's 450 a second, so 3555 ms (5333 ms at 300); a walking shaman
    takes 10273 ms and a walking dagger wearer 7237 ms.
    The mount and polymorph arms and the mid-walk recomputes stay open (V2).

15. **A frame after a move waits for the world (V10, ledger 229b).** A descriptor reads a
    client's next frame only once the world has run its move, pose or talking line, which it
    does at its next Pulse, so the records they cause keep the client's order (a shout or an
    answer never overtakes them). A read holding N such frames is therefore applied over up to
    N Pulses, where legacy runs a whole read in one pass (`G/desc.cpp:252-349`); item steps and
    syncs already worked this way. The plan keeps it. Waits on it: nothing blocks; the play
    test (step 5) shows whether a client sends such bursts.

16. **Others hear of a drop before the store writes it (V12, provisional, ledger 229c).** Since
    229c a ground item is an entity of the view. The world sends the other players in view its
    add when a drop lays it, and its removal when a whole pick-up takes it, as it runs the step,
    before the store writes the item rows. A drop never waits. A pick-up of an item a drop laid
    waits for the drop's rows before it writes its own, and its records wait with them, as the
    mover's records have waited for their write since ledger 211. A drop whose write fails closes
    the dropper, and a pick-up waiting on that drop closes with its step unwritten; the others have
    already seen the item fall or go, as the world holds it. Legacy sends everything at once and
    saves later. The monster drops (229f onwards) need the same order, because legacy's drops
    reach the view before the monster's death record. The other choice is to hold the others'
    copies until the write, which a monster drop cannot do. Waits on it: nothing blocks; the order
    stays provisional in STATUS.

17. **A drop where no sectree is (a Defect not reproduced, provisional, ledger 229c).** Legacy
    takes the item, or the dropped part of a stack, from its cell, and then `AddToGround` fails
    and the item is lost with no line (`G/char_item.cpp:7503-7541`). The Rewrite refuses the
    drop after every check legacy makes, so the item stays in its cell
    (`MoveRefused::NoSectree`). Only a character kept outside every sectree (question 13) can
    reach this. The other choices are legacy's loss, or the drop answered with a line. Waits on
    it: nothing blocks; the refusal stays provisional in STATUS.

18. **`LoadAffect`'s @fixme118 records after entry (V9, provisional, ledger 229d).** Legacy runs
    `load_affect_login_event` 1 to 25 Pulses after entry, and `LoadAffect` then refills the
    entrant's HP and SP to their maximums, unless it is dead, with `POINT_CHANGE` records
    (`G/char_affect.cpp:535-554`, `:563-665`, @fixme118 at `:660-665`). The Rewrite sends none of
    them, so an entrant's HP and SP are not refilled at entry. The other choice is to port the
    refill as a one-shot event after entry, with its records. Waits on it: nothing blocks; the
    omission stays provisional in STATUS.

    Addendum (ledger 229k): with no refill, a town restart's 50 hit points stay 50 after the
    Warp, where legacy refilled them at entry, and the player recovery event's hit point heal
    and `DistributeSP` wait with this question (V9, V18b). The disconnect-while-dead backdoor
    this refill made pay is closed on its own (B5).

19. **The stamina wait's clock (V9, provisional, ledger 229d).** Legacy waits three seconds of its
    millisecond clock after a body stops, before the stamina refill. The stop time is set when the
    character is constructed at load and reset at each stop (`G/char.cpp:353`, `:7416`,
    `G/input_main.cpp:1833`, `:1872`; the refill at `G/char_affect.cpp:219`). The Rewrite starts the
    wait at the Pulse the character enters the world, and counts 75 Pulses. The difference is the
    length of the loading phase and the rounding of a Pulse. The other choice is to carry the load's
    clock reading into the world and count 3000 ms from it. Waits on it: nothing blocks; the 75
    Pulses stay provisional in STATUS.

20. **`GD_REMOVE_AFFECT` to the database (V9, provisional, ledger 229d).** Legacy sends
    `GD_REMOVE_AFFECT` to the database whenever an affect is removed or expires, although the
    revive-invisible affect is never saved and the database's delete matches no row
    (`G/char_affect.cpp:95-99`, called at `:439` and `:856`). The Rewrite sends no such write. The
    client cannot see the difference. The other choice is a write that deletes nothing, so the store
    sees the same number of writes as legacy's. Waits on it: nothing blocks; the omission is
    recorded as a Divergence in STATUS.

21. **The alignment in the affect update and the additional-info record (V9, provisional, ledger
    229d).** The Rewrite has no alignment yet, because `sys.char.pk` is missing, so the update and the
    additional-info record send 0. Legacy sends `m_iAlignment / 10` (`G/char.cpp:1314`), which is the character's saved
    `alignment` (`legacy/sql/schema/player.sql:847`). The other choice is to load the saved value
    now and send it divided by 10, before the PK system changes it. Waits on it: nothing blocks; the
    0 stays provisional in STATUS, and `sys.char.pk` is where it changes.

22. **A group-group's probabilities (D1, reproduced provisionally, ledger 229f).** Each member line of
    `group_group.txt` names a group and a probability. `LoadGroupGroup` parses the probability and
    then calls `AddMember` without it (`G/mob_manager.cpp:287-293`), so every member weighs 1 and the
    pick is uniform. The Rewrite reproduces the uniform pick, which is what the owner's players have
    seen. The other choice is to weigh each group by its probability, as the
    `ADD_MOB_GROUP_GROUP_PROB` code was written to. Waits on it: nothing blocks; STATUS lists it as
    reproduced, provisional.

23. **A group entry at a single point (D4, reproduced provisionally, ledger 229f).** `regen_spawn`
    sends any entry whose box is one point to `SpawnMob`, so a `g`, `ga` or `G` entry at a point
    spawns the mob whose vnum is the group's number, if one exists, instead of the group
    (`G/regen.cpp:346-353`). The Rewrite does the same. The other choices are to spawn the group, or
    to refuse the entry at start-up with a warning. Waits on it: nothing blocks.

24. **`AFF_SPAWN` stays set (D5, reproduced provisionally, ledger 229f).** `Show` with the spawn
    motion sets `AFF_SPAWN` and never resets it (`G/char.cpp:1886-1887`, `:1913` resets only the
    state flag), so every boxed stone and spawn-motion monster carries the flag in each later
    insert. The Rewrite sends it the same way. How the Reference client draws a late viewer's
    flagged insert is checked in the play test. The other choice is to reset the flag after the
    first show. Waits on it: nothing blocks.

25. **One dice stream for the whole process (provisional, ledger 229f).** Legacy's boot draws come
    from one `thecore_random` stream per Core process, so each Core's maps drew from their own stream.
    The Rewrite's game thread holds one stream for every Channel (ADR-0002), so Channel 1's boot draws
    change which points Channel 99's monsters stand on. No player can tell a stream from its draws.
    The other choice is one stream per Channel. Waits on it: nothing blocks.

26. **The AI flag names (provisional, ledger 229f).** The proto reader's name table lists
    `NOATTSHINSU, NOATTCHUNJO, NOATTJINNO` in that order (`D/ProtoReader.cpp:674-698`), so a proto
    naming `NOATTCHUNJO` sets bit 4, which the enum calls `AIFLAG_NOATTACKJINNO`
    (`C/length.h:686-700`), and the reverse; the state machine tests the enum
    (`G/char_state.cpp:336-339`, `:346-349`). So a monster written to spare Chunjo spares Jinno
    (research note X6). 229f reads only `AIFLAG_AGGRESSIVE`, bit 0, which the swap does not touch.
    The legacy-following choice is to keep the bits as the reader sets them; the other is to swap
    the names back. Waits on it: the monster state machine (229g onwards).

27. **Which Shared Channel routes open at launch (ledger 229f).** The owner decided on
    2026-10-14 that Channels 2-4 are off and that the Shared Channel stays on if Channel 1's
    players need it. In legacy they need it for three routes:
    - **The OX event, to map 113.** NPC 20011 sends players there while `oxevent_status` is 1
      (`event/ox_event.quest:28-39`).
    - **Guild battle and flag wars, to maps 110 and 111.** The observer NPCs 11001, 11003 and
      11005 send players there.
    - **The kingdom and siege war buttons, to map 103.** The buttons send players there, and their
      handlers check no event flag.

    So the Shared Channel stays on. Two points are open:
    - Whether the OX event and the guild, kingdom and siege wars should run once the server opens.
      No server has run yet: there is no `prodomo.toml` beyond the example, and the store is a
      fresh database (owner, 2026-10-14).
    - Whether `event_panel` (`object/notarget/button/event_panel.quest:26, 137-138`) lets an
      ordinary player set `oxevent_status`. This was traced in the source only.

    None of these routes is live in the Rewrite until `pc.warp` lands. Waits on it: the quest warps.

28. **Evasion reads the attacker's level (B5, kept provisionally, ledger 229j).** `CalcAttackRating`
    takes the victim's evasion from the victim's dexterity and the attacker's level
    (`G/battle.cpp:282`, `int victim_lv = pkAttacker->GetLevel();`), so a high-level attacker
    raises the evasion of whatever it hits. The Rewrite keeps it (`world::character::attack_rating`
    already did): Alpha's punch on dog 101 takes 322. Reading the victim's level makes the same
    punch take 329. It is a balance formula, not a crash or a backdoor, so it waits for the owner.
    The other choice is the victim's level. Waits on it: nothing blocks.

29. **The automatic restart's penalty (B15/D3, provisional, ledger 229k).** Legacy's
    `dead_event` warps the player to town but applies `DeathPenalty(0)`, the here penalty
    (`G/char_battle.cpp:520`), while `/restart_town` applies `DeathPenalty(1)`, which costs no
    exp (`G/cmd_general.cpp:692`). It also spends `AFFECT_NO_DEATH_PENALTY`, which the
    developers' own `NO_DEATH_PENALTY_BUG_FIX` spares a town restart
    (`G/char_battle.cpp:365-376`). The Rewrite treats it as a Defect and applies the town
    penalty, so a player who waits out the three minutes loses nothing (V18j). The other choice
    is legacy's: an idle death costs what a here restart costs. Waits on it: nothing blocks.
30. **No exp loss at the top (B18, kept, ledger 229k).** At `max_level` or above (120 in the
    owner's configuration, 99 by default), a player whose exp is at least a quarter of the next
    level's loses nothing at a death and hears nothing (`G/char.cpp:4113-4116`). The Rewrite
    keeps it. The other choice is to take the loss at every level. Waits on it: nothing blocks.
31. **Drop the five `cg.dead.*` rows (ledger 229k).** `CInputDead` is declared and never
    installed: `m_inputDead` has one line, its declaration (`G/desc.h:197`), and
    `SetPhase(PHASE_DEAD)` installs `m_inputMain` (`G/desc.cpp:526-531`). 229k marks
    `cg.dead.pong`, `.time_sync`, `.chat`, `.whisper` and `.hack` `unused` with that evidence;
    `.scratch/parity/spec.md` asks the owner to confirm before a row is dropped. Waits on it:
    the row cleanup only.
32. **When `[LS;901]` is said (D2, provisional, ledger 229k).** Legacy rolls `number(0, 2)` and
    may say `[LS;901]` before it checks whether the death owes a penalty, so a player killed by
    a player or restarted in town hears it two times in three, and a lucky roll leaves the debt
    for a later death (`G/char_battle.cpp:347-363`). The Rewrite checks first, clears the debt
    on every path and says nothing when nothing is owed (V18k). The other choice is legacy's
    chatter with the debt cleared. Waits on it: nothing blocks.
