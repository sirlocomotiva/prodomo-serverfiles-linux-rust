# Ledger 187 — `CG_ENTERGAME`: everything that crosses the wire when a character enters the game

Read-only research of the frozen legacy tree. No legacy file was modified. No Cargo command was
run. All paths are relative to the repository root.

Feature macros are resolved from `server/server/common/prodomodefines.h`, which
`server/server/game/stdafx.h:16` includes. Anything **active** is marked `[ON]`; anything not
defined in that file (or explicitly commented out) is marked `[OFF]`.

| macro | state | consequence here |
|---|---|---|
| `__SASH_SYSTEM__` | `[ON]` | `CHR_EQUIPPART_NUM` is 6, not 4 |
| `__AURA_SYSTEM__` | `[ON]` | same |
| `__CONQUEROR_LEVEL__` | `[ON]` | `dwConquerorLevel` in records 136 and 19; `SetSungMaWill` at enter-game |
| `ENABLE_REFINE_ELEMENT` | `[ON]` | `bRefineElementType` in 136 and 19 |
| `__FIX_UPDATE_LEVEL__` | `[ON]` | `dwLevel` in record 19 |
| `__FIX_UPDATE_PLAYTIME_AND_ITEMS__` | `[ON]` | extra guard in `UpdatePacket` |
| `ENABLE_SHOW_LIDER_AND_GENERAL_GUILD` | `[ON]` | `dwNewIsGuildName` in 136 and 19 |
| `__ENABLE_PREMIUM_PLAYERS__` | `[ON]` | `byPremium`, `iPremiumTime` in 136 and 19 |
| `__MULTI_LANGUAGE_SYSTEM__` | `[ON]` | `bLanguage` in 136 and 19; `LC_LOCALE_TEXT` in `ChatPacket` |
| `LOCALE_STRING_RENEWAL` | `[ON]` | `bool bCanFormat` in records 4 and 5 |
| `ENABLE_SHOWNPCLEVEL` | `[OFF]` | `char.cpp:1058` is `// #define ENABLE_SHOWNPCLEVEL`; the level is sent only for a PC |
| `ENABLE_DICE_SYSTEM` | `[OFF]` | `CHAT_TYPE_DICE_INFO` does not exist; `CHAT_TYPE_MAX_NUM` is 10 |
| `ENABLE_REMOVE_LIMIT_GOLD` | `[ON]` | record 2 is sent in addition to record 16, at loading time |
| `__DUNGEON_INFO__` | `[ON]` | `SendDungeonCooldown(0)` at enter-game |
| `__EVENT_MANAGER__`, `__PREMIUM_PRIVATE_SHOP__`, `__ENABLE_ADVANCE_SKILL_SELECT__`, `OFFLINE_MESSAGE_REWORKED`, `__DAILY_GIFT_SYSTEM__`, `ENABLE_PET_COSTUME_SYSTEM`, `ENABLE_MOUNT_COSTUME_SYSTEM`, `ENABLE_FISH_EVENT`, `ENABLE_MOVE_CHANNEL`, `ENABLE_GLOBAL_RANK`, `__ENABLE_SHAMAN_SYSTEM__`, `ENABLE_SWITCHBOT`, `PRODOMO_HIDE_COSTUME`, `ENABLE_UPDATE_LASTPLAY_REAL_TIME`, `ENABLE_MULTI_FARM_BLOCK`, `__MELEY_LAIR_DUNGEON__` | `[ON]` | all contribute conditional traffic, see §4 |
| `_IMPROVED_PACKET_ENCRYPTION_` | `[OFF]` | the non-improved TEA branch is the live one |
| `ENABLE_PRIVATE_SHOP_CHEQUE` | `[OFF]` | the "Wons" sentence is not compiled |

---

## 1. The inbound record

`server/server/game/packet.h:594-597`:

```c
typedef struct packet_enter_game
{
	BYTE	header;
} TPacketCGEnterGame;
```

`server/server/game/packet.h:20`:

```c
	HEADER_CG_ENTERGAME				= 10,
```

So the record is **one byte**: the header `0x0a` and nothing else. There is no payload, no
count, and **no client version field** (see §1.2). The frame the client sends is therefore a
single byte, encrypted, at the 8-byte TEA block granularity.

`server/server/game/packet_info.cpp:116` registers the width used for framing:

```c
	Set(HEADER_CG_ENTERGAME, 					sizeof(TPacketCGEnterGame), "EnterGame");
```

`server/server/game/input.cpp:83-93` is the only place that width is used:

```c
		else if (!m_pPacketInfo->Get(bHeader, &iPacketLen, &c_pszName))
		{
			sys_err("UNKNOWN HEADER: %d, LAST HEADER: %d(%d), REMAIN BYTES: %d, fd: %d host(%s)",
					bHeader, bLastHeader, iLastPacketLen, m_iBufferLeft, lpDesc->GetSocket(), lpDesc->GetHostName());
			lpDesc->SetPhase(PHASE_CLOSE);

			return true;
		}

		if (m_iBufferLeft < iPacketLen)
			return true;
```

The `packet_info` table is one shared table for every phase (`input.cpp:47-51`), so this row
governs the frame length in the LOGIN, LOADING, GAME and AUTH phases alike. A client that sends
header `10` inside a TEA block padded to 8 bytes is read as one 1-byte record, the remaining
7 bytes are treated as the next record's header and are rejected as `UNKNOWN HEADER` unless they
happen to be `0` (which `input.cpp:81-82` maps to a 1-byte "padding" record).

### 1.1 Dispatch

`server/server/game/input_login.cpp:1197-1199`:

```c
		case HEADER_CG_ENTERGAME:
			Entergame(d, c_pData);
			break;
```

This is inside `CInputLogin::Analyze`, which is the processor for `PHASE_LOGIN` **and**
`PHASE_LOADING` (`server/server/game/desc.cpp:515-524`, the `PHASE_SELECT` / `PHASE_LOGIN` /
`PHASE_LOADING` arms all install `&m_inputLogin`). The descriptor reaches `Entergame` in
`PHASE_LOADING`, which is what `CInputDB::PlayerLoad` set at
`server/server/game/input_db.cpp:414`.

The handler name is `CInputLogin::Entergame` (lowercase `g`), declared at
`server/server/game/input.h:66` and defined at `server/server/game/input_login.cpp:562`.
There is **no** `CInputMain::EnterGame`, and `CInputMain::Analyze` (`input_main.cpp:3643`)
does not mention header 10.

### 1.2 The client version is a separate record

`server/server/game/packet.h:2465-2477`:

```c
typedef struct command_client_version
{
	BYTE header;
	char filename[32+1];
	char timestamp[32+1];
} TPacketCGClientVersion;

typedef struct command_client_version2
{
	BYTE header;
	char filename[32+1];
	char timestamp[32+1];
} TPacketCGClientVersion2;
```

`server/server/game/packet.h:93-94`:

```c
	HEADER_CG_CLIENT_VERSION					= 0xfd,
	HEADER_CG_CLIENT_VERSION2					= 0xf1,
```

`server/server/game/input_login.cpp:1246-1252` dispatches both, and
`server/server/game/input.cpp:161-169` reads it:

