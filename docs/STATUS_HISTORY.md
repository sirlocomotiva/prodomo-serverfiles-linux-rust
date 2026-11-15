# Rewrite status: history

This page holds what `docs/STATUS.md` carried until the direction review of 2026-11-15 (ledger
679): the per-section "Since N" narrative, the step-by-step summary, the build-order prose, and
the two step-1 and step-3 tables. It is moved here unchanged and is no longer updated. Every
statement in it is true only as of the ledger section it names; `docs/REWRITE_LEDGER.md` is the
record of each change, and `docs/STATUS.md` says where the Rewrite stands now.

## The per-section narrative (ledger 229 to 678)

Last reviewed: 2026-11-13, after ledger section 678 (the event-stone detector).
Steps 1, 2 and 3 are done; step 4 is in progress, and its next work is 679
onwards. The
messenger is `ported`; since 257 a session's quest `login` event restores the block byte its
`game_option` flags name, since 258 the level-30 letter titles its quest in the client's
quest window (`GC_QUEST_INFO`), and since 259 a script's state change runs the `leave`,
`enter` and `letter` scripts legacy's `EndRunning` runs (`G/questpc.cpp:352-371`), so the
sash letter marks Theowahdan (`find_npc_by_vnum`, `target.vid`, `GC_TARGET_CREATE`). Since
260 a click on a marked NPC runs its quest's `target.click` script (`CQuestManager::Click`'s
target arm, `G/questmanager.cpp:918-950`, `NPC::OnTarget`, `G/questnpc.cpp:206-264`), each
mark's event sends where the marked character stands (`target_event`, `G/target.cpp:57-132`,
`GC_TARGET_UPDATE`), and `item_name` answers, so the sash quest moves to `go_to_disciple` and
marks the captain. Since 261 `pc.get_playtime` reads the minutes played the world loaded and
each save banked (`pc_get_playtime`, `G/questlua_pc.cpp:874-878`), so the captain's "Yes"
moves the quest to `time_to_wait` and its window counts the wait down. Since 262
`pc.give_item2` runs `AutoGiveItem` (`G/char_item.cpp:9081-9223`) on a dialog step or a click:
the stacks, a free cell or the ground, stored before the client hears of it, so
`guild_create`'s reward page gives its book. The captain's 85000 has no proto on the owner's
data, so the sash quest's material never arrives. Since 263 `pc.count_item` reads
`CountSpecifyItem` and `pc.remove_item` runs `RemoveSpecifyItem` (`G/char_item.cpp:8791-8940`),
so a seeded `__complete` sends the player back to the captain for the material. The removal takes
items only where the give gives them, and no caller on the owner's data reaches it yet. Since 264
a used `ITEM_QUEST` runs its own `use` scripts (`case ITEM_QUEST`, `G/char_item.cpp:2842-2888`,
`CQuestManager::UseItem`) and becomes the quest item that `item.get_vnum` reads, so the boost and
ward reset book (71000) answers its early pages. Since 265 `pc.get_job` answers `GetJob()` and
`pc.set_skill_level` runs `SetSkillLevel` and `SkillLevelPacket` (`G/questlua_pc.cpp:2112-2123`,
`G/char_skill.cpp:183-258`, with the 7/8th skills' `SkillCanUp` exclusion), stored like a
GRAND climb, so the boost and ward books teach their chosen skill through `select_table` (which
already ran through the `select` wait) and the reset book forgets it. Since 266
`pc.give_item2_select` gives and makes the given item the quest item, and `item.set_socket`
writes its socket (`G/questlua_item.cpp:125-136`, `CItem::SetSocket`); their one caller, the sash
quest's reward, stays unreachable while 85000 has no proto. Since 267 a quest reads the
real alignment, moves it with the bands following (`UpdateAlignment`,
`G/char_battle.cpp:3477-3565`, the exact-equality removals, `ALIGNMENT_BONUS` 570-577), and
reads and removes the no-delay book affect, all through the GRAND book 50525's live `use`
script, whose failed read moves the alignment; the kill's alignment routes through the
same update. Since 268 the whole flow runs over the wire: Alpha is refused at the rest
gate, and Zulu's failed read moves the alignment, which the disconnect banks beside the
`next_time` flag. The five alignment rows are `ported`. Since 269 a quest reads its
reader back (gold, empire, HP/SP, exp, race, name, VID, leadership, the positions, the worn
vnums, the shown alignment, the guild triad, the NPC's empire) and asks the client to make
the guild (`game.request_make_guild`); the guild man's founding chat runs over the wire.
Since 270 a quest names another quest's flags (`pc.getf`/`pc.setf`), deletes its own
(`pc.delqf`), and moves the gold (`pc.change_money` under four names, with the below-0
and cap refusals); the sash quest's `__complete` deletes its wait flags over the wire.
Since 271 a quest hides a costume part (`pc.hide_costume`), `/hide_costume` hides it with
the 1.5-second move gate, and the enter-game restore plus the `costume_option` login script
Since 272 a quest reads its held item's socket, proto value, count and type
(`item.get_socket`, `item.get_value`, `item.get_count`, `item.get_type`,
`item.get_sub_type`), checks the room (`pc.enough_inventory`), and reads the GM gate
and level (`pc.is_gm`, `pc.get_gm_level`, over the stored level: the test-server
silence is V26); the Time Spiral shortens the attr67add wait over the wire. Since 273
a quest parts (`pc.remove_from_guild`) and disbands (`pc.destroy_guild`) past the war
line and the master and leader gates, finds a PC by name (`find_pc_by_name`), reads
the test-server mode (`is_test_server`) and stamps (`time_to_str`); the guild man's
chats part Yankee and disband `MyGuild` over the wire. Since 274 a quest warps
(`pc.warp`, past `IsHack`, on the Channel hosting the target), drops an owned item at
the reader's feet (`game.drop_item_with_ownership`) and reads the conqueror level
(`pc.get_conqueror_level`); the temple guardian's "Da" warps to map 209 over the wire.
Since 275 a quest warps to the village (`warp_to_village`, to the empire start), asks
the warp gate (`pc.can_warp`) and makes a conqueror (`pc.set_conqueror`, with the
already and marble lines or the level-1 transition); the statue's Return "Da" warps to
the Shinsoo start over the wire, and the four rows are `ported`. Since 276 a quest
opens the sash (`pc.open_sash`, only a boolean, with the `SASH_OPEN`/`SASH_CLOSE`/
`COMBINE_CLOSE` key chats) and the changelook call is bridged; Theowahdan's
Combination "Yes" opens the 23-byte window over the wire, and `open_sash` plus
`gc.sash` are `ported`. Since 277 a quest opens the changelook window
(`pc.open_changelook`, no argument read, with the already-open line); Theowahdan's
Transmutation "Yes" opens the 10-byte window with the 50,000,000 cost over the
wire, and `open_changelook`, `gc.cl` and `quest.changelook` are `ported`. Since
278 a quest reads the party (`party.is_party`, `party.is_leader`), the mount
(`pc.is_mount`, false: no ported path mounts) and the Channel
(`pc.get_channel_id`); the forest room's solo click refuses over the wire, and
`is_party` is `ported`. Since 279 a quest reads and sets the party flags
(`party.getf`, `party.setf`), reads the leader PID (`party.get_leader_pid`, -1
with none) and finds no dungeon under any map index (`d.select`, `d.find`,
false: no dungeon manager exists); a party member's click on the forest room
passes the party gate and the leader gate refuses over the wire, and the five
plus `is_leader` are `ported`. Since 280 a quest pushes the on-map member PIDs
(`party.get_member_pids`) and runs a block for another online PC
(`q.begin_other_pc_block`/`q.end_other_pc_block`, with the level, name, VID,
points, count and removal running against the blocked PC and a block's removals
applied after the step); the leader's click on the forest room runs the ticket
loop and refuses for want of a ticket over the wire, and the three are `ported`.
The refusal names nobody, as legacy's own loop walks the level list
(`enchanted_forest_zone.lua:169`). Since 281 a quest grants the dragon-soul
qualification (`ds.give_qualification`, in the call, as legacy's `AddAffect`), reads
it (`ds.is_qualified`, 1/0) and opens the refine (209/sub 0 with
`INVENTORY`/`WORD_MAX`, only with the qualification) and attr-change (the one-byte
211) windows; the alchemist's chats open both over the wire, 540 survives the death
as no-clear, and the four names, the two records and the three quest scripts are
`ported`. Since 282 the statue's Yohara trip is scenario-covered (no new
bridge: the 17-line quest's six calls were already ported): the click offers
Return, the trip and the close, "Da" reads the conqueror level 33 and the step
orders the warp to (537500, 492700) on map 374, and the quest is `ported`.
Since 283 the guild man's score chat is scenario-covered (no new bridge: the
19-line quest's four names were already ported): the master-only click offers
the disband and the three ranking chats, the score page says rank 1 and 1000
points, and the quest is `ported`. Since 284 the sash's Absorption chat is
scenario-covered (no new bridge: `open_sash(false)` rode with combination):
the second click offers the eight chats, "Yes" opens the absorb window with
the window byte 0, and the quest is `ported`. Since 285 the disconnect runs
the `logout` scripts (`LogoutPC` before the flag taking, `DisconnectPC`
after): the forest's sets `dungeon_enter` 1 and stamps `limit_activity` over
the wire, and `pc.setf` and the quest are `ported`. Since 286 the dungeon
info's button runs over the wire (no new bridge: the only reached `input()`):
`getinputbegin`, the prompt, `TEST#2598` answered, the `[INPUT]` page with the
resume, `getinputend` and the two lines, and the record and the quest are
`ported`. Since 287 the ring's plain warp runs over the wire (no new bridge:
the 7-line quest's six calls were already ported): the use lists the twelve
maps and the quit, Valea Seungryong reads `can_warp` true and warps to
(666300, 855200) on map 64, and the quest is `ported`. Since 288 the statue's
marked click runs over the wire (no new bridge: the find, the mark, the
target click, the pages, the state move and its letter were already ported):
the login's letters mark the statue, the click deletes the mark and says the
two pages, the quest moves to `information_hunt` and letters, and the logout
banks the state; the quest stays `partial`. Since 289 the emotions run over the
wire: the ride, mask, woman, victim, distance, sex and consent gates, the
`CHAT_TYPE_COMMAND` broadcast to the view, and the silent consent insert; the 40
command rows and the system are `ported`. Since 290 `/priv_empire` grants the
empire rate over the wire: the double clamp (0..1000, then 0..200), the week, the
five usage lines, the `[LS;724]` notice (or the raw removal line) to every
client, the `[LS;871-874]` lines in the enter burst, and the exp, gold and drop
terms; `cmd.priv_empire` is `ported` and `cmd.priv_guild` is `unused` (its empty
buffer runs nothing). Since 291 `/eventflag` sets the quest table's flag over
the wire (the `strtol` value, the nine `mob_*` clamps, the raw
`RequestSetEventFlag` answer) and `/geteventflag` lists it in name order, so a
late login's `cards icon` fires and its `event_info` reads the set flag; the
two command rows and `lua.game.get_event_flag` are `ported`. Since 292
`/give_reward` trades the six letters (the busy gates, the `letters_event` flag,
the missing-letter line, the six removals, the one draw of group 55021) and
`/letters_open` opens the window (`letters_drop` per group row, then
`OpenLetterWindow`); the owner's file holds no 55021 group, so no drop lines
run and the draw lands nothing, and a late login's burst reads `letters_event 1`.
The two command rows are `ported`. Since 293 `/stat` raises the named attribute
by 1 for a stat point (both arrays, compute, the change with its battle tail,
the crossed pool line, the spent counter, compute again) and `/stat-` lowers it
back for a reset count (the point returns, the reset spends, each pool's own
line), and the disconnect save banks the four and both counters; the two
command rows are `ported`. Since 294 `/pkmode` lands the line's number as the
caller's mode (the unsigned low byte, the protect and level gates, the same-mode
silence, the max refusal, the Meley force to guild, the guildless fallback to
free) with an update to its view; the row is `ported`. Since 295 `/set_walk_mode`
and `/set_run_mode` walk and run the caller (both flags land, only the second's
change fans the walk-mode record to the view), and the flag feeds `IsWalking`
records read it; the two rows are `ported`. Since 296 the guild line fans over
the wire (id 0, the empire byte, `bCanFormat` true, to every online member),
rows are `ported`. Since 297 the friend ask by VID runs over the wire (the
target hears the same `messenger_auth` ask the name arm sends), and the name
`ported`. Since 298 a sync claim on an NPC is skipped over the wire (no
ownership, no batch, while a player's claim in the same frame lands), and the
rows and the three Position rows are `ported`. Since 299 the guild withdraw
stays quiet over the wire (both money arms return before their bodies), and
`ported`. Since 300 the leader's HEAL stays quiet over the wire (legacy's dead
body), a member's use is told `[LS;684]`, a gone VID the raw summon line and
an outsider `[LS;685]`, so both `PartyUseSkill` rows are `ported`. Since 301
a game-phase PONG is read and not answered over the wire (the character keeps
playing), and a `TIME_SYNC` naming a wrong token closes silently, so both rows
are `ported`. Since 302 a game-phase hack report closes over the wire, and a
select-screen one is consumed with the `@fixme120` pair, so both hack rows are
`ported`. Since 303 the four stubs answer `[LS;916]` over the wire (the GM one
only to a granted caller), so all four `do_inputall` rows are `ported`. Since
304 `/mount` stays quiet standing over the wire (the body is empty) and is
scolded `[LS;918]` dead, so the row is `ported`. Since 305 `/ungroup` runs
over the wire (quiet partyless, `[LS;532]` and REMOVE from three, silent
disband from two), so the row is `ported`. Since 306 `/vote_block_chat`
stays quiet over the wire (the body returns first) and `/block_chat_list`
lists one `%s %s sec` line per silence past the lay's gate (whose refusal is
`there is no such command`, as written), so both rows are `ported`. Since
307 `/dc` runs over the wire (the usage, the no-such-player line with the
name as typed, the exact-match miss, the self refusal, and the target's
close), so the row is `ported`. Since 308 `/transfer` runs over the wire
(the usage, the no-character line, the cross-Channel line, the self refusal
in another case, and the target's warp to the caller's feet), so the row is
`ported`. Since 309 `/kill` and `/poison` run over the wire (the usages, the
misses with no stop, the exact-match misses, the poison's add and first tick,
and the kill's death through the poison), so both rows are `ported`. Since
310 `/stun` runs over the wire (the usage, the miss, the lowered landing with
the confirmation, and the target's add), so the row is `ported`. Since 311
`/warp` and `/rewarp` run over the wire (the usage, the miss, the
cross-Channel line, the lowered landing on the truncated metres, the metres'
warp, and the rewarp's own pixels), so both rows are `ported`. Since 312
`/goto` runs over the wire (the usage, the silent miss, the metres' `Show`
with the view re-sent and no warp, and the name's warp without the spurious
line, a Defect), so the row is `ported`. Since 313 `/level` runs over the
wire (the syntax, the reset's 64 records with the conqueror-off gate first,
the two skill bytes, the row's starts/counters/pools, and the disconnect
save), so the row is `ported`. Since 314 `/setskill`, `/setskillother`,
`/setskillpoint` and `/setjob` run over the wire (the syntaxes, the misses,
the vnum and name sets with their compute and byte 76, the other's silent
caller, the counter's amount-0 change, the job's byte 112 with its clear and
confirming line, and the disconnect save), so all four rows are `ported`.
Since 315 `/invisible` and `/cooltime` run over the wire (the refusal, the
203's add and remove with the flag-3 updates on both wires, and the silent
cooltime), so both rows are `ported`. Since 316 `/affect_remove` runs over
the wire (the refusal, both syntaxes with the three-row list, the unknown-name
fallback, the 203's remove with the flag-3 fall, the success and the miss), so
the row is `ported`. Since 317 `/stat_reset` runs over the wire (the refusal,
the one kind-112 record, and the disconnect save banking 12), so the row is
`ported`. Since 318 `/purge` and `/weaken` run over the wire (both refusals,
the weaken's 10-HP broadcast to the targeter, the near dog's release and
removal, and `/purge all`'s sweep of every NPC in view with the far dog's
pair to its viewers only), so both rows are `ported`. Since 319 `/advance`
runs over the wire (the refusal, the syntax, the miss, the caller's
silence, the target's 64-record reset, and the row written straight to the
target), so the row is `ported`. Since 320 `/reset` runs over the wire (the
refusal, the hit refill, the spell refill, and the save banking the filled
pools), so the row is `ported`. Since 321 `/greset` runs over the wire (the
refusal, the quiet refill to the maximum, and the row standing at 400), so
the row is `ported`. Since 322 `/user` runs over the wire (the refusal, the
caller's Channel in cells of 17, and the total), so the row is `ported`.
Since 323 `/who` runs over the wire (the refusal, the empires across every
Channel, and this server the caller's), so the row is `ported`. Since 324
`/respawn` runs over the wire (the refusal, the around and the everywhere,
and the holding regen's due moved), so the row is `ported`. Since 325 `/mob`
runs over the wire (the refusal, the usage, the miss, and the stood dog at
126 of 126), so the row is `ported`. Since 326 `/item` runs over the wire
(the refusal, the usage, the two misses, and the potion's set and row), so
the row is `ported`. Since 327 `/mob_ld` runs over the wire (the refusal,
the same usage, and the dog at the metres), so the row is `ported`. Since
328 `/ma` and `/mc` run over the wire (the refusals, the two usages, the
miss, the hunting dog's swing, and the coward's escape), so both rows are
`ported`. Since 329 `/mm` runs over the wire (the refusal, the syntax, the
two failures, and the stood line's name and centimetres), so the row is
`ported`. Since 330 `/group` and `/grrandom` run over the wire (the
refusals, the two usages, the silence, and the stood dogs), so both rows
are `ported`. Since 331 `/forgetme` and `/aggregate` run over the wire
(the refusals, the swing, the 5 quiet seconds, and the hide), so both
rows are `ported`. Since 332 `/attract_ranger` and `/pull_monster` run
over the wire (the refusals, the nearer sync, the held far dog, and the
archer's walk in to 150), so both rows are `ported`. Since 333 `/getqf`,
`/setqf` and `/delqf` run over the wire (the refusals, the named state,
the confirmation, the miss, the success and the failure, and both
list), so both rows are `ported`. Since 337 `/view_equip` runs over the
wire (the 1303-byte record with the dagger at slot 4, the empty silence,
the unknown silence, and the block refusal), so both rows are `ported`.
Since 338 `/stat_val` runs over the wire (the banked amount, the clamps,
and the silences), so the row is `ported`. Since 339 the four `*+` lines
run over the wire (the banked amount, the 90 clamp, and the loud refusals),
so the four rows are `ported`. Since 340 the four `t*` lines run over the
wire (the moved other, the cannot-find arms, and the confirmation), so the
four rows are `ported`. Since 341 `/cannot_dead` and `/can_dead` run over the
wire (the silent lines, the flagged damage without the change, and the change
after the clear), so both rows are `ported`. Since 342 `/all_skill_master` runs
over the wire (the one byte 76 with every usable skill at 40, and the rows),
so the row is `ported`. Since 343 `/full_set`, `/item_full_set` and
`/attr_full_set` run over the wire (the eight fresh pieces per job with the
looks, the forty forced pairs with the updates, and the master line first),
so the three rows are `ported`. Since 344 `/use_item` and `/do_clear_affect`
run over the wire (the named cell's drink with its store, the no-item line, the
203's remove packet with one update, and the kept silence standing), so both
rows are `ported`. Since 345 `/dragon_soul` and `/ds_list` run over the wire (the
deck's affect with its stones' points and socket-2 updates, stored, and the held stones'
lines), so both rows are `ported`. Since 346 `/dice` and `/gift` run over the wire (the
roll to the caller alone or the party with the name, and the `gift` command line), so
format with its value and the `[LS;651]` date, the status with the hair's attribute, and
the named piece's takeoff through `use_item`, stored), so both rows are `ported`. Since 348
`/state` and `/inventory` run over the wire (the dump with its target and the cells'
lines), so both rows are `ported`. Since 349 `/slow` and `/ipurge` run over the wire (the
ten numbered armours and the purge's clear), so both rows are `ported`. Since 350 `/console`
and `/book` run over the wire (the console's line and the book's skill in socket 0), so both
rows are `ported`. Since 351 `/set` moves a named character's gold, exp, maxima and skill over
the wire, so the row is `ported`. Since 352 `/set` moves race, sex, job, alignment and gaya over
the wire, so the row stays `ported`. Since 353 `/safebox` sizes over the wire (the empty word,
the 2-page grow, and the past-3 clamp), so the row is `ported`. Since 354 `/makeguild` and
`/deleteguild` run over the wire (the fee-less founding with its codes and the disband's REMOVE),
so both rows are `ported`. Since 355 `/refine_rod`, `/refine_pick` and `/max_pick` run over the
wire (the rod's swap, the pick's max-then-swap, and the max alone), so the three rows are
`ported`. Since 356 `/fish_simul` runs over the wire (the usage, the byte-ordered name
lines with their summary, and the past-end table's usage alone), so the row is `ported`.
Since 357 `/private` warps over the wire (the usage, the miss, and the copy's spawn with
the row naming the copy and the exit), so the row is `ported`. Since 358 `/reload` runs
over the wire (the empty line's three tells, the `u` arm's `state_user_count` reread feeding
the status list's signed rule over the registry's fresh count, the `p`/`s`/`q` tells, the
`f` arm's fish swap with the GM notice, the `t`/`m` arms' read-first swap with the Romanian
miss/success lines, and the silent `c` and unknown arms), so the row is `ported`. Since 359
`/ds_change_attr` redraws over the wire (the clears, the weighed basic and drawn additional
applies with the updates riding and the rows saved, the step's flames and 5 million spent,
the success line and the result byte, and the gates' own lines and silences), so the row is
`ported`. Since 360 `/shutdown` counts down over the wire (the `[LS;509;10]` to every
Channel, each `[LS;508;N]`, the 0 tick's `quit` to the players with the `SHUTDOWN` bar at
auth and login, and the -10 tick's force-close; a line after the bar restarts nothing, and
a line mid-countdown restarts at 10), so the three rows are `ported`. Since 361
`/logout`, `/quit` and `/phase_select` count down over the wire (each notice, the silent
`IsHack` gate skipped on SungZi and war maps, the second line's `[LS;511]` cancel, each
`[LS;510;N]`, and at 0 the close with a save, the bare `quit` line, and the return to
select with the reselect on the same connection), so the five rows are `ported`. Since 362
`/pvp` duels over the wire (the challenge with its fan, lines and whisper notify, the
accept with its fight fan and begins lines, the miss's silence, the won fight's revenge
fan with the agreed death skipping the item-drop penalty, and the late entrant's list),
so the two rows are `ported`. Since 363 `/detaillog` and `/monsterlog` log over the
wire (both silent toggles, the begin's three lines in order with the thief's VID, the
off toggle's silence, and the detail line naming the hitter's name, VID and enter
point), so the two rows are `ported`. Since 364 `/polymorph` and `/polyitem` work over
the wire (the reencode with the morph race, the spawn state bit and `AFF_SPAWN`, the clear's
job race, an unchanged vnum's silence, and the 70104 ball with the vnum in socket 0 and its
row), so the two rows are `ported` and `sys.char.polymorph` is `partial`. Since 365 an event
flag's set runs `SetEventFlag`'s arms (the six rates, 0 included; the Christmas, `newyear_boom`,
`eclipse` and `day` lines to every in-game character; the firework sellers, the tree, the Santa,
`pre_event_hc`'s and `new_xmas_event`'s NPCs), a loading character hears the non-zero display
flags (`BroadcastEventFlagOnLogin`), and `/xmas_boom`, `/xmas_snow`, `/xmas_santa`,
`/eclipse` and `/eventhelper` work over the wire, so the five rows are `ported` and
`sys.event.xmas` is `partial`. Since 366 `/socketitem` gives its item by `AutoGiveItem` and
writes its leading sockets over the wire, so the row is `ported`. Since 367 `/item_id_list`,
`/get_mob_count`, `/effect` and `/add_socket` work over the wire, so the four rows are
`ported`, and `/set_socket` writes a held or lying item's socket, so its row is `partial`
(an open safebox's or mall's item is not found). Since 368 `/change_attr` and `/add_attr`
roll the worn weapon's attributes over the wire, so both rows are `ported`. Since 369 the
boot loads the `item_attr_rare` rows, and `/add_rare_attr` and `/change_rare_attr` draw the
worn weapon's rare pair over the wire, so both rows are `ported`. Since 370
`/reset_subskill` clears a named character's sub-skills and `/force_logout` closes the
caller's own descriptor, so both rows are `ported`. Since 371 a conqueror's kill on a
new-world map pays champion experience, whose steps grant conqueror points and raise the
champion level; `/conqueror_level`, `/conqueror_point` and `/set exp` drive it over the wire,
the step and point are stored, and the quest reads `pc.get_conqueror_exp` and
`pc.get_conqueror_next_exp` answer, so `sys.skill.conqueror`, both command rows and both
quest rows are `ported`. Since 372 `/sort_inventory` lifts the base inventory and gives it
back by locale name, merging plain stacks, so its row is `ported`. Since 373 item 70002
grants the realtime pick-up affect, `/pickup_affect` switches it off and on, and while it is
on a kill's drop goes straight into its owner's inventory, so `cmd.pickup_affect` and
`sys.item.pickup_effect` are `ported`; a logout now saves the affects it holds. Since 374
`/move_channel` counts down and warps the character to the same place on another Channel
that hosts its map, so `cmd.move_channel` and `event.char.move_channel` are `ported`, and a
warp within 15 seconds of an inventory sort is refused as legacy refuses it. Since 375
`/flush` writes the caller's row at once, so `cmd.flush` is `ported`. Since 376
`/observer` hides the caller with the view dance and `/observer_exit` warps to the saved
place before clearing the mode, with the party, messenger, PvP, skill, pick-up, quest-item
and war-map arms, so both rows are `ported`. Since 377 a bow shot lands over the wire and
its arrow spend is stored as one Transfer, so `cg.game.shoot` and `cg.game.use_skill` are
`ported`. Since 378 `/saveati` writes the named map's attributes as a TGA, so `cmd.saveati`
is `ported`. Since 379 the client switches its language with `CG_CHANGE_LANGUAGE`, which is
answered, stored, and shown to the view, and a login-phase move is consumed, so
`cg.game.change_language`, `gc.request_change_language` and `cg.login.move` are `ported`. Since
380 a destroy clears the stack's cell, deletes its slots, destroys its row and names it in
`[LS;1947;...]`, and the enter-game step answers a missing or wrong client version with the
conflict notice and a close, so `cg.game.item_destroy`, `cg.login.client_version`,
`cg.login.client_version2` and `cg.game.client_version` are `ported`. Since 381 a gift runs
an NPC's whole `CanReceiveItem` with its `TakeItem` log line and take scripts, and a warp
within 15 seconds of an inventory sort is refused with the line, so `cg.game.item_give` is
`ported`. Since 382 the event panel names a quest whose button runs, so
`cg.game.request_event_quest` is `ported`. Since 383 a whisper partner's details name its
language on any Channel, and a language switch reaches the client registry, so the shout and
the notices follow it; `cg.game.whisper_details` and `gc.whisper_details` are `ported`. Since
384 the target window lists what a monster or stone may drop, so `cg.game.target_info_load`
and `gc.target_info` are `ported`. Since 385 a smith takes a refinable item, opens its
window with the recipe and refines it for its fee, so `cg.game.refine`,
`gc.refine_information_new` and `table.refine_proto` are `ported` and `sys.item.refine` is
`partial`. Since 386 a stone used on a weapon or armour sits in a free socket and a
tuning scroll used on an item opens the refine window of its type and refines with its own
gates and odds, so `cg.game.item_use_to_item` and `sys.item.sockets` are `partial` (the
other target arms, such as the detachment and attribute scrolls, are not ported). Since 387
the event calendar is stored and installed at boot and a client's month ask is answered with
the board, so `cg.game.request_event_data` and `gc.event_info` are `ported` and
`sys.event.manager` is `partial` (the start/end queues and the state arms are not ported). Since
388 a script's `select_item` and `confirm` wait for the client's `CG_SCRIPT_SELECT_ITEM` and
`CG_QUEST_CONFIRM` (with `GC_QUEST_CONFIRM` and the confirm timeout), and the guild alchemists'
diamond and stone melts (`pc.diamond_refine`, `pc.ore_refine`) run over the wire. Since 389 a
bait goes on the worn rod, `CG_FISHING` casts and takes, and the fishing event bites and lets
the fish get away, so `cg.game.fishing`, `gc.fishing`, `event.fishing.fishing_event` and
`data.fishing` are `ported` and `sys.life.fishing` is `partial` (the multi-farm gate, the
`fishevent` highscore and the premium fish mind wait for their systems). Since 390 `pc.mining`
digs a clicked ore vein with the worn pick: `GC_DIG_MOTION` goes around, every `OnMove` and a
second call cancel the dig, and the mining event drops the raw ore or fails and trains the
pick, so `lua.pc.mining`, `gc.dig_motion` and `event.mining.mining_event` are `ported` and
`sys.life.mining` is `partial` (the multi-farm gate waits). Since 391 a quest's `__refine_pick`
and `__fish_real_refine_rod` refine the tool in a named inventory cell and return 1, 0 or 2,
so both are `ported`. Since 392 a character's horse keeps its level, health, stamina and drop
time on `player`, regenerates from `EnterHorse`, and is refilled at every login with its skill
130 planted again (`__FIX_NIVEL_CAL__`, owner question 35); `/horse_state`, `/horse_level` and
`/horse_set_stat` and the quest's horse readers, `horse.set_level`, `horse.advance` and
`horse.feed` run over the wire, so those rows are `ported` and `sys.char.horse` is `partial`
(the summon, riding, the consume and death events, revive and the names wait). Since 393 the
horse is summoned and sent off by `/horse_summon`, `/horse_unsummon`, `/user_horse_back` and
the quest's `horse.summon` and `horse.unsummon`, follows and wanders by `StateHorse`, lies a
minute when summoned dead, goes when it dies, and is revived by `horse.revive`,
`pc.revive_horse` and the herb and fed by a gift, so those rows and
`event.char_horse.horse_dead_event` are `ported` (riding and the names wait). Since 394 the
horse is ridden by `/horse_ride`, `/user_horse_ride`, `horse.ride` and `horse.unride`: the
mount shows in the re-show and the update, the compute pays the horse floor and bonus, the
stop sends the four zero changes and the far summon back, and death, polymorph and the
stamina end stop the ride, so those rows, `lua.horse.is_riding`,
`event.horse_rider.horse_stamina_consume_event` and a live `lua.pc.is_mount` are `ported`
(the riding gates, the arena and war silences, the feed line and the names wait). Since
395 the horse is named by `horse.set_name` and `horse.get_name`: the europe judge, the
flag with the month's `AFFECT_HORSE_NAME` affect, the rewrite with the summon back, the
manager's name on the summon, the mounted armour floor with the name's 20, and
`Validate`'s tick and stale re-summon, so both rows are `ported` (the riding gates, the
arena and war silences and the feed line wait). Since 396 `/user_horse_feed` spends one of
the grade's food on the summoned horse and names it, or names it missing, or answers that no
horse stands, so its row is `ported` (the riding gates and the arena and war silences
wait). Since 397 the riding gates read the ride: a rider neither emotes nor is kissed, steps
past the walker's limit, draws for the horse book, casts and computes only what its horse
grade allows, and puts on no body armour, costume body or ride item; a morph wears nothing
but arrows; and `pc.is_riding` is bridged, so its row is `ported`. A target-less emotion's
`[LS;436]` now ends the line, as legacy's does. Since 398 a rider on a grade-1 horse hits
no monster and, outside a guild war, no player (`CanAttack`'s horse gate). Since 399 a
computation lays every held affect's point again (`RefreshAffect`), and `pc.mount`,
`pc.mount_bonus`, `pc.unmount` and `/unmount` mount and dismount; a login takes a saved mount
off again. Since 400 `/ride` stops a ride, dismounts, rides a summoned horse or uses the first
ride item or seal it holds. Since 401 the 366 quest-api names no Lua file references are
`unused`, pending owner question 37; 111 called names remain. Since 402 a kill scores its
counter through `napierdalator`, the board mails at login, `/open_global_rank` lists it,
`/rank_end_season` awards and empties it, and 20016's chat claims the prize once (also
correcting the `data.lua.unread` finding: `settings.lua:174` dofiles `BlueDragon.lua`). Since 403
a party enters its forest through `d.new_jump_party`, and the report reads back the index, the
flags, the event flag, a notice and a spawned dog; the cleared exit never fires and the second
sends both home (`d.get_map_index`, `d.setf`, `d.getf`, `d.getf_from_map_index`, `d.notice`,
`d.set_warp_location`, `d.exit_all`, `d.spawn_mob`, `game.set_event_flag`, `server_timer`,
`clear_server_timer`, `get_server_timer_arg`). Since 404 a solo login takes its own
dungeon at the entry (`d.new_jump`, unscaled where the party jump scales), the tail clears
nothing and kills the dog (`d.clear_regen`, `d.kill_all`), and the flagged hit is silent
(`d.set_damage_from_race`, `d.set_damage_from_affect`, which ride the character past a
warp and die on disconnect). Since 405 a script's empire exchange moves the account
(`pc.change_empire`, `pc.get_change_empire_count`, `pc.is_polymorphed`, `pc.is_engaged`,
`pc.is_married`, `char_log`, `sys_err`): the session shadows the refusal, the world judges it
again after the step, and the re-show carries the new empire. Since 406 a used quest
item reads its name, size, refined vnum and refine level and removes itself
(`item.remove`, `item.get_name`, `item.get_size`, `item.get_refine_vnum`,
`item.get_level`): the removal lands in the call, like `set_socket` does. Since 407 a
script sets its window's clock and counter (`q.set_clock_name`, `q.set_clock_value`,
`q.set_counter_name`, `q.set_counter_value`), which go out in one `GC_QUEST_INFO`
when it ends. Since 408 the boot keeps the war reservations with their bets
(`guild.get_reserve_war_table`, `guild.is_bet`, `guild.war_bet`), an observer warps to
the war's watchers' start (`guild.get_warp_war_list`,
`pc.warp_to_guild_war_observer_position`), and the bet quest's format error is recorded:
no bet completes through it, on legacy either. Since 409 a regist moves a weapon or armor
into the 6th/7th-bonus window (`CG_ATTR67_ADD`), spends the level's fragment and the
support, and rolls the success the quest reads back (`attr67add.holding`,
`attr67add.success`, `attr67add.collect`, `attr67add.item_vnum`, `attr67add.window_type`,
`attr67add.window_cell`): the wait page, the won roll's rare attribute, and the collect
home. Since 410 a script reads its party's cached near count (`party.get_near_count`),
which the Update's resend keeps. Since 411 `/cards` opens, pulls, ends, destroys,
accepts and restores the Okey hand, the ended games log to the Okey table, and the owner
quest's chats read both ranks (`pc.get_okay_global_rank`, `pc.get_okay_rund_rank`). Since 412 a script reads the tail names (`pc.get_player_id`, `pc.get_account_id`, `pc.get_empty_inventory_count`, `pc.is_trade0`, `pc.is_busy0`, `pc.give_exp2`, `pc.set_skill_group`, `pc.select`, `pc.get_killee_drop_pct`, `pc.warp_local`, `game.drop_item`). Since 413 a script reads the tongue (`LC`), broadcasts a notice (`notice_all`) through the quest translations, runs a line (`command`, battle-pass only), re-arms the Ochao temple (`temple_ochao.initialize`), and moves its quest (`set_quest_state`). Since 414 a click can land an OX quiz and `/show_quiz` lists what landed, `npc.get_guild` answers 0 with no building standing, and `timer` and the party cinematics are `unused` (no owner Lua calls them). The
Since 415 a click can open the Gaya window and market, the aura windows and the exp event (with its priv grant), and `gc.aura` carries the open. The
Since 416 a guild master claims its land at the claim stone and rebuilds its refinery, through the five `building.*` calls over the dumped lands and prototypes. The
Since 417 a guild registers for Meley's Lair, enters it, and leaves it: the
lobby's nine quest names run their `CMgr` gates, the ladder debit and the `[LS;1001]` line land, and the
enter warps to the recall spot on the run's private index. Since 418 a captain boards the ship defence, starts it and lands it: the thirteen `ship_defense_mgr` names are bridged and judged, the deck stands, the waves run, and the heal ignores its argument as legacy does. Since 419 the guild gate click starts Meley's fight (`meley_gate_click`, the two guild lines, the gate's death, step 1 and the hour), the statues flag at their HP line and strip on the kill limits, seals break step-4 statues, and `MeleyLair.CanGetReward`/`MeleyLair.Reward` bridge the chest's take. Since 420 the ship's six ticks and Meley's four carry pulse unit tests, and their ten timed-event rows are
`ported`. Since 421 the temple's two ticks are `ported` too: its portal clears at 60 s, as
legacy does, not the 120 s ledger 413 had written. Since 422 the poison and fire ticks, the guild invite lapse, and the party's request, invite and update ticks are
`ported`. Since 423 the dungeon's dead tick is `ported` and its two eliminate timers are
`unused` (owner question 39). Since 424 the mob hit tick is `ported` and the gwihwan timer is
`unused` (owner question 40). Since 425 the save, ping and disconnect ticks are
`ported`, each proved by its binary scenario. Since 426 the server quest timer is `ported` and the per-player timer is
`unused` (owner question 41). Since 427 the war map's begin, timeout, end and flag-reset ticks are
`ported`. Since 428 the three monarch timers are `unused` (owner question 42). Since 429 the three quest warp timers are
`unused` (owner question 43). Since 430 the spam reload timer is `unused` (owner question 44). Since 431 a lapsed Gaya market refreshes on the 2 s tick, and the row is
`ported`. Since 432 the marriage and wedding timers are `unused` (owner question 45). Since 433 Santa hops maps and returns on the timer, and the row is
`ported`. Since 434 a used campfire stands ahead, burns 40 s and goes out, and the row is
`ported`. Since 435 the mount, pet and shaman timers are `unused` (owner question 46). Since 436 the dungeon-regen, watchdog, gated-system and skill timers are `unused` (owner questions 47-50). Since 437 the six item expiry timers are `unused` (owner question 51). Since 438 the arena, lair and siege timers are `unused` (owner questions 52-53). Since 439 the OX, spam, war, boss and event-manager timers are `unused` (owner questions 54-56), and no `missing` timed-event row is left. Since 440 the jigsaw fish event is
`ported`: the report, the two arms, the reward and the save round-trip. Since 441 the daily
gift is `ported`: the line, the week, the cash and ticket collects, the renew and the save. Since
442 the sash's add, remove and refine are `ported`: the window, the guards, the combine and the
absorption over the wire. Since
443 the changelook's add, remove and refine are `ported`: the guarded pair, the stamp, the
clean and the banked look. Since 444 the Gaya craft and market are `ported`: the craft's
guards and spend, the shelf's guards, buy and unlock, the refresh, and `/w_gaya`. Since
445 the biologist is `ported`: the ten missions, the takes, the waits, the picks, the
rewards, and the save round-trip; and every creation stamps the `SIGVnum`, a worn unique
or ring appends its attr rows and answers its effect path, and a stamped quest item runs
its group's `sig_use`. Since 446 the protected inventory is `ported`: the lock and the
unlock, the password change, the twenty gates, the kick's auto-lock and the save
round-trip. Since 447 the premium players are `ported`: the window, the list, the
activation, the minute sweep and the save round-trip. Since 448 the cube is `ported`:
the recipes, `/cube open`, the make and the close, and `/reload c`. Since 449 the
inventory unlocks are `ported`: the knock, the price list, the mixed spend, the shortfall
chat, the `POINT_INVEN` rise and the save round-trip. Since 450 the element refine is
`ported`: the scroll's window, the draw, the downgrade and the change, and 255 closes.
Since 451 the party roles are `ported`: the leader's set and unset, the `[LS;671]` and
`[LS;672]` gates, the capped quiet, and the NORMAL and role-8 quiet. Since 452 the
attr67 arms are `ported`: the open and regist ride the waits-and-collects scenario, and
the close shuts the window silently. Since 453 the daily-gift arms are `ported`: the
open, collect and ticket ride the collects scenario, and the close clears the open week.
Since 454 `sys.custom.multi_farm` and `cmd.multi_farm` are `ported`: the toggle with its
10 s protect, the per-address registry, the two-farming cap, the fan-out, the premium use,
and the mining, fishing and drop gates. Since 455 `cmd.advance_skill_select` and
`cmd.do_click_special_inv` are `ported`: the select's syntax and `YOU_CAN_NOT_DO_THAT`
gates, the set and the clear with no confirmation line, and the click's
`ShowSpecialInventory`. Since 456 `cmd.mount_test` is `ported`: the word's vnum
mounts through the ported `MountVnum`. Since 457 `cmd.fetch_casket_items` is `ported`:
the group's rows and the sort. Since 458 `cmd.break_marriage` and `cmd.close_shop` are
`ported` and `cmd.special_item` is `unused` (a local file conversion no client observes).
Since 459 `cmd.temp` and `cmd.check_mmoney` are `ported`: the test-server gate, the
guards, and the store install. Since 460 `cmd.mto`, `cmd.mtr`, `cmd.mtax` and
`cmd.mmob` are `ported`: the monarch, cooldown and money gates, the warps, the
two tax broadcasts, and the summon list. Since 461 `cmd.setmonarch`,
`cmd.rmmonarch`, `cmd.minfo` and `cmd.mnotice` are `ported` (implemented in 460's
code commit, inventoried here) and `cmd.elect` and `cmd.rmcandidacy` are
`unused` (the election tally reads nothing and its answers have no game-side
reader). Since 462 `cmd.reward_system` and `sys.custom.reward` are `ported`: the
`RewardData` open pair, the solo/global row tables with the completion prize, and
the kill and level-up call sites. Since 463 `cmd.dungeon_info` is `ported`: the
six-row table, the store-read `dungeon_log_info` rank, the `dungeon_info_cooldown`
line with the `test_cooldown` stamps, and the `Damage` arm. Since 464
`cmd.log_oxevent`, `cmd.get_oxevent_att` and `sys.event.ox` are `ported`: the enter
seat with the multi-IP kick, the roster pair, and the warp-keep/disconnect-drop
leave. Since 465 `cmd.threeway_info` and `cmd.threeway_myinfo` are `ported` and
`sys.event.threeway_war` is `partial`: the full forked map index, the death hook up to
the score notice, and the forked-map penalty skip; the elimination and victory arms are
next, with the two timers ledger 465 moved back to `missing`. Since 466 that is
corrected: the owner's map index loads none of the forked maps 114-128, so the hook never
runs on the owner's data, the two timers are `unused` again (owner question 58), and the
arms wait on the map folders. Since 467 the GM arena duel on map 112 is live: `/show_arena_list`,
`/duel`, `/end_duel`, `/end_all_duel`, the ready countdown, `GC_DUEL_START` and the time-out are
`ported` and `sys.event.arena` is `partial`; `CanAttack` and `OnDead` scoring is next, then the
observers, the login re-register and the `IsArenaMap` hooks. Since 468 duelists can hit each
other and an arena death scores a round, restarts it or ends the match; the observers, the login
re-register and the `IsArenaMap` hooks (with `/pvp`'s arena gate) are next. Since 469 those
are live too: the login arms, `/pvp`'s gate, the item, potion and horse refusals and the potion
limit are `ported`, arena observers are confirmed unreachable (owner question 60), and only the
battle arena keeps `sys.event.arena` `partial`. Since 470 the battle arena is live too:
`/weeklyevent`, its eight states, the stone and boss spawns and the home warp are `ported`, and
`sys.event.arena` stays `partial` only on the guards that wait for other systems. Since 471
`/siege` and `/frog` are `ported`, refusal-only because none of the castle maps 181-183 loads,
and `sys.event.castle_monarch` is `partial` with the siege core and the monarch powers waiting
on those maps. Since 472 the world boss is live: `/worldboss`, its flag bridge, the three
timers, the spawn, escape, kill, damage, reward and ranking arms, the CG/GC 148 records and the
boot table are `ported`, with the season bank dropped as website-only. Since 473 the battle
pass is live: `/battle_pass` and `/battle_pass_admin`, the 158-mission boot table, the mission
buckets, the kill/use/sell/craft/drop hooks and the 238 record are `ported`; 238 is
client-unhandled, so the send is wire-only, and the MESSAGES/CHEST_OPEN/PLAYTIME arms wait on
their unwired paths. Since 474 `/build` and `/clear_land` are `ported`: the six arms, the
land gates, the proto gates, the spend and the drain, with the wipe, the owner drop and the
claim stone; the single and block walls parse and run but the owner's 41 object protos name
no wall vnum, so like legacy they stand nothing there. Since 475 `gc.land_list` is `ported`:
the enter-game list and the ownership fan, with the guild NAMEs leading each. Since 476
the stood buildings show: the object entity enters its sectree on the stand, so the ADD
and the DEL fan, through a `building_places` reverse map and a shared ADD encoder; the
attribute region the legacy `Show` sets is not ported, and the Rewrite walks by
server-attr cells, which no object writes. Since 477 `sys.char.empire_change` and
`sys.world.channel_move` are `ported` (their scenarios already covered them),
`sys.world.time` is `partial` (the login TIME is ported, the quest time is backed but
scenario-less, and no day/night driver exists in legacy), and `sys.net.udp_block` is
`unused` (legacy compiles the UDP socket out); questions 59 and 60 ask about the mall's
missing website and the UDP row. Since 478 `sys.item.blend`, `data.item.blend` and `event.item.blend_use_expire_event` are `ported`: the use lays, refuses and toggles, creation rolls the owner's 24 rows, and the tick spends; the item lock is not ported and the GM grant path rolls no sockets. Since 479 `sys.gm.priv` and its eight quest calls are `ported`: `GetPriv` whole (the character loss, the empire and guild max), the quest gives queue, and the getters shadow the step; no quest calls them. Since 480 `cg.login.login` is `ported`: the live server refuses
`VERSION` under the handshake key and the wire stays open, and past the gate the record
waits silent, since `HEADER_GD_LOGIN` has no DB-side handler. Since 481 `gc.login_failure`
is `ported` and `sys.login.by_key` lost a `Left`: with `user_limit = 1`, one character in
game fills the server and the second `LOGIN2` hears `FULL` under the handshake key. Since
482 the three world-boss sub-headers are `ported`: they were live since 472 with the event
system, and the scenario sends all three arms (`INFO`, `REWARD`, `RANKING`). Since 483
`gc.item_use`, `gc.mount` and `gc.character_add2` are `unused`: the first two have an
encoder and a struct no sender calls, the third is named nowhere in the checked-in trees.
Since 484 `gc.dungeon`, the hybridcrypt pair and `gc.login_success3` are `unused`: the
dungeon arms need a quest call nobody makes, the crypt pair needs a `package/` dir the
snapshot lacks, and byte 6's send writes byte 32 instead. Since 485 `data.item.ori_to_new`
is `ported`: the target window shows the mask's vnum for a drop that names the new one. Since 486
`sys.item.stack_attribute` is `ported` and `event.item.real_time_expire_event` is `ported`: a kill
feeds the worn skin's row and the flood refuses a use within the second, and a passed real-time
item leaves held, worn or stored. Since 487 six `gamedata.md` rows are `ported` without new
code: the stack table (486's reader, unmarked), the skill power (whose file no legacy path
opens — the rows live in `common.sql`), the dragon soul table, the battle pass, the exp
tables and the locale table, each live since its system's ledger. Since 488 `cg.game.aura`,
`gc.aura` and `sys.custom.aura` are `ported`: the absorb window checks in, previews, checks out,
accepts and cancels, with the locks and the death close; growth and evolve wait for their systems. Since 489 `cg.game.dragon_soul_refine` is `ported` and `sys.custom.dragon_soul` is `partial`: the window opens, closes and refines grade, step, strength and all, with the gamble, the fee, the spend, the made stone's drawn attributes and the strength refresh; the extract and the heart charge stay. Since 490 `cg.game.switchbot`,
`gc.switchbot` and `sys.custom.switchbot` are `ported`: a sword registers on a slot, starts with
alternatives, and the 2-pulse dice spends a switcher per miss, changes the attributes, and always
flushes the spend and the item copy; the finish whispers, the empty bot stops, and a warp stills
the register. Since 491 `cg.game.myshop`, `gc.shop_sign`, the two Switchbot sub-headers and the armed `cmd.close_shop` are `ported`: a sword and a paper bundle open a bundle shop for 500 yang under the sign `Swords`, the sign reaches a witness, and `/close_shop` clears it and shows the job again; the two Switchbot arms were live since 490. The
Since 492 `sys.trade.private_shop` is `partial`: a click on the seller opens its bundle window, and a buy of slot 3 moves the sword by id, charges the full 500, pays the seller whole without the flag, trails the sold update on both settled sides, and settles both sides in one transaction; a rebuy answers sold out, and the tax, the treasury and the refusals are unit-tested. The
Since 493 `sys.trade.private_shop` is `partial`, with its own scope back: the panel opens and closes, and a build stands the shop on the empire's market map, stores the shop row and the wares in one transaction, and sends `LOAD` and the stand's `ADD_ENTITY`; the bundle buy ledger 492 filed there is `sys.npc.shop`'s PC arm instead. Since 494 the browse and the buy are `ported`: the window opens by the stand's VID and closes with `END`, and the buy settles the ware, the gold and the stash in one transaction and tells the viewers, the owner and the buyer. Since 495 the close, the withdraw and the modify are `ported`: the modify flips the state, the withdraw empties the stash and destroys the empty shop, and the close returns the wares biggest-first, cleans the viewers and deletes the rows. Since 496 the price, the move, the checkin and the checkout are `ported`: the four share the modify gates, the price and the move sync to the owner and the viewers, and the checkin and checkout move the ware between the window and the shop with the row to match. Since 497 the rename and the unlock are `ported`: the modify-state title change reaches the owner and the stand's neighbours, and the key at the named cell spends through `RemoveSpecifyItem`, growing the stand's or the point's count and telling the viewers; the build's and checkin's `i32` owner bind, which poisoned sqlx's per-connection statement cache for the unlock's `i64` destroy on the same text, now binds `i64` too. Since 498 the warp request and its countdown are `ported`: the owner closes its panel, zeroes the shop time and counts 3 calm or 10 fighting, and at 0 warps to the stand's spot past `CanWarp`, with the DB peer's channel match collapsed into the ownership since one process holds the shop. The
Since 499 `sub.PrivateShop.SUBHEADER_CG_PRIVATE_SHOP_SEARCH_CLOSE` and `sub.PrivateShop.SUBHEADER_CG_PRIVATE_SHOP_SEARCH` are `ported`: the glasses open the search window, a search answers the matching wares with `SEARCH_RESULT` past the stun and the two-second wait, and a sale, a flip and a checkout tell the searchers with `SEARCH_UPDATE`. The
Since 500 `sub.PrivateShop.SUBHEADER_CG_PRIVATE_SHOP_SEARCH_BUY` is `ported`: the ten picks check in order against the standing shops and settle through the shared settlement, without browsing. The
Since 501 the two market-price sub-headers are `ported`: a sale notes its price, the hourly tick averages the pending sales with the standing price, and both requests price from the averages, zero until a tick folds a sale in. The sub-header table is fully `ported`. The
Since 502 `cg.game.private_shop` and `sys.trade.private_shop` are `ported`: every dispatched client arm is live, and `STATE_UPDATE` has an enumerator but no dispatch arm, so the client cannot send it. The
Since 503 `sys.item.addon` is `unused`: the code is ported and unit-tested, but the 152 protos with a nonzero `sAddonType` never reach an honest client on the owner's data. The
Since 504 `gc.private_shop` is `ported`: `DEL_ENTITY` is encoded and relayed on close, and the ledger-494 off-by-one (`START` through `REMOVE_MY_ITEM` sent as 11-14, legacy's 10-13) is fixed. The
Since 505 `sys.custom.new_set_bonus` is `partial`: the energy buff sums the worn attributes per type and lands each sum's energy percent before the energy's own record, with the wear, take-off and recompute arms. The
Since 506 `cmd.set_socket` is `ported`: a socket write to an item in an open safebox or mall reaches the opener and the account row. Since 507 `cmd.in_game_mall` is `ported`: any character hears the signed web-shop line, and the command table has no `missing` row left. Since 508 `cg.any.analyzer_refusal` is `ported`: an undersized variable frame closes the connection end to end. Since 509 `sys.item.casket_preview` is `ported` without new code: the fetch arm is live and the quest preview map is provably empty. Since 510 `sys.guild.bonuses` is `ported`: the four level windows land their affects through all six seams, the overlaps resolve first-match, and the removal's first-kind break keeps the stale rows while guildless. Since 511 `sys.net.ip_ban` is `ported`: the `[access]` table's `ban_ip` ranges refuse at the auth listener with the `BANIP` mask-and-hash match, and its `block_country_ip` ranges refuse at Channel login before the shutdown gate unless `block_exception` names the lowercased login. Since 512 `sys.dungeon.info` is `ported`: the admit fans the full cooldown line after the recovery event starts, and every login reader hears it ahead of the rank lines. Since 513 `sys.adminpage` is `unused`: byte 64 is registered but undispatched, `IsAdminPage` has no caller, the password is only logged, and the Rewrite closes on the byte. The Since 514 `sys.char.mount` is `ported`: a used mount costume summons its follower, `/ride` mounts and unmounts it, and the take-off, login, restart, leave, tick, purge and state-idle arms are live. Since 515 `sys.custom.pets` is `ported`: a used pet costume wears and stands its follower with the mob's own name and the 150 speed, and the take-off, login, restart-here, leave, tick, purge and state-idle arms are live. Since 516 `sys.item.attr` is `ported`: the four bonus scrolls redraw, add and rare-change their target with the cooldown, the shade gates and the socket-3 count.
Since 517 `sys.skill.extra` is `partial`: the 164-166 new passives climb through their books with the `10 * k` refresh, the sub-skill list holds the flag-on eighteen, and the `AdvanceSkillSelectWindow` line rides the login burst and the level-up.
Since 518 `sys.item.costume` is `ported`: the weapon cover, its `/hide_costume 5` unhide, and the take-off-first cascade run under the fan-fit and stamped-clock checks; `sys.item.skin` is `unused` (`__SKIN_SYSTEM__` is defined but no game source names it).
Since 519 `sys.npc.shop` is `ported`: keeper 9001 opens `START_EX` with its `shopex` tab and a buy follows the row's price selector, stamping the row's sockets and attributes; `sys.trade.item_shop` is `unused` (the shop lives in the retired DB server only, and no game source, record, quest or locale file reaches it).
Since 520 `sys.skill.buff_npc` is `ported`: the 69700 toggle flips its socket-3 bit (unstacking past 1) and stands 20114 within 100 with the name `Buff` and the armour and weapon parts, the follower walks and runs to 250 of its owner and re-stands 200 behind past 4500, the five buffs cast in order past the 3 s stamp for the hour with `GC_AUTO_SHAMAN_SKILL`, the books train past the read-time gate, and 69699 chats the success line and is kept, as its training arm is commented out. The TEST chats stay out as Defects.
The
Since 521 `sys.marriage` is `unused`: a marriage forms only through `marriage.engage_to`, which no quest names, and the DB peer that carried the add is retired, so the wedding map never stands and `GC_LOVER_INFO` is never sent; the live hook is ported (71069-71074 refuse with `[LS;467]`).
Since 522 `sys.event.manager` carries its siege and kingdom war arms (13-14): the stones stand at the base plus offsets, the cross-empire kill fans the score record to 103 (and the entrant's burst carries it), the ends pay the rank table for 48 hours, kill the map and chat the teleport line, and the 3 s/60 s home warps queue with the empire-0 skip; the stone-defeat kill is unit-tested, the 2.9M-HP stone being unkillable over the wire in reasonable time. `gc.event_kw_score` is `ported` and `event.event_manager.warp_all_to_village_event` is `partial` (the 60 s fire is unit-tested only). Since 523 `sys.event.manager` carries its OX arm (11): the panel index starts the event over the wire (the start line, then the dialog), Uriel's herald warps the entrant to the attender seat on 113, and the engine — the 567-quiz loader, the open/quiz/end process, the three-step timer, the answer check with its effects and cheers, the miss warp, the 50109 reward and the login mount-strip — is unit-tested in the state, no scenario waiting out the 5-minute registration. `data.ox`, `quest.ox_event`, `event.OXEvent.oxevent_timer` and `event.event_manager.ox_event_process` are `ported`; `sys.event.ox` stays `ported` with the scenario. The
Since 524 `sys.dungeon.temple_ochao` and `quest.temple_of_the_ochao` are `ported`: the portal's forest chat runs over the wire (the map-209 gate, the `enchanted_forest.limit_activity` 0 write, the warp to (816700, 1505000) on the Channel hosting 210), and the 6311 kill table drops owned for 300 s in the world and manager tests, a 218k-HP boss no wire kill reaches in reasonable time.
Since 525 `quest.guild_manage` is `ported`: the guild man's menu gates both GM chats on `pc.is_gm`, the info chat sends the triad and empires, and the re-entry tool reads a parted member's stamp through `pc.select` (whose switch now moves the flag map and quest, `GetPC`) before dying on the never-assigned `u_disbandTime`, with the relog-moved quest proving the prefix follows the target. The
Since 526 `quest.guild_building` is `ported`: the staged seller's dialog sells land 4 to a level-10 guild master over the wire (the price leaves, the claim fans, the buy notice chats, the store banks both), porting `pc.changegold`, `notice`, `pc.get_guild`, `pc.get_x` and `pc.get_y` with their aliases. The
Since 527 `lua.get_time`, `lua.get_global_time` and `lua.pc.getf` are `ported`: the Time Spiral's scenario already runs all three arms, and the mutants prove it — a 0 clock, a 0 flag read, and a swallowed removal each kill it. `pc.remove_item` stays `partial` for its no-store arm (ledger 265). The
Since 528 `lua.number` and `lua.setskin` are `ported`: the temple's kill table draws ten times in the world test (a wire kill of the 218k-HP boss is out of reach), and the attr67 dialog's `setskin(NOWINDOW)` ends `[DONE]` in skin 0 — each proven by a killing mutant. `setleftimage` and `settopimage` stay `partial`: no owner quest calls them. The
Since 529 `lua.npc.getrace`, `lua.pc.enough_inventory` and `lua.pc.get_gm_level` are `ported`: the race alias shares its arm, the attr67 collect's gate reads room, and the implementor's login passes the panel letter's gate — the two gates each proven by a killing mutant. The
Since 530 `quest.kingdom_war_event` is `ported`: every event flag's change runs the no-target `event_begin`/`event_end` scripts on every online character, the war letters quest 16, the button joins (No refuses, Yes warps to the war map's corner), and the end clears all twelve event letters; the 522, exp, OX, siege-unfought and kingdom-kill scenarios now read the same traffic. The
Since 531 the twenty-four `pc.get_*` gauges are `ported`: one probe says them all on a naked level-1 warrior — 650 HP, 200 SP, 300 to level 2, race 1, the local cells, nil wear, bare weapon and armor, the name, and the enter's VID — and a max-HP mutant dies on the page. The
Since 532 `lua.pc.give_item2_select`, `lua.item.get_cell`, `lua.item.get_socket`, `lua.item.set_socket`, `lua.item.get_count`, `lua.item.get_type` and `lua.item.get_sub_type` are `ported`: the satchel probe gives two red potions into cell 1, says the stack's count, type pair, cell and socket, writes socket 0, and a count mutant dies on the page. The
Since 533 `lua.setleftimage`, `lua.settopimage`, `lua.getnpcid` and `lua.get_locale_base_path` are `ported`: the frame probe names the teacher's NPC id, misses an unknown name, shows the temp locale base, and frames a line with both image tags — a swapped-tags mutant dies on the suffix. The
Since 534 `lua.target.npc`, `lua.target.pc`, `lua.target.id` and `lua.target.clear` are `ported`: the mark probe marks the clicker's VID under both spellings, says both ids, clears both marks, and says 0 twice — a wrong-quest mutant dies on the page. The
Since 535 `lua.pc.remove_item`, `lua.pc.countitem` and `lua.pc.removeitem` are `ported`: the take-back probe gives five red potions, counts 5, takes 2 under the long spelling, counts 3, takes the rest under the short spelling, and counts 0 — a short-count mutant dies on the page. The
Since 536 `lua.q.setstate` and `lua.q.set_state` are `ported`: three clicks walk the state probe from `start` to `away` under the short spelling and back under the long one — a dropped short spelling dies on the second click. The
Since 537 `lua.pc.in_dungeon` and `lua.pc.get_channel_id` are `ported`: both ledger notes were stale — the forest dungeon lives — and the forest report proves both, the login gate leaning on the first and a new `ch 1` notice pinning the second after `evt 1` let a channel-0 mutant survive. The
Since 538 `lua.pc.getplaytime` and `lua.q.done` are `ported`: the playtime probe says both spellings seconds after the login and they agree, while `q.done` is a re-read — the 530 clears already prove the bridge, and the callerless helpers are questlib Lua, not the row. The
Since 539 `lua.is_test_server` is `ported`: the mode probe says it on a default boot (true) and a `test_server = false` boot (false) — a negated bridge dies on the first click. The
Since 540 the loading burst sends the map's music: `gc.main_character3_bgm`, `gc.main_character4_bgm_vol`, `lua.add_bgm_info` and `lua.set_bgm_volume_enable` are `ported` — a map-1 select hears 138 with `enter_the_east.mp3` at 0.5, map 72 hears 113, and a flag-off boot hears 137. The
Since 541 `lua.arena.add_map` is `ported`: the ledger-227 note was stale — `with_quests` seeds the world from `settings.arenas`, and the duel scenario's list names the four map-112 arenas (an unseeded world dies there). One API partial left: `set_state`. The
Since 542 `lua.set_state` is `ported` and the quest-API table has no partial left: the `__status` flag changes at once (`SetCurrentQuestStateName`), and the flag probe's `getqf` reads 0 before the call and non-zero right after it. The
Since 543 the seven party walk records are `ported` (`gc.party_invite/add/update/link/parameter/unlink/remove`): the request scenario pins the invite and the join walk, the role scenario the unlink and the ADD bodies, the ungroup scenario the REMOVE. The
Since 544 `gc.change_skill_group` and `gc.request_make_guild` are `ported`: `/setjob` leads with byte 112 and the group, and the guard's yes asks with the bare byte 82. The
Since 545 `gc.item_set` and `gc.item_update` are `ported`: the letters trade pins the six clears, and the move/split/merge pins the UPDATEs. The
Since 546 `gc.add_fly_targeting` and `gc.auto_shaman_skill` are `ported`: the assassin scenario pins the extra relay, and the toggle scenario the buff cast. The
Since 547 `gc.main_character`, `gc.login_key` and `gc.bindudp` are `unused`: byte 15 is the dead `_OLD` generation, byte 118 has no sender, and byte 254's only sender is unreachable behind commented-out UDP input and `__UDP_BLOCK__`. The
Since 548 eight more `codec` rows are `unused`: the enumerator-only speed, the callerless motion, the three enumeratorless bytes (73, 117, 213), the struct-only safebox money, and the lover pair that waits on the unreachable marriage. Ledger 550 corrects its event-reload aside: the `bReload` guard is never true, so `gc.event_reload` is `unused` too, not a port. The
Since 549 `gc.pickup_item_sc` and `gc.specific_effect` are `unused`: the pickup's switch is never defined, and the effect's wear arm wants a SPECIAL group the owner's file lacks while its pet arm wants a quest call no quest makes. The
Since 550 the server-records table has no `codec` left: `gc.skill_level` is the dead `_OLD` generation (byte 76 is live) and `gc.event_reload`'s whole chain is dead code, so both are `unused`. The
Since 551 `sys.item.special_inventory` is `ported`: a bank-0 book moves from base cell 0 to cell 290, and a plain item is refused from 291 with the line. The
Since 552 `event.event_manager.warp_all_to_village_event` is `ported`: the stone-fall
scenario waits out the 3 s fire and proves the empire filter, which the 60 s end-arm
shares. Since 553 `event.horse_rider.horse_stamina_regen_event` is `ported`: the world test
fires the twelve-minute timer through the Pulse wiring, the rider scenario already
dismounts, and the unit test pins the handover. Since 554 the destroy timer's auto-give arm keeps `item_destroy_time_autogive`
(`event.item.item_destroy_event` stays `partial`, its gold arm still scenarioless): the
scenario lays a sword from a full inventory and watches it go 2 s on. Since 555 `event.item.item_destroy_event` is `ported`: the gold scenario proves the
last unwired lifetime, and every configured destroy clock now has its two-second proof. Since 556 `event.item.ownership_event` is `ported`: the dug ore's 15 s claim lapses
in view, ending ledger 229p's D5 wait without a kill. Since 557 `event.char_battle.dead_event` is `ported`: the party reviver's 3 s/1 s
pair finally has its scenario, and `IsRevive`'s `Reward(false)` is shown pre-empted by the
spent exp. One `partial` timer remains (`affect_event`). Since 558 `cg.game.item_use` covers the automatic potions' toggle and `event.char_affect.affect_event` their tick (both still `partial`); the PvP gate's live-duel composition is review-only. Since 559 `gc.item_ownership` is `ported`: the mining dig pins the named set and the dug ore's 15 s lapse pins the empty-name clear, while a kill's item drops carry ownership by unit-tested code and its gold piles are unowned in legacy too, and the stray-dog GM scenario's two flakes are fixed (Yankee parks out of the dog's sight; the move reader skips heal ticks). Since 560 `sys.dungeon.core` is `partial`: the leader-unlink arm (`FExitDungeon`) and the login re-stamp are ported with the disconnect-home scenario, and the relog's homecoming is legacy's `SetPlayerProto` end state; the create/jump/exit/timer quest surface and the dead-event wait, and owner question 70 asks whether the forest middle runs on the owner's server. Since 561 `sys.dungeon.dragon_lair` is `partial`: `BlueDragon_Damage` is ported (the host snapshots the four stone rows at load, every melee and mob damage mirror scales through the snapshot, and the scenario pins 172→129 past one live 8031 and 1244→1555 past one live 8032), while the skills, the battle state and the lair itself are next — unreachable on the owner's data, where the dragon and its stones never spawn. Since 562 `quest.change_empire` is `unused`: both `qc`s refuse its unbalanced `move_pc` (ledger 227's wait for a source fix is replaced by owner question 71), and no quest references it. Since 563 `data.lua.settings` is `ported`: the host loads legacy's four libraries in order and fails the load as its `return false` does, each file wire-observed (the duel lists `settings.lua`'s arenas, the observer's menu names `translate.lua`'s strings through `locale.lua`'s aliases). Since 564 `data.dungeon_info` is `unused`: the whole file manager is behind `__DUNGEON_SYSTEM__`, which no checked-in file defines (owner question 72 asks the owner to confirm their build agrees). Since 565 `data.quest.sources` is `ported`: the `qc` port, the load order and the refuse-and-continue load stand, 50 of the 51 scripts load into 49 quests (the `guild_building` pair shares one number), and the burst's three-quest letters witness it. Since 566 `sys.log` is `partial`: the MySQL sink is refused, `dev_log` is the tracing setup, five methods are dead, and which live calls to trace waits on owner question 73. Since 567 `data.regen.dungeon` is `partial`: all 119 dungeon and event regen files parse with the shared reader, but no ported path reads them — the callers are Hydra's waves and five event-manager arms. Since 568 `event.char_affect.affect_event` and `sys.event.core` are `ported`: the tick strips a naked aura at its head, the guild band before the drain, and updates when either raw speed moves; the premium and hair arms are dead (no granter, `unused` quest calls), and no `partial` timer remains. Since 569 V3 is closed: `StateMove` spends a running player's stamina a point a step
in the fight window and restores a walking player's past five seconds, `PointChange`'s
switch walks the flag at 0 and restores the mode line's point off it, the refill runs
through the switch, and the consume gauge starts and stops with its lines; the uninitialized
walk stamp is a recorded Defect, and the dead-pool gate stays a pre-existing 229d gap. Since 570 `sys.dungeon.ship_defense` is `partial` (no `missing` row remains anywhere): the wave monsters retarget the mast, swing on it through `Damage`'s NORMAL path with its SPECIAL reflect, and the mast broadcasts its HP — while the fake hydras refuse every striker. Since 571 the detachment scroll breaks the live stones out of the first three sockets and gives each back (`sys.item.sockets` stays `partial`; the socket-adding scrolls wait).
Since 572 `sys.item.sockets` is `ported`: the socket-adding scroll rolls its 50 in 100 and opens one of three, and the jewels fit by `CanPutInto`'s table and ladder and set with the downgrade countdown (the expiry timer stays `unused`; a graded accessory refuses the wear). The next slice the forest's leader branch is ported, the sash
Since 573 `sys.char.pk` runs the killer mode: a same-empire hit on a blameless player marks the attacker with the killer bit, cleared on death and 30 seconds after the set, and `g_protectNormalPlayer` keeps a negative attacker off a blameless target in peace. Since 574 it costs the same-empire killer 20 000 alignment past the agreed, war, marked, duel and forked gates, with the 33/20 spare draw, the `[LS;910]` line and the party split. Since 575 the revenge arm is proved live: a blameless player in revenge strikes a negative player of its own empire unmarked. Since 576 the item drop penalty refuses a character with its bundle shop open, and the penalty note's five gaps are reconciled: agreed, arena, forked and protected were already ported. Since 577 the hide-alignment cloak (70048) wears: the body holds the shown value beside the card's real one, the wear and take-off hide and restore it through the step's own records, `pc.get_alignment` reads the shown and `pc.get_real_alignment` the real. Every other unique still refuses, and the unique expiry event is still unported.
quest's reward waits on 85000's missing proto, which is Game data, not code, and the Hydra
kill waits on the kill event reaching quest scripts with a target. Since 578 `sys.char.save`'s last `Not ported` is live: a bank's minutes new since the last bank earn 5 each outside a `BANPK` cell, or 60 (120 with the 70050 unique) while negative, and the kept row reads the earned alignment; a logout's re-bank earns nothing twice. Since 579 `event.item.unique_expire_event` is `ported` and `sys.char.pk`'s uniques wear: the wear arms the event from the socket-4 bank, the fire ticks socket 5 down or removes at 1 or less, the take-off banks the seconds left, and the cloak's expiry shows the real alignment again. Since 580 `sys.net.flood` is `ported`: the "packet flood limit" its note waited on does not exist in the tree — only the ported command count, the ported stack-attribute use stamp, and the dead `ENABLE_FLOOD_PRETECTION` define. Since 581 `cg.game.guild` is `ported`: the arm audit finds every one of the 15 wire arms cased and routed — the "notices" tail was the comment `!` gate, already live. Since 582 `cg.game.item_use` cures: `USE_CLEAR` strips the bad affects and spends one of the stack, and the strip's poison arm now takes the affect off before cancelling the event. Since 583 `cg.game.item_use` cloaks: `USE_INVISIBILITY` lands 300 s and spends one, and the mirror's ten-minute cooldown refuses with the wait line. Since 584 `cg.game.item_use` raises points: `USE_ABILITY_UP` lands the apply's affect, and a held speed refuses with legacy's line. Since 585 `cg.game.item_use` frees the supplements: the 50801-50820 branch lands 516, correcting 584's misroute. Since 586 `cg.game.item_use` times the mall: main-switch `USE_AFFECT` lands value(0) on the point, and a held kind row refuses with `[LS;99]`. Since 587 `cg.game.item_use` premiums the shop: the 507 arm lands the row, banks the account expiry, and refuses near the max. Since 588 `cg.game.item_use` recalls: the town and memory portals mark the spot, pay 30% of the maximum life, and warp to town or the mark, with the map, arena, duel, empire and level gates and their lines. Since 589 `cg.game.item_use` melts gold: the six bars pay `sGold`'s yang with a `ChangeGold` record, and the open-windows and cap gates refuse in Romanian. Since 590 `cg.game.item_use` gathers: the cape of courage starts the victimless monsters in reach on the caller and uses one up. Since 591 `cg.game.item_use` pockets: the Nog pocket lands three 302 rows and refuses a held one with `[LS;99]`. Since 592 `cg.game.item_use` refuses the duellist's potion: with `no_potions_on_pvp` up a fighting pair's limited use hears `[LS;1205]` and keeps its row. Since 593 `sys.char.top_players` is `ported`: `/level 120` grants the first top row with its store row, and a move off 120 strips it and fells the row. Since 594 the disband clears its land: `ClearLandByGuildID` runs with the destroy, the walk INFO carries `has_land`, `guild.has_land0` answers the index, and the symbol upload refuses a landless guild off a test server (refuse where legacy closes). Since 595 `sys.item.id_range` is `ported`: the boot resolves the id span from `MAX(id)` before the ready gate opens, and the grant scenario pins the chain — `/item 27003` stores the potion under id 100 000 000, the span's first. Since 596 `sys.char.chat`'s spam block is live: the `spam_db` words load at boot, an under-level IP whose score crosses the bar is blocked for 900 seconds of Pulses with the legacy ladder (`[LS;1030;15;0]`), a second line hears the countdown, and past `spam_block_max_level` the same line goes through; the whisper arm runs the same check past the blocks. Since 597 `sys.char.polymorph`'s consumables are live: the ball's use morphs through the timed `AFFECT_POLYMORPH` pair with the four gates and their lines, spending the ball, and the 50322 book's use runs the master's duration, the chance and the dog's run row, stamping socket 1 down; `GiveBook` and `BookUpgrade` stay unported behind their two `unused` quest names. Since 598 the morphed stat gates refuse: `/stat`, `/stat-` and `con+` with `[LS;521]`, `stat_val` in Romanian, `/tcon` on a morphed target to the caller, `/skillup` only when the dispatch arms reach, and the book arms with 521 or 1041 by ladder — all reading the live race. Since 599 `sys.event.manager`'s hide-and-seek arm is live (still `partial`): the teacher's click starts the event, the hinted relog finds the box, its click pays one 50109 and moves it, and the end falls the flag — which ledger 598's code never did, so the event quest's own `event_end` now fires with the rest. Since 600 `sys.event.manager`'s moonlight, hexegonal and fishing arms are live (still `partial`): three clicks start all three with each begin letter replaying behind the later starts, and three end them with each end sparing exactly the quests whose flags still run. Since 601 `sys.event.manager`'s Tanaka arm is live (still `partial`): the teacher's click starts event 12 with the start notice and the begin letter, round 1 lands its pirates on the desert within the second, and the end clears every event quest's begin; the two later rounds and the run past the end are unit-pinned. Since 602 `sys.event.manager`'s boss, metin, mining and frog arms are live (still `partial`): on the owner's data (no map 230) four clicks start all four silently with each begin letter replaying behind the later starts, and four end them with each end chatting (the frog's keeping its missing space) and clearing exactly the quests whose flags no longer run; the walks, the mining plant and the ends' bodies are unit-pinned on a hosted 230. Since 603 `quest.tanaka_event` is `ported`: the letter's button and the quest window's entry run the `button or info` dialog, Yonah's menu scolds without the ear and offers the exchange with it, and a yes spends the ear while the first `math.random(1, 15)` draw is 13, whose Bewitch Item has no proto on the owner's data, so nothing arrives — the script's only unreached line is the reward's arrival, which is Game data, not code. Since 604 `quest.fishing_event`, `quest.moonlight_drop_event` and `quest.hexegonal_drop_event` are `ported`: every letter's button and quest window entry runs its `button or info` dialog, and the fisherman's menu scolds with fewer than 3 minnows while a yes with 3 spends them and pays the Mystery Box into gift bank 3's first cell. Since 605 `quest.boss_event`, `quest.metin_event`, `quest.mining_event` and `quest.golden_frog_event` are `ported`: every letter's button and quest window entry runs its `button or info` dialog with the enter select, a no closes each, and a yes to the boss orders the quest warp, which finds no 230 and stays silent while the save still names map 1. Since 606 `quest.hide_seek_event` and `quest.siege_war_event` are `ported`: the hidden box's letter rides the login burst and reactivates after a relog, the siege letter drops alone when its flag falls, and a yes to the war map's point is refused with the dialog closed and no record, as the point is map 103 and no Channel hosts it. Since 607 `quest.event_panel` is `ported`: the implementor's button runs the fifteen-option select, the moonlight arm says its state, and Enable and Disable start and end the moon with the start and end notices, the moon's begin letter, and every other event quest's begin clear. Since 608 `quest.guild_war_bet` and `quest.guild_war_observer` are `ported`: with no wars the observer's bet entry explains and scolds, and the watch entry says no guild fights, over the live scripts. Since 609 `quest.guild_building_melt` is `ported`: the master's click on its 20062 stone offers the timetable, its own attribute is refused as already active, and Diamant rebuilds it to 14043 with the fee paid and the row swapped. Since 610 `quest.guild_create` is `ported`: seeded at `run`, the letter and button run the run page, and a partyless kill of the staged orc lord runs its `691.party_kill` to `reward`, whose live letter's button gives 70101. Since 611 `quest.guild_war_join` is `ported`: the member's no re-buttons the join ask, and after `/gwstop` a stale press finds `get_any_war() == 0` and says the war-over page. Since 612 `quest.guild_building_npc` is `ported`: three stood towers show the weapon, armour and jewel NPCs, and the master's clicks hear the three own-guild pages whole while the guildless stranger's hear the three stranger pages. Since 613 `quest.guild_building_alter_of_power`'s 20077 arm is done: at guild level 1 the first click sets `build_level` and says the low-power scold, and grown to 11 the click offers the rebuild, puts the no off, and rebuilds on the yes with 25M and the materials; the 20078, 20079 and non-master arms are next. Since 614 `quest.guild_building_alter_of_power` is `ported`: the 20078 offer, refusal and crowning rebuild to 14063, the 20079 already-best page, and the lay member's use page. Since 615 `quest.sistem_gaia` is `ported`: the click menus the two chats and the close, the first chat says the stones word, the gift of 5 stones runs the take whose yes opens the Gaya window, and the second chat says the bonus word whose yes opens the market. Since 616 `quest.aura_system` is `ported`: past the sash mark the keeper menus the four aura chats with the five sash ones, the about says the bla pages around a wait, and absorb, refine and evolve ask, refuse on no, and open windows 0, 1 and 2 on yes. Since 617 `quest.yohara_system` is `ported`: the staged Hydra's kill counts 1, resets, says the congrats and moves the quest to `killReward_new`, and the statue's reward chat makes Beta a champion over the wire for the first time (275's wire run only did the village warp, and its `killReward_new` seed was wrong by exactly 10000). Since 618 a quest-set event flag runs its world arm: `apply_dungeon_moves` routes `event_sets` through `GameState::set_event_flag`, so the probe's `xmas_boom 1` rings and stands map 1's firework seller. Since 619 `quest.meleylair` is `ported`: the click menus the three chats and the close, the lore runs around a wait, the fight lists the 3/600 requirements and registers on Yes (the repeat refuses from CH 1), and the shelter refuses without guild PK mode before entering with it. Since 620 `gc.walk_mode` is `ported`: the idle wander's `test_server` walk toggle sets a wandering monster running 60 times in 101, with the boundary and the off-server silence pinned. Since 621 `quest.meleylair_zone` is `ported`: the run's leave chat asks and warps to the sub-map spot on map 62, the No closes quietly, and the login `Check` warps a guildless stranger to the same spot. Since 622 `quest.7and8th_skills` is `ported`: the ward book offers all nine wards over the wire, teaches 221, and refuses once learned, joining 265's boost/reset and 268's GRAND runs. Since 623 the live ship defence boards, starts and leaves over the wire (`a_party_boards_starts_and_leaves_the_live_ship_defence`), the console grant stamps `LIMIT_REAL_TIME` expiry like `CreateItem`, and the kill's wood drop and the mast's take are content-dead (31107 has no `item_proto.txt` row). Since 624 `table.banword` and `sys.char.chat` are `ported`: the game thread holds the loaded Name rules, so a banned shop sign and title are refused over the wire, and the chat row's five gaps are all closed (Whisper, the commented-out conversion, spam, the dead prism, the `/` dispatch). Since 625 `sys.item.proto`'s mask clause is corrected: the stamp no longer overwrites the vnum (`Item::mask_vnum` holds the ori), `a_masked_gift_lands_as_its_new_vnum` proves a 76002 gift lands as 76002, and the row stays `partial` on the undrawn bag. Since 626 the bag is drawn: `use_giftbox` ports the `ITEM_GIFTBOX` use with all eight arms (`a_moonlight_box_draws_its_group_and_tells_the_gift`), and the row stays `partial` on the treasure-key arm. Since 627 `sys.item.proto` is `ported`: the key arm closes it (`a_key_names_a_non_box_a_missing_box_and_its_groupless_box`), and a failed draw now sends its landed gifts before the failure line. Since 628 `sys.world.move`'s mount and polymorph arms are live (still `partial`): the mob `WALK`/`RUN` speeds load per folder with the zero-carry RUN drop, a morphed player moves at its morph's run clip (322 ms, the dog's 438.38) and a mounted one at its mount's (134 ms, the boar's 743.49 under MOV 140), and every NPC moves at its stamped run clip. Since 629 `sys.char.polymorph`'s battle arms are live (still `partial`): a morphed attacker skips the weapon gate, rates with its morph's scaled dexterity and adds its scaled damage range, in melee, arrow and the skill functor, with the unknown-vnum arm; a dog-morphed Alpha at power 41 punches the 406 for 228 or 229. Since 630 the victim's morph dulls the swing too (still `partial`): the PvP damage, the mob swings and shots, and the mob skill's `atk` and splash `ar` rate a morphed victim's scaled dexterity, and a bowman-morphed Alpha at power 100 takes Dave's free punch for 151, not the plain-DX 158. Since 631 a click on a morphed player shows nothing (still `partial`): `SetTarget`'s polymorph arm hides even the maximum, and the clear shows the numbers again. Since 632 the 71092 scroll clears a morph without spending itself (still `partial`): with a target set the target's morph goes, else the caller's own, each `SetPolymorph(0)` then the affect's remove. Since 633 a morphed reader refuses the skillbook with 1041 (still `partial`), and the private-shop stand was read and needs no morph tail. Since 634 `sys.char.polymorph` is `ported`: the 36-hit morph sweep is fully accounted, with the forget arm's gate waiting on `sys.skill.core`. Since 635 the arm is ported (`ITEM_SKILLFORGET`'s use with `SkillLevelDown`, its silent morph gate, and the blank-book Divergence), proved by `a_forget_book_drops_the_named_skill_and_a_bad_name_refuses`; `sys.skill.core` still waits on roles/bonuses/parameter, heal/warp/exp. Since 636 `sys.skill.extra`'s note is corrected: `SkillCanUp` is enforced at all three reachable sites, the up/down gates are dead for want of hosted type-6 rows, and only the 7/8 functor arms stay out. Since 637 `sys.combat.resist` is `ported`: the pct arms, `NEW_BONUS`'s emptiness, the resist body and the target element are all accounted, proved by `a_targeted_wind_mob_shows_its_element_in_the_target_record`. Since 638 `sys.skill.affect` is `ported`: the four missing `OnMove(true)` strippers uncloak (the swing, the melee damage, the shoot pipe, the PvP damage), proved by `a_cloaked_assassin_is_not_found_and_a_swing_uncloaks_it`. Since 639 `sys.char.horse` is `ported`: the `no_mount_at_guild_war` event flag strips the war entry and refuses the ride and the quest mount on the war's map, proved by `the_war_entry_strips_the_mount_under_the_no_mount_flag`; the riding attack-speed and combo arms are dead behind the off `gHackCheckEnable`, and the arena silences, the stamina drain and the grade-1 gate were already live. Since 640 `sys.world.time` is `ported`: the `day` flag's broadcast ring, unset and login replay, proved by `the_day_flag_rings_dark_and_light_and_a_late_login_hears_dark`; no day/night ticker exists in legacy, and the time-gap setters are retired DB-peer code. Since 641 `sys.char.quickslot` is `ported`: the plain equip's sync re-points the slot at the bare wear index, pinned by `a_plain_equip_repoints_the_slot_at_the_wear_index_and_a_swap_syncs_nothing`; the sync is vacuous for honest clients (slots name `ITEM_USE`, worn items never are), and every other sync site was already ported. Since 642 `sys.combat.target` is `ported`: no code — its drop-list remainder was ledger 384's, and the row simply was never closed.

Since 643 `sys.skill.extra`'s 7/8 rows host: the sixteen 221-228/236-243 type-6 rows load with their `24*k`/`16*k` polys (229/244 stay out, unread by anything), `SkillLevelUp`'s :738 `SkillCanUp` gate is enforced so a quest-learned ward climbs by point and refuses beside a sibling, and the help functor arm grows each trigger's 230d/230e cast by the caster's learned boost. The ward arms wait for player victims (PvP).

Since 644 `quest.TeleportRing`'s gated branches run over the wire: the level refusals, the ticket spend with its bank-cell clear, and the sub-index menu with its bare-`[DONE]` close and re-entry.
Since 645 `quest.battle_pass` is `unused` (no proto for 25150-25162, owner question 74) and `quest.enchanted_forest_zone` is `ported` (the 279 note was stale; 280 proved the ticket refusal and the full make).
Since 646 `questlib.TeleportRingLib` is `ported`: browsing trips `IsHack`, so the valley pick says to wait ten seconds with no warp and no spend.
Since 647 four quest libraries are `unused`: `dungeonFunctions`, `dungeonInformationLibrary`, `luaFunctions` and `GFquestlib` all load but nothing calls them (owner question 75).
Since 648 `questlib.locale` is `ported` (the land dialog renders its yes/no over the wire) and `questlib.multiLocale` is `unused` (loaded, never called; owner question 76).
Since 649 `quest.sash_mission` and `quest.ship_defense` are `ported`: every reachable branch runs over the wire, and the protoless remainders (85000, 31107) are documented, not ported.
Since 650 `sys.life.fishing` reads the fish-mind premium: a live premium draws the mind's table and previews the bite.
Since 651 `sys.combat.core` reads the EXP/ITEM/GOLD premiums: the +50% exp grant and the buyer rates over the wire (`a_premium_kill_pays_half_again_the_monsters_exp`), with the login affects and the tick sync for all five held kinds.
Since 652 the extractors run: the heart melt lands a charged heart and the soul pull-out lands or breaks, both over the wire (`a_heart_melt_spends_the_pair_and_lands_or_fails`); the heart carries its `CreateItem` expiry stamp, and the refine-made stone's duration moved from socket 2 to socket 0. Since 653 an active deck stone's wear timer ticks: the first fire comes `remain % 60` in, each fire falls a minute with the update riding and the row ordered, zero deactivates with socket 2 clear, and `d` keeps the ran partial minute (`an_active_stone_ticks_its_wear_timer_down_and_deactivates_at_zero`); the scenario sits the heartbeat's 60 s cycle out, and the non-soul starts stay refused (`WornSystem::Timer`, no `ITEM_TOGGLE` use arm). Since 661 the soul charge lands (`a_percent_charge_fills_a_soul_and_a_second_meets_it_full`, `a_heart_and_a_fix_charge_land_their_seconds`; the heart reads socket 2, the fix adds seconds, the full chats `[LS;1066]`).

Since 678 `cg.game.item_use` buries and finds the event stone: `case 50004` arms on first use, finds within 1000 with the band's prize, burns out at 10 tries with a 27002, else points with `StoneDetect` (`a_fresh_event_detector_buries_and_finds_its_stone_at_once` + prize-walk tests).

Since 677 `cg.game.item_use` sweeps the map's stones: `case 27989` / `case 76006` count socket 0 up, answer `StoneDetect <vid> <val> <degree>` for the nearest live stone (or `[LS;1461]`), and break at 6 uses (`a_detector_names_the_nearest_stone_and_counts_its_use`).

Since 676 `cg.game.item_use` cracks the clam shell: `case 27987` registers the pass mission, spends one, and rolls `number(1, 100)` over the five bands with their lines and gifts (`a_cracked_shell_tells_its_band_and_spends_one` + fixed-dice band tests).

Since 675 `cg.game.item_use` flies the sky fireworks: `case 50100`-`case 50106` pay vnum - 50100 + 7 around from the lighter to herself and spend one (`two_sky_fireworks_fly_from_the_lighter_to_herself`), and 71013 rolls `number(7, 12)` the same way.

Since 674 `cg.game.item_use` lights the fireworks: `case 50107` / `case 50108` refuse on an arena map and in a `no_potions_on_pvp` duel fight with `[LS;1205]`, and past the gates pay the effect around (9/10) and spend one (`two_fireworks_light_their_effects_and_spend_one_each`); the stun arm is commented out at `:2716`.

Since 673 the stat-reset tonic raises: `case ITEM_GIVE_STAT_RESET_COUNT_VNUM` runs the use's checks, raises the reset count by 1 through the shared counter arm with amount 0, and spends one (`a_stat_tonic_raises_the_reset_count_and_spends_one`); the castle note no longer says the monarch war powers wait for the maps — their only writers are the `unused` quest arms and timers, so the damage tenths never fire.

Since 672 the monarch's blessing heals: `case 70021` through `HealMyEmpire` refills her empire's pools on her map, chats the `[559]` notice and `[LS;570]`, charges the treasury and spends one, with the commoner's refusal proved off the test mode (`a_monarchs_blessing_heals_her_empire_and_charges_the_treasury`).

Since 671 the aura boost eraser erases: vnum-routed past its `USE_SPECIAL` owner proto, the target's socket 2 goes to 0 and the eraser shrinks or leaves by its flags, while an unboosted target chats the line with the eraser kept (`an_aura_eraser_erases_the_boost_and_keeps_the_last`).

Since 670 the elk gold bar pays its socket: the whole bar leaves, `[LS;1269]` names the gold, and it pays through `ChangeGold`, still consumed past the cap (`an_elk_bar_pays_its_socket_gold_and_leaves`).

Since 669 the wonso trio fill the pools: the bean the hit points, the sugar the spirit, the fruit the stamina, each by the gap to its maximum, spending one (`wonso_fills_a_pool_by_its_gap_and_spends_one`).

Since 668 the hair colorants dye and wait: the hair becomes the vnum minus 70201, a hair at or past 1001 refuses as a dyed one, a second use within 3 seconds waits, and the colorant is never spent (`a_hair_colorant_dyes_and_waits_three_seconds`).

Since 667 the stone-removal water breaks stones out: used on a weapon or armour, the top live stone comes back and its socket reads 1, and the water spends only then; an ear, wrist or neck, or a target with no live stone, is refused with `[LS;1032]` (`a_water_breaks_the_top_stone_out_of_a_sword`).

Since 666 the mythical peach gives alignment: the throttle refuses a second bite within 8 hours of the flag, the ceiling refuses at 200000, else `value(0) * 10` lands, one peach spends, and the gain talks and names (`a_mythical_peach_gives_alignment_and_refuses_a_second_bite`).

Since 665 the poison apple burns: one apple spends and the eater is poisoned with no attacker, the affect landing before the first tick (`a_poison_apple_spends_one_and_its_first_tick_burns`).

Since 664 the shop bundles open the panel: the paper and silk bundles open the personal-shop panel without spending, the silk also telling `SetPrivateShopPremiumBuild`, while a shop owner hears the close line (`a_bundle_use_opens_the_panel_and_an_owner_hears_the_close_line`).

Since 663 the aura booster lands: on an unboosted aura costume socket 2 stamps `index * 100000000 + value(1)` with the success line and the scroll spends, while a boosted or non-aura target chats the refusal with the scroll kept (`a_booster_lands_on_an_aura_and_a_second_is_refused`).

Since 662 the silent uses run: `USE_BUCKET` and `USE_CHANGE_ATTRIBUTE_PLUS` have no arm in the subtype switch, so the use succeeds with the item kept and nothing on the wire (`a_bucket_and_a_plus_scroll_are_used_and_heard_nowhere`), and the pass hook counts past the step.

Since 661 the soul charge runs: the percent and fix arms charge wear-timed stones with TruncatedI64 rounding and the `[LS;1066]`/`[LS;1093]` lines over the wire (`a_percent_charge_fills_a_soul_and_a_second_meets_it_full`, `a_heart_and_a_fix_charge_land_their_seconds`).

Since 660 `data.mob.group` is `ported` (`a_group_regen_stands_its_leader_and_its_members`; the party was already ported, and legacy has no member-follows-leader walk).

Since 659 the stone spends and the recipes brew (`a_stone_scroll_spends_and_gives_no_stone`, `a_recipe_consumes_its_sources_and_misses_its_bottle`; on the owner's data the stone gives nothing and every brew fails after consuming).

Since 658 the reinforcements run: on a costume of the scroll's kind below two bonuses and without its apply, the fixed pair lands with the success line and the scroll spends, while the wrong kind, the full and the duplicate chat their Romanian lines with the scroll kept (`a_body_reinforcement_lands_and_a_repeat_is_a_duplicate`; pet/mount/weapon unit-proved).

Since 657 the socket cleaner runs: with a broken stone in sockets 0-2 the live sockets compact down, the tail past 0 becomes 1, and the scroll spends with no line, while a whole sword chats `[LS;395]` and keeps it (`a_cleaner_compacts_a_broken_sword_and_a_second_refuses`).

Since 656 the costume scrolls run: under the kind, set and count gates the change redraws and counts socket 3, the reset clears and rolls one to three without counting, both chat `[LS;399]` and spend (`a_costume_change_redraws_and_its_reset_rolls_again`, on timeless hair 45155, since the costume weapons expire in a day).

Since 655 the glove scrolls run behind the always-on glove build: the add lands below five with the same success line, the change redraws and counts socket 3, both spend, and the socket case's costume-and-glove gate narrowed to its own labels, so a plain add still refuses a glove while the kind scrolls land (`a_glove_add_lands_and_its_change_redraws_and_counts`).

Since 654 the talisman scrolls run: the add lands below five with its success line, the change redraws on the plain table and counts socket 3, both spend, and a non-talisman or empty change is silent (`a_talisman_add_lands_and_its_change_redraws_and_counts`); the fail line's Korean is lost in the frozen source but its roll cannot miss.

## The summary until ledger 678

**A client can log in and play the vertical slice** (step 3, done in ledger 228): auth, the Channel
login by key, the select screen, loading and entering the game, movement, chat and the shout, the
save cycle, items and the inventory, every NPC of the hosted maps with its shop, trades, the safebox
and the mall, the quest NPCs' dialogs, and the Warp between maps through the warp NPCs. Since ledger
229a every hosted map's cell attributes are loaded, and a character entering the game is moved off a
blocked cell. Since 229b each player sees what legacy's view shows it: the players and NPCs within
`VIEW_RANGE + 500` in the sectrees around it, inserted as they come into range and removed as they
leave it, and a move, a pose or a sync reaches that view. Since 229c the items on the ground are
in that view too. Since 229d a player entering the game is sent the revive-invisible affect's update
and its flag for five seconds, and its stamina refills three seconds after it stops. Since 229e a player walks and runs at its race's speed for its weapon's stance, from the owner's `.msa` clips. Since 229f every hosted map's
monsters, stones, groups and ore veins stand where legacy's boot draws put them, and a player is
shown those in its view; they do not move, fight or die yet. Since 229g a click on a
player or a monster targets it and shows its hit points, which follow each change; before it the
click closed the connection. Since 229h each regen runs every `time` seconds and stands what its
count lacks, and an ore vein ends 7 to 15 minutes after it stands, its regen standing another. Since 229i
the NPCs a player stands near run legacy's state machine: they wait, wander one time in seven,
speak their `locale.monster_chat` lines, and a coward runs away; none has a victim yet. Since 229j a
player's punch or weapon hit lands on a monster with legacy's damage, a lethal one stuns it, and it
dies three seconds later and goes a second after that; a hit monster recovers its hit points, and does
not fight back yet. Since 229k a player can die and come back: a lethal hit stuns it, it dies three
seconds later, and it restarts where it lies, in its empire's town or at once for 300000 gold, or
in town on its own after three minutes. Since 229l a monster fights back: a hit one turns on its attacker (every battle type, 229s), an aggressive one chases a player it sees, its party answers with it (229t), a guard fights its monsters (229w), a stone spawns its sons (229x), a skilling one casts (229u), and a player it kills restarts as 229k proved in memory. Since 229n a kill pays experience: the damage map, `DistributeExp`, the live `GiveExp`, the kill alignment, the quest `kill` dispatch and the kill recovery, with the exp record and the exp orb. Since 229p a kill pays gold (the chance, the multiplier and the branch draws run in legacy's order) and every drop table's draws run. Since 229q (D5) a hitting row creates its item and lays it with the legacy ownership; proved in-memory, since no hosted spawn drops on the owner's data. Since 229o a player can drop gold piles and pick them up. Since 229v/229z a hit lands its poison, stun and slow on either victim, and since 229y the victim's reflect answers. Since 230a a gold steal pays the purse, since 230b a dead monster with a living reviver in its party stands back up, and since 230c a swing holds the monster its clip's seconds. Since 230d/230e a player fights with its melee, arrow and magic skills with the SPLASH walk, and since 230f it ups its skills, holds its self buffs and pays the grandmaster cost. Since 230g it reads its books and trains to grandmaster; since 230h it toggles its combo, charges its dash and hides from monsters; since 230i-230m it parties (invite, roles, heal, warp, exp, unlink, chat); since 230n a guild is made, since 230o it opens its door (invite, accept, join, removal), since 230p it names its grades (rename, re-auth, re-grade, general), since 230q it takes money (offer, charge) and posts (post, refresh, delete), since 230r it ups and casts its skills, since 230s its quests' `guild.*` calls are backed (the bridge test names them, no wire scenario), and since 230t the select list names each slot's linked guild. Since 230w-230y a guild war is declared, refused and accepted, every PC hears the on-war list and an entrant its `WAR_LIST`, a field kill scores, and the end lands: `gwstop` forces the draw the ladder math takes while the over notice still names the score winner, `gwcancel` ends without the math and names the pair to every PC, and the GM level both commands need is read from `gm_grant` at character select. Since 230z a GM's
`/gwlist` names each live pair and its age. Since 230aa a GM's `/notice` reaches every Channel,
`/notice_map` its own Channel and map, and `/big_notice` the caller alone. Since 232 a player hits a
player of another empire: the swing takes the victim's hit points with legacy's damage, the claim
follows the hit, and a hit on a player of his own empire is refused. Since 233 a guild war's arm
opens the fight between two guilds at war, whatever the PK policy says, and a kill in that war
scores the war with the victim's level for both sides' masters. Since 234 a character's
alignment is stored, loaded and shown: the client's summary carries the stored alignment in
tenths, and a save writes the alignment the world holds. Since 235 a player that dies to
another player loses the items its alignment and the draws choose: the nine alignment bands
and their table, the two draws, the level and the GM level, the anti-flags, the unique item
that skips the drop, and the worn item's points. Since 236 a guild war knows the map it is
fought on: the three war types and the two war maps, the declare's map-index check (the
atlas is legacy's map index) and the Channel's map set, and the private copy a started war
takes of its map. Since 237 that map is a map of its own: it holds the two teams, the players that watch it and the counts it tells them, and a player that walks onto it joins it. The owner's index holds the battle map, so a battle war's declare lands
and its accept starts the war on that type. Since 239 the war asks its members to join it: the
quest's letter carries the button, the client presses it, the quest asks, the member's yes
warps it to the war map's start position, and the row it comes back to names the map and the
place it came from. Since 240 the war's map goes with the war: the war's end closes the map,
ten seconds later the map sends its members back to the place they came from, and five
seconds after that the copy of the map is unhosted and the players still on it are
disconnected; the member that joined hears the war's own record and the two scores, and the
enemy's guild name reaches it with the enter, as the war's walk says. Since 241 the war's map
judges the war: a kill on it scores the war by one point, a score that reaches the war's end
score ends the war, a side with no member on it makes the map say so and wait a minute, a
member that comes calls the wait off, a member that leaves asks at once and still hears it,
and the wait's end judges a war younger than five minutes a draw. Since 242 the flag war has
flags: the nearest player that may take one carries it, a player of the flag's own guild may
not take it from its base, the base scores the other team's flag, the flags reset and come
back ten seconds later, and the flag lies where its carrier fell. Since 243 the flag and its
base are characters on the war's own map, and they are destroyed with the war. Since 244 a quest's
flags live in the store with the character, so a flag outlives the session that set it, and the war's
member joins its team after the warp to the war's map. Since 245 the flag's take and the
base's score are on the wire: the carrier gets the flag's affects before the flag's
character dies, the map says who took it, and the base's score resets the flags, says so
and only then sends the score record, because the record is the database's answer; the
war's end score is a tenth in the test-server mode, as legacy's is. Since 246 a whisper
reaches the character it names, on the sender's Channel or on another one, the name is
matched without regard to the case, the sender's own name drops the line and an empty line
sends nothing; a line for a character that is not there waits for it in the world's memory
and the sender is told, and the target reads the line when it next enters. Since 248 a
friend add, its yes and its remove are live: the request reaches the target as the
`messenger_auth` line, the yes lists both directions and stores both rows, the relogin
keeps them, and the remove takes them back. Since 249 the `/setblockmode` byte lands
beside the seven `game_option.block_*` quest flags, the login restores it through `quest
game_option`, a sender who blocks whispers hears the sender-blocked record, a target who
blocks them answers the target-blocked record, and a target who blocks friend requests
answers `[LS;657]`. Since 251 the character deletion wipes every messenger row the
character's name is in, so no friend list names a character that is gone. Since 252 the
whisper's chat-counter gate is live: the tenth line in a run is consumed with no record
and closes the connection. Since 253 a player that lists a grant hears LOGOUT for the
name, never LOGIN, while the grant hears the player as LOGIN. Since 254 the team's
login and logout fans are proved: the entrant hears its TEAM_LIST, every holder hears
the TEAM_LOGIN, and the grant's disconnect fans the LOGOUT then the TEAM_LOGOUT. Since
255 a GM silences a character for a parsed time: the lay, the `[LS;852]` and the add,
the talk gate's hour-minute-second ladder, the whisper gate's `[LS;652]` info line,
the offline name's stored row, and the 0 lay that stands on the next tick. Since
256 a new session starts with no block byte and no offline-line clock, the offline read
takes the waiting lines before it checks the block, and a line is dropped at five hours.
The rest of combat, skills,
parties, guilds and the rest of step 4 are not ported yet. The two-process layout (a `game-server` and a `db-server`
talking the legacy DB-peer protocol over MySQL) was retired in ledger section 177; sections 1-175
record how it was built and stay as history. Since section 178 the one `prodomo` binary reads
`prodomo.toml` and binds the auth listener and every Channel port, and since section 179 it migrates
a PostgreSQL 18 store before admitting anyone; `prodomo account` and `prodomo gm` create accounts,
set passwords, change Coins and Cash, and grant GM authority.

The direction since then:

- Only the client protocol and the Game data file formats are a contract (ADR-0001).
- One `prodomo` process runs auth, Channels 1-4, and the Shared Channel on one game thread at 25
  Pulses per second (ADR-0002).
- The store is a fresh PostgreSQL 18 database (ADR-0003).
- Quests run on Lua 5.1 through `mlua`, compiled by a Rust port of `qc` (ADR-0004).
- The safebox and the mall belong to the account, in a column of their own (ADR-0005).

## The build order until ledger 678


| step | scope | state |
|---|---|---|
| 1. Restructure | Retire the DB-peer and GG code and move the pure rule modules into `gamedata`. Rename `game-server` to the single `prodomo` binary with TOML configuration for the auth and Channel listeners, the Channel map sets, and PostgreSQL. First PostgreSQL schema and migrations. The Operator command that creates accounts and GMs. | **Done.** Retirement and `gamedata` (177). The rename, the TOML document, and the listeners (178). The account and GM schema, store readiness, and the Operator commands (179). |
| 2. Parity inventory | Every legacy system and handler, listed from the source in `.scratch/parity/`, each with a porting status. The scripted-client test crate. | **Done** (181). 1,643 rows in nine tables; `.scratch/parity/spec.md` has the statuses, the regeneration command, and four findings for the owner. The scripted client is the `parity` crate; its scenarios are `prodomo/tests/parity.rs`. Two rows are `ported` (the keepalive and unknown-header framing rules). |
| 3. Vertical slice | Handshake and TEA, auth (`LOGIN3`), login by key, character select, create, and delete, loading, entering the game, movement and chat, a Warp between maps, and logout with save. | **Done** (228). The handshake, TEA, time sync, and the ping cycle are live (182), and so are the Channel status list (183) auth `LOGIN3` with its login keys (184), the Channel login by key (`LOGIN2`) with the character list (185), the select screen's empire choice, character create, delete, and forced rename (186), and character select, the loading burst, and enter-game (187), each with a scripted-client scenario. Movement, chat, the Warp home move, and the save cycle and logout save are now live (188, 189, 190). The in-game Warp waited for a caller an honest player can reach, so step 4 started at items and inventory. Ledger 228 found one in the boot NPCs of ledger 223: the warp NPC event (`StartWarpNPCEvent`), whose NPCs stand on every starting map. A player within 300 of one is sent its own `GC_CHARACTER_DEL` and `GC_WARP`, reconnects to the address and port the record names with its login key, and loads at the destination, with a scenario. The other callers (`pc.warp` and the rest in the quests, `GUILD_SKILL_TELEPORT`, `/mto`, `GoHome`) land with their step 4 systems. |
| 4. Game systems | In dependency order: items and inventory; NPCs, shops, and Transfers (trade, safebox); the quest runtime (`qc` port and Lua 5.1, with its API growing as each later system lands); monsters, combat, drops, and exp; skills and affects; party, guild, messenger, and the cross-Channel bus; dungeons, events, guild war, and OX; the Prodomo custom systems (sash, aura, pets, battle pass, switchbot, item shop, premium shop, and the rest); GM commands, the adminpage, and logs. | **Started.** The item prototype and the item locale names are read from the legacy files (191), and `special_item_group.txt` through the ported `CTextFileLoader` (192), with the tables, lookups, refusals, and the compiled apply-type range pinned against the owner’s data. `data.item.special_group` and `sys.item.proto` are `partial`; no row is `ported` yet, because a data reader is not a system a client can reach. The four item-window game-to-client records are now codec-only (193): `GC_ITEM_SET` 72, `GC_ITEM_DEL` 62, `GC_ITEM_UPDATE` 59 and `GC_ITEM_GROUND_ADD` 21, with the six sockets and seven attributes the `ENABLE_EXTENDED_SOCKETS` build actually uses. `sys.item.core` is `partial`: an Operator console (207) can give an online character an item and take it away again, both under a scripted-client scenario (208), and the grant's `GC_ITEM_SET` really reaches the client's socket. A client can move, split and merge items in the base inventory, the six banks and the belt (211): `CG_ITEM_MOVE` runs `MoveItem`'s checks in legacy's order, the rows are stored in one transaction before the records are sent, and a cell is cleared with a byte-21 item set of vnum 0. Equipping, the dragon soul window and the switchbot are refused as not ported; use, drop and pick up are still unhandled. The inventory load is ported for the windows this build can hold (210): at character select the stored rows are placed the way `CInputDB::ItemLoad` places them, one `GC_ITEM_SET` per placed item follows the loading burst, then the gold and points records again, and the world admits the character already holding the items, so a relog puts every item back in the client's window and a later grant cannot be offered a loaded item's cell. Switchbot and attribute-window rows are refused with a warning and kept, because those systems are not ported. The equipment load is ported (213): an `EQUIPMENT` row is worn at `180` plus its cell when the cell and its level limit pass, and set aside into the inventory otherwise, as `CInputDB::ItemLoad` does, and a worn item that would start a system not ported yet (the dragon soul deck, an aura, mount or pet costume, a unique item, the item timers, accessory stones, immunities, a bonus whose point is not ported) is refused and kept. Ledger 209 corrected what that load hangs off: there is **no `CG_ITEM_LOAD` on the client wire** (`ITEM_LOAD` in `packet.h` is 0 against 11 for `CG_ITEM`; the name is the retired DB-peer `HEADER_DG_ITEM_LOAD`, `tables.h:196`). The trigger is `CG_CHARACTER_SELECT` (byte 6), whose loading phase `CInputDB::ItemLoad` runs inside, so the inventory is the server's send and needs no new inbound handler. The hook is `select_character` (`prodomo/src/main.rs:1467`), not `enter_world`. The points engine is ported (212): `world::character::Points` holds legacy's two point arrays and ports `PointChange`, `ComputePoints`, `ComputeBattlePoints`, `CheckMaximumPoints`, `GetLimitPoint` and the conqueror will, so the loading burst carries the battle grades a character really has, the item load clamps a stored pool above its maximum with a `GC_CHARACTER_POINT_CHANGE`, and the save writes the clamped pools. The level-up chain, the currencies and the inventory unlocks are refused as not ported, and the passive-skill bonuses are an input that is 0 until skills supply it. Since 213 the load's points apply every worn item's bonuses (`ModifyPoints` and `ApplyPoint`), the worn armour and the set bonuses, and the parts the client draws are the worn armour, weapon, hair and sash over the base part. Ledger 214 laid the groundwork the equip reducer needs: the cell key is a deferrable constraint, so a swap commits in one transaction and a real clash is still refused; a socket change is a row change; `world::character::dice` ports `number()` over a source the game thread will own; and `Equipment::part_change`, `removal_applies` and `refine_element_type` give the part, the negated bonuses and the look's element that taking an item off or putting it on sets. Ledger 215 ports equipping through `CG_ITEM_MOVE`: `world::character::equip` is `EquipItem`, `SwapItem`, `UnequipItem`, `EquipTo` and `Unequip` with their checks, notices, effects and the sash roll, and the move sends the item and points records and `GC_CHARACTER_UPDATE`, stores the rows, and holds the new points and parts for the save (`an_armour_is_worn_swapped_and_taken_off_and_the_store_follows`). Ledger 216 ports `CG_ITEM_USE` for the eight item types whose use is `EquipItem` or `UnequipItem`, with `UseItem`'s job, sex, level, champion and belt checks before them (`an_armour_is_used_on_swapped_and_used_off_and_the_store_follows`); any other use is refused as not ported. Ledger 217 ports drop and pick-up: `world::character::ground` is `DropItem` and `PickupItem`, the game thread holds the ground with each item's destroy pulse, and every client on the map is sent `GC_ITEM_GROUND_ADD` and `GC_ITEM_GROUND_DEL` (`a_dropped_stack_lies_on_the_map_until_someone_picks_it_up`, `a_dropped_item_nobody_picks_up_is_destroyed_on_time_and_its_view_sees_it_go`); gold on the ground, `CG_ITEM_DESTROY` and dropping a worn item are not ported. Ledger 218 ports the potions (`USE_POTION` and `USE_POTION_NODELAY`) and the recovery half of the affect event, which the game thread runs each second and whose points the world hands back for the save (`a_drunk_potion_pays_its_hit_points_over_the_next_seconds_and_the_save_keeps_them`); every other `ITEM_USE` sub-type, the gold bars and the ability potions are refused as not ported. Ledger 219 ports the quickslots: `SetQuickslot`, `DelQuickslot` and `SwapQuickslot` with the client's `ITEM_USE` check, a `quickslot` table the save writes, and the load's set of every stored slot between the main character and the gold (`the_quickslots_are_set_swapped_and_deleted_and_a_relog_sets_them_again`). Ledger 220 ports `SyncQuickslot` and `ChainQuickslotItem` for the item steps already ported: a whole move in the inventory window sets the slot again on the new cell, an item a merge or a drink uses up chains the slot to the first item of its vnum left (a potion, an ability potion or vnum 70020) or deletes it, and a drop deletes it (`a_quickslot_follows_its_potion_chains_when_it_is_used_up_and_a_drop_deletes_it`). Ledger 221 reads the eleven `locale_string.txt` files at start-up (`gamedata::locale_string`) and ports `CHARACTER::ChatPacket` (`prodomo::chat_line`): every server line is looked up in the descriptor's language (`LC_LOCALE_TEXT`) unless it is a command line, then formatted, and `GC_CHAR_ADDITIONAL_INFO` and `GC_CHARACTER_UPDATE` carry the descriptor's language instead of 0. No line the Rewrite sends is a key in any of the owner's tables, so the owner's players read the same text as before. Ledger 222 ports the shout: the line goes to every Channel through the client registry, to the shouter's empire or, under `[game] enable_global_shout`, to every empire, each listener reading it in its own language and empire, with the level limit from `[game] shout_limit_level` and the fifteen-second cooldown (`a_shout_reaches_its_empire_on_every_channel`, `a_shout_from_another_empire_stays_in_that_empire`, `a_global_shout_reaches_every_empire_on_every_channel`). It also resets the chat counter every five seconds of Pulses, which the Rewrite had never done, so until 222 a session's fourth line was dropped and its tenth disconnected (`the_chat_counter_is_reset_every_five_seconds_of_pulses`). Ledger 223 stands up the NPCs: the mob proto, the `LOCALE_YMIR` mob names and each hosted map's four regen files are read at start-up (`gamedata::mob_proto`, `gamedata::mob_locale_names`, `gamedata::regen`), and `world::npc` spawns every NPC, warp and goto entry in legacy's order with legacy's draws. A client entering a map is sent every one of them as `EncodeInsertPacket` writes it, then the map's `GC_NPC_POSITION` list, and a click on anything is logged and answers nothing (`entering_a_map_shows_its_npcs_and_a_click_keeps_the_connection`). The same section corrected the own insert's `bType`, which the Rewrite had sent as 1 (`CHAR_TYPE_NPC`) since ledger 187; it is `CHAR_TYPE_PC`, 6. Monsters, stones, groups and ore veins are read and counted but not spawned, because they need the cell attributes and the monster AI. Ledger 224 ports the NPC shops: the `shop` and `shop_item` tables are read from the owner's `player.sql` and laid out as `CShop::SetShopItems` lays them out (`gamedata::npc_shop`), a click on a keeper whose click trigger is the shop's opens its window, and `CG_SHOP`'s buy, sale of a whole stack or of a count, and close follow `CShopManager` and `CShop::Buy` (`world::character::shop`, `prodomo::game_state::shop`). Each buy and sale stores the item rows and `player.gold` in one transaction (`db::items::apply_transfer`, ADR-0003) before its records are sent (`a_keeper_opens_its_shop_and_a_buy_and_a_sale_are_stored`). The renewal `shopex`, the personal shop, the logs and the Monarch tax are not ported. Ledger 225 ports the trade: `CG_EXCHANGE`'s start, offers, accept and cancel follow `CInputMain::Exchange` and `CExchange` (`world::character::trade`, `prodomo::game_state::trade`), the settlement moves both sides' items and gold on copies of the two characters, and `db::items::apply_exchange` stores both sides in one transaction before either side is told (`a_trade_moves_an_item_and_gold_between_two_players_in_one_transaction`). Since then only a Transfer writes `player.gold`, adding its change to what the row holds, and the save leaves the column alone. Ledger 226 ports the safebox and the mall: `interpret_command` runs a chat line that starts with `/` (`prodomo::command`: the table, the prefix walk, the position check and the anti-flood count; every character is `GM_PLAYER` until the GM audit, and every command but the safebox's and the mall's answers nothing), the password is an argon2id hash (`db::safebox`), the rows belong to the account (`item.account_id`, ADR-0005), and each checkin, checkout and move is stored in one transaction before the client is told (`world::character::safebox`, `prodomo::game_state::safebox`, `an_account_keeps_items_in_its_safebox_and_takes_them_from_its_mall`). An open safebox refuses a trade and a shop, and a safebox load or close holds back its character's trade steps for 10 seconds. The premium and large-safebox pages, dragon soul items, the mall's item awards and the inventory lock are not ported. Ledger 227 stands up the quest runtime (ADR-0006): `quest::qc` ports `qc` and writes the 95 files of `object/` byte for byte from the 16 `quest_list` scripts, 50 of the 51 scripts compile (`_basic/change_empire.lua` is refused, as legacy's `qc` refuses it), and `quest::host` loads them and the chain of libraries into one Lua 5.1 state through `mlua`, translating legacy's Lua 5.0 dialect at load and shimming the Lua 5.0 idioms the sources use. `quest::manager` ports `CQuestManager::Click` and the state machine of a running script: a click on an NPC runs its quests before its click trigger, `select`, `wait` and `input` send the dialog as `GC_SCRIPT`, and `CG_SCRIPT_ANSWER` and `CG_QUEST_INPUT_STRING` answer it (`prodomo::game_state::quests`, `a_quest_npc_answers_a_click_with_its_dialog_and_runs_it`). Of the 699 API names 24 are backed: the 18 the dialog, the chat lines and the NPC lookups need, the 4 `settings.lua` recorders, `get_locale_base_path` and `q.yield`; every other one ends its script as not ported. A character whose script waits for its client stores nothing in the safebox, uses and drops nothing, picks up no quest item and refuses a trade when both sides accept, as legacy's `IsRunning` checks do. Ledger 228 wires the in-game Warp through the warp NPC event (`prodomo::game_state::warp_npc`), which finishes step 3: every twelfth Pulse each warp NPC checks the players within 300 of it with legacy's empire and `IsHack` checks, and the descriptor of a player it sends runs `WarpSet` (`warp_set` in the binary), which saves the row with the destination, releases the login and sends `GC_CHARACTER_DEL` and `GC_WARP` (`a_warp_npc_sends_its_neighbour_away_and_the_client_comes_back_at_the_target`, `a_warp_to_no_map_sends_nothing_and_the_character_stays`, `a_trader_beside_a_warp_is_told_why_and_stays`, `a_warp_whose_save_fails_closes_without_sending_the_client_away`); a goto NPC shows its neighbour at its target on the same map (`a_goto_npc_shows_its_neighbour_at_its_target_on_the_same_map`). Ledger 229a reads the cell attributes the monsters need: `gamedata::lzo` decodes LZO1X as `lzo1x_decompress_safe` does (a differential witness against liblzo2 matched every block of the owner's 69 maps), `gamedata::server_attr` reads each hosted map's `server_attr` whole at start-up and refuses what legacy would misread, and `world::cells` answers `GetAttribute`, `IsMovablePosition` and `GetMovablePosition` with legacy's 161-point `aArroundCoords` walk. Entering the game now places the character as `Entergame` does: the first movable point around the saved position, else the empire's recall position from `Town.txt` when the map has a sectree there and the saved position otherwise, the loading burst keeping the saved position and the logout saving the new one (`an_entering_character_stands_on_the_first_movable_point_or_at_its_empires_recall`). `data.map.setting`, `data.map.attr` and `data.map.town` are `ported`. Ledger 229b ports the view (`prodomo::game_state::view`, `view_encode`, `motion` and `sync`): each hosted map is a grid of sectrees, each player and NPC stands in one, and each player sees the players and NPCs within `VIEW_RANGE + 500` in the nine sectrees around it, kept by legacy's four view verbs and `UpdateSectree`. Entering the game, a goto and the 16-Pulse sample of a moving player insert what comes into range and remove what leaves it, both ways, with the `GC_WALK_MODE` records `EncodeInsertPacket` sends (`players_and_npcs_within_view_range_see_each_other_on_entry`, `a_walk_into_view_inserts_both_ways_at_the_sample_and_a_walk_out_removes_both`, `an_npc_appears_and_disappears_as_a_player_walks`, `at_zero_stamina_the_walk_mode_rides_with_each_insert`). A move runs in the world as `Goto`, `Move`, `Stop` and `StateMove`'s interpolation run it; the move, the pose, the sync, the look and the equip effects are relayed to the mover's view, the move with the Goto's duration, which had been 0 since ledger 188; a trade's distance is measured as `StateMove` measures it; and a leaving player is removed from its viewers alone (`a_leaving_player_is_removed_from_its_viewers_only`). The motion speed was the 300 units a second of a missing motion file until 229e (V2), and stamina is neither consumed nor restored (V3). Since 229c an item on the ground is an entity of the view: a drop lays it in its sectree and inserts it both ways with the players in range, a whole pick-up and the destroy event remove it, and an entering player is shown what lies in its view inside its `Show` (`a_dropped_stack_lies_on_the_map_until_someone_picks_it_up`, `a_dropped_item_nobody_picks_up_is_destroyed_on_time_and_its_view_sees_it_go`). Ledger 229d ports the revive-invisible affect and the affect event's stamina refill. Ledger 229e ports the `.msa` motions: `gamedata::pc_motion` reads the 128 player clips at start-up (`load_pc_motions` in `prodomo/src/main.rs`), and a goto's duration is the clip's speed for the mover's race, weapon stance and walk state (`GetMotionMode` and `GetMoveMotionSpeed`, `G/char.cpp:3513-3577`), 300 when the clip is missing. Owner question 14 is resolved. Ledger 229f stands the monsters up (`gamedata::mob_group`, `world::npc`): the Locale's `group.txt` and `group_group.txt` are read at start-up as `LoadGroup` and `LoadGroupGroup` read them, and boot runs `regen_spawn` for every entry of every hosted map, with legacy's draws in legacy's order, on the map's cells: monsters and stones in their boxes (`SpawnMobRange`, a stone with the spawn motion and `AFF_SPAWN`), groups and group-groups (`SpawnGroup`, `SpawnGroupGroup`), anywhere spawns (`SpawnMobRandomPosition`), and ore veins with their lifetime draw. Each entry keeps its count and its first fire's `number(0, 16)` for the regen event. A client is sent each monster, stone and vein in its view, a monster and a stone with no summary (`a_map_shows_its_monsters_and_stones_in_view`). The parity harness sets the boot-drawn characters aside in the scenarios about players and NPCs, because where they stand differs from run to run. Ledger 229g ports the target (`prodomo::game_state::target`): `CG_TARGET` runs `CInputMain::Target` and `SetTarget`, which answer with `GC_TARGET` and the target's percentage, hit points, maximum and, for a monster, its element; each `POINT_HP` arm sends the change to the characters targeting the player (`BroadcastTargetPacket`), and a character leaving the world clears the target of each one targeting it (`ClearTarget`). A monster's hit points are its proto's maximum, which nothing lowers yet. The loading phase consumes a `CG_TARGET` or `CG_ITEM_USE` as `@fixme120` does (`a_click_targets_a_monster_and_a_player_and_a_leave_clears_it`). Until 229g a click on any character closed the connection. `CheckTarget`'s distance drop came with the player recovery event in 229k. 229h ports the regen event (`regen_event`): each regen first runs `number(0, 16)` plus its time seconds after boot and then every `time` seconds, and stands `max_count - count` more units as boot did (`a_full_regen_stands_nothing_more_each_time_it_runs`). An ore vein ends at its drawn 420 to 900 seconds (`kill_ore_load_event`), and a non-player's `Destroy` counts its regen one fewer, clears the target of each one targeting it, frees its followers, and sends its removal to each viewer; its regen's next run stands another (the world test `an_ore_vein_ends_at_its_time_and_its_regen_stands_another`). The dungeon regen waits for the dungeons (V16). Ledger 229i ports the state machine of the NPCs (`prodomo::game_state::mob_ai`): each sectree counts the players around it, so an NPC a player's view takes in, or that enters a sectree a player stands near, joins the state set on the next Pulse, and one whose sectree no player stands near any more leaves it (`StartStateMachine`, `IncreasePC`, `DecreasePC`). `CFSM` runs each NPC's Idle and Move: a stone waits a second, a non-player 5 seconds and a monster 1 to 5, one time in seven a monster runs and a non-player walks to a random point 300 to 700 or 200 to 400 away whose way is movable, telling its view with `GC_MOVE` `FUNC_WAIT` (and a non-player its walk mode), a coward runs away along a movable way, and a waiting NPC may say its `locale.monster_chat` line as a talk with its VID; an insert of an NPC mid-walk carries its move (`an_npc_near_a_player_wanders_and_tells_its_view`). The fight, the protector and `Follow`, a stone's spawns and the monster clips are not ported (V2). Ledger 229j ports a player's melee hit on a monster (`prodomo::game_state::fight`, `world::character::combat::melee`): `CG_ATTACK` runs `CInputMain::Attack`, the reach of `battle_melee_attack`, `battle_hit` and `CalcMeleeDamage` with every draw in legacy's order, and `Damage`'s normal branch, which shows the attacker that targets the victim the damage before the clamp, its targeters its hit points, and its view a critical or penetrating hit's effect, a steal's fly and the monster's `MonsterChat(ATTACKED)`. The attacker claims the monster (`SetSyncOwner`, `GC_OWNERSHIP`) and its revive invisibility ends. A hit that leaves no hit points stuns the monster (`GC_STUN`), and 75 Pulses later `StunEvent` runs `Dead`, which clears the killer's target and sends the release and `GC_DEAD`; a hit on a stunned monster runs `Dead` at once. 25 Pulses after the death `dead_event` destroys it, which sends the release and its removal and frees its regen. A hit monster gives back its regen percentage every regen cycle until it is full (`a_punch_on_a_targeted_dog_stuns_it_and_it_dies_three_seconds_later_and_leaves`, `a_second_punch_on_a_stunned_dog_kills_it_at_once_with_no_number`, `an_untargeted_punch_shows_no_number_and_one_beyond_reach_does_nothing`, `skill_shoot_and_fly_frames_and_refused_victims_keep_the_connection`). No other victim, no reward and no monster fighting back yet (V17). 229k ports the player's death and restart (`prodomo::game_state::death`): a lethal hit stuns a player (`GC_STUN`) and 75 Pulses later `Dead` releases its own ownership and what it owns, enters `PHASE_DEAD` and sends `GC_DEAD`; 180 seconds later `dead_event` sends it to its empire's town with half its hit points. `/restart_here` (allowed 9.04 seconds after the death) and `/restart_now` (for 300000 gold, through the store) revive it where it lies with the revive invisibility, and `/restart_town` (allowed after 6.04 seconds) warps it home with 50 hit points; `DeathPenalty` takes the exp a monster kill owes. The Dead phase reads the main input as legacy does, the player recovery event drops a target 4800 or more away, a disconnect while dead stores 50 hit points and pays the penalty, and a sit while dead is refused (two legacy backdoors fixed), and `/restart` no longer charges gold. No binary path kills a player before 229l, so world tests prove the dead arms (owner, 2026-10-14) and `a_living_player_restarting_is_told_to_close_the_restart_window` the living one (V18). The next step-4 work is 229l onwards: the monster fighting back, then drops and exp. The slot space that floor needs is now measured: one flat array of 1370 cells plus a 1152-cell dragon soul array, the 11-member `EWindows` (the earlier 8-member table was wrong from byte 6), and legacy's two disagreeing `IsValidItemPosition` functions, all recorded in ledger 194. That floor now has a tenant (195): `world::item` holds the instance as an owned value with a validated count, socket and attribute write path, and `world::item::ItemIds` allocates monotonic ids that are never reused and never 0; `world::character::items::CharacterItems` holds the four arrays with the one-based grid, the 5-cell and 8-cell stack walks, and explicit `Rejected` answers for a conflict, a duplicate owner, a missing cell and a footprint that does not fit. That unit also corrected three live constants the workspace had wrong: the socket count is 6 under `ENABLE_EXTENDED_SOCKETS` (two literals in `common` said 3 and 6), and the Dragon Soul stack stride is 8 (`length.h:84`), not the 6 ledger 194's probe transcribed. The instance also **lost** the limits 195 had given it, because `aLimits` belongs to `TItemTable` (`tables.h:879`), the prototype, and not to the instance record (196).

The instance now has a table and a query path (196, 197). `db/migrations/0005_items.sql` stores one item per row, with `owner_id` a typed reference whose NULL means the ground -- which is what makes the window/owner biconditional a real rule -- and a unique index on `(owner_id, window_type, pos)` so two rows cannot claim one cell. Probing PostgreSQL 18 with boundary values found six columns that had been sized from legacy's SQL type rather than from the C++ field they store, all now `bigint` or widened. `db/src/items.rs` reads and writes it. Two rules there are load-bearing: the table is never written with `REPLACE`, because MySQL answers the cell conflict by deleting the item that was in the way, and every delete names the owner, which legacy's does not. The load also refuses to repeat legacy's silent row-dropping, so a row this build cannot decode is an error and not a lost item. The grant now has somewhere to put an item (198). `gamedata::item_custom_category` ports `CItem::IsCustomCategory` (`item.cpp:3002-3148`), the hard-coded six-bank membership table that is not proto data and so had to be transcribed; the tests pin that the `71083` guard and category 5's vnum list are both dead, and that **5 of the owner's 7,305 items are in two categories**, so legacy's "first category with a free cell" scan and `CItem::GetItemCategory`'s "first category that matches" genuinely disagree. `CharacterItems` can now find a free cell, and the search and `set` share one `footprint_is_clear` so the search cannot name a cell that placement would refuse. Its one rule is the 45-cell page, which each custom bank counts from its own start. Legacy's pickup-shaped search ignores the inventory unlock stat and can drop an item into a page the client will not draw; that is a Defect and the Rewrite bounds on the usable cell count instead (198.3).

An item can now be given to a character and the client record built (199). `Character` owns its four item windows, because legacy has `pItems` and the rest as `CHARACTER` members (`char.h:458-478`) and the type ledger 195 built was owned by nothing, so a grant was impossible for a structural reason no store work would have fixed. `world::character::grant` ports the placement half of `ACMD(do_item)`: every matching custom bank in ascending order, **first bank with a free cell**, then the base inventory bounded by the unlock stat. The matching banks are an argument rather than a lookup, because category membership is `gamedata` data and `world` does not depend on `gamedata` -- so every placement test runs without a Game data file. `Item::gc_item_set` is the first client record: the tests pin all nine fields individually, assert the measured 72 bytes and header 21, and decode socket 2 back out of the wire to answer the sign question without the missing i686 compiler.

The grant is now reachable from a real client. It is one transport-free reducer (200), `prodomo::item_grant::grant_item`, which returns the store row and the client record together so they cannot disagree about the id or the cell. It runs on the game thread that owns the world (201, 202), the world's item-id allocator is installed from the store after the listeners bind and before the ready gate opens, so no client can reach a world that answers no grant (203), and `prodomo::item_persist::persist_grant` writes the row before the client is told and takes the world mutation back when the write is refused -- `Persisted::Undone` when the repair worked, `Persisted::Diverged` when it did not, kept apart because only the second leaves a player holding an item with no row (204). A live client's character now enters and leaves that world (205), under the VID the enter-game burst already published, and the world writes to the client through a clone of the descriptor's existing outbox. That last one was the gap nobody had closed: until 205 a live client was in `ChannelClients` and `PositionTable` and nowhere else, so the game thread stepped an empty room and every unit test that "proved" a grant built the world itself. A scripted-client scenario now runs the whole legacy login order against the real binary and reads the console, which is what makes ADR-0002 true of a live client. Leaving the world runs before the final save, because `PlayerDestroy` frees the cells the character's items hold. **Still `codec`**, and the reason is now only the delivery: no path calls `persist_grant` and then sends `GC_ITEM_SET`, and an Operator had no way to ask. Both are closed. `grant_and_deliver` (206) does the world, the row, and the client in that order and names all three outcomes, so a row that landed for a character whose descriptor has gone is `Stored` and must not be re-granted, and a refused write sends no record at all because the world was already repaired. The Operator interface is a named pipe the serve process reads (207): `[game] operator_console` creates a `0600` FIFO and one command per write, `item give <name> <vnum> [count]`. It has to be in-process — the world is only `serve`'s memory (ADR-0002) and a channel from another process is the protocol class ADR-0001 retired — and legacy has no console at all, so it is new rather than a port. Since ledger 221 the console holds its pipe open for reading and writing, so a command written right after another is never lost, and it answers in the log only (the first option of 209.2). **Still `codec`**, and the reason is now one step: the console grants to an *online* character (decision D7), and what is missing is the destroy half and the scripted-client scenario that survives a relog. The decisions are written down in `.scratch/ledger198-operator-item-grant.md`.
| 5. Game data and play test | Finish the importers, fix what the full data set breaks, then the owner's play test with the Reference client. | Not started. |

The game thread now owns a world (201). `prodomo::game_state::GameState` holds the
`CharacterManager`, the `ItemIds` allocator, and the `ItemProtos` table, and implements
`PulseProcessor`; `serve` still passes `|_| {}` to the loop, and that is deliberate
rather than forgotten. The client ports open before the world is ready, so there is no
point at which `serve` holds both the bound port and the store's item-id range, and
inventing a range to get past the gap would put a made-up default in front of every
stored item. The loop is spawned, then the accept loop migrates, then the gate opens;
the range is installed through the game thread in the unit that adds the command
channel. One test in `prodomo/tests/game_loop_thread.rs` was renamed in the process:
it claimed to prove a game state ran on the thread while spawning an empty closure, and
it now asserts what it checks.

A grant can now cross into the thread that owns the world (202). `GameCommand` carries
`GrantItem { request, reply }` and is neither `Copy` nor `Clone`, which is what a
per-request `oneshot` answer costs and what it buys: a caller that acts on a dropped
effect has acted on nothing, so a grant answers through a channel that either delivers
the outcome or closes, and the caller reports "no answer" as a different type from "the
world said no". The loop delegates everything that is not `Stop` to the processor, and
`GameState::apply` runs the grant against the target's own `Inven_Point`, which needed
a new `envanter` field on `Character` and gives Divergence 202.1 when it is clamped.


`serve` now starts a world (203). It loads the item protos before it binds, moves a real
`GameState` into the game thread, and installs the world's item id allocator from inside
the accept loop once the store has migrated, before the ready gate opens. The allocator
had to be a command rather than a constructor argument: its start id is `MAX(id)` over
the item table, and reading that needs a connection, which `serve` documents it does not
open before the ports are bound. The span is a `[game]` key,
`item_id_range = [first, last]`, defaulting to the owner's own `ITEM_ID_RANGE`; it is
refused in `validate` when it does not ascend, and a span with fewer than 10,000 ids
left stops the process rather than admitting a client to a world that cannot give items
out. What is still missing is the far end: nothing writes the row, sends `GC_ITEM_SET`,
or authorizes a caller.

The Game data arrived before step 1 (`legacy/`), so each system's reader or importer is built with
the system that first needs it, starting with the maps and protos in the vertical slice. Step 5 is
what is left over.

Quests come early in step 4 because NPC clicks, level-up rewards, and skill selection all go
through them.

## What exists and what happens to it

| code | today | in the Rewrite |
|---|---|---|
| `protocol` CG and GC codecs, TEA, inventories | 91 of 92 CG and 106 of 134 GC records, golden-byte tested. Most CG codecs have no caller. | Kept. |
| `protocol` `db_*` and `gg*` modules | Deleted in 177. `TSimplePlayer` moved to `protocol::simple_player`. | Done. |
| `net` | Client framing. `buffer.rs` and the DB-peer transport were deleted in 177. | Kept. |
| `db` | `store` (the PostgreSQL pool, the embedded migrations, and the transient-error rule), `credentials` (logins, argon2id passwords, delete codes), `accounts` (accounts, Coins and Cash, GM grants), and `item_id_range`. Tested against PostgreSQL 18.6 when `DATABASE_URL` is set. | Grows with each system's tables. |
| `gamedata` | Packed table record layouts and nine table rules (banword, event, item_attr, land, object_proto, refine, renewal_shop, shop, skill) as typed builders over caller-supplied rows. New in 177. File readers added with the system that needs them: `mob_names` (186), `item_proto` for `item_proto.txt` and `item_names.txt` (191), the `CTextFileLoader` port `text_file` and the `special_item_group.txt` bag reader (192), and the `ProtoReader.cpp` value tables `item_proto_value`, including the compiled apply-type range. | Readers and importers for `legacy/gamedata` are added with the system that first needs them. |
| `world` | Spatial model, characters, events, and some combat rules. Never imported by a binary. | Kept; grows with step 4. |
| `quest` | The `qc` port, the dialect translator, the Lua 5.1 host with its Lua 5.0 shims, and the manager with the click, the chat menu and the dialog state machine (227, ADR-0006). | Its API grows with each ported system. |
| `prodomo` | Renamed from `game-server` in 178. `prodomo serve` reads `prodomo.toml`, creates a lazy store, binds the auth listener and every Channel port (`listeners`), starts the game loop, migrates the store (retrying while it is unreachable), and only then opens `ReadyGate` (179). Every admitted connection runs `client_live` (182): the handshake on accept, TEA, time sync, keepalive, and the ping cycle. `prodomo account` and `prodomo gm` are the Operator commands (`operator`, 179). Reducers: `ClientLifecycle`, handshake, heartbeat, `DescriptorCrypto`, `client_live`, `AccountPlayerSession` and its router (re-based on the typed `account_records` in 177), `sync_position`. | `ReadyGate` will also wait for the loaded Game data once step 3 loads it. |
| `db-server` | Deleted in 177. GM list rules moved to `common::gm`, item-ID ranges to `db::item_id_range`, and the nine table rules to `gamedata`. The `item_proto` and `mob_proto` SQL decoders did not move, because the protos are read from the text files; the `object`, `market_price`, `monarch`, `player`, `player_index`, `quest`, and `login` modules did not move, because that state lives in the fresh store. | Done. |
| `tools/packet_compare` | Deleted in 177. | Done. |

## Vertical slice: what exists

| stage | present in Rust | missing |
|---|---|---|
| Handshake and TEA | Live on every connection with scenarios (182). The Channel status list (183). The login's `SetSecurityKey` (185). | The status list counts characters in game once entering the game publishes the count (`ChannelStatusBoard::set_online`). |
| Auth | Live with a scenario (184): every legacy check in order, `AUTH_SUCCESS` (0x96) with a login key, and `LOGIN_FAILURE`. `prodomo::auth_login` holds the rules and the login-key registry. | Premium times on the login data (no columns yet). |
| Login by key | Live with scenarios (185): `SHUTDOWN` under the handshake key, then the client key pair, the login-key judgement (`NOID`), the logon registry (`ALREADY` and the kick of a holder without a character), `GC_EMPIRE`, the 357-byte character list with each map's Channel address from the map atlas (`gamedata::map_atlas`), and `PHASE(SELECT)`. `prodomo::channel_login` holds the rules; `db::players` reads the characters. | `FULL` (unit-tested; the online count arrives with entering the game). The delayed kick of a holder in game. The blocked-country IP list (`is_blocked_country_ip`, `G/block_country.cpp`; empty tables behave as today). Guild ids and names (zeros until guilds exist). |
| Select, create, delete | Empire choice, create, delete, and forced rename are live with scenarios (186): the Name rules (letters and digits, 2 to 24 bytes, the `banword` table read from `legacy/sql/gamedata/player.sql`, and the lowercase mob names from the text protos), the create checks in legacy order, the job points, the create start and spread, the 30-second create cooldown per account, the delete code and level limits (`[game]`), and the deleted row kept in `player_deleted`. `prodomo::select_phase` holds the rules; `db::players` writes the rows. Choosing a character (`CHARACTER_SELECT`) is live since 187: the slot index, the store read, and the empty-slot close. | The `CREATE PLAYER` character log row (`G/input_db.cpp:274`; the log store does not exist yet). |
| Loading | Live with scenarios (187): the store read, `GC_ENTITY` (249), `GC_MAIN_CHARACTER2_EMPIRE` (113), `GC_CHARACTER_GOLD` (224), `GC_CHARACTER_POINTS` (16), `GC_SKILL_LEVEL` (76), and the map test at the legacy split. `prodomo::loading_phase` holds the bursts. | The quickslots (28-30), the package SDB (153), and the safebox query: not records this deployment can send, per ledger 187.5. `GetValidLocation` and the movable-position fallback need a map instance. |
| Enter game, movement, chat | The enter-game burst is live with a scenario (187). Live with scenarios since 188: `CHAT` (3) with the counter, the block-chat arm, and the map broadcast that reaches the sender; `MOVE` (7) with the 750/999 distance limits, `CanMove`, the refusal's `Show`, and `Goto`, `Move`, `Stop` and `StateMove`'s interpolation in the world, relayed to the mover's view with the move's duration (229b); `CHARACTER_POSITION` (28) for sit and stand; `SYNC_POSITION` (8) with the whole element loop, `SetSyncOwner`, `GC_OWNERSHIP` (62) relayed to the view, the owner range, the 100 ms interval, and the displacement close. The shout to every Channel with its empire filter, level limit and cooldown, and the chat counter reset every five seconds of Pulses (222). The view (229b): the players and NPCs within `VIEW_RANGE + 500` in the 3x3 sectrees, inserted on entry, on a goto and every 16 Pulses while a player moves, and removed as they leave it, with `GC_WALK_MODE` (111); the map's `GC_NPC_POSITION` list; `CG_ON_CLICK` (26) is decoded, logged and answered with nothing (223), except that a click on a shop keeper opens its shop (224). A chat line that starts with `/` runs `interpret_command` (226). `prodomo::chat`, `prodomo::movement`, and `prodomo::sync_position` hold the rules, and the live framing now resolves a variable-length frame (`net::ClientFrameTransport`). | The post-phase events. Every NPC click trigger but the quests' and the shop's. Whisper, the banword conversion and the prism check; every command of `interpret_command` but the safebox's and the mall's, and its GM levels (every character is `GM_PLAYER` until the GM audit); the shout log, the battle pass shout mission, and the GM clause of the shout filter (no character has a GM level yet); the riding and OX-event speed checks; `StateMove` beyond its interpolation, its 16-Pulse sample and its trade check (stamina, walking); a skill move's `IsUsableSkillMotion` check, whose `SKILL_HACK` hack log and delayed disconnect wait for the skills (`G/input_main.cpp:1841-1866`); `SetSyncOwner`'s `AIFLAG_NOMOVE` and `battle_is_attackable` arms; the `sync_hack_count` restore with the save path. |
| Warp | The login home move is live with a scenario (189): a character whose stored map is not hosted by the Channel is moved to its empire start in the same transaction that refuses it (`db::players::save_position`), and the close is silent because legacy's is. `prodomo::warp` holds the whole pure policy with 20 tests, including the 15-byte `GC_WARP` projection, `judge_warp_set`, and `judge_warp_end`. `EMPIRE_START_MAP` (`g_start_map`, `[0, 1, 21, 41]`) is kept apart from `EMPIRE_START` and is checked against the real atlas. The in-game Warp is live with scenarios (228): the warp NPC event and its `IsHack` check (`prodomo::game_state::warp_npc`), `WarpSet` on the player's descriptor with the destination saved before `GC_WARP` leaves (`prodomo::warp::departure_records`), the reconnect by login key at the address and port `GC_WARP` names, and `CG_WARP` (65), whose `WarpEnd` returns at once on every descriptor the Rewrite keeps. The goto NPC's `Show` and `Stop` are ported, through the view since 229b (`prodomo::game_state::warp_npc`), with a scenario on a synthetic goto NPC, because no goto NPC stands on a map the owner's Channels host (`a_goto_npc_shows_its_neighbour_at_its_target_on_the_same_map`). | The other callers: `pc.warp`, `pc.warp_local`, `pc.warp_to_guild_war_observer_position`, `d.new_jump`, and `warp_to_village` land with their quest systems, `GUILD_SKILL_TELEPORT` (158) with the guild, `/mto` and `GoHome` with theirs. A private map (the row has no `map_index` column); the Shared Channel return (a Divergence, reached by no warp NPC); the switchbot and the `CharLog` table. The war- and wedding-map login Warp (`G/input_login.cpp:876-885`) is off under the owner's `test_server = true`. The OX-map login Warp (`G/input_login.cpp:853-874`), which sends a `GM_PLAYER` who enters map 113 while the OX event is finished or off its two spots to the empire start, lands with the OX event (`sys.event.ox`). Whether the Reference client sends `CG_WARP` is confirmed in the owner's play test. |
| Logout and save | Live with scenarios (190): the save event that `StartSaveEvent` arms at enter-game, the drain the game loop would do every 29 Pulses, the disconnect save that `FlushDelayedSave` performs, the `SaveReal` guards, the whole-row `db::players::save_character` (every column but the gold since 225), and the `POINT_PLAYTIME` carry with its `> 60000` guard and sub-minute remainder. `prodomo::save` holds the rules. A Channel change and a shutdown are both a disconnect here, so they are covered by the same save. Since 229k a player that leaves stunned or dead pays the here penalty and is stored standing with 50 hit points (a legacy backdoor fixed). | The shared delayed-save queue and its 29-Pulse drain, which need ADR-0002's game thread; the event currently runs on the owning descriptor. The logon record. `sync_hack_count` is still in memory only, so it is not in the save; so is `POINT_MOV_SPEED`, which nothing changes yet. Item and affect saves (`FlushDelayedSaveItem`, `SaveAffect`). The `save_event_second_cycle` default of 120 seconds is legacy's value, not the 3 minutes its comment claims. |
