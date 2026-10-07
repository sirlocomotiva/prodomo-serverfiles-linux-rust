# Rewrite status

Last reviewed: 2026-10-01, after ledger section 229b (the view: each player sees the players and NPCs within `VIEW_RANGE + 500` in the nine sectrees around it, inserted and removed as legacy's `CEntity` view does, and a move runs in the world with `StateMove`'s interpolation and its 16-Pulse sample).
Steps 1, 2 and 3 are done; step 4 is in progress, and its next work is 229c (ground items in the view), 229d (the revive-invisible affect) and 229e (the `.msa` motions), then the monsters (229f onwards).

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
leave it, and a move, a pose or a sync reaches that view. Monsters, combat, skills, parties, guilds
and the rest of step 4 are not ported yet. The two-process layout (a `game-server` and a `db-server`
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

Each step lands as one or more ledger sections with a gate receipt.

| step | scope | state |
|---|---|---|
| 1. Restructure | Retire the DB-peer and GG code and move the pure rule modules into `gamedata`. Rename `game-server` to the single `prodomo` binary with TOML configuration for the auth and Channel listeners, the Channel map sets, and PostgreSQL. First PostgreSQL schema and migrations. The Operator command that creates accounts and GMs. | **Done.** Retirement and `gamedata` (177). The rename, the TOML document, and the listeners (178). The account and GM schema, store readiness, and the Operator commands (179). |
| 2. Parity inventory | Every legacy system and handler, listed from the source in `.scratch/parity/`, each with a porting status. The scripted-client test crate. | **Done** (181). 1,643 rows in nine tables; `.scratch/parity/spec.md` has the statuses, the regeneration command, and four findings for the owner. The scripted client is the `parity` crate; its scenarios are `prodomo/tests/parity.rs`. Two rows are `ported` (the keepalive and unknown-header framing rules). |
| 3. Vertical slice | Handshake and TEA, auth (`LOGIN3`), login by key, character select, create, and delete, loading, entering the game, movement and chat, a Warp between maps, and logout with save. | **Done** (228). The handshake, TEA, time sync, and the ping cycle are live (182), and so are the Channel status list (183) auth `LOGIN3` with its login keys (184), the Channel login by key (`LOGIN2`) with the character list (185), the select screen's empire choice, character create, delete, and forced rename (186), and character select, the loading burst, and enter-game (187), each with a scripted-client scenario. Movement, chat, the Warp home move, and the save cycle and logout save are now live (188, 189, 190). The in-game Warp waited for a caller an honest player can reach, so step 4 started at items and inventory. Ledger 228 found one in the boot NPCs of ledger 223: the warp NPC event (`StartWarpNPCEvent`), whose NPCs stand on every starting map. A player within 300 of one is sent its own `GC_CHARACTER_DEL` and `GC_WARP`, reconnects to the address and port the record names with its login key, and loads at the destination, with a scenario. The other callers (`pc.warp` and the rest in the quests, `GUILD_SKILL_TELEPORT`, `/mto`, `GoHome`) land with their step 4 systems. |
| 4. Game systems | In dependency order: items and inventory; NPCs, shops, and Transfers (trade, safebox); the quest runtime (`qc` port and Lua 5.1, with its API growing as each later system lands); monsters, combat, drops, and exp; skills and affects; party, guild, messenger, and the cross-Channel bus; dungeons, events, guild war, and OX; the Prodomo custom systems (sash, aura, pets, battle pass, switchbot, item shop, premium shop, and the rest); GM commands, the adminpage, and logs. | **Started.** The item prototype and the item locale names are read from the legacy files (191), and `special_item_group.txt` through the ported `CTextFileLoader` (192), with the tables, lookups, refusals, and the compiled apply-type range pinned against the owner’s data. `data.item.special_group` and `sys.item.proto` are `partial`; no row is `ported` yet, because a data reader is not a system a client can reach. The four item-window game-to-client records are now codec-only (193): `GC_ITEM_SET` 72, `GC_ITEM_DEL` 62, `GC_ITEM_UPDATE` 59 and `GC_ITEM_GROUND_ADD` 21, with the six sockets and seven attributes the `ENABLE_EXTENDED_SOCKETS` build actually uses. `sys.item.core` is `partial`: an Operator console (207) can give an online character an item and take it away again, both under a scripted-client scenario (208), and the grant's `GC_ITEM_SET` really reaches the client's socket. A client can move, split and merge items in the base inventory, the six banks and the belt (211): `CG_ITEM_MOVE` runs `MoveItem`'s checks in legacy's order, the rows are stored in one transaction before the records are sent, and a cell is cleared with a byte-21 item set of vnum 0. Equipping, the dragon soul window and the switchbot are refused as not ported; use, drop and pick up are still unhandled. The inventory load is ported for the windows this build can hold (210): at character select the stored rows are placed the way `CInputDB::ItemLoad` places them, one `GC_ITEM_SET` per placed item follows the loading burst, then the gold and points records again, and the world admits the character already holding the items, so a relog puts every item back in the client's window and a later grant cannot be offered a loaded item's cell. Switchbot and attribute-window rows are refused with a warning and kept, because those systems are not ported. The equipment load is ported (213): an `EQUIPMENT` row is worn at `180` plus its cell when the cell and its level limit pass, and set aside into the inventory otherwise, as `CInputDB::ItemLoad` does, and a worn item that would start a system not ported yet (the dragon soul deck, an aura, mount or pet costume, a unique item, the item timers, accessory stones, immunities, a bonus whose point is not ported) is refused and kept. Ledger 209 corrected what that load hangs off: there is **no `CG_ITEM_LOAD` on the client wire** (`ITEM_LOAD` in `packet.h` is 0 against 11 for `CG_ITEM`; the name is the retired DB-peer `HEADER_DG_ITEM_LOAD`, `tables.h:196`). The trigger is `CG_CHARACTER_SELECT` (byte 6), whose loading phase `CInputDB::ItemLoad` runs inside, so the inventory is the server's send and needs no new inbound handler. The hook is `select_character` (`prodomo/src/main.rs:1467`), not `enter_world`. The points engine is ported (212): `world::character::Points` holds legacy's two point arrays and ports `PointChange`, `ComputePoints`, `ComputeBattlePoints`, `CheckMaximumPoints`, `GetLimitPoint` and the conqueror will, so the loading burst carries the battle grades a character really has, the item load clamps a stored pool above its maximum with a `GC_CHARACTER_POINT_CHANGE`, and the save writes the clamped pools. The level-up chain, the currencies and the inventory unlocks are refused as not ported, and the passive-skill bonuses are an input that is 0 until skills supply it. Since 213 the load's points apply every worn item's bonuses (`ModifyPoints` and `ApplyPoint`), the worn armour and the set bonuses, and the parts the client draws are the worn armour, weapon, hair and sash over the base part. Ledger 214 laid the groundwork the equip reducer needs: the cell key is a deferrable constraint, so a swap commits in one transaction and a real clash is still refused; a socket change is a row change; `world::character::dice` ports `number()` over a source the game thread will own; and `Equipment::part_change`, `removal_applies` and `refine_element_type` give the part, the negated bonuses and the look's element that taking an item off or putting it on sets. Ledger 215 ports equipping through `CG_ITEM_MOVE`: `world::character::equip` is `EquipItem`, `SwapItem`, `UnequipItem`, `EquipTo` and `Unequip` with their checks, notices, effects and the sash roll, and the move sends the item and points records and `GC_CHARACTER_UPDATE`, stores the rows, and holds the new points and parts for the save (`an_armour_is_worn_swapped_and_taken_off_and_the_store_follows`). Ledger 216 ports `CG_ITEM_USE` for the eight item types whose use is `EquipItem` or `UnequipItem`, with `UseItem`'s job, sex, level, champion and belt checks before them (`an_armour_is_used_on_swapped_and_used_off_and_the_store_follows`); any other use is refused as not ported. Ledger 217 ports drop and pick-up: `world::character::ground` is `DropItem` and `PickupItem`, the game thread holds the ground with each item's destroy pulse, and every client on the map is sent `GC_ITEM_GROUND_ADD` and `GC_ITEM_GROUND_DEL` (`a_dropped_stack_lies_on_the_map_until_someone_picks_it_up`, `a_dropped_item_nobody_picks_up_is_destroyed_on_time_and_the_map_sees_it_go`); gold on the ground, `CG_ITEM_DESTROY` and dropping a worn item are not ported. Ledger 218 ports the potions (`USE_POTION` and `USE_POTION_NODELAY`) and the recovery half of the affect event, which the game thread runs each second and whose points the world hands back for the save (`a_drunk_potion_pays_its_hit_points_over_the_next_seconds_and_the_save_keeps_them`); every other `ITEM_USE` sub-type, the gold bars and the ability potions are refused as not ported. Ledger 219 ports the quickslots: `SetQuickslot`, `DelQuickslot` and `SwapQuickslot` with the client's `ITEM_USE` check, a `quickslot` table the save writes, and the load's set of every stored slot between the main character and the gold (`the_quickslots_are_set_swapped_and_deleted_and_a_relog_sets_them_again`). Ledger 220 ports `SyncQuickslot` and `ChainQuickslotItem` for the item steps already ported: a whole move in the inventory window sets the slot again on the new cell, an item a merge or a drink uses up chains the slot to the first item of its vnum left (a potion, an ability potion or vnum 70020) or deletes it, and a drop deletes it (`a_quickslot_follows_its_potion_chains_when_it_is_used_up_and_a_drop_deletes_it`). Ledger 221 reads the eleven `locale_string.txt` files at start-up (`gamedata::locale_string`) and ports `CHARACTER::ChatPacket` (`prodomo::chat_line`): every server line is looked up in the descriptor's language (`LC_LOCALE_TEXT`) unless it is a command line, then formatted, and `GC_CHAR_ADDITIONAL_INFO` and `GC_CHARACTER_UPDATE` carry the descriptor's language instead of 0. No line the Rewrite sends is a key in any of the owner's tables, so the owner's players read the same text as before. Ledger 222 ports the shout: the line goes to every Channel through the client registry, to the shouter's empire or, under `[game] enable_global_shout`, to every empire, each listener reading it in its own language and empire, with the level limit from `[game] shout_limit_level` and the fifteen-second cooldown (`a_shout_reaches_its_empire_on_every_channel`, `a_shout_from_another_empire_stays_in_that_empire`, `a_global_shout_reaches_every_empire_on_every_channel`). It also resets the chat counter every five seconds of Pulses, which the Rewrite had never done, so until 222 a session's fourth line was dropped and its tenth disconnected (`the_chat_counter_is_reset_every_five_seconds_of_pulses`). Ledger 223 stands up the NPCs: the mob proto, the `LOCALE_YMIR` mob names and each hosted map's four regen files are read at start-up (`gamedata::mob_proto`, `gamedata::mob_locale_names`, `gamedata::regen`), and `world::npc` spawns every NPC, warp and goto entry in legacy's order with legacy's draws. A client entering a map is sent every one of them as `EncodeInsertPacket` writes it, then the map's `GC_NPC_POSITION` list, and a click on anything is logged and answers nothing (`entering_a_map_shows_its_npcs_and_a_click_keeps_the_connection`). The same section corrected the own insert's `bType`, which the Rewrite had sent as 1 (`CHAR_TYPE_NPC`) since ledger 187; it is `CHAR_TYPE_PC`, 6. Monsters, stones, groups and ore veins are read and counted but not spawned, because they need the cell attributes and the monster AI. Ledger 224 ports the NPC shops: the `shop` and `shop_item` tables are read from the owner's `player.sql` and laid out as `CShop::SetShopItems` lays them out (`gamedata::npc_shop`), a click on a keeper whose click trigger is the shop's opens its window, and `CG_SHOP`'s buy, sale of a whole stack or of a count, and close follow `CShopManager` and `CShop::Buy` (`world::character::shop`, `prodomo::game_state::shop`). Each buy and sale stores the item rows and `player.gold` in one transaction (`db::items::apply_transfer`, ADR-0003) before its records are sent (`a_keeper_opens_its_shop_and_a_buy_and_a_sale_are_stored`). The renewal `shopex`, the personal shop, the logs and the Monarch tax are not ported. Ledger 225 ports the trade: `CG_EXCHANGE`'s start, offers, accept and cancel follow `CInputMain::Exchange` and `CExchange` (`world::character::trade`, `prodomo::game_state::trade`), the settlement moves both sides' items and gold on copies of the two characters, and `db::items::apply_exchange` stores both sides in one transaction before either side is told (`a_trade_moves_an_item_and_gold_between_two_players_in_one_transaction`). Since then only a Transfer writes `player.gold`, adding its change to what the row holds, and the save leaves the column alone. Ledger 226 ports the safebox and the mall: `interpret_command` runs a chat line that starts with `/` (`prodomo::command`: the table, the prefix walk, the position check and the anti-flood count; every character is `GM_PLAYER` until the GM audit, and every command but the safebox's and the mall's answers nothing), the password is an argon2id hash (`db::safebox`), the rows belong to the account (`item.account_id`, ADR-0005), and each checkin, checkout and move is stored in one transaction before the client is told (`world::character::safebox`, `prodomo::game_state::safebox`, `an_account_keeps_items_in_its_safebox_and_takes_them_from_its_mall`). An open safebox refuses a trade and a shop, and a safebox load or close holds back its character's trade steps for 10 seconds. The premium and large-safebox pages, dragon soul items, the mall's item awards and the inventory lock are not ported. Ledger 227 stands up the quest runtime (ADR-0006): `quest::qc` ports `qc` and writes the 95 files of `object/` byte for byte from the 16 `quest_list` scripts, 50 of the 51 scripts compile (`_basic/change_empire.lua` is refused, as legacy's `qc` refuses it), and `quest::host` loads them and the chain of libraries into one Lua 5.1 state through `mlua`, translating legacy's Lua 5.0 dialect at load and shimming the Lua 5.0 idioms the sources use. `quest::manager` ports `CQuestManager::Click` and the state machine of a running script: a click on an NPC runs its quests before its click trigger, `select`, `wait` and `input` send the dialog as `GC_SCRIPT`, and `CG_SCRIPT_ANSWER` and `CG_QUEST_INPUT_STRING` answer it (`prodomo::game_state::quests`, `a_quest_npc_answers_a_click_with_its_dialog_and_runs_it`). Of the 699 API names 24 are backed: the 18 the dialog, the chat lines and the NPC lookups need, the 4 `settings.lua` recorders, `get_locale_base_path` and `q.yield`; every other one ends its script as not ported. A character whose script waits for its client stores nothing in the safebox, uses and drops nothing, picks up no quest item and refuses a trade when both sides accept, as legacy's `IsRunning` checks do. Ledger 228 wires the in-game Warp through the warp NPC event (`prodomo::game_state::warp_npc`), which finishes step 3: every twelfth Pulse each warp NPC checks the players within 300 of it with legacy's empire and `IsHack` checks, and the descriptor of a player it sends runs `WarpSet` (`warp_set` in the binary), which saves the row with the destination, releases the login and sends `GC_CHARACTER_DEL` and `GC_WARP` (`a_warp_npc_sends_its_neighbour_away_and_the_client_comes_back_at_the_target`, `a_warp_to_no_map_sends_nothing_and_the_character_stays`, `a_trader_beside_a_warp_is_told_why_and_stays`, `a_warp_whose_save_fails_closes_without_sending_the_client_away`); a goto NPC shows its neighbour at its target on the same map (`a_goto_npc_shows_its_neighbour_at_its_target_on_the_same_map`). Ledger 229a reads the cell attributes the monsters need: `gamedata::lzo` decodes LZO1X as `lzo1x_decompress_safe` does (a differential witness against liblzo2 matched every block of the owner's 69 maps), `gamedata::server_attr` reads each hosted map's `server_attr` whole at start-up and refuses what legacy would misread, and `world::cells` answers `GetAttribute`, `IsMovablePosition` and `GetMovablePosition` with legacy's 161-point `aArroundCoords` walk. Entering the game now places the character as `Entergame` does: the first movable point around the saved position, else the empire's recall position from `Town.txt` when the map has a sectree there and the saved position otherwise, the loading burst keeping the saved position and the logout saving the new one (`an_entering_character_stands_on_the_first_movable_point_or_at_its_empires_recall`). `data.map.setting`, `data.map.attr` and `data.map.town` are `ported`. Ledger 229b ports the view (`prodomo::game_state::view`, `view_encode`, `motion` and `sync`): each hosted map is a grid of sectrees, each player and NPC stands in one, and each player sees the players and NPCs within `VIEW_RANGE + 500` in the nine sectrees around it, kept by legacy's four view verbs and `UpdateSectree`. Entering the game, a goto and the 16-Pulse sample of a moving player insert what comes into range and remove what leaves it, both ways, with the `GC_WALK_MODE` records `EncodeInsertPacket` sends (`players_and_npcs_within_view_range_see_each_other_on_entry`, `a_walk_into_view_inserts_both_ways_at_the_sample_and_a_walk_out_removes_both`, `an_npc_appears_and_disappears_as_a_player_walks`, `at_zero_stamina_the_walk_mode_rides_with_each_insert`). A move runs in the world as `Goto`, `Move`, `Stop` and `StateMove`'s interpolation run it; the move, the pose, the sync, the look and the equip effects are relayed to the mover's view, the move with the Goto's duration, which had been 0 since ledger 188; a trade's distance is measured as `StateMove` measures it; and a leaving player is removed from its viewers alone (`a_leaving_player_is_removed_from_its_viewers_only`). The motion speed is the 300 units a second legacy uses without a motion file, and stamina is neither consumed nor restored (V2, V3). The next step-4 work is 229c (ground items in the view), 229d (the revive-invisible affect) and 229e (the `.msa` motions), then the monsters (229f onwards). The slot space that floor needs is now measured: one flat array of 1370 cells plus a 1152-cell dragon soul array, the 11-member `EWindows` (the earlier 8-member table was wrong from byte 6), and legacy's two disagreeing `IsValidItemPosition` functions, all recorded in ledger 194. That floor now has a tenant (195): `world::item` holds the instance as an owned value with a validated count, socket and attribute write path, and `world::item::ItemIds` allocates monotonic ids that are never reused and never 0; `world::character::items::CharacterItems` holds the four arrays with the one-based grid, the 5-cell and 8-cell stack walks, and explicit `Rejected` answers for a conflict, a duplicate owner, a missing cell and a footprint that does not fit. That unit also corrected three live constants the workspace had wrong: the socket count is 6 under `ENABLE_EXTENDED_SOCKETS` (two literals in `common` said 3 and 6), and the Dragon Soul stack stride is 8 (`length.h:84`), not the 6 ledger 194's probe transcribed. The instance also **lost** the limits 195 had given it, because `aLimits` belongs to `TItemTable` (`tables.h:879`), the prototype, and not to the instance record (196).

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
| 1 | 30003, 30005 | Hosts 8 maps that Channels 2-4 lack. |
| 2, 3, 4 | 30007/30009, 30011/30013, 30015/30017 | |
| 99 (Shared Channel) | 30019 | 29 maps that exist once for everyone. Never picked at login. |