```c
void CInputProcessor::Version(LPCHARACTER ch, const char* c_pData)
{
	if (!ch)
		return;

	TPacketCGClientVersion * p = (TPacketCGClientVersion*) c_pData;
	sys_log(0, "VERSION: %s %s %s", ch->GetName(), p->timestamp, p->filename);
	ch->GetDesc()->SetClientVersion(p->timestamp);
}
```

`SetClientVersion` / `GetClientVersion` are `desc.h:175-176` over a `std::string`
(`desc.h:252`), so `GetClientVersion()` returns `""` and **never `NULL`**.

**The client must send `CG_CLIENT_VERSION` (0xfd) or `CG_CLIENT_VERSION2` (0xf1) before
`CG_ENTERGAME`**, because the enter-game version check at `input_login.cpp:744-768` reads it.
See §7 for the consequence.

---

## 2. Every check in the handler, and what each one does

The whole handler is `input_login.cpp:562-959`. There is **one** failure path.

### 2.1 No character bound → close the descriptor (`:566-570`)

```c
	if (!(ch = d->GetCharacter()))
	{
		d->SetPhase(PHASE_CLOSE);
		return;
	}
```

`SetPhase(PHASE_CLOSE)` also writes `GC_PHASE` (253) with `phase = 0` before closing
(`desc.cpp:498-501`, `:505-509`). This is the only `SetPhase(PHASE_CLOSE)` in the handler.

### 2.2 Blocked position → fall back to the empire recall point (`:572-585`)

```c
	PIXEL_POSITION pos = ch->GetXYZ();

	if (!SECTREE_MANAGER::instance().GetMovablePosition(ch->GetMapIndex(), pos.x, pos.y, pos))
	{
		PIXEL_POSITION pos2;
		SECTREE_MANAGER::instance().GetRecallPositionByEmpire(ch->GetMapIndex(), ch->GetEmpire(), pos2);

		sys_err("!GetMovablePosition (name %s %dx%d map %d changed to %dx%d)",
				ch->GetName(),
				pos.x, pos.y,
				ch->GetMapIndex(),
				pos2.x, pos2.y);
		pos = pos2;
	}
```

`GetMovablePosition` (`server/server/game/sectree_manager.cpp:789-814`) probes the 161
neighbour offsets in `aArroundCoords` (`constants.h:121`, `ARROUND_COORD_MAX_NUM 161`) and
returns `false` when every one of them is missing from a sectree or carries
`ATTR_BLOCK | ATTR_OBJECT`.

This path does **not** disconnect. It only logs and moves the character. `pos.z` is *not*
replaced: `pos = pos2` copies the struct, but `GetRecallPositionByEmpire`
(`sectree_manager.cpp:493`) writes only `x` and `y`, so `z` stays the value from
`ch->GetXYZ()` at `:572`.

### 2.3 Checks that are **not** in this handler

The four guards the audit asked about all live elsewhere:

| guard | where it actually is |
|---|---|
| "is this the right character / account" (index bound, player id, pending rename) | `CInputLogin::CharacterSelect`, `input_login.cpp:265-305`. Note the recorded Defect: `c_r.players[pinfo->index].dwID` is read at `:278` **before** the `pinfo->index >= PLAYER_PER_ACCOUNT` check at `:285`. |
| "the descriptor already has a main state" | `CInputDB::PlayerLoad`, `input_db.cpp:372-377`, not `Entergame`. |
| "this map is allowed here" | `CInputDB::PlayerLoad`, `input_db.cpp:424-435`; failure sets a warp location and `SetPhase(PHASE_CLOSE)`. |
| blocked country IP, `g_bNoMoreClient` ("SHUTDOWN"), `g_iUserLimit` ("FULL") | `CInputLogin::LoginByKey`, `input_login.cpp:160-202`, in the LOGIN phase. |
| **guild id** | There is no guild-id check in `Entergame`. `CGuildManager::instance().LoginMember(ch)` at `:587` is a map lookup that silently does nothing when the player is not in `m_map_pkGuildByPID` (`guild_manager.cpp:162-170`). |
| **VIP** | There is no VIP check in `Entergame`. `__ENABLE_PREMIUM_PLAYERS__` only adds two fields to records 136 and 19. |
| **party / guild lookup** | Not a gate. `CPartyManager::instance().SetParty(ch)` at `:673` and `CGuildManager::instance().SendGuildWar(ch)` at `:674` are unconditional side effects that emit their own records (§4). |

### 2.4 The `Show()` result is discarded (`:590`)

```c
	ch->Show(ch->GetMapIndex(), pos.x, pos.y, pos.z);
```

`CHARACTER::Show` returns `false` when there is no sectree at the position
(`char.cpp:1851-1854`) and in that case sends nothing at all. `Entergame` ignores the return
value and still runs `SetPhase(PHASE_GAME)` at `:608`, so a character whose sectree vanished
between loading and enter-game is advanced into the GAME phase having received no record 1 and
no record 136. Flagged as a Defect for the owner; it is not reachable in the owner's snapshot
because `CInputDB::PlayerLoad` resolved the position through `GetValidLocation` first.

---

## 3. Widths

Measured by compiling the verbatim struct bodies with the host `g++ -m32 -c` and reading
`nm -S` sizes. The canonical `i686-linux-gnu-g++-12` from `AGENTS.md` is **not installed on
this machine** and no 32-bit link libraries exist, so these are `-m32` codegen sizes, not
`i686-linux-gnu-g++-12` sizes. Every probe re-measured five already-settled widths as controls
(`TPacketGCPVP` = 10, `TPacketGCItemPickup` = 9, `TPacketGCTargetUpdate` = 13, a packed
two-`long` struct = 8, a packed single-`DWORD` = 4); all five passed, and the packed two-`long`
control confirms 4-byte `long`. `packet.h` has one `#pragma pack(1)` at `:274` and one
`#pragma pack()` at `:3540`; every struct cited here lies between them.

| record | struct | source | packed width |
|---|---|---|---|
| CG 10 | `TPacketCGEnterGame` | `packet.h:594` | **1** |
| GC 1 | `TPacketGCCharacterAdd` | `packet.h:884-901` | **35** |
| GC 136 | `TPacketGCCharacterAdditionalInfo` | `packet.h:903-931` | **70** |
| GC 19 | `TPacketGCCharacterUpdate` | `packet.h:933-970` | **55** |
| GC 4 | `TPacketGCChat` | `packet.h:978-990` | **10 + text length** |
| GC 253 | `TPacketGCPhase` | `packet.h:806-810` | **2** |
| GC 106 | `TPacketGCTime` | `packet.h:2372-2375` | **5** |
| GC 121 | `TPacketGCChannel` | `packet.h:2479-2483` | **2** |
| GC 115 | `TPacketGCNPCPosition` + `count` × `TNPCPosition` (34) | `packet.h:2426-2441` | **5 + 34 × count** |
| GC 126 | `TPacketGCAffectAdd` | `packet.h:2545-2549`, element `common/tables.h:1081-1089` (21) | **22** |
| GC 127 | `TPacketGCAffectRemove` (see §6.2) | `char_affect.cpp:111-126` for the add side | — |
| GC 17 | `packet_point_change` | `packet.h:1064-1071` | **22** |
| GC 75 | `TPacketGCGuild` (+ body) | `packet.h:2186-2191` | **4 + body** |
| GC 75 | `TPacketGCGuildName` | `packet.h:2193-2200` | **20** |
| GC 130 | `TPacketGCLandList` + `count` × `TLandPacketElement` (24) | `packet.h:2509-2521` | **3 + 24 × count** |
| GC 41 | `TPacketGCPVP` | `packet.h:1780-1786` | **10** |
| GC 111 | `TPacketGCWalkMode` | `packet.h:2383-2388` | **6** |
| GC 39 | `TPacketGCShopSign` | `packet.h:2354-2359` (`szSign[32+1]`, `common/length.h:16`) | **38** |
| GC 249 | `TPacketGCEntity` + body | `packet.h:3368-3372` | **3 + body** |
| — | `TPacketEntityInfo` | `packet.h:3373-3379` | **28** |
| GC 113 | `TPacketGCMainCharacter` | `packet.h:1004-1013` | **46** |
| GC 137 | `TPacketGCMainCharacter3_BGM` | `packet.h:1016-1031` | **71** |
| GC 138 | `TPacketGCMainCharacter4_BGM_VOL` | `packet.h:1033-1049` | **75** |
| GC 16 | `TPacketGCPoints` | `packet.h:1052-1056` | **2041** (`1 + 255 × long long`) |
| GC 76 | `TPacketGCSkillLevel` | `packet.h:1058-1062` | **1531** (`1 + 255 × 6`) |
| CG 253 / 241 | `TPacketCGClientVersion` | `packet.h:2465-2470` | **67** |

