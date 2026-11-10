# Rewrite status

Last reviewed: 2026-11-09, after ledger section 514 (the costume mount).
Steps 1, 2 and 3 are done; step 4 is in progress, and its next work is 515
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
Since 506 `cmd.set_socket` is `ported`: a socket write to an item in an open safebox or mall reaches the opener and the account row. Since 507 `cmd.in_game_mall` is `ported`: any character hears the signed web-shop line, and the command table has no `missing` row left. Since 508 `cg.any.analyzer_refusal` is `ported`: an undersized variable frame closes the connection end to end. Since 509 `sys.item.casket_preview` is `ported` without new code: the fetch arm is live and the quest preview map is provably empty. Since 510 `sys.guild.bonuses` is `ported`: the four level windows land their affects through all six seams, the overlaps resolve first-match, and the removal's first-kind break keeps the stale rows while guildless. Since 511 `sys.net.ip_ban` is `ported`: the `[access]` table's `ban_ip` ranges refuse at the auth listener with the `BANIP` mask-and-hash match, and its `block_country_ip` ranges refuse at Channel login before the shutdown gate unless `block_exception` names the lowercased login. Since 512 `sys.dungeon.info` is `ported`: the admit fans the full cooldown line after the recovery event starts, and every login reader hears it ahead of the rank lines. Since 513 `sys.adminpage` is `unused`: byte 64 is registered but undispatched, `IsAdminPage` has no caller, the password is only logged, and the Rewrite closes on the byte. The Since 514 `sys.char.mount` is `ported`: a used mount costume summons its follower, `/ride` mounts and unmounts it, and the take-off, login, restart, leave, tick, purge and state-idle arms are live.
next slice is the owner's call: the forest's leader branch is ported, the sash
quest's reward waits on 85000's missing proto, which is Game data, not code, and the Hydra
kill waits on the kill event reaching quest scripts with a target.

This page records where the Rewrite stands, the build order, and the next step. Rules live in
`AGENTS.md`, terms in `CONTEXT.md`, and the change history in `docs/REWRITE_LEDGER.md`. When this page
and the ledger disagree, the most recent ledger section wins; update this page in the same change.

## Summary

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

## Build order


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

The owner runs the play test on a Linux machine or VM with a `compose.yaml` (Podman or Docker) that
starts PostgreSQL 18 and `prodomo`. The README covers creating an account and making it a GM
(179); it will also cover matching the auth and Channel ports to the Reference client's
`serverinfo`. Ports default to the legacy ones.

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

None of these warps runs in the Rewrite yet, because `pc.warp` is not ported. Today the Shared Channel
carries only the warp NPC pair between maps 72 and 73, and the login of a character saved on a Shared
Channel map. The Shared Channel stays on so that these features work when they land (ledger 229f.5).
`config/prodomo.toml.example` runs Channel 1 and the Shared Channel. Channels 2-4 stay in it as
commented-out tables with their legacy maps, so each can be turned on again by uncommenting its table.

Hosting only these two Channels bounds the one game thread's load (ADR-0002). Measured
on the release build at ledger 229f, Channel 1's 57 maps stand 57,697 characters in about 0.8 s, the
process starts accepting clients 2.1 s after it starts, and it holds 262 MB (131 MB before the
monsters). With the Shared Channel's 29 maps too, it holds 65,632 characters and 291 MB. An idle
world uses no measurable CPU, because no monster acts yet; the cost per Pulse is measured again when
the monster state machine lands.

Each Channel in `config/prodomo.toml.example` keeps its legacy map set. The binary
refuses a topology it cannot run before binding anything (ledger 178.3). Two legacy settings the
owner should know about (178.5):

- `test_server` defaults to on in legacy and none of the owner's `CONFIG` files turns it off, so the
  deployment ran in test-server mode. The example keeps it on for Parity. One of its effects: legacy
  gives **every** character IMPLEMENTOR authority while it is on (`G/gm.cpp:53`), so on the owner's
  deployment every player was a GM. With it, `SetPlayerProto` also gave every character the GM
  flag `AFF_YMIR` and the protect PK mode at load (`G/char.cpp:2231`, `:2362-2375`,
  `:6354-6358`). The GM grants of section 179 do not reproduce that yet, so only a character
  below the PK protect level of 15 is in the protect mode (229b); the owner decides when
  `test_server` is audited. The restart's wait and `IsHack` run as production, which
  `test_server` skips in legacy (229k, V18i).
