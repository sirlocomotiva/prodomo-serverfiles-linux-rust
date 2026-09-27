
# Ledger 187: my own first-hand reads (verify against the children's reports)

## CInputDB::PlayerLoad (server/server/game/input_db.cpp:328-453) — the LOADING phase

Order, with line numbers:
1. `:417` `d->SetPhase(PHASE_LOADING)` -> writes GC_PHASE (253) first.
2. `:418` `SECTREE_MANAGER::Instance().SendEntity(ch)` -> HEADER_GC_ENTITY (249).
3. `:419` `ch->MainCharacterPacket()` -> HEADER_GC_MAIN_CHARACTER (15), or
   MAIN_CHARACTER3_BGM (bgm present + volume) / MAIN_CHARACTER4_BGM_VOL, per
   `CHARACTER_GetBGMInfo(mapIndex)` at char.cpp:1965-2028. `FIX_DUNGEON_MUSIC` IS defined
   (prodomodefines.h:108), so `mapIndex = GetMapIndex() < 10000 ? GetMapIndex() : GetMapIndex()/10000`.
   The stray `?` sits in the `#else` branch and is not compiled.
4. `:421-425` `SendClientPackageSDBToLoadMap` — the map SDB supplementary data, NOT a
   client record. The land list is not sent here.
5. `:426-440` `map_allow_find` check; failure sets PHASE_CLOSE.
6. `:442` quest login event.
7. `:444-445` `ch->SetQuickslot(i, pTab->quickslot[i])` for i in 0..QUICKSLOT_MAX_NUM.
8. `:447` `ch->PointsPacket()` -> HEADER_GC_CHARACTER_POINTS (16); also
   HEADER_GC_CHARACTER_GOLD under ENABLE_REMOVE_LIMIT_GOLD.
9. `:448` `ch->SkillLevelPacket()` -> char_skill.cpp:171.
10. `:461` `ch->QuerySafeboxSize()`.

Location rules before all that (`:334-374`):
- `lMapIndex == 0` -> `GetMapIndex(x, y)`; still 0 -> empire start map/x/y.
- `GetValidLocation(map, x, y, out map, out pos, empire)`; failure logs and (without
  ENABLE_GOHOME_IF_MAP_NOT_EXIST) closes. Check whether that define is set.
- The player's own (x, y) is replaced by the found position.

Guard: a second character on the descriptor, or a name already in game, returns without
sending anything (and is a defect: `p` is dereferenced in the log when NULL).

## ITEM_SET2 (21) is NOT sent by this server at all
`command grep -arn 'ITEM_SET2' server/server/` returns nothing: neither the header nor
`TPacketGCItemSet2` exists in the checked-in server. `CInputDB::ItemLoad`
(input_db.cpp:1451+) only calls `ITEM_MANAGER::CreateItem` and `AddToCharacter` /
`EquipTo`; the client learns about the items through the item-add records, not a bulk set.
So `gc.item_set2` is a reachability question like the eight GC bytes in AGENTS.md, NOT
porting work. STATUS.md's "missing: ITEM_SET2 (21)" should be corrected.

## CInputLogin::Entergame (server/server/game/input_login.cpp:562-720) — ENTER GAME

Order, with line numbers:
1. `:563-567` no character -> PHASE_CLOSE.
2. `:569-582` `GetMovablePosition(map, x, y, pos)`; failure -> `GetRecallPositionByEmpire`.
3. `:584` `CGuildManager::LoginMember(ch)`.
4. `:587` `ch->Show(map, pos.x, pos.y, pos.z)` -> `EncodeInsertPacket` + `UpdateSectree`
   (char.cpp:1847-1911). This is what produces CHARACTER_ADD (1) and, through the sectree
   update, the other entity records. `Show` returns false with no sectree and sends nothing.
5. `:589` `SECTREE_MANAGER::instance().SendNPCPosition(ch)` -> sectree_manager.cpp:1089.
6. `:590` `ch->ReviveInvisible(5)`.
7. `:602` `d->SetPhase(PHASE_GAME)` -> GC_PHASE (253).
8. `:676` `building::CManager::instance().SendLandList(d, map)` -> HEADER_GC_LAND_LIST (130),
   building.cpp:938-985. The land list is sent at ENTER GAME, not at loading.
9. `:687-690` TPacketGCTime (106), TPacketGCChannel (121).
10. `:692` `ch->SendGreetMessage()` (char.cpp:7518) -> ChatPacket(CHAT_TYPE_NOTICE, <row>) for
    every row of `DBManager::GetGreetMessage()`. Data source still to check.
11. `:694` `_send_bonus_info(ch)`.

Also in the guarded tail: QUICKSLOT (28) via SetQuickslot, points, skill packets.


## CORRECTED: defines from server/server/common/prodomodefines.h
- `__CONQUEROR_LEVEL__` defined (line 53) -> extra point fields, `SetSungMaWill`, `POINT_CONQUEROR_LEVEL/EXP/NEXT_EXP/MOV_SPEED`.
- `FIX_DUNGEON_MUSIC` defined (line 108) -> the `#ifdef` branch of MainCharacterPacket.
- `ENABLE_REMOVE_LIMIT_GOLD` defined (line 157) -> `PointsPacket` ALSO sends
  `HEADER_GC_CHARACTER_GOLD` (2 records, gold then points).
- `__DUNGEON_INFO__` defined (line 168) -> `ch->SendDungeonCooldown(0)` at enter game.
- `ENABLE_GOHOME_IF_MAP_NOT_EXIST` NOT defined -> an invalid load location closes the descriptor.