`CHARACTER_NAME_MAX_LEN` is 24 (`common/length.h:15`), so every `char name[N+1]` name field is
**25** bytes. `GUILD_NAME_MAX_LEN` is 12 (`common/length.h:42`) and `TPacketGCGuildName` uses
`guildName[12]` with no `+1`.

Records 16 and 76 are the largest fixed game-to-client records in the build, and they are sent
at **loading** time, not enter-game time.

---

## 4. The outbound sequence, in order

`Entergame` is the whole of the enter-game trigger. Everything below is what reaches the
entering client's socket while `input_login.cpp:562-959` runs. Every one of these frames is
**TEA-encrypted** (see §5); there is no plaintext in the enter-game stream.

### 4.1 Before `GC_PHASE(GAME)` — `input_login.cpp:562-608`

| # | record | line | condition |
|---|---|---|---|
| 1 | 75 `GUILD` subheader 16 `GUILD_NAME` | `char.cpp:1069-1070` → `char.cpp:8632-8641` | only if `GetGuild() != NULL` **and** the guild id is not already in `m_known_guild` (`char.cpp:8623-8630`) |
| 2 | 1 `CHARACTER_ADD` | `char.cpp:1073-1110` | always (`Show` succeeded) |
| 3 | 136 `CHAR_ADDITIONAL_INFO` | `char.cpp:1111-1208` | `IsPC() == true \|\| m_bCharType == CHAR_TYPE_NPC` (`:1111`) — true for the entering player |
| 4 | 1 + 136 for **every entity in view range** | `char.cpp:1905` `UpdateSectree()` → `entity_view.cpp:137-138` `ForEachAround` → `entity_view.cpp:60-63` `ViewInsert` → `entity->EncodeInsertPacket(this)` | one pair per entity within `VIEW_RANGE + VIEW_BONUS_RANGE` (`entity_view.cpp:94`) |
| 5 | 111 `WALK_MODE` | `char.cpp:1217-1222` | `iDur != 0`, i.e. the character was moving — impossible on a fresh login, since `m_posDest` is set equal to the position at `char.cpp:1892-1894` |
| 6 | 111 `WALK_MODE` | `char.cpp:1230-1234` | `ch->IsWalking()` — a fresh PC is not walking, so this is normally **not** sent |
| 7 | 39 `SHOP_SIGN` | `char.cpp:1238-1240` | `GetMyShop()` — only a private-shop owner |
| 8 | 115 `NPC_POSITION` | `input_login.cpp:592` → `sectree_manager.cpp:1102-1127` | `!m_mapNPCPosition[map].empty()` (`:1097-1098`) |
| 9 | 17 `PLAYER_POINT_CHANGE` ×2 | `input_login.cpp:593` → `char_affect.cpp:661-665` → `char.cpp:3872` | see §6.3 — this is **unconditional** for a live non-stunned PC and is sent **twice** |
| 10 | 19 `CHARACTER_UPDATE` | `char_affect.cpp:743-744` → `char.cpp:1411` `PacketAround` (includes self, `entity.cpp:104`) | `pkAff->dwFlag != 0` — `AFF_REVIVE_INVISIBLE` is a non-zero auto-increment member of the flag enum at `affect.h:243` |
| 11 | 126 `AFFECT_ADD` | `char_affect.cpp:750` → `char_affect.cpp:111-126` | `IsPC()` |
| 12 | 4 `CHAT` `"SungMaAttr %d %d %d %d"` | `input_login.cpp:596` → `char.cpp:11714` | `GetConquerorLevel() > 0` **and** the map is in `SungMaWillMap` |

Step 4 is the one the audit brief did not name: **`Show()` does not only tell the client about
the entering player, it also tells the entering client about everything already standing near
it**, and it does so *before* `GC_PHASE(GAME)`.

### 4.2 The phase switch — `input_login.cpp:608`

```c
	d->SetPhase(PHASE_GAME);
```

`desc.cpp:494-542` writes `GC_PHASE` (253) with `phase = 5`, then sets `m_bEncrypted = true`
(already true, a no-op) and installs `&m_inputMain` (`:531`). `PHASE_GAME` is 5 from the `EPhase`
enum at `packet.h:790-804`.

Between `:608` and `:685` the handler does not send anything except what the listed helpers
send; `PRODOMO_HIDE_COSTUME` (`:611-637`) and `ENABLE_UPDATE_LASTPLAY_REAL_TIME` (`:599-606`)
are pure local state and one direct SQL string.

### 4.3 After `GC_PHASE(GAME)` — `input_login.cpp:641-959`