In legacy each port belongs to one Core, and a Warp to a map on the other Core of the same Channel
reconnects the client to that Core's port. The Rewrite has no Cores: each Channel listens on all of
its ports, and any of them admits the Channel's players.

`config/prodomo.toml.example` holds this topology with each Channel's legacy map set. The binary
refuses a topology it cannot run before binding anything (ledger 178.3). Two legacy settings the
owner should know about (178.5):

- `test_server` defaults to on in legacy and none of the owner's `CONFIG` files turns it off, so the
  deployment ran in test-server mode. The example keeps it on for Parity. One of its effects: legacy
  gives **every** character IMPLEMENTOR authority while it is on (`G/gm.cpp:53`), so on the owner's
  deployment every player was a GM. With it, `SetPlayerProto` also gave every character the GM
  flag `AFF_YMIR` and the protect PK mode at load (`G/char.cpp:2231`, `:2362-2375`,
  `:6354-6358`). The GM grants of section 179 do not reproduce that yet, so only a character
  below the PK protect level of 15 is in the protect mode (229b); the owner decides when
  `test_server` is audited.
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
| Login by key | Live with scenarios (185): `SHUTDOWN` under the handshake key, then the client key pair, the login-key judgement (`NOID`), the logon registry (`ALREADY` and the kick of a holder without a character), `GC_EMPIRE`, the 357-byte character list with each map's Channel address from the map atlas (`gamedata::map_atlas`), and `PHASE(SELECT)`. `prodomo::channel_login` holds the rules; `db::players` reads the characters. | `FULL` (unit-tested; the online count arrives with entering the game). The delayed kick of a holder in game. The blocked-country IP list (`is_blocked_country_ip`, `G/block_country.cpp`; empty tables behave as today). The mark-login table keyed by handle and random key (guild marks). Guild ids and names (zeros until guilds exist). |
| Select, create, delete | Empire choice, create, delete, and forced rename are live with scenarios (186): the Name rules (letters and digits, 2 to 24 bytes, the `banword` table read from `legacy/sql/gamedata/player.sql`, and the lowercase mob names from the text protos), the create checks in legacy order, the job points, the create start and spread, the 30-second create cooldown per account, the delete code and level limits (`[game]`), and the deleted row kept in `player_deleted`. `prodomo::select_phase` holds the rules; `db::players` writes the rows. Choosing a character (`CHARACTER_SELECT`) is live since 187: the slot index, the store read, and the empty-slot close. | The `CREATE PLAYER` character log row (`G/input_db.cpp:274`; the log store does not exist yet). |
| Loading | Live with scenarios (187): the store read, `GC_ENTITY` (249), `GC_MAIN_CHARACTER2_EMPIRE` (113), `GC_CHARACTER_GOLD` (224), `GC_CHARACTER_POINTS` (16), `GC_SKILL_LEVEL` (76), and the map test at the legacy split. `prodomo::loading_phase` holds the bursts. | The quickslots (28-30), the package SDB (153), and the safebox query: not records this deployment can send, per ledger 187.5. `GetValidLocation` and the movable-position fallback need a map instance. |
| Enter game, movement, chat | The enter-game burst is live with a scenario (187). Live with scenarios since 188: `CHAT` (3) with the counter, the block-chat arm, and the map broadcast that reaches the sender; `MOVE` (7) with the 750/999 distance limits, `CanMove`, the refusal's `Show`, and `Goto`, `Move`, `Stop` and `StateMove`'s interpolation in the world, relayed to the mover's view with the move's duration (229b); `CHARACTER_POSITION` (28) for sit and stand; `SYNC_POSITION` (8) with the whole element loop, `SetSyncOwner`, `GC_OWNERSHIP` (62) relayed to the view, the owner range, the 100 ms interval, and the displacement close. The shout to every Channel with its empire filter, level limit and cooldown, and the chat counter reset every five seconds of Pulses (222). The view (229b): the players and NPCs within `VIEW_RANGE + 500` in the 3x3 sectrees, inserted on entry, on a goto and every 16 Pulses while a player moves, and removed as they leave it, with `GC_WALK_MODE` (111); the map's `GC_NPC_POSITION` list; `CG_ON_CLICK` (26) is decoded, logged and answered with nothing (223), except that a click on a shop keeper opens its shop (224). A chat line that starts with `/` runs `interpret_command` (226). `prodomo::chat`, `prodomo::movement`, and `prodomo::sync_position` hold the rules, and the live framing now resolves a variable-length frame (`net::ClientFrameTransport`). | The post-phase events. Every NPC click trigger but the quests' and the shop's. Whisper, the banword conversion and the prism check; every command of `interpret_command` but the safebox's and the mall's, and its GM levels (every character is `GM_PLAYER` until the GM audit); the shout log, the battle pass shout mission, and the GM clause of the shout filter (no character has a GM level yet); the riding and OX-event speed checks; `StateMove` beyond its interpolation, its 16-Pulse sample and its trade check (stamina, walking); a skill move's `IsUsableSkillMotion` check, whose `SKILL_HACK` hack log and delayed disconnect wait for the skills (`G/input_main.cpp:1841-1866`); `SetSyncOwner`'s `AIFLAG_NOMOVE` and `battle_is_attackable` arms; the `sync_hack_count` restore with the save path. |
| Warp | The login home move is live with a scenario (189): a character whose stored map is not hosted by the Channel is moved to its empire start in the same transaction that refuses it (`db::players::save_position`), and the close is silent because legacy's is. `prodomo::warp` holds the whole pure policy with 20 tests, including the 15-byte `GC_WARP` projection, `judge_warp_set`, and `judge_warp_end`. `EMPIRE_START_MAP` (`g_start_map`, `[0, 1, 21, 41]`) is kept apart from `EMPIRE_START` and is checked against the real atlas. The in-game Warp is live with scenarios (228): the warp NPC event and its `IsHack` check (`prodomo::game_state::warp_npc`), `WarpSet` on the player's descriptor with the destination saved before `GC_WARP` leaves (`prodomo::warp::departure_records`), the reconnect by login key at the address and port `GC_WARP` names, and `CG_WARP` (65), whose `WarpEnd` returns at once on every descriptor the Rewrite keeps. The goto NPC's `Show` and `Stop` are ported, through the view since 229b (`prodomo::game_state::warp_npc`), with a scenario on a synthetic goto NPC, because no goto NPC stands on a map the owner's Channels host (`a_goto_npc_shows_its_neighbour_at_its_target_on_the_same_map`). | The other callers: `pc.warp`, `pc.warp_local`, `pc.warp_to_guild_war_observer_position`, `d.new_jump`, and `warp_to_village` land with their quest systems, `GUILD_SKILL_TELEPORT` (158) with the guild, `/mto` and `GoHome` with theirs. A private map (the row has no `map_index` column); the Shared Channel return (a Divergence, reached by no warp NPC); the switchbot and the `CharLog` table. The war- and wedding-map login Warp (`G/input_login.cpp:876-885`) is off under the owner's `test_server = true`. The OX-map login Warp (`G/input_login.cpp:853-874`), which sends a `GM_PLAYER` who enters map 113 while the OX event is finished or off its two spots to the empire start, lands with the OX event (`sys.event.ox`). Whether the Reference client sends `CG_WARP` is confirmed in the owner's play test. |
| Logout and save | Live with scenarios (190): the save event that `StartSaveEvent` arms at enter-game, the drain the game loop would do every 29 Pulses, the disconnect save that `FlushDelayedSave` performs, the `SaveReal` guards, the whole-row `db::players::save_character` (every column but the gold since 225), and the `POINT_PLAYTIME` carry with its `> 60000` guard and sub-minute remainder. `prodomo::save` holds the rules. A Channel change and a shutdown are both a disconnect here, so they are covered by the same save. | The shared delayed-save queue and its 29-Pulse drain, which need ADR-0002's game thread; the event currently runs on the owning descriptor. The logon record. `sync_hack_count` is still in memory only, so it is not in the save; so is `POINT_MOV_SPEED`, which nothing changes yet. Item and affect saves (`FlushDelayedSaveItem`, `SaveAffect`). The `save_event_second_cycle` default of 120 seconds is legacy's value, not the 3 minutes its comment claims. |

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
| An NPC's VID counts up from 2^31, so no NPC shares a number with a player (ledger 223). | One counter numbers every character (`CHARACTER_MANAGER::AllocVID`, `G/char_manager.cpp:91-95`). |
| The boot NPCs are spawned once. No regen event is kept, nothing respawns, and the event's `number(0, 16)` is not drawn (ledger 223). | Every entry with a regen time schedules a `regen_event` (`G/regen.cpp:684-688`), which spawns again whatever of the entry has died. |
| An NPC never moves and has no state machine (ledger 223). The view calls the hook where legacy starts it (`G/entity_view.cpp:110-117`, `G/sectree.cpp:152-155`); `sys.mob.ai` fills it. | A PC coming into view starts the state machine of every non-PC that is not a warp or goto (`G/entity_view.cpp:110-117`). |
| Every warp and goto NPC's event fires on the Pulses that are a multiple of 12 (ledger 228). | Each NPC's event fires 12 Pulses after that NPC was created and every 12 Pulses after that (`StartWarpNPCEvent`, `G/char.cpp:8017-8030`). |
| V1. The sectrees around a point are walked in legacy's neighbour order. The entities of one sectree, the entries of a view and the moving players of each Pulse are walked in key order: players by VID, then NPCs by VID. A warp NPC judges the players around it in that order, and a player one NPC has warped is judged by no other NPC on that Pulse (ledger 229b). | Hash order: `m_set_entity` and `m_map_view` by pointer (`G/entity.h:9`, `:62`), and the players' `UpdateCharacter`, which runs their `StateMove`, over `m_map_pkPCChr`, a hash map by name (`G/char_manager.h:22`, `:122`, walked at `G/char_manager.cpp:689-708`; `:716-725` walks the NPC state machines of `m_set_pkChrState`), so the order is unspecified; `WarpSet` takes a warped player out of its sectree at once (`G/char.cpp:6749-6755`, `:7940-7974`, `:8011`). |
| A warp or goto NPC's name is parsed once, when the NPC stands up, and a name that does not parse, or whose target overflows 32 bits, is logged once; that NPC then sends nobody (ledger 228). | The name is parsed on every fire, a failure is logged when `number(1, 100)` draws below 5, and an overflowing target wraps (`G/char.cpp:7909-7922`). No name of a placed warp or goto NPC in the owner's data fails or overflows; four unplaced GOTO protos (10814, 10817, 10818 and 20039) have names that do not parse. |
| The warp NPC's `IsHack` has no `[TestOnly]` line, and the refine, personal shop, cube and aura checks and `CanHandleItem(false, true)` refuse nobody (ledger 228): none of those systems exists yet. This shrinks as each is ported. | `IsHack` adds `[TestOnly]Pulse %d LoadTime %d PASS %d` under `test_server`, and each of those windows and timers refuses the player (`G/char.cpp:8226-8293`). |
| `GC_WARP` names the port the client is connected to when its Channel hosts the target map, and the Shared Channel's first port otherwise (ledger 228). Any port of a Channel admits the Channel's players. | The port of the Core that hosts the map (`CMapLocation::Get`, `G/map_location.cpp:10-41`), so the 14 warp NPCs of each of Channels 1-4 whose target is on the other Core name the other port (the Shared Channel runs one Core). |
| A Warp from a Shared Channel map to a map the Shared Channel does not host is refused as a position no map serves, until the return of the first row of this table is built (ledger 228). No warp NPC leaves a Shared Channel map for another Channel. | The Shared Channel's Core knows Channel 1's maps, so the player goes to Channel 1 (`D/ClientManager.cpp:1343-1369`). |
| `WarpSet` saves the row with the destination before `GC_WARP` leaves, and a failed save closes the connection without it (ledger 228). | `Save` only queues the character (`G/char.cpp:1472-1476`, `:6747`); the queued `SaveReal` runs after `m_posWarp` is set, at the next drain of the queue (every 29 Pulses, `G/main.cpp:276-277`) or at the close's `FlushDelayedSave` (`G/char.cpp:1786-1789`), whichever comes first, and writes the destination (`:1551-1557`). `GC_WARP` leaves without waiting for the store, so no failed write holds it back. |
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
| A quest's flags and state live in memory while its character is logged in and end with it (ADR-0006, ledger 227). No ported API writes one, so no script can change what a later login would see, and `Loading quest. Please wait a moment.` is never said. | The `quest` table is loaded with the character and saved with it (`D/ClientManagerPlayer.cpp:352-394`, `D/ClientManager.cpp:692-698`), and a quest waits, with that line, until the load arrives (`G/questmanager.cpp:920-931`). |
| `_basic/change_empire.lua` is not loaded: an `end` on line 132 closes its `if` chain early, so `qc` refuses it (ADR-0006, ledger 227). This goes when the owner fixes the source. | The same: legacy's `qc` aborts on it and it never reached `object/`. |
| Only an NPC's `click` and `chat` scripts run (ledger 227). The `login`, `logout`, `levelup`, `kill`, `timer`, `target` and every other event is raised by no ported system, and the hide-and-seek reward of `OnClick` is not ported. | Each system raises its event through `CQuestManager`, and `CHARACTER::OnClick` gives the hide-and-seek reward while that event runs (`G/char.cpp:6299-6330`). |
| A quest API name whose system is not ported raises a "not ported" error, which ends the script as any Lua error does (ADR-0006, ledger 227). Of the 699 names 24 are backed; `pc.warp` and `game.set_safebox_level` are among the refused. | Every name runs. |
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
| V2. A player's move is interpolated at 300 units a second, legacy's speed for a missing motion file (ledger 229b; transitional, 229e loads the motions). | From the `.msa` motion of its race, weapon and walk state, else 300 (`G/char.cpp:3555-3578`, `G/motion.cpp:264-356`). |
| V3. A running player's stamina is not consumed and a walking player's is not restored; a player is never set walking or reset; stamina at or below 0 means walking for `GC_WALK_MODE` (ledger 229b). | `StateMove` consumes and restores stamina and sets walking at 0 (`G/char_state.cpp:812-851`); `PointChange(POINT_STAMINA)` sets and resets walking (`G/char.cpp:4287-4295`), and `SetNowWalking` relays `GC_WALK_MODE` (`G/char.cpp:7345`, `:7364`). |
| V4. A player entering at a point no sectree holds is shown to itself, sees nobody and is seen by nobody; a later move to a point a sectree holds places it and fills its view (ledger 229b, provisional). | `Show` returns before placing it and `Sync` refuses every move (`G/char.cpp:1847-1854`, `:3386-3387`), so it stays unplaced. |
| V6. The view never holds an observer or an object, and an insert sends no guild name, shop sign or ownership (ledger 229b). | `G/entity_view.cpp:100-104`, `:140-210`, `G/char.cpp:1070`, `:1238-1247`. |
| V7. The loading `GC_ENTITY` list is empty (ledger 187; recorded in 229b). | It lists every player on the Core, the loading one included (`G/sectree_manager.cpp:1697-1746`, `G/input_db.cpp:415`). |
| V8. A ground item's add and removal reach the whole map, and an entering player gets the map's ground items after `GC_PHASE` (ledger 229b; transitional, 229c puts ground items in the view). | They reach the item's view, and the entering player's `Show` inserts them before the phase (`G/item.cpp:541`, `:581`, `G/input_login.cpp:590`, `:608`). |
| V9. Entering sends no `GC_CHARACTER_UPDATE` for the revive-invisible affect, and an insert's affect flags are 0 (ledger 229b; transitional, 229d). | `ReviveInvisible(5)` after `Show` sends the update to the view and the player, and every insert of the next 5 s carries `AFF_REVIVE_INVISIBLE` (`G/input_login.cpp:593`, `G/char.cpp:7490-7493`, `G/char_affect.cpp:743-744`). |
| V10. A client's next frame is read only once the world has run its move, pose or talking line, at the world's next Pulse, so every record keeps the order of the frames that caused it; a read holding N such frames is applied over up to N Pulses, as one holding N item steps or syncs already was (ledger 229b). | `DESC::ProcessInput` runs every record of a read in one pass (`G/desc.cpp:252-349`), on the one thread that also runs the Pulse (`G/main.cpp:787`). |
| V11. A warped player's viewers are told of its removal when its connection leaves the world, a few milliseconds after the warp is decided (ledger 229b). | `WarpSet` removes it from its sectree and its viewers at once (`G/char.cpp:6749-6755`). |