- `BLOCK_LOGIN` refuses accounts created on or after its date. Its legacy default is `30000705`; an
  empty value would refuse every account.

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
| A save banks the minutes played at the save cycle, a Warp and the logout, and only the save cycle's bank reaches the world's `POINT_PLAYTIME` while the character stays, so `pc.get_playtime` reads the minutes the last save cycle banked: up to one cycle (120 seconds by default) of play late (ledger 261). The minutes a bank adds do not move the alignment yet. | `CreatePlayerProto` banks at every `SaveReal` (`G/char.cpp:1519-1545`), which the delayed-save queue runs within 29 Pulses of each `CHARACTER::Save()` (`G/char.cpp:1472-1476`, `G/main.cpp:276-277`): a level step (`:4001`, `:4241`), a quest reward (`G/questpc.cpp:332-337`), a skill change (`G/char_skill.cpp:716`, `:902`), a trade, a shop and more, as well as the save event. Each bank adds 5 alignment a minute, or 60 or 120 for a negative alignment, outside a `BANPK` cell (`:1529-1539`). |
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
| `_basic/change_empire.lua` is not loaded: an `end` on line 132 closes its `if` chain early, so `qc` refuses it (ADR-0006, ledger 227). This goes when the owner fixes the source. | The same: legacy's `qc` aborts on it and it never reached `object/`. |
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
| A second drop within a second is refused (ledger 217). Ledger 227 found that the only file setting `item_drop_limit_time`, `questlib_extra.lua`, is never loaded, so the owner's limit is the stored event flag, which the snapshot lacks; the owner decides the value, and the Rewrite keeps 1000 ms until then. | `g_ItemDropTimeLimitValue` is 0, no limit, unless the event flag is stored (`G/questmanager.cpp:35`, `:1603-1606`). |
| `/click_safebox` is never refused on a dungeon or war map (ledger 226): no map of the Rewrite is one yet. | `do_click_safebox` refuses a character that is not a GM there (`G/cmd_general.cpp:3494-3498`). |
| A command this build does not port answers nothing, every character is `GM_PLAYER`, and a character is either standing or sitting (ledger 226). The GM audit gives the levels, and each system's commands arrive with it. | `interpret_command` runs every entry of `cmd_info` for a character of a high enough GM level, and refuses by the entry's position among eight (`G/cmd.cpp:610-717`, `:666-679`). |
| A checkin of an item offered in a trade is refused silently (ledger 226). | Unreachable: an open safebox refuses a trade, and an open trade refuses the safebox's load (`G/input_db.cpp:1140`). |
| The `LOCALE_YMIR` table is not loaded (ledger 221). | Loaded from the English folder (`G/locale_service.cpp:437-438`) and never read, because `locale_find` answers the text itself for `LOCALE_YMIR` (`G/locale.cpp:73-74`). |
| A dropped gold pile is laid unnumbered, and its count is `u16` (ledger 229o). A pile past 65535 gold keeps its gold on the drop and pays it on the pick-up; the count is never sent. | `CreateItem(1, gold)` numbers the pile from the shared id counter and keeps the count in a `DWORD` (`G/item_manager.cpp:160-410`, `G/item.h:384`). |
| The common-drop table sorts stably by level end (ledger 229o). It shows only with tied level ends; the owner's file is empty. | `std::sort`, unstable on ties (`G/item_manager_read_tables.cpp:102-104`). |
| V2. A player's walk or run is interpolated at the speed of its race's `.msa` clip for its weapon's stance and walk state (ledger 229e). Not ported: the mount arm and the polymorph arm of that choice, and the two recomputes of the move duration, on a POINT_MOV_SPEED change mid-walk and after a forced Goto of a victim, so a walk keeps the duration its Goto set (V2). No monster `.msa` clip is loaded, so every non-player moves at the fall-back 300 units a second (ledger 229i). Every monster's `MOTION_NORMAL_ATTACK` clip is loaded, and a swing holds it `(DWORD)(seconds * 25)`, else 50 (`G/char_state.cpp:1179-1181`; ledger 230c). | From the `.msa` motion of its race, weapon and walk state, else 300 (`G/char.cpp:3555-3578`, `G/motion.cpp:264-356`). |
| V3. A running player's stamina is not consumed and a walking player's is not restored in `StateMove`. No player is set walking or reset (`SetNowWalking`, `ResetWalking`, `/set_walk_mode`, `/set_run_mode`), and a player at stamina 0 or below is walking for `GC_WALK_MODE` (ledger 229b). The affect event's stamina refill is ported (ledger 229d); the walking flags, `PointChange`'s switch, `StateMove`'s restore and consumption, and the two commands are not (parts A, B, D and E of the stamina work). | `StateMove` consumes and restores stamina and sets walking at 0 (`G/char_state.cpp:812-851`); `PointChange(POINT_STAMINA)` sets and resets walking (`G/char.cpp:4287-4295`), and `SetNowWalking` relays `GC_WALK_MODE` (`G/char.cpp:7345`, `:7364`); the two commands set it (`G/cmd_general.cpp:2016-2026`). |
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
- Three spawn behaviours are reproduced, **provisionally**, until the owner names each a Quirk or a
  Defect (ledger 229f, owner questions 22 to 24). **D1:** a group-group picks its group uniformly:
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