| # | record | line | condition |
|---|---|---|---|
| 13 | *(nothing)* | `input_login.cpp:649` `ch->EnterHorse()` | `ch->GetHorseLevel() > 0`. This is a red herring: `CHorseRider::EnterHorse` (`horse_rider.cpp:87-105`) only toggles local state, and the `SendHorseInfo()` it calls on the riding path is an **empty virtual** (`horse_rider.h:62: virtual void SendHorseInfo() {}`). No record leaves the process. |
| 14 | 4 `CHAT` `"dungeon_info_cooldown %d %s"` | `input_login.cpp:660` → `char.cpp:11862` | always, under `__DUNGEON_INFO__`; `cmd` is `"-"` when no dungeon applies |
| 15 | 41 `PVP` × *N* | `input_login.cpp:669` → `pvp.cpp:555-596` | once per in-progress PVP in the process; **nothing** when the list is empty, because the send is inside the `while` |
| 16 | 78 `PARTY_ADD`, 91 `PARTY_LINK`, 79 `PARTY_UPDATE`, 83 `PARTY_PARAMETER` | `input_login.cpp:673` → `party.cpp:37-47` `SetParty` → `party.cpp:536-583` `CParty::Link`, which calls `SendPartyJoinOneToAll` (`:687`), `SendPartyJoinAllToOne` (`:704`), `SendPartyLinkOneToAll` (`:743`), `SendPartyLinkAllToOne` (`:764`), `SendPartyInfoAllToOne` (`:837`), `SendPartyInfoOneToAll` (`:816`) and `SendParameter` (`:1591`) | the player is in a party (`m_map_pkParty` hit at `party.cpp:39`); each of those loops over the member map, so the burst is proportional to the party size and most of it goes to the *other* members |
| 17 | 75 `GUILD` subheader 17 `GUILD_WAR_LIST` | `input_login.cpp:674` → `guild_manager.cpp:807-822` | **unconditional** — the 4-byte header goes out even with an empty war list |
| 18 | 75 `GUILD` subheader 16 `GUILD_NAME` + 130 `LAND_LIST` | `input_login.cpp:676` → `building.cpp:957-961` and `building.cpp:980-985` | per guild-owned land on this map; `wCount != 0` for the 130 (`:978`) |
| 19 | kingdom-war score | `input_login.cpp:682` | `GetMapIndex() == KINGDOM_WAR_MAP_INDEX` and that event is live |
| 20 | **106 `TIME`** | `input_login.cpp:685-688` | **always** |
| 21 | **121 `CHANNEL`** | `input_login.cpp:690-693` | **always** |
| 22 | **4 `CHAT` (greet)** | `input_login.cpp:695` `SendGreetMessage()` | one per line of the DB string `GREET`; **zero** lines in the owner's snapshot (§6.4) |
| 23 | 4 `CHAT` × up to 4 | `input_login.cpp:697` `_send_bonus_info` | only if a `CPrivManager` bonus is non-zero |
| 24 | 4 `CHAT` | `input_login.cpp:709` | private-shop owner with gold in the stash |
| 25 | 4 `CHAT` `"DailyGiftEvent %d"` | `input_login.cpp:922` | `__DAILY_GIFT_SYSTEM__` and a pending daily gift |
| 26 | 4 `CHAT` `"server_info %d %d"` | `input_login.cpp:943` | `ENABLE_MOVE_CHANNEL` |
| 27 | 4 `CHAT` `letters_event %d` | `input_login.cpp:731` `SEventLetters` (`input_login.cpp:69-82`) | **always** — either branch calls `ChatPacket` |
| 28 | 4 `CHAT` version-mismatch notice | `input_login.cpp:756` | see §7 — on the owner's configuration this fires for every client |
| 29 | 4 `CHAT` `"ConsoleEnable"` | `input_login.cpp:774-775` | `ch->IsGM()` |
| 30 | 126 `AFFECT_ADD` × *N* | `input_login.cpp:740` | one per premium tier with `GetPremiumRemainSeconds(i) > 0`; each `AddAffect` also re-runs the §6.3 point-change path and can re-send record 19 |
| 31 | `CHAT_TYPE_INFO` war/wedding-map notice | `input_login.cpp:883`, `:872-873`, `:839` | war map or wedding map, and not a test server |
| 32 | `ARENA`-related | `input_login.cpp:796-834` | arena map: observer or duelist branch; `HEADER_GC_DUEL_START` at `:811-815` |
| 33 | OX-event entry | `input_login.cpp:869` | map 113 |
| 34 | 76 `SKILL_LEVEL` | `input_login.cpp:896` | `__FIX_NIVEL_CAL__` and a horse |
| 35 | shaman, global-rank, switchbot, fish-event, pet/mount costume records | `input_login.cpp:947-957`, `:928`, `:935`, `:939` | per macro, all `[ON]` |

Step 27 is the one unconditional `CHAT` the audit brief's expected order does not mention: the
sequence `… 106, 121, [greet], [bonus], [dungeon is earlier] …, letters_event …` always contains
at least one record 4.

---

## 5. TEA and plaintext boundaries

`_IMPROVED_PACKET_ENCRYPTION_` is not defined, so the live branch is the classic one.

The key pair is derived from the login record, **not** from the handshake.
`server/server/game/input_login.cpp:207-209`, inside `LoginByKey`:

```c
#ifndef _IMPROVED_PACKET_ENCRYPTION_
	d->SetSecurityKey(pinfo->adwClientKey);
#endif
```

`server/server/game/desc.cpp:951-963`:

```c
void DESC::SetSecurityKey(const DWORD * c_pdwKey)
{
	const BYTE * c_pszKey = (const BYTE *) "JyTxtHljHJlVJHorRM301vf@4fvj10-v";

	c_pszKey = GetKey_20050304Myevan() + 37;

	thecore_memcpy(&m_adwDecryptionKey, c_pdwKey, 16);
	TEA_Encrypt(&m_adwEncryptionKey[0], &m_adwDecryptionKey[0], (const DWORD *) c_pszKey, 16);
```

Before `LoginByKey` the pair is the literal seed `desc.cpp:237-238`:

```c
	thecore_memcpy(m_adwEncryptionKey, "1234abcd5678efgh", sizeof(DWORD) * 4);
	thecore_memcpy(m_adwDecryptionKey, "1234abcd5678efgh", sizeof(DWORD) * 4);
```

The `m_bEncrypted` flag is set in exactly three places, all inside `SetPhase`
(`desc.cpp:521`, `:529`, `:536`) — i.e. it is turned on by the *transition*, and the `GC_PHASE`
frame that announces the transition is written at `desc.cpp:498-501`, **before** the flag flips:

```c
void DESC::SetPhase(int _phase)
{
	m_iPhase = _phase;

	TPacketGCPhase pack;
	pack.header = HEADER_GC_PHASE;
	pack.phase = _phase;
	Packet(&pack, sizeof(TPacketGCPhase));

	switch (m_iPhase)
	{
		...
		case PHASE_SELECT:
		case PHASE_LOGIN:
		case PHASE_LOADING:
#ifndef _IMPROVED_PACKET_ENCRYPTION_
			m_bEncrypted = true;
#endif
			m_pInputProcessor = &m_inputLogin;
			break;
```

Boundaries, end to end:

| frame | cipher |
|---|---|
| `GC_PHASE` (253) `phase = 1` (HANDSHAKE), `desc.cpp:242` | plaintext |
| handshake, `CInputProcessor::Handshake` `input.cpp:131-159` | plaintext |
| `GC_PHASE` (253) `phase = 2` (LOGIN), `input.cpp:152` → `desc.cpp:501` | **plaintext** — the last plaintext frame, because `Packet` runs at `:501` and `m_bEncrypted` is set at `:521` |
| `CG_LOGIN2`, `input_login.cpp:155` | encrypted (the key is installed *by* this handler, at `:208`, for the frames that follow) |
| `GC_PHASE` (253) `phase = 4` (LOADING), `input_db.cpp:414` | encrypted |
| records 249, 113/137/138, 16, 76, 28 (`input_db.cpp:415-443`) | encrypted |
| `CG_ENTERGAME` (header 10, 1 byte) | **encrypted** |
| every record in §4 | **encrypted** |
| `GC_PHASE` (253) `phase = 5` (GAME), `input_login.cpp:608` | encrypted |

`m_bEncrypted` is not a function of the phase number; it is a latch. `SetPhase(PHASE_GAME)`
re-sets it to the value it already has. So the enter-game handler runs entirely inside the
encrypted window, and `GC_PHASE(GAME)` is an ordinary encrypted record.