## Legacy defects not to reproduce

- When a character enters the game with no movable point within 1000 of its saved position,
  legacy calls `Show` at its empire's recall position without testing that the map has a
  sectree there, and at an uninitialised `PIXEL_POSITION` when the map has no recall position,
  because `GetRecallPositionByEmpire`'s result is ignored (`G/input_login.cpp:572-590`). Where
  no sectree of the map holds that point, `Show` returns before placing the character
  (`G/char.cpp:1847-1854`): it stays at its saved position and its client never gets its own
  insert. The owner's map 235 (`metin2_map_n_flame_dungeon_01`) recalls every empire to
  (614200, 706800), a point of map 62. The Rewrite keeps the saved position, shows the
  character there and warns (`prodomo::loading_phase::entering_position`, ledger 229a). The view
  treats such a character as the V4 Divergence says (ledger 229b).
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
- D1. `StateMove` with a zero duration on its start millisecond divides 0 by 0, and `(int)NaN` is
  `INT_MIN`, so `Sync` logs a hack and keeps the spot (`G/char_state.cpp:784-793`). The Rewrite
  skips the Pulse's move and arrives on the next (ledger 229b).
- D2. `EncodeInsertPacket` sends a MOVE whenever `iDur` is not 0, so an arrival the Pulse has not
  applied yet sends a duration near 2^32 (`G/char.cpp:1097-1108`, `:1211-1223`). The Rewrite sends
  MOVE and its `GC_WALK_MODE` only when `iDur > 0` (ledger 229b).
- D4. `int dwViewRange(VIEW_RANGE + VIEW_BONUS_RANGE)` overflows for a `VIEW_RANGE` near `INT_MAX`
  (`G/entity_view.cpp:85-95`). The Rewrite adds in 64 bits (ledger 229b).

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
