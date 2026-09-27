# Ledger 187 — working notes (parent agent, first-hand)

## Protocol codecs added this session (all green)
| module | record | bytes |
|---|---|---|
| `protocol/src/gc_entity.rs` | `GcEntity` 249 + `GcEntityInfo` | 3 + 28n, 28 each |
| `protocol/src/gc_chat.rs` | `GcChat` 4 | 10 + text |
| `protocol/src/gc_npc_position.rs` | `GcNpcPosition` 115 + entry | 5 + 34n, 34 each |

GC coverage: 99 implemented / 35 missing (was 96 / 38).
Also removed 14 duplicate adjacent doc lines in `protocol/src/gc_fields.rs`.

## Legacy facts verified first-hand (line numbers checked by the parent)
### `CInputLogin::CharacterSelect` (input_login.cpp:264-306)
1. no account table -> nothing sent
2. `c_r.players[pinfo->index].dwID == 0` -> `SetPhase(PHASE_CLOSE)`  (BEFORE the bound check — Defect)
3. `index >= PLAYER_PER_ACCOUNT` -> nothing sent
4. `bChangeName` -> nothing sent
5. otherwise `HEADER_GD_PLAYER_LOAD` {account_id, player_id, account_index}

### Loading burst — `CInputDB::PlayerLoad` (input_db.cpp:414-455)
1. `d->SetPhase(PHASE_LOADING)`  -> GC_PHASE 253, 2 bytes
2. `SECTREE_MANAGER::SendEntity(ch)` -> GC_ENTITY 249, every PC descriptor in the
   process, no map filter, order from an `unordered_set` (not reproducible)
3. `ch->MainCharacterPacket()` -> GC_MAIN_CHARACTER **113** (46 bytes)
   - 15 is `HEADER_GC_MAIN_CHARACTER_OLD`, dead: no struct, no send site
   - the BGM variants (137/138) need `CHARACTER_AddBGMInfo`, fed by the legacy
     `map_bgm_info` DB table, which the SQL snapshot does not have -> never sent
4. `SendClientPackageSDBToLoadMap` -> GC_HYBRIDCRYPT_SDB 153, only when the
   `PackageCrypt` knows the map's SDB stream. `legacy/` has no `package_info.txt`
   -> never sent. Codec not built: a reachability question.
5. `!map_allow_find(map)` -> set the empire start as the warp location, then
   `SetPhase(PHASE_CLOSE)`
6. `BroadcastEventFlagOnLogin` -> one GC_CHAT 4 `CHAT_TYPE_COMMAND` per non-zero
   quest event flag: `worldboss N` (`__WORLD_BOSS_EVENT__` is on), `xmas_snow N`,
   `xmas_boom N`, `xmas_tree N`, `DayMode dark`, `newyear_boom N`,
   `DayMode dark|light`. All flags are 0 in a fresh DB -> none sent.
7. quickslots: `SetQuickslot(i, pTab->quickslot[i])` -> GC_QUICKSLOT_ADD 28, 4 bytes,
   ascending slot, only for non-empty slots. A new character has none.
8. `ch->PointsPacket()` (char.cpp:2031-2087):
   - GC_CHARACTER_GOLD 224, 9 bytes (`ENABLE_REMOVE_LIMIT_GOLD` is on)
   - GC_CHARACTER_POINTS 16, 1 + 255*8 = 2041 bytes
   - **Defect**: `TPacketGCPoints pack;` is uninitialised. Slots 0 (`POINT_NONE`)
     and 2 (`POINT_VOICE`) are never written, so 16 bytes of stack garbage reach
     the client. Not reproduced.
9. `ch->SkillLevelPacket()` -> GC_SKILL_LEVEL 76, 1 + 255*3 = 766 bytes
10. `ch->QuerySafeboxSize()` -> an async DB round trip; sends nothing itself

### Enter game — `CInputLogin::Entergame` (input_login.cpp:562-...)
1. no character on the descriptor -> `SetPhase(PHASE_CLOSE)`
2. `GetMovablePosition` fails -> log, move to `GetRecallPositionByEmpire`
3. `ch->Show(map, x, y, z)` -> `EncodeInsertPacket` -> GC_CHARACTER_ADD 68
   (+ GC_CHAR_ADDITIONAL_INFO 136 for a PC), then to itself and to what can see it
4. `SECTREE_MANAGER::SendNPCPosition(ch)` -> GC_NPC_POSITION 115 (header only
   when the map has none)
5. `ch->ReviveInvisible(5)`, `SetSungMaWill()`
6. `d->SetPhase(PHASE_GAME)` -> GC_PHASE 253, 2 bytes
7. `building::CManager::SendLandList` -> GC_LAND_LIST 130, only when the map has
   building land. The Rewrite has no land -> never sent. Reachability question.
8. GC_TIME 106, 5 bytes: `get_global_time()` = `time(0) + global_time_gap`
9. GC_CHANNEL 121, 2 bytes: `g_bChannel`
10. `ch->SendGreetMessage()` -> one GC_CHAT 4 `CHAT_TYPE_NOTICE` per legacy DB
    `string` row named `GREET`. The SQL snapshot has no such row -> none sent.

### Widths measured with `g++ -m32` (i386 ABI) in `.scratch/ledger187/root-probe/`
`i686-linux-gnu-g++-12` is NOT installed on this machine. Controls re-measured
each run: packed two-`long` = 8, `unsigned long` = 4, `bool` = 1, pointer = 4.

### STATUS.md corrections needed (the child report was right that 21 is wrong)
- STATUS.md claims "loading never sends GC_ITEM_SET (21)". FALSE.
  `input_db.cpp:1451` is `CInputDB::ItemLoad`, the **DB arrival** handler. The
  send is `CHARACTER::SetItem`, reached from `AddToCharacter`/`EquipTo`
  (`input_db.cpp:1506-1532`), and it arrives *after* the synchronous burst while
  the client is still in PHASE_LOADING. The handler then sends a **second**
  `GC_CHARACTER_GOLD` + `GC_CHARACTER_POINTS` pair (`input_db.cpp:1563-1566`).
- STATUS.md claims "no ITEM_SET2 exists in the legacy server". FALSE for the
  client: byte 21 is `HEADER_GC_ITEM_SET2`/`TPacketGCItemSet2` in the client
  registration table, and the server sends `TPacketGCItemSet` (72 bytes, one item
  per record). PROTOCOL_NOTES already calls byte 21 a **live two-sizes**
  collision. The right note is that the checked-in server never sends 21.