`desc.cpp:456-476` is the encrypting write path: the plaintext is copied into the output buffer
and TEA-encrypted in place over the aligned prefix, with the residual bytes zero-padded by
`buffer_adjust_size`.

---

## 6. The three records in detail

### 6.1 Record 1 — `CHARACTER_ADD` (wire byte 1, 35 bytes)

Struct, `packet.h:884-901`:

```c
typedef struct packet_add_char
{
	BYTE	header;
	DWORD	dwVID;

	float	angle;
	long	x;
	long	y;
	long	z;

	BYTE	bType;
	WORD	wRaceNum;
	BYTE	bMovingSpeed;
	BYTE	bAttackSpeed;

	BYTE	bStateFlag;
	DWORD	dwAffectFlag[2];	// 보너스
} TPacketGCCharacterAdd;
```

Field fills, `char.cpp:1073-1095`:

| offset | field | source | line |
|---|---|---|---|
| 0 | `header` | `HEADER_GC_CHARACTER_ADD` (= 1) | 1075 |
| 1 | `dwVID` | `m_vid` | 1076 |
| 5 | `angle` (float) | `GetRotation()` | 1078 |
| 9 | `x` | `GetX()` | 1079 |
| 13 | `y` | `GetY()` | 1080 |
| 17 | `z` | `GetZ()` | 1081 |
| 21 | `bType` | `GetCharType()` | 1077 |
| 22 | `wRaceNum` | `GetRaceNum()` | 1082 |
| 24 | `bMovingSpeed` | `150` if `IsPet()`, else `GetLimitPoint(POINT_MOV_SPEED)` | 1083-1090 |
| 26 | `bAttackSpeed` | `GetLimitPoint(POINT_ATT_SPEED)` | 1091 |
| 27 | `bStateFlag` | `m_bAddChrState` | 1095 |
| 28 | `dwAffectFlag[0]` | `m_afAffectFlag.bits[0]` | 1092 |
| 32 | `dwAffectFlag[1]` | `m_afAffectFlag.bits[1]` | 1093 |

There is **no `memset`** of `pack` before the field writes. On the enter-game path every field
is written, so there are no stray bytes; but the struct is not zero-initialised, which matters
for a rewrite.

`bMovingSpeed` and `bAttackSpeed` are `BYTE` here and are written from `long` accessors with no
range check, so they wrap. That is legacy behaviour; note it rather than reproduce a silent
truncation rule without a decision from the owner.

`dwVID` and the entity-identity fields are **not** the player id. `m_vid` is assigned by
`CEntity::SetVirtualID`; for a PC it equals the map-local VID, not the 32-bit `player.id` the
client sees in some other records.

### 6.2 Record 136 — `CHAR_ADDITIONAL_INFO` (70 bytes)

Struct, `packet.h:903-931`:

```c
typedef struct packet_char_additional_info
{
	BYTE    header;
	DWORD   dwVID;
	char    name[CHARACTER_NAME_MAX_LEN + 1];
	WORD    awPart[CHR_EQUIPPART_NUM];
	BYTE	bEmpire;
	DWORD   dwGuildID;
	DWORD   dwLevel;
#if defined(__CONQUEROR_LEVEL__)
	DWORD dwConquerorLevel;
#endif
	short	sAlignment;
	BYTE	bPKMode;
	DWORD	dwMountVnum;
#ifdef ENABLE_REFINE_ELEMENT
	BYTE	bRefineElementType;
#endif
#ifdef ENABLE_SHOW_LIDER_AND_GENERAL_GUILD
	BYTE	dwNewIsGuildName;
#endif
#ifdef __ENABLE_PREMIUM_PLAYERS__
	BYTE byPremium;
	long int iPremiumTime;
#endif
#ifdef __MULTI_LANGUAGE_SYSTEM__
	BYTE bLanguage;
#endif
} TPacketGCCharacterAdditionalInfo;
```

`memset(&addPacket, 0, sizeof(...))` **is** done at `char.cpp:1114`. `CHR_EQUIPPART_NUM` is 6
(`packet.h:869-882` with `__SASH_SYSTEM__` and `__AURA_SYSTEM__` `[ON]`).

| offset | field | source | line |
|---|---|---|---|
| 0 | `header` | `HEADER_GC_CHAR_ADDITIONAL_INFO` (= 136) | 1116 |
| 1 | `dwVID` | `m_vid` | 1117 |
| 5 | `name[25]` | `strlcpy` from `GetName()` | 1176 |
| 30 | `awPart[0]` ARMOR | `GetPart(PART_MAIN)` | 1119 |
| 32 | `awPart[1]` WEAPON | `GetPart(PART_WEAPON)` | 1120 |
| 34 | `awPart[2]` HEAD | `GetPart(PART_HEAD)` | 1121 |
| 36 | `awPart[3]` HAIR | `GetPart(PART_HAIR)` | 1122 |
| 38 | `awPart[4]` SASH | `GetPart(PART_SASH)` | 1123 |
| 40 | `awPart[5]` AURA | `GetPart(PART_AURA)` | 1125 |
| 42 | `bPKMode` | `m_bPKMode` | 1128 |
| 43 | `dwMountVnum` | `GetMountVnum()` | 1129 |
| 47 | `bEmpire` | `m_bEmpire` | 1130 |
| 48 | `bRefineElementType` | `GetRefineElementType()` | 1132 |
| 49 | `dwLevel` | `GetLevel()` for a PC | 1141 |
| 53 | `dwConquerorLevel` | `GetConquerorLevel()` | 1143 |
| 57 | `dwGuildID` | `GetGuild()->GetID()`, else 0 | 1180 / 1195 |
| 61 | `dwNewIsGuildName` | 3 master, 2 general, 1 member, 0 no guild | 1184 / 1187 / 1190 / 1197 |
| 62 | `sAlignment` (short) | `m_iAlignment / 10` | 1201 |
| 64 | `byPremium` | `m_byPremium` | 1203 |
| 65 | `iPremiumTime` (long) | `m_iPremiumTime` | 1204 |
| 69 | `bLanguage` | `GetDesc()->GetLanguage()` | 1174 |

`dwLevel` is 0 for anything that is not a PC (`char.cpp:1146-1149`), because
`ENABLE_SHOWNPCLEVEL` is `[OFF]`. The `if (false)` at `char.cpp:1151` is a disabled
empire-visibility filter: with it, the whole name/guild/alignment/premium block is
**unconditionally** sent to every viewer (`char.cpp:1170-1206`, label `show_all_info`).

### 6.3 Record 19 — `CHARACTER_UPDATE` (55 bytes)

Struct, `packet.h:933-970`:

```c
typedef struct packet_update_char
{
	BYTE	header;
	DWORD	dwVID;

	WORD        awPart[CHR_EQUIPPART_NUM];
	BYTE	bMovingSpeed;
	BYTE	bAttackSpeed;

	BYTE	bStateFlag;
	DWORD	dwAffectFlag[2];

	DWORD	dwGuildID;
	short	sAlignment;
#ifdef __FIX_UPDATE_LEVEL__
	DWORD	dwLevel;
#endif
#if defined(__CONQUEROR_LEVEL__)
	DWORD	dwConquerorLevel;
#endif
	BYTE	bPKMode;

	DWORD	dwMountVnum;
#ifdef ENABLE_REFINE_ELEMENT
	BYTE	bRefineElementType;
#endif

#ifdef ENABLE_SHOW_LIDER_AND_GENERAL_GUILD
	BYTE	dwNewIsGuildName;
#endif
#ifdef __ENABLE_PREMIUM_PLAYERS__
	BYTE byPremium;
	long int iPremiumTime;
#endif
#ifdef __MULTI_LANGUAGE_SYSTEM__
	BYTE bLanguage;
#endif
} TPacketGCCharacterUpdate;
```

