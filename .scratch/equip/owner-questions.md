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
   - `monkey_dungeon.lua`, which is never loaded, and the data files with
     no reader (`MapProperty.txt`, `charset.txt`, `pet_skill_names.txt`) (ledger 402 corrects
     the `BlueDragon.lua` half: `settings.lua:174` dofiles it, so `BlueDragonSetting` is set);
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
    first show. `SetPolymorph` sets it on a player the same way (`G/char.cpp:7701`; only the
    `__FIX_CHANGE_SEX_WITHOUT_RELOG__` path resets it, `:2171`), so a GM who used `/polymorph`
    carries it in each later insert and update, even after the clear, until it leaves; ledger
    364 reproduces that too. Waits on it: nothing blocks.

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
33. **A worn piece's attributes written in place (ledgers 343 and 368).** `/attr_full_set`,
    `/change_attr` and `/add_attr` write a worn piece's attributes without re-applying its
    points, as legacy does. Because the takeoff's `ModifyPoints(false)` subtracts the new
    attributes, a stat the old ones gave can stay behind or go negative. The Rewrite keeps
    legacy's behaviour. The other choice is to take the piece's points off before the write and
    put them back after it. Waits on it: nothing blocks.
34. **Quest functions no script calls (ledger 390).** Of the 508 `missing` rows in
    `.scratch/parity/quest-api.md`, 391 name a function that no file the quest runtime can load
    mentions: every file under `locale/europe/quest/` except the name list `quest_functions`, plus
    `settings.lua` and `translate.lua`, searched as `mod.fn`, `mod["fn"]` or a bare global name,
    comments included (control: `pc.ore_refine` and `pc.get_level` are found, `pc.mining` and
    `__refine_pick` are not). No GM command runs Lua text: `do_priv_guild`, the one that loads a
    buffer, loads an empty one (`G/cmd_gm.cpp:3169-3200`), so a client cannot reach them. Earlier
    ledgers ported such functions with probe quests (27 `ported` rows have no caller in the
    owner's scripts), and the Rewrite keeps doing so. The other choice is to mark the 391 rows
    `unused` with this evidence and port a function only when a script starts calling it. Waits
    on it: nothing blocks; it decides how much of the quest API is left.
35. **The horse refill at every login (ledger 392).** The owner's build defines
    `__FIX_NIVEL_CAL__` (`C/prodomodefines.h:95`), so `CInputLogin::Entergame` runs
    `SetHorseLevel(GetHorseLevel())` for every character with a horse
    (`G/input_login.cpp:887-897`): both pools refill, the health drop time moves three days on,
    and skill 130 is planted at the horse level. A dead or starved horse therefore stands fed and
    rested after a relog, which makes `/horse_set_stat`, the drop clock and the revive matter only
    within one session. The Rewrite ports it as written and treats it as a Quirk. The other choice
    is to call it a Defect and keep the stored pools, planting only the skill. Waits on it:
    nothing blocks; a change is one call in `enter_horse`.
36. **A revive herb given to any dead horse (ledger 393).** `CanReceiveItem` lets a herb of the
    right grade reach any dead horse NPC, and `ReceiveItem` then calls the giver's own
    `ReviveHorse()`, spends one herb and says `[LS;452]` whatever the call answered
    (`G/char_item.cpp:9483-9499`). So a herb given to another player's dead horse revives the
    giver's horse if it is dead, and is spent with the line even when the giver's horse is alive
    or the giver has none. The Rewrite ports it as written and treats it as a Quirk. The other
    choice is to call it a Defect and spend the herb only on the giver's own horse when it
    revives. Waits on it: nothing blocks; a change is one guard in `horse_gift`.
37. **The quest API no Lua file calls (ledger 401).** The owner chose to mark these `unused` rather
    than port them. 366 of the 477 `missing` quest-api names are referenced by none of the 164 files
    the quest runtime could load: every file under `locale/europe/quest` except `quest_functions`
    (the `qc` name list), and every `locale/europe/*.lua`, including the never-loaded ones. A client
    reaches a quest function only through a script, and the GM's `ADMIN_QUEST` buffer in
    `do_priv_guild` is empty (`G/cmd_gm.cpp:3171`). The rows are `unused`; a name a future quest
    needs is ported then. Please confirm the list, or name the ones to port:
    - `pc` (108): `pc.aggregate_monster`, `pc.change_name`, `pc.change_sex`, `pc.change_sp`, `pc.charge_cash`, `pc.clear_one_skill`, `pc.clear_skill`, `pc.clear_sub_skill`, `pc.dc_delayed0`, `pc.dc_direct0`, `pc.dec_skill_point`, `pc.delf`, `pc.do_reward`, `pc.equip_slot0`, `pc.forget_my_attacker`, `pc.get_account`, `pc.get_buff_level`, `pc.get_client_version0`, `pc.get_dx`, `pc.get_equip_refine_level`, `pc.get_exp_level`, `pc.get_exp_level0`, `pc.get_ht`, `pc.get_informer_item`, `pc.get_informer_type`, `pc.get_ip0`, `pc.get_iq`, `pc.get_language`, `pc.get_logoff_interval`, `pc.get_part`, `pc.get_premium_remain_sec`, `pc.get_sex`, `pc.get_sig_items`, `pc.get_skill_point`, `pc.get_socket_items`, `pc.get_special_ride_vnum`, `pc.get_st`, `pc.get_start_location`, `pc.get_worldboss_total_damage`, `pc.getcurrentmapindex`, `pc.give_award`, `pc.give_award_socket`, `pc.give_exp`, `pc.give_exp_perc`, `pc.give_gold`, `pc.give_item`, `pc.give_item_from_special_item_group`, `pc.give_lotto`, `pc.give_poly_marble`, `pc.give_polymorph_book`, `pc.give_random_book0`, `pc.has_master_skill`, `pc.have_map_scroll`, `pc.have_pos_scroll`, `pc.is_arena0`, `pc.is_arena_observer0`, `pc.is_available0`, `pc.is_clear_skill_group`, `pc.is_dead`, `pc.is_engaged_or_married`, `pc.is_flag_fire`, `pc.is_flag_invisible`, `pc.is_flag_poison`, `pc.is_flag_slow`, `pc.is_flag_stun`, `pc.is_near_vid`, `pc.is_premium`, `pc.is_pvp0`, `pc.is_secured`, `pc.learn_shaman_skill`, `pc.make_guild0`, `pc.make_item0`, `pc.pc_attract_ranger`, `pc.pointchange`, `pc.polymorph`, `pc.pullmob`, `pc.refine_equip`, `pc.remove_polymorph`, `pc.reset_point`, `pc.reset_status`, `pc.save_exit_location`, `pc.select_pid`, `pc.select_vid`, `pc.set_change_empire_count`, `pc.set_dx`, `pc.set_flag_dead`, `pc.set_flag_fire`, `pc.set_flag_invisible`, `pc.set_flag_kill`, `pc.set_flag_poison`, `pc.set_flag_slow`, `pc.set_flag_stun`, `pc.set_gm_level`, `pc.set_ht`, `pc.set_iq`, `pc.set_level`, `pc.set_level0`, `pc.set_max_health`, `pc.set_part`, `pc.set_race0`, `pc.set_skill_point`, `pc.set_st`, `pc.set_warp_location`, `pc.set_warp_location_local`, `pc.teleport`, `pc.unequip_slot0`, `pc.upgrade_polymorph_book`, `pc.warp_exit`.
    - `d` (58): `d.all_near_to`, `d.check_eliminated`, `d.count_monster`, `d.delete_item_in_item_group_from_all`, `d.exit`, `d.exit_all_by_item_group`, `d.exit_all_to_start_position`, `d.get_damage_from_affect`, `d.get_damage_from_race`, `d.get_kill_mob_count`, `d.get_kill_stone_count`, `d.get_unique_vid`, `d.increase_completed`, `d.is_available0`, `d.is_unique_dead`, `d.is_use_potion`, `d.join`, `d.jump_all`, `d.kill_unique`, `d.new_jump_all`, `d.new_jump_all_guild`, `d.new_jump_guild`, `d.purge`, `d.purge_area`, `d.purge_unique`, `d.regen_file`, `d.reset_damage_from_affect`, `d.reset_damage_from_race`, `d.revived`, `d.say_diff_by_item_group`, `d.set_dest`, `d.set_exit_all_at_eliminate`, `d.set_fastest`, `d.set_item_group`, `d.set_regen_file`, `d.set_unique`, `d.set_warp_at_eliminate`, `d.setqf`, `d.setqf2`, `d.spawn`, `d.spawn_goto_mob`, `d.spawn_group`, `d.spawn_mob_ac_dir`, `d.spawn_mob_dir`, `d.spawn_move_group`, `d.spawn_move_unique`, `d.spawn_name_mob`, `d.spawn_stone_door`, `d.spawn_unique`, `d.spawn_wooden_door`, `d.unique_get_hp_perc`, `d.unique_set_def_grade`, `d.unique_set_hp`, `d.unique_set_maxhp`, `d.unset_damage_from_affect`, `d.unset_damage_from_race`, `d.update_info`, `d.warp_all`.
    - `(globals)` (38): `LANG`, `__get_empire_priv`, `__get_empire_priv_string`, `__get_guild_priv`, `__get_guild_priv_string`, `__get_guildid_byname`, `__give_char_priv`, `__give_empire_priv`, `__give_guild_priv`, `add_casket_item`, `add_restart_city_pos`, `block_chat`, `cleartimer`, `find_pc_cond`, `get_channel_id`, `get_locale`, `get_quest_state`, `get_special_item_group`, `get_table_postfix`, `item_log`, `kill_all_in_map`, `loop_timer`, `mob_vnum`, `mysql_direct_query`, `mysql_escape_string`, `mysql_password`, `notice_in_map`, `purge_area`, `regen_in_map`, `say_in_map`, `server_loop_timer`, `spawn_mob`, `spawn_mob0`, `spawn_mob_in_map`, `sys_log`, `under_han`, `warp_all_in_area_to_area`, `warp_all_to_village`.
    - `item` (25): `item.add_attr0`, `item.change_attr0`, `item.clear_attr0`, `item.copy_and_give_before_remove`, `item.count_attr0`, `item.equip_to0`, `item.get_antiflag0`, `item.get_attr0`, `item.get_immuneflag0`, `item.get_level_limit`, `item.get_transmutation`, `item.get_wearflag0`, `item.has_antiflag0`, `item.has_flag`, `item.has_immuneflag0`, `item.has_wearflag0`, `item.is_available0`, `item.is_transmulated`, `item.next_refine_vnum`, `item.set_attr0`, `item.set_count0`, `item.set_transmutation`, `item.set_value`, `item.start_realtime_expire`, `item.unequip0`.
    - `npc` (23): `npc.dec_remain_hairdye_count`, `npc.dec_remain_skill_book_count`, `npc.get_leader_vid`, `npc.get_level0`, `npc.get_name0`, `npc.get_pid0`, `npc.get_remain_hairdye_count`, `npc.get_remain_skill_book_count`, `npc.get_vid`, `npc.get_vid_attack_mul`, `npc.get_vid_damage_mul`, `npc.get_vnum0`, `npc.is_available0`, `npc.is_near`, `npc.is_near_vid`, `npc.is_quest`, `npc.kill`, `npc.lock`, `npc.open_shop`, `npc.purge`, `npc.set_vid_attack_mul`, `npc.set_vid_damage_mul`, `npc.unlock`.
    - `forked` (15): `forked.get_pass_start_pos`, `forked.get_sungzi_start_pos`, `forked.getdeadcount`, `forked.getpassmapindexbyempire`, `forked.getpasspathbyempire`, `forked.getsungzimapindex`, `forked.initforked`, `forked.initkillcount`, `forked.is_registered_user`, `forked.isforkedmapindex`, `forked.issungzimapindex`, `forked.purge_all_monsters`, `forked.register_user`, `forked.setdeadcount`, `forked.warp_all_in_map`.
    - `marriage` (15): `marriage.end_wedding`, `marriage.engage_to`, `marriage.find_married_vid`, `marriage.get_married_time`, `marriage.get_wedding_list`, `marriage.in_my_wedding`, `marriage.join_wedding`, `marriage.remove`, `marriage.set_to_marriage`, `marriage.warp_to_my_marriage_map`, `marriage.wedding_client_command`, `marriage.wedding_dark`, `marriage.wedding_is_playing_music`, `marriage.wedding_music`, `marriage.wedding_snow`.
    - `oh` (15): `oh.frog_to_empire_money`, `oh.info`, `oh.isguildmaster`, `oh.ismonarch`, `oh.monarch_mob`, `oh.monarchbless`, `oh.monarchdefenseup`, `oh.monarchpowerup`, `oh.notice`, `oh.spawnguard`, `oh.spawnmob`, `oh.takemonarchmoney`, `oh.transfer`, `oh.transfer2`, `oh.warp`.
    - `affect` (11): `affect.add`, `affect.add_collect`, `affect.add_collect_point`, `affect.add_hair`, `affect.get_apply_on`, `affect.remove`, `affect.remove_all_collect`, `affect.remove_bad`, `affect.remove_collect`, `affect.remove_good`, `affect.remove_hair`.
    - `oxevent` (11): `oxevent.close`, `oxevent.end_event`, `oxevent.end_event_force`, `oxevent.get_attender`, `oxevent.get_oxquiz_fn`, `oxevent.get_status`, `oxevent.give_item`, `oxevent.open`, `oxevent.quiz`, `oxevent.set_oxquiz_fn`, `oxevent.set_oxquiz_fn0`.
    - `party` (11): `party.chat`, `party.clear_ready`, `party.delete_party`, `party.get_max_level`, `party.get_min_level`, `party.give_buff`, `party.is_in_dungeon`, `party.is_map_member_flag_lt`, `party.leave_party`, `party.setqf`, `party.syschat`.
    - `game` (7): `game.get_safebox_level`, `game.get_worldboss_min_damage`, `game.give_worldboss_affect`, `game.open_mall`, `game.open_safebox`, `game.open_web_mall`, `game.set_safebox_level`.
    - `pet` (5): `pet.count_summoned`, `pet.is_summon`, `pet.spawn_effect`, `pet.summon`, `pet.unsummon`.
    - `arena` (4): `arena.add_observer`, `arena.get_duel_list`, `arena.is_in_arena`, `arena.start_duel`.
    - `q` (4): `q.getcurrentquestname`, `q.no_send`, `q.set_icon`, `q.set_title2`.
    - `member` (3): `member.chat`, `member.clear_ready`, `member.set_ready`.
    - `guild` (2): `guild.is_war`, `guild.set_war_data0`.
    - `highscore` (2): `highscore.register`, `highscore.show`.
    - `mgmt` (2): `mgmt.monarch_change_lord`, `mgmt.monarch_state`.
    - `mob` (2): `mob.spawn`, `mob.spawn_group`.
    - `DragonLair` (1): `DragonLair.startRaid`.
    - `MeleyLair` (1): `MeleyLair.OpenRanking`.
    - `ba` (1): `ba.start`.
    - `dance_event` (1): `dance_event.gohome`.
    - `target` (1): `target.pos`.
38. **The guild-war bet quest cannot complete (ledger 408).** `guild_war_bet.lua:39` formats
    `_80_say`'s three verbs with one argument, so `string.format` errors and the dialog ends
    after the title — on the legacy server too, since 5.0.3 errors the same way. No bet on
    any reservation can complete through the owner quest on either server. The Rewrite runs
    the source as-is (ADR-0004) and records it as a Defect. If the owner wants the bet path
    playable, the quest source needs the fix (one argument per verb); say the word and the
    Rewrite will carry the fixed line as a Divergence. Waits on it: nothing blocks; the
    bridges (`guild.get_reserve_war_table`, `guild.is_bet`, `guild.war_bet`) are ported and
    proved by probe.
39. **The dungeon eliminate timers (ledger 423).** `dungeon_jump_to_event` and
    `dungeon_exit_all_event` fire only when `CheckEliminated` finds their flag set
    (`G/dungeon.cpp:1319-1357`), and only `d.set_warp_at_eliminate` /
    `d.set_exit_all_at_eliminate` set those flags (`G/questlua_dungeon.cpp:265,306`).
    Both names are `unused`: no Lua file calls them (ledger 401, question 37). The rows
    are `unused`; if a future quest needs them, the monster count and the two timers
    are ported then. Waits on it: nothing blocks.
40. **The gwihwan recall timer (ledger 424).** `skill_gwihwan_event` recalls the
    caster to its empire's spawn on a level-scaled roll (`G/char_skill.cpp:2072-2113`),
    but nothing arms it: no skill row casts it, no code creates the event, and the name
    appears nowhere else in the tree. The row is `unused`. Waits on it: nothing blocks.
41. **The per-player quest timer (ledger 426).** `quest_timer_event` fires a PC's named
    timer script (`G/questevent.cpp:49-105`), but only `timer` and `loop_timer` arm it
    (`G/questlua_global.cpp:206-262`), and both are `unused`: no Lua file calls them
    (ledgers 401, 414, question 37). The row is `unused`; the server timer twin is
    `ported` (ledger 403's scenario). Waits on it: nothing blocks.
42. **The monarch timers (ledger 428).** `monarch_powerup_event`, `monarch_defenseup_event`
    and `monarch_transfer2_event` are armed only by `oh.monarchpowerup`, `oh.monarchdefenseup`
    and `oh.transfer2` (`G/questlua_monarch.cpp:207-216, 264-273, 828-946`), and all three
    names are `unused`: no Lua file calls them (ledger 401, question 37). The `mto`/`mtr`/
    `minfo`/`mtax` commands are a separate missing system; these timer rows are `unused`.
    Waits on it: nothing blocks.
43. **The quest warp timers (ledger 429).** `warp_all_to_map_event` (forked) and
    `warp_all_to_village_event` (global) are armed only by `forked.warp_all_in_map` and
    `warp_all_to_village`, both `unused` (ledger 401, question 37); the empire warp
    `warp_all_to_map_my_empire_event` has no arming site at all. The event-manager twin
    `warp_all_to_village_event` is separate and stays: the siege stones arm it. The three
    rows are `unused`. Waits on it: nothing blocks.
44. **The spam-table reload timer (ledger 430).** `reload_spam_event` re-reads `spam_db`
    every `spam_block_reload_cycle`, but its only `event_create` sits behind `#ifdef
    ENABLE_SPAMDB_REFRESH` (`G/db.cpp:770-775`), and that switch is commented out and
    defined nowhere. The row is `unused`. The spam block itself (`block_chat_by_ip_event`)
    stays `missing`: it needs the per-IP score store. Waits on it: nothing blocks.
45. **The marriage and wedding timers (ledger 432).** `near_check_event` and
    `wedding_end_event` run only for a standing marriage, and a marriage forms only
    through `marriage.engage_to` (`G/questlua_marriage.cpp:18-28`), which no Lua file
    calls — all 15 `marriage.*` names are `unused` (ledger 401, question 37). Both rows
    are `unused`. Waits on it: nothing blocks.
46. **The mount, pet and shaman timers (ledger 435).** Each fires only while its
    system runs, and each system starts only through an equip arm the port refuses:
    mount and pet costumes at equip and at load (`WornSystem::Mount`/`Pet`), the
    `TOGGLE_SHAMAN` toggle through an unported use arm. All three rows are `unused`.
    Waits on it: nothing blocks.
47. **The dungeon regen timer (ledger 436).** `dungeon_regen_event` fires only for a
    regen `regen_do` loads into a dungeon with `bOnce` false (`G/regen.cpp:487-495`),
    and only `d.set_regen_file` passes both together — `d.regen_file` defaults
    `bOnce` true (`G/dungeon.h:72`) — which no Lua file calls (`lua.d.set_regen_file`
    is `unused`, ledger 401, question 37). The row is `unused`. Waits on it: nothing
    blocks.
48. **The three watchdog timers (ledger 436).** `check_speedhack_event` never
    increments: both writers sit behind `enable_hack_check`, which the owner's
    config leaves off (`G/battle.cpp:871`, `G/config.cpp:154`, `G/input_main.cpp:1562`),
    and the port relays `FUNC_COMBO` with no combo count. `destroy_when_idle_event`
    fires only for a disbanded NPC party (`G/party.cpp:326-335`), and the port hosts
    PC parties only (`delete_party`, `prodomo/src/game_state/party.rs:1691`).
    `load_affect_login_event` is the retired DB-peer's deferred load
    (`G/char_affect.cpp:508-558`); the port loads affects synchronously at entry
    (`GameState::load_affects`). All three rows are `unused`. Waits on it: nothing
    blocks.
49. **The three gated-system timers (ledger 436).** `private_shop_warp_event`
    fires only from `WarpToPrivateShop` on the DB's warp answer
    (`G/char.cpp:12434-12474`, `G/input_main.cpp:5038-5054`), while the whole
    `HEADER_CG_PRIVATE_SHOP` record and `sys.trade.private_shop` are `missing`.
    `premium_players_update_event` is armed only by the activate spend and the
    premium login (`G/char.cpp:11553-11559`, `G/input_login.cpp:717-720`), while
    `sys.custom.premium`, `cg.game.premium_players` and its sub-rows are `missing`.
    `switchbot_event` fires only while `CSwitchbot::SwitchItems` runs
    (`G/refactorized_switchbot.cpp:151-170,547,757`), while `sys.custom.switchbot`
    and `cg.game.switchbot` are `missing`. All three rows are `unused`; each stands
    ready for its system's port (the shop warp shares `TimedEvent`). Waits on it:
    nothing blocks.
50. **The two skill timers (ledger 436).** `ChainLightningEvent` is created only
    from `FuncSplashDamage`'s chain arm (`G/char_skill.cpp:1970-1978`), and skill
    108 is on the refused list (`gamedata/src/pc_skill_data.rs:16`), so no cast
    reaches it. `skill_muyoung_event` starts only from `ComputeAffect`'s type-78
    arm (`G/char_affect.cpp:805-810`) after `ComputeSkill`'s MUYEONG arm attaches
    it (`G/char_skill.cpp:2740-2756`), and skill 78 is refused at the same load.
    Both rows are `unused`; each walks home with its skill row when the loader
    hosts it. Waits on it: nothing blocks.
51. **The six item expiry timers (ledger 437).** Each starts only from a path
    the port refuses: unique expiry from `EquipTo`/create/buff-NPC while
    `WornSystem::Unique` refuses every `ITEM_UNIQUE` at equip and at load;
    timer-based-on-wear from `EquipTo`/create/toggle/dragon-soul while
    `WornSystem::Timer` refuses every timed item; real-time expiry from
    create/first-use/load/quest while the same `Timer` gate refuses and the quest
    bridge is `unused`; blend-use expiry from the extended-blend arm while
    `use_item` answers `Unported::Use` for `ITEM_BLEND`; accessory-socket expiry
    from `EquipTo`/re-arm while `WornSystem::AccessoryTimer` refuses every graded
    accessory; aura-boost expiry from `EquipTo` while `WornSystem::Aura` refuses
    every aura costume. All six rows are `unused`; each fires home with its
    `WornSystem` when the equip gate opens. Waits on it: nothing blocks.
52. **The four arena timers (ledger 438).** `ready_to_start_event` and
    `duel_time_out` run only from `StartDuel`, which runs only from
    `arena.start_duel` and the GM `do_duel` (`G/questlua_arena.cpp:41`,
    `G/cmd_gm.cpp:4052`); the bridge is `unused` and `cmd.duel` is `missing`.
    `battle_arena_event` starts only from `ba.start` and the GM `do_weeklyevent`
    (`G/questlua_ba.cpp:12`, `G/cmd_gm.cpp:4488-4492`); the bridge is `unused`
    and `cmd.weeklyevent` is `missing`. `DragonLair_Collapse_Event` is armed only
    by the dragon's death in a rented lair (`G/DragonLair.cpp:243-250`), and the
    lair rents only from `DragonLair.startRaid`, which no Lua file names. All
    four rows are `unused`. Waits on it: nothing blocks.
53. **The two castle siege timers (ledger 438).** Both are created only by
    `castle_start_siege` (`G/castle.cpp:591-618`), which runs only from
    `castle_siege`, which runs only from the GM `do_siege` (`G/cmd_gm.cpp:4354`)
    and the retired game-to-game siege packet (`G/input_p2p.cpp:677`, ADR-0001);
    `cmd.siege` is `missing`. Both rows are `unused`. Waits on it: nothing
    blocks.
54. **The three OX timers and the spam timer (ledger 439).** `oxevent_timer`
    is created only by `Quiz` (`G/OXEvent.cpp:296`), which runs only from the
    process's quiz phase and `oxevent.quiz`; `end_oxevent` only from
    `oxevent.end_event`; `ox_event_process` only from `SetOXEvent(true)` — and
    the whole `oxevent.*` bridge family is `unused` while `start_game_event`
    stops at the flag for index 11. `block_chat_by_ip_event` is created only by
    `SpamBlockCheck`, whose per-IP score the chat judge documents as unported
    (`prodomo/src/chat.rs:421-423`). All four rows are `unused`. Waits on it:
    nothing blocks.
55. **The three-way war timer and the three world boss timers (ledger 439).**
    `regen_mob_event` is created only by `CThreeWayWar::onDead`'s elimination arm
    (`G/threeway_war.cpp:410-418`) from `Dead` on a forked map, and the port reads
    only the SungZi half of `forkedmapindex.txt`. The three world boss timers are
    created only by `SetCooldown`/`SetRunTime`, which `Enable(true)` runs from the
    `world_boss_event` flag arm (`G/questmanager.cpp:1344-1348`); the port's
    `set_event_flag` leaves that name on the table with no arm, and
    `cmd.worldboss` is `missing`. All four rows are `unused`. Waits on it:
    nothing blocks.
56. **The three event-manager spawn timers (ledger 439).** Every arm of
    `warp_all_to_village_event` is a `Set*Event` end-arm or the siege-stone
    kill; `dynamic_spawn_cycle_event` is created only by the boss and metin
    start-arms; `static_spawn_cycle_event` only by the frog and Tanaka
    start-arms — and `start_game_event` ports only the exp and loot arms (1-2)
    past the flag (`prodomo/src/game_state/quest_quiet.rs:467`). All three rows
    are `unused`; each fires home with its first event start or end. With them
    the timed-events inventory has no `missing` row left. Waits on it: nothing
    blocks.
57. **Two timers withdrawn from questions 43 and 55 (ledger 465).** Both are
    reachable after all, so they are `missing`, not `unused`. Question 43 said nothing
    arms `warp_all_to_map_my_empire_event`, but its sweep missed the `quest::`-qualified
    creates in `CThreeWayWar::onDead` (`G/threeway_war.cpp:356-385`, `:443-470`).
    Question 55 said `regen_mob_event` waits on an unported war. Ledger 465 ports the
    war's death hook, which runs on every forked map from `Dead`
    (`G/char_battle.cpp:1325-1362`), because `Initialize` leaves the regen flag at 0.
    Its elimination arm, which arms both timers, runs once a `HIGH_WIZARD` sets
    `threeway_war_kill_count` with `/eventflag` and the scores on a SungZi map leave
    exactly one empire below it. No owner quest sets the flag. Shared Channel 99 hosts
    the SungZi maps 121 and 125 and the pass maps 118-120. The rest of questions 43
    and 55 stands. Nothing to decide; both rows port with the arms.
58. **The three-way war never runs on your data (ledger 466; replaces question 57).**
    Question 57 was wrong to call the two timers reachable. Your `locale/europe/map/index`
    lists 59 maps and none of the forked maps 114-128, and the Rewrite builds every map from
    that index, as legacy's `SECTREE_MANAGER` does. Shared Channel 99's `MAP_ALLOW` names
    118-128, but none of them loads, so nobody can die on a forked map and
    `CThreeWayWar::onDead` never runs. The rows `event.threeway_war.regen_mob_event` and
    `event.questlua_global.warp_all_to_map_my_empire_event` are therefore `unused` again,
    for this reason. Question 43's evidence for the second row was wrong: `onDead` does arm
    it, but `onDead` never runs. `/threeway_info` and `/threeway_myinfo` work and show the
    idle war. The war's elimination and victory arms wait on the SungZi and pass map folders.
    Waits on it: whether you plan to add those map folders. If you do, the arms are the next
    port; if not, confirm the two rows `unused`.