## Environment

- PostgreSQL 18 runs in Podman (`postgres:18`, PostgreSQL 18.6), pulled once in section 179 under
  the owner's approval (planning Q23). On the current machine `host.docker.internal` does not
  resolve, so the container publishes its port on `127.0.0.1`; `AGENTS.md` has the commands.
- The one online `cargo fetch` the owner approved (planning Q23) ran in section 177. The offline
  gates build from the local cache.
- `rustfmt` 1.8.0 and Clippy 0.1.85 were installed by the owner after section 179; section 180
  applied `cargo fmt` to the drift that built up while they were missing and cleared every Clippy
  finding. Both gates run again.
- `i686-linux-gnu-g++-12`, used by the width probe, is not installed on the current machine.
- The **database gate runs on the current machine** as of section 187: `postgres:18` (18.6) is
  already pulled, so a Podman container on `127.0.0.1:55432` and an exported `DATABASE_URL` are
  enough to exercise every store-backed test. Section 187 ran it and left no scratch database
  behind. `DATABASE_URL` is unset by default, so every gate stays green without a database.

## Code-quality backlog

Take these on when a step touches the code.

- **Duplication.** 62 error enums, 42 `check_exact` helpers, and 37 little-endian readers. `TItemPos`
  is defined 4 times, and `TSimplePlayer`, `TQuickslot`, and `TPlayerSkill` 3 times each. There are
  4 phase enums and a duplicate `ServerState`. A shared `wire` reader and writer, one `CodecError`, a
  `FixedRecord` trait, and a `CgPacket` enum for dispatch would remove most of it. Retiring the
  DB-peer code removes some of these counts.
- **Stale slot constants in `common/src/constants.rs`** (found in ledger 210). `INVENTORY_MAX_NUM`
  is 90 there, `DRAGON_SOUL_EQUIP_SLOT_START` 154, `BELT_INVENTORY_SLOT_START` 184 and
  `INVENTORY_AND_EQUIP_SLOT_MAX` 200: the values without `ENABLE_EXTEND_INVEN_SYSTEM` and
  `ENABLE_CUSTOM_INVENTORY`. The measured values are in `common::item_slots` (180, 244, 274 and
  1370), and nothing reads the stale copies today. The future item save subtracts
  `INVENTORY_MAX_NUM` from an equipment cell (`G/item_manager.cpp:504-509`), so remove the copies or
  derive them from `item_slots` before that save is written.
- **`common/src/tables.rs`** declares 72 `repr(C, packed)` structs. Do not size wire records from
  them.
- **Silent data loss:** `bytes_to_str` (`protocol/src/lib.rs:624`) returns `""` on invalid UTF-8.
  Legacy names are raw bytes, so this hides data rather than rejecting it.
- **Panic points in non-test code.** `common/src/logging.rs:64` panics if the log directory cannot be
  created. The rest are invariant panics that cannot fire today: 8 `.expect` calls in
  `protocol/src/cg_account.rs`, and one each at `protocol/src/cg_attack.rs:134` and
  `prodomo/src/sync_position.rs:177`.
- **`#[allow]` is banned** (`AGENTS.md`: "Fix Clippy findings by refactoring, never by
  `#[allow]`"), and **59 of them are checked in**. The rule is stated as how a new finding is
  fixed, and every current use predates the rule, so this is debt rather than a violation to
  correct in passing. It is worth its own section: 14 are `clippy::too_many_arguments` in
  `protocol/src/gc_actors.rs` alone, 14 in `gamedata/src/records.rs` and 12 in
  `gamedata/src/renewal_shop.rs`. Most take a wire struct field by field, so a builder that takes
  the struct would remove them all at once, and the same builder would also remove the field-by-
  field constructor duplication the first bullet describes. Do this when a step already touches the
  file; do not do it as its own sweep, because the diff is large and unrelated to any one system.
- **Toolchain:** Rust is not pinned. Consider a `rust-toolchain.toml` for 1.85.1.