`CHARACTER::UpdatePacket` is at `char.cpp:1277-1413`. It has three early returns:

```c
    if (!m_pointsInstant.computed)
        return;
	
	if (GetSectree() == NULL) return;

#ifdef __FIX_UPDATE_PLAYTIME_AND_ITEMS__
	if (IsPC() && (!GetDesc() || !GetDesc()->GetCharacter()))
		return;
#endif
```

Both are satisfied on the enter-game path, and the reachability is worth spelling out because
both are easy to get wrong:

- `m_pointsInstant.computed` is set to `true` at `char.cpp:3160`, at the end of
  `CHARACTER::SetPlayerProto` (`char.cpp:2233-3163`), which `CInputDB::PlayerLoad` calls at
  `input_db.cpp:386` — well before enter-game. `ComputePoints` (`char.cpp:2862-…`) sets it back
  to `false` at `:2890`, but it is not called again in between.
- `GetSectree()` is `NULL` when `SetPlayerProto`'s own `UpdatePacket()` at `char.cpp:3162`
  runs, so **that** call is suppressed by the second guard; after `ch->Show(...)` at
  `input_login.cpp:590` the character is in a sectree, so the later call succeeds.

The send is `PacketAround(&pack, sizeof(pack))` at `char.cpp:1411` in the non-observer case.
`CEntity::PacketView` includes the entity itself at `entity.cpp:104`:

```c
	if (!m_bIsObserver)
		for_each(m_map_view.begin(), m_map_view.end(), f);

	f(std::make_pair(this, 0));
```

so record 19 reaches the entering player as well as everyone in view.

The enter-game caller chain is `input_login.cpp:593` → `char.cpp:7490-7493`
(`ReviveInvisible` → `AddAffect(AFFECT_REVIVE_INVISIBLE, POINT_NONE, 0, AFF_REVIVE_INVISIBLE, 5, 0, true)`)
→ `char_affect.cpp:668`. The record is emitted at `char_affect.cpp:743-744`:

```c
	if (pkAff->dwFlag || wMovSpd != GetPoint(POINT_MOV_SPEED) || wAttSpd != GetPoint(POINT_ATT_SPEED))
		UpdatePacket();
```

`pkAff->dwFlag` is `AFF_REVIVE_INVISIBLE` (`affect.h:243`, an auto-increment member of the flag
enum, non-zero), so the branch is taken on the enter-game path regardless of movement.

**The two records that bracket it** are easy to miss. `AddAffect` begins at
`char_affect.cpp:668` with this:

```c
    if (!IsDead())
    {
        PointChange(POINT_HP, GetMaxHP() - GetHP());
        PointChange(POINT_SP, GetMaxSP() - GetSP());
    }
```

`POINT_HP` is 5 and `POINT_SP` is 7 (`char.h:147`, `char.h:149`). The `POINT_HP` call with a
zero delta is suppressed at `char.cpp:4800-4801`:

```c
	if (type == POINT_HP && amount == 0)
		return;
```

`POINT_SP` has no such guard, so it always falls through to the write at `char.cpp:4803-4821`
and emits record 17 (`header = 17`, `type = 7`, `value = GetSP()`, `amount = 0`). Worse, the
tail of `PointChange` calls `UpdatePointsPacket` (`char.cpp:4823`), which builds and sends the
**same struct again** at `char.cpp:2093-2118`. So every live, non-stunned PC emits record 17
**twice** with `type = POINT_SP` on the enter-game path, between records 136/115 and record 19.
Flagged as a Defect for the owner: the duplicate is byte-identical and harmless to a correct
client, but it is a real redundant write.

`AFFECT_REVIVE_INVISIBLE` is 215 (`affect.h:40`). It is added with `bApplyOn = POINT_NONE` (0),
`lApplyValue = 0`, `lDuration = 5`, `lSPCost = 0`, and `bOverride = true`. On a re-login where
the affect survived, the `pkAff && bOverride` branch at `char_affect.cpp:704-711` also emits
record 127 `AFFECT_REMOVE` first.

### 6.4 Record 4 — `CHAT`

Struct, `packet.h:978-990`:

```c
typedef struct packet_chat	// 채팅 관련
{
	BYTE	header;
	WORD	size;
	BYTE	type;
	DWORD	id;
	BYTE	bEmpire;
#if defined(LOCALE_STRING_RENEWAL)
	bool	bCanFormat;
	packet_chat() : bCanFormat(true) {}
#endif

} TPacketGCChat;
```

Width: `1 + 2 + 1 + 4 + 1 + 1 = 10`, then the text. `CHARACTER::ChatPacket`
(`char.cpp:5140-5189`) is the single builder for every record 4 in the game:

```c
	pack_chat.header    = HEADER_GC_CHAT;
	pack_chat.size      = sizeof(struct packet_chat) + len;
	pack_chat.type      = type;
	pack_chat.id        = 0;
	pack_chat.bEmpire   = d->GetEmpire();
```

with, when `__MULTI_LANGUAGE_SYSTEM__` is `[ON]` (`char.cpp:5148-5157`):

```c
	if (type != CHAT_TYPE_COMMAND)
		localeFormat = LC_LOCALE_TEXT(format, d->GetLanguage());
	else
		localeFormat = format;
```

`CHAT_TYPE_NOTICE` is 2 and `CHAT_TYPE_COMMAND` is 5 (`common/length.h:403-419`, with
`ENABLE_DICE_SYSTEM` `[OFF]`). `CHAT_MAX_LEN` is 512 (`common/length.h:49`), so `chatbuf` is
513 bytes and `len` is at most 512.

**The enter-game notice.** The record the expected order names is
`input_login.cpp:695` → `CHARACTER::SendGreetMessage`, `char.cpp:7518-7526`:

```c
void CHARACTER::SendGreetMessage()
{
	auto v = DBManager::instance().GetGreetMessage();

	for (auto it = v.begin(); it != v.end(); ++it)
	{
		ChatPacket(CHAT_TYPE_NOTICE, it->c_str());
	}
}
```

So:

- **message type** `CHAT_TYPE_NOTICE` = 2;
- **`id`** `0`;
- **`bEmpire`** `d->GetEmpire()`;
- **argument values** none — the text is passed as the `format` string with no `%` arguments;
- **the text itself is not in the source.** It comes from the database string `GREET`, split on
  newlines, `db.cpp:494-503`:

```c
				if (m_map_dbstring.find("GREET") != m_map_dbstring.end())
				{
					std::istringstream is(m_map_dbstring["GREET"]);
					while (!is.eof())
					{
						std::string str;
						getline(is, str);
						m_vec_GreetMessage.push_back(str);
					}
				}
```

loaded by `db.cpp:598-600`:

```c
void DBManager::LoadDBString()
{
	ReturnQuery(QID_DB_STRING, 0, NULL, "SELECT name, text FROM string%s", get_table_postfix());
}
```

**In the owner's snapshot this sends zero records 4.** The checked-in `legacy/sql` tree
(`legacy/sql/schema/{common,player}.sql`, `legacy/sql/gamedata/{common,player}.sql`) has no
`string` table at all, in any schema or data file, so no `GREET` row exists. A rewrite that
sends a hard-coded welcome line would be a **Divergence**, not Parity; if the owner wants a
welcome, it goes in as data and a Divergence.

Note the `while (!is.eof())` loop: a `GREET` value ending in `\n` produces a trailing empty
line, which becomes an extra record 4 with a zero-length text. `locale_find`
(`game/locale.cpp:71-88`) returns its input unchanged when the key is absent, so the DB text
passes through the multi-language layer untouched.

The other unconditional record 4 in the enter-game window is
`SEventLetters` (`input_login.cpp:69-82`, called at `:731`), which always sends
`"letters_event %d"` with `0` or `1` depending on `quest::CQuestManager::GetEventFlag("letters_event")`.

---

## 7. The version check at the end of the handler, and why it matters

`input_login.cpp:744-768`:

```c
	if (g_bCheckClientVersion)
	{
		sys_log(0, "VERSION CHECK %s %s", g_stClientVersion.c_str(), d->GetClientVersion());

		if (!d->GetClientVersion())
		{
			d->DelayedDisconnect(10);
		}
		else
		{
			if (0 != g_stClientVersion.compare(d->GetClientVersion())) // @fixme103 (version > date)
			{
				ch->ChatPacket(CHAT_TYPE_NOTICE, LC_TEXT("..."));
				d->DelayedDisconnect(0); // @fixme103 (10);
				LogManager::instance().HackLog("VERSION_CONFLICT", ch);
```

Three facts:

1. `GetClientVersion()` returns `c_str()` of a `std::string` (`desc.h:176`), so it is `""` at
   worst and **never `NULL`**. The `!d->GetClientVersion()` branch at `:748` is dead code.
2. `g_bCheckClientVersion` defaults to `true` (`config.cpp:86`) and `g_stClientVersion`
   defaults to the string `"1215955205"` (`config.cpp:87`). The only two things that change them
   are the `CHECK_VERSION_SERVER` and `CHECK_VERSION_VALUE` CONFIG keys (`config.cpp:1185-1200`)
   and a `VERSION` file in the process working directory (`config.cpp:1740-1755`). **Neither
   exists** in `legacy/config/ch1/core1/CONFIG` (verified: the file is 836 bytes and contains
   neither key), and there is no `VERSION` file beside it.
3. Therefore, on the owner's recorded configuration, the comparison `"" != "1215955205"` is
   true for every client, so every client receives a `CHAT_TYPE_NOTICE` record 4 and
   `DelayedDisconnect(0)` (`desc.cpp:829-840`, `event_create(..., PASSES_PER_SEC(0))`) — that
   is, an immediate close at the very end of enter-game — plus a `VERSION_CONFLICT` hack-log
   entry.

That is a legacy Defect, not Parity. The Rewrite must **not** reproduce it. It should send the
notice and log, and not disconnect on a version mismatch, and that difference needs a recorded
Divergence with the owner's agreement. It also means the recorded enter-game sequence can only
be observed against a live client that has been configured to send the expected timestamp.

---

## 8. Map and land data: not sent at enter-game

**The map's supplementary data (SDB) is a loading-phase record.**
`CInputDB::PlayerLoad`, `input_db.cpp:414-423`:

```c
	d->SetPhase(PHASE_LOADING);
	SECTREE_MANAGER::Instance().SendEntity(ch);
	ch->MainCharacterPacket();

	long lPublicMapIndex = lMapIndex >= 10000 ? lMapIndex / 10000 : lMapIndex;
	const TMapRegion * rMapRgn = SECTREE_MANAGER::instance().GetMapRegion(lPublicMapIndex);
	if( rMapRgn )
	{
		DESC_MANAGER::instance().SendClientPackageSDBToLoadMap( d, rMapRgn->strMapName.c_str() );
	}
```

`SendClientPackageSDBToLoadMap` (`server/server/game/desc_manager.cpp:552-572`) frames and
sends record **153** `HYBRIDCRYPT_SDB` (`packet.h` `HEADER_GC_HYBRIDCRYPT_SDB`, and
`protocol/src/gc_inventory.rs` maps `0x99` to `HEADER_GC_HYBRIDCRYPT_SDB`). The record's own
header layout is `packet.h:2690-2719` (`bHeader`, `uDynamicPacketSize`, `iStreamLen`, then
`iStreamLen` raw SDB bytes). The map name is the `rMapRgn->strMapName` of the **public** map,
i.e. dungeon maps `>= 10000` are divided by 10000 first.

**The land list is an enter-game record**, `input_login.cpp:676`:

```c
	building::CManager::instance().SendLandList(d, ch->GetMapIndex());
```

`building::CManager::SendLandList` (`server/server/game/building.cpp:938-985`) walks the
guild-land table, keeps the entries whose `lMapIndex` matches, emits a `GUILD_NAME` record per
distinct guild it meets (`:957-962`), and then, **only if `wCount != 0`** (`:978`), sends
record 130 with `size = sizeof(TPacketGCLandList) + count * 24`.

So the answer to "the map/land record for the map the player is entering" is two records in
two different phases: **153 `HYBRIDCRYPT_SDB` at loading** (`input_db.cpp:422`) and
**130 `LAND_LIST` at enter-game** (`input_login.cpp:676`, only for maps that have guild land).

**Correction to a nearby claim.** `docs/STATUS.md` says the loading phase sends
`249, 15, 28, 16, 76`. The record the server actually sends for the main character is
**113**, not 15. `char.cpp:2017-2027`:

```c
		TPacketGCMainCharacter pack;
		pack.header = HEADER_GC_MAIN_CHARACTER;
```

and `packet.h:188` is `HEADER_GC_MAIN_CHARACTER = 113`; `packet.h:115` is
`HEADER_GC_MAIN_CHARACTER_OLD = 15`, a different wire byte that this server never sends. With
BGM configured for the map the server instead sends 137 (`char.cpp:1998-2010`) or 138
(`char.cpp:1980-1993`). `protocol/src/gc_inventory.rs` already names 15
`HEADER_GC_MAIN_CHARACTER` and 113 `HEADER_GC_MAIN_CHARACTER2_EMPIRE`, which agrees with the
byte pairing and not with the server's misleading enumerator name.

---

## 9. GAME-phase dispatch registration

`PHASE_GAME` is 5 (`packet.h:790-804`). `DESC::SetPhase` installs the processor at
`desc.cpp:526-532`:

```c
		case PHASE_GAME:
		case PHASE_DEAD:
#ifndef _IMPROVED_PACKET_ENCRYPTION_
			m_bEncrypted = true;
#endif
			m_pInputProcessor = &m_inputMain;
			break;
```

`m_pInputProcessor` is a `CInputProcessor*` (`desc.h:194-198` holds the five concrete
processors as members: `m_inputHandshake`, `m_inputLogin`, `m_inputMain`, `m_inputClose`,
`m_inputAuth`). `CInputMain::Analyze` is `input_main.cpp:3643-…`, and its first case is
`HEADER_CG_PONG` at `:3661`, then `HEADER_CG_TIME_SYNC`, `HEADER_CG_CHAT`, `HEADER_CG_MOVE`, …

The GAME phase needs **no table change and no phase-enum change** for enter-game to work. The
`CPacketInfoCG` table is a single object created in the `CInputProcessor` constructor
(`input.cpp:47-51`) and filled once, so both the LOGIN phase and the GAME phase resolve header
widths from the same rows. `header 10` stays registered for the rest of the session, and
`CInputMain::Analyze` has no `HEADER_CG_ENTERGAME` case, so a client that re-sends it in the
GAME phase hits `default:` — which logs `sys_err` and `return (0)` without closing
(the `SetPhase(PHASE_CLOSE)` is commented out at `input_login.cpp:1262`; the same pattern is
absent from `CInputMain`'s default, so the frame is consumed and ignored).

Two consequences worth writing down:

- A second `CG_ENTERGAME` is **not** a disconnect. It is a logged no-op. That is a Quirk-shaped
  behaviour; the owner should decide whether the Rewrite closes or ignores.
- `DESC::SetPhase` has **no `case PHASE_DBCLIENT` and no `case PHASE_P2P`**; those enum values
  exist in `packet.h:800-803` but fall through the switch and leave the previous processor
  installed. Unreachable in this build (ADR-0001 retired both), but it is why the enum has more
  values than the switch has arms.

---

## 10. Defects and quirks found

Listed for the owner to rule on. None is reproduced by the Rewrite without a decision.

1. **`Show()`'s result is discarded** (`input_login.cpp:590`). A missing sectree advances the
   descriptor to the GAME phase with no record 1 and no record 136.
2. **Record 17 is sent twice** for every `PointChange`. `char.cpp:4803-4821` writes it, then
   `char.cpp:4823` calls `UpdatePointsPacket`, which writes the identical struct again at
   `char.cpp:2093-2118`. Reachable on the enter-game path through
   `char_affect.cpp:661-665` with `type = POINT_SP`.
3. **The version check kills every client on the recorded configuration.** §7.
   `if (!d->GetClientVersion())` is dead (`std::string::c_str()` is never `NULL`), and
   `g_stClientVersion` defaults to `"1215955205"` with no `VERSION` file and no
   `CHECK_VERSION_*` key in `legacy/config`.
4. **`GetMovablePosition` fallback leaves `z` stale** (`input_login.cpp:572-585`): `pos = pos2`
   copies the recall point, but `GetRecallPositionByEmpire` only writes `x` and `y`.
5. **`CInputDB::PlayerLoad` can dereference a null character** in its own log line
   (`input_db.cpp:372-377`): the guard is `d->GetCharacter() || d->IsPhase(PHASE_GAME)`, and
   the log then calls `p->GetName()` where `p` is `d->GetCharacter()`. Loading-phase, noted for
   completeness.
6. **The `Entergame` handler is 397 lines with one guard.** Every guild, party, PvP, premium,
   arena, dungeon, OX and battle-zone side effect is unconditioned by the character's own
   validity. The Rewrite should make the state transition explicit rather than copy the shape.
7. **Quirk, not Defect:** `SEventLetters` sends a `CHAT_TYPE_COMMAND` record 4 on *every*
   login, so a literal reading of "the enter-game chat is the last record" is never true.

---

## 11. Reproduction of the width probe

Probe sources are in `.scratch/ledger187/probe/` (`probe4.cpp`, `probe5.cpp`, `probe6.cpp`,
and the earlier ones). Each compiles with `g++ -m32 -c -std=c++11` and reads the symbol sizes
with `nm -S`. Every probe includes the five controls listed in §3. The canonical
`i686-linux-gnu-g++-12` is not installed here, so the owner should re-run the probes on a
machine that has it before the widths are frozen into the ledger.


---

## 12. Cross-check against `protocol/src/gc_inventory.rs`

Every wire byte this report names was looked up in the Rewrite's machine-readable table, so the
byte values here are cross-checked against a second, independent source (the client's decode
table) rather than the server's enumerator names alone. Agreement, with two name traps noted:

| byte | server enumerator | Rewrite name | note |
|---|---|---|---|
| 1 | `HEADER_GC_CHARACTER_ADD` | `HEADER_GC_CHARACTER_ADD` | |
| 4 | `HEADER_GC_CHAT` | `HEADER_GC_CHAT` | |
| 15 | `HEADER_GC_MAIN_CHARACTER_OLD` | `HEADER_GC_MAIN_CHARACTER` | **never sent by this server** |
| 16 | `HEADER_GC_CHARACTER_POINTS` | `HEADER_GC_PLAYER_POINTS` | name differs, byte agrees |
| 17 | `HEADER_GC_CHARACTER_POINT_CHANGE` | `HEADER_GC_PLAYER_POINT_CHANGE` | name differs, byte agrees |
| 19 | `HEADER_GC_CHARACTER_UPDATE` | `HEADER_GC_CHARACTER_UPDATE` | |
| 39 | `HEADER_GC_SHOP_SIGN` | `HEADER_GC_SHOP_SIGN` | |
| 40 | `HEADER_GC_DUEL_START` | `HEADER_GC_DUEL_START` | |
| 41 | `HEADER_GC_PVP` | `HEADER_GC_PVP` | |
| 75 | `HEADER_GC_GUILD` | `HEADER_GC_GUILD` | |
| 76 | `HEADER_GC_SKILL_LEVEL` | `HEADER_GC_SKILL_LEVEL_NEW` | name differs, byte agrees |
| 78 / 79 | `HEADER_GC_PARTY_ADD` / `_UPDATE` | same | |
| 83 | `HEADER_GC_PARTY_PARAMETER` | same | |
| 91 / 92 | `HEADER_GC_PARTY_LINK` / `_UNLINK` | same | |
| 106 | `HEADER_GC_TIME` | `HEADER_GC_TIME` | |
| 111 | `HEADER_GC_WALK_MODE` | `HEADER_GC_WALK_MODE` | |
| 113 | `HEADER_GC_MAIN_CHARACTER` | `HEADER_GC_MAIN_CHARACTER2_EMPIRE` | **name trap**: the server's 113 is the "modern" main-character record; the byte 15 record is the old one and is dead here |
| 115 | `HEADER_GC_NPC_POSITION` | same | |
| 121 | `HEADER_GC_CHANNEL` | same | |
| 126 / 127 | `HEADER_GC_AFFECT_ADD` / `_REMOVE` | same | |
| 130 | `HEADER_GC_LAND_LIST` | same | |
| 136 | `HEADER_GC_CHAR_ADDITIONAL_INFO` | same | |
| 137 / 138 | `HEADER_GC_MAIN_CHARACTER3_BGM` / `_4_BGM_VOL` | same | |
| 153 | `HEADER_GC_HYBRIDCRYPT_SDB` | same | |
| 224 | `HEADER_GC_CHARACTER_GOLD` | same | loading phase only |
| 249 | `HEADER_GC_ENTITY` | same | |
| 253 | `HEADER_GC_PHASE` | same | |

The two "name trap" rows are the same cross-direction renaming class the table's own module
documentation warns about. Neither changes a byte; both are why every width in §3 and every
record number in §4 is keyed on the byte, never on the identifier.
