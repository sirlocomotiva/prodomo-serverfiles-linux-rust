# Ledger 187 — legacy character selection and the LOADING phase send path

Read-only research on the frozen legacy C++ under `server/server/`. Nothing in the
repository was modified. No cargo was run.

All paths are relative to the repository root. Line numbers are from the files as
checked in.

---

## 0. How the widths in this document were measured

`i686-linux-gnu-g++-12` is **not installed on this machine** (`which` returns nothing;
`dpkg -l | grep i686` is empty). Per the project rule I did not fall back to a hand sum.
Instead I used the host `g++` (Debian 14.2.0) with **`-m32`**, which selects the i386
SysV ABI, and compiled the *verbatim* struct bodies under `#pragma pack(1)` with
`-fsyntax-only`, reading each size out of a deliberate template-argument error:

```cpp
template<int N> struct Size;
Size<sizeof(SomeStruct)> probe;   // error: aggregate 'Size<NNN>' has incomplete type
```

Probe file: `.scratch/ledger187/widths.cpp`.

Controls re-measured in the same run, all correct for i386:

| control | expected (i386) | measured |
|---|---|---|
| `#pragma pack(1) struct { unsigned long a; unsigned long b; }` | 8 | **8** |
| one `unsigned long` | 4 | **4** |
| one pointer | 4 | **4** |
| one `bool` | 1 | **1** |
| one `time_t` | 4 | **4** |

Caveats, stated plainly:

* `i686-linux-gnu-g++-12` is missing, so this is **not** a measurement made with the
  project's named tool. It is a measurement made with the same ABI via `-m32`.
* 32-bit glibc headers are absent on this machine, so `<time.h>` cannot be included
  under `-m32`. `time_t` is declared in the probe as `typedef long time_t;`, which is
  the i386 glibc definition (`__TIME_T_TYPE` is `long int` for 32-bit). The control row
  above confirms the 4-byte stand-in, and `AGENTS.md` states the same rule.
* The probe reproduces the struct bodies verbatim and resolves the `#ifdef`s by hand
  from `server/server/common/prodomodefines.h` (section 6). Packing regions verified:
  `game/packet.h:274` (`#pragma pack(1)`) to `game/packet.h:3540` (`#pragma pack()`);
  `common/tables.h:345` to `common/tables.h:2291`; `common/length.h:956`
  (`#pragma pack(push, 1)`) to `common/length.h:1274` (`#pragma pack(pop)`).

Measured i386 widths (all under `#pragma pack(1)`):

| struct | bytes |
|---|---|
| `TPlayerItemAttribute` | 3 |
| `TQuickslot` | 2 |
| `TPlayerSkill` | 6 |
| `SItemPos` | 3 |
| `TPacketCGPlayerSelect` | 2 |
| `TPacketGCPhase` | 2 |
| `TPacketGCEntity` | 3 |
| `TPacketEntityInfo` | 28 |
| `TPacketGCMainCharacter` | 46 |
| `TPacketGCMainCharacter3_BGM` | 71 |
| `TPacketGCMainCharacter4_BGM_VOL` | 75 |
| `TPacketGCPoints` | 2041 |
| `TPacketGCSkillLevel` | 1531 |
| `packet_quickslot_add` | 4 |
| `packet_quickslot_del` | 2 |
| `packet_quickslot_swap` | 3 |
| `TPacketGCItemSet` | 72 |
| `TLandPacketElement` | 24 |
| `TPacketGCLandList` | 3 |
| `TPacketGCGold` | 9 |
| `TPlayerLoadPacket` | 9 |
| `TPacketGCPackageSDB` (fixed part) | 7 |

Every one of these also agrees with the hand sum, so the measured value and the
cross-checked value coincide.

Integer widths come from `server/server/libthecore/typedef.h:12-19` (the `#ifndef
__WIN32__` branch, which is the branch this tree takes):

```c
typedef unsigned int		DWORD;
typedef unsigned char		BYTE;
typedef unsigned short 		WORD;
typedef long			LONG;
typedef int			INT;
```

and `libthecore/typedef.h:4`: `typedef unsigned long int QWORD;` — 4 bytes on i386, a
known trap. `QWORD` does not appear in any struct on this path.

---

## 1. Character selection at the select screen

### 1.1 The dispatch arm

`server/server/game/input_login.cpp:1185-1187`:

```c
		case HEADER_CG_CHARACTER_SELECT:
			CharacterSelect(d, c_pData);
			break;
```

`HEADER_CG_CHARACTER_SELECT` is resolved across every definition form. Its only
definition in the tree is a plain enumerator with an explicit value in the CG header
enum at `server/server/game/packet.h:17`:

```
	HEADER_CG_CHARACTER_SELECT					= 6,
```

(There is no `#define`, no `const T`, no anonymous-enum duplicate, and no
auto-incremented alias for it anywhere in `server/server/`.)

### 1.2 The CG record and its exact wire width

`server/server/game/packet.h:497-501`:

```c
typedef struct command_player_select
{
	BYTE	header;
	BYTE	index;
} TPacketCGPlayerSelect;
```

Two fields, `BYTE header` then `BYTE index`, no packing slack needed. Measured width on
i386: **2 bytes**, and that is the **whole frame**, header byte included.

The frame length the server expects comes from the CG registration table,
`server/server/game/packet_info.cpp:113`:

```c
	Set(HEADER_CG_CHARACTER_SELECT, 			sizeof(TPacketCGPlayerSelect), "Select");
```

The frame is 2 bytes. This table's semantics are pinned by the read loop in
`server/server/game/input.cpp:58-124`: `bHeader` is read from the first byte of
`c_pData` (line 78) and `iPacketLen` is then the full frame length including that byte
(line 83, consumed at lines 112-114); `Analyze` is handed `c_pData` still pointing at
the header byte (line 100).

So: **1 header byte (value 6) + 1 index byte = 2 bytes on the wire.** The handler's
`data` pointer is the *start of the record*, so `pinfo->header` is the header byte and
`pinfo->index` is byte 1.

### 1.3 Which phases can reach the arm

`server/server/game/desc.cpp:515-524`:

```c
		case PHASE_SELECT:
			// 로그인 사용을 않기 위해 주석 처리
			//MessengerManager::instance().Logout(GetAccountTable().login); // 다음에 오면 안됨 주석 해제
		case PHASE_LOGIN:
		case PHASE_LOADING:
#ifndef _IMPROVED_PACKET_ENCRYPTION_
			m_bEncrypted = true;
#endif
			m_pInputProcessor = &m_inputLogin;
			break;
```

`PHASE_SELECT`, `PHASE_LOGIN` and `PHASE_LOADING` all fall through to the *same*
input processor (`CInputLogin`). `CInputLogin::Analyze` performs **no phase check of
its own** before the `HEADER_CG_CHARACTER_SELECT` arm. Consequence: a client can send
CG 6 while it is in `PHASE_LOADING` as well as in `PHASE_SELECT`, and the handler runs
again. The Rewrite needs to decide whether that is parity or a Divergence; it is at
least a reachability fact, and it is not blocked by any guard.

`_IMPROVED_PACKET_ENCRYPTION_` is **not defined anywhere in the tree**
(`command grep -rn '#define _IMPROVED_PACKET_ENCRYPTION_' -a .` returns nothing, and
`libthecore/typedef.h`/`prodomodefines.h` do not define it), so the non-improved TEA
branch is the compiled one. That matches the project note that `KEY_AGREEMENT` is dead.

### 1.4 The handler, verbatim

`server/server/game/input_login.cpp:265-305`:

```c
void CInputLogin::CharacterSelect(LPDESC d, const char * data)
{
	struct command_player_select * pinfo = (struct command_player_select *) data;
	const TAccountTable & c_r = d->GetAccountTable();

	sys_log(0, "player_select: login: %s index: %d", c_r.login, pinfo->index);

	if (!c_r.id)
	{
		sys_err("no account table");
		return;
	}

	if (!c_r.players[pinfo->index].dwID)
	{
		sys_err("No player id for login %s", c_r.login);
		d->SetPhase(PHASE_CLOSE);
		return;
	}
	
	if (pinfo->index >= PLAYER_PER_ACCOUNT)
	{
		sys_err("index overflow %d, login: %s", pinfo->index, c_r.login);
		return;
	}

	if (c_r.players[pinfo->index].bChangeName)
	{
		sys_err("name must be changed idx %d, login %s, name %s",
				pinfo->index, c_r.login, c_r.players[pinfo->index].szName);
		return;
	}

	TPlayerLoadPacket player_load_packet;

	player_load_packet.account_id	= c_r.id;
	player_load_packet.player_id	= c_r.players[pinfo->index].dwID;
	player_load_packet.account_index	= pinfo->index;

	db_clientdesc->DBPacket(HEADER_GD_PLAYER_LOAD, d->GetHandle(), &player_load_packet, sizeof(TPlayerLoadPacket));
}
```

### 1.5 Checks, in order, and what each failure does

`PLAYER_PER_ACCOUNT` resolves to **4**, a plain enumerator in the `EMisc` enum at
`server/server/common/length.h:13`:

```c
	PLAYER_PER_ACCOUNT		= 4,
```

`c_r.players` is a fixed 4-element array, `server/server/common/tables.h:405-418`:

```c
typedef struct SAccountTable
{
	DWORD		id;
	char		login[LOGIN_MAX_LEN + 1];
	char		passwd[PASSWD_MAX_LEN + 1];
	char		social_id[SOCIAL_ID_MAX_LEN + 1];
	char		status[ACCOUNT_STATUS_MAX_LEN + 1];
	BYTE		bEmpire;
	TSimplePlayer	players[PLAYER_PER_ACCOUNT];
#ifdef __MULTI_LANGUAGE_SYSTEM__
	BYTE bLanguage;
#endif

} TAccountTable;
```

Order of checks and failure behaviour:

| # | line | check | on failure |
|---|---|---|---|
| 1 | `input_login.cpp:272` | `!c_r.id` | `sys_err("no account table")`, `return`. **No GC record.** Descriptor stays in its current phase. |
| 2 | `input_login.cpp:278` | `!c_r.players[pinfo->index].dwID` | `sys_err`, `d->SetPhase(PHASE_CLOSE)`, `return`. `SetPhase` writes **GC_PHASE (253)**, `phase = 0` (`PHASE_CLOSE`, `packet.h:792`), then swaps the input processor to `CInputClose` (`desc.cpp:505-509`). |
| 3 | `input_login.cpp:285` | `pinfo->index >= PLAYER_PER_ACCOUNT` | `sys_err`, `return`. **No GC record.** |
| 4 | `input_login.cpp:291` | `c_r.players[pinfo->index].bChangeName` | `sys_err`, `return`. **No GC record.** |

**Check 2 is the legacy Defect `AGENTS.md` already records.** `c_r.players[pinfo->index]`
at line 278 reads the array *before* the `pinfo->index >= PLAYER_PER_ACCOUNT` bound check
at line 285. `pinfo->index` is a single unchecked `BYTE` straight off the wire, so
values 4..255 read past the end of a 4-element array inside `TAccountTable`, which is a
member of `DESC`. An honest client cannot trigger it (the client only sends 0..3), so
under the project rule it is a Defect; the Rewrite must bound-check, and the difference
is a recorded Divergence, not parity.

`bChangeName` is a `BYTE` in `TSimplePlayer` (`common/tables.h:355` region). Check 4
means a character whose stored name is flagged for change cannot be entered; legacy
simply refuses and leaves the client on the select screen with no error record.

On **success** the handler sends nothing to the client at all. It builds
`TPlayerLoadPacket` (`server/server/common/tables.h:928-933`):

```c
typedef struct SPlayerLoadPacket
{
	DWORD	account_id;
	DWORD	player_id;
	BYTE	account_index;	/* account 별 정보 */
} TPlayerLoadPacket;
```

measured **9 bytes** on i386, and hands it to the **DB-peer** process:

```c
	db_clientdesc->DBPacket(HEADER_GD_PLAYER_LOAD, d->GetHandle(), &player_load_packet, sizeof(TPlayerLoadPacket));
```

The DB server side is `server/server/db/ClientManager.cpp:2373` →
`CClientManager::QUERY_PLAYER_LOAD` at `server/server/db/ClientManagerPlayer.cpp:275`.
That is the retired-by-ADR-0001 process boundary. Every loading-phase send happens
later, on the answer, in `CInputDB::PlayerLoad`.

Failure paths on the DB answer are in the game server's DB dispatch,
`server/server/game/input_db.cpp:1960-1990`:

```c
	case HEADER_DG_PLAYER_LOAD_SUCCESS:
		PlayerLoad(DESC_MANAGER::instance().FindByHandle(m_dwHandle), c_pData);
		break;
	...
	case HEADER_DG_ITEM_LOAD:
		ItemLoad(DESC_MANAGER::instance().FindByHandle(m_dwHandle), c_pData);
		break;
```

and note `case HEADER_DG_PLAYER_LOAD_FAILED: break;` (`input_db.cpp:1975-1976`) — a DB
refusal sends **nothing** to the client and the descriptor hangs in `PHASE_SELECT`.

The DB server sends, in this order, right after `PLAYER_LOAD_SUCCESS`
(`db/ClientManagerPlayer.cpp:309-360`): `PLAYER_LOAD_SUCCESS`, then
`HEADER_DG_NEED_LOGIN_LOG` (conditional on `packet->player_id != pkLD->GetLastPlayerID()`),
then `HEADER_DG_ITEM_LOAD`, then the quest / affect / safebox queries. That ordering is
what makes item records arrive *after* the loading burst.

---

## 2. The send path for the loading phase — `input_db.cpp:328-457`

### 2.1 The whole function, verbatim

`server/server/game/input_db.cpp:327-457` (the `#define` on line 327 is itself a
feature switch, see 6):

```c
#define ENABLE_GOHOME_IF_MAP_NOT_EXIST
void CInputDB::PlayerLoad(LPDESC d, const char * data)
{
	TPlayerTable * pTab = (TPlayerTable *) data;

	if (!d)
		return;

	long lMapIndex = pTab->lMapIndex;
	PIXEL_POSITION pos;

	if (lMapIndex == 0)
	{
		lMapIndex = SECTREE_MANAGER::instance().GetMapIndex(pTab->x, pTab->y);

		if (lMapIndex == 0)
		{
			lMapIndex = EMPIRE_START_MAP(d->GetAccountTable().bEmpire);
			pos.x = EMPIRE_START_X(d->GetAccountTable().bEmpire);
			pos.y = EMPIRE_START_Y(d->GetAccountTable().bEmpire);
		}
		else
		{
			pos.x = pTab->x;
			pos.y = pTab->y;
		}
	}
	pTab->lMapIndex = lMapIndex;
	if (!SECTREE_MANAGER::instance().GetValidLocation(pTab->lMapIndex, pTab->x, pTab->y, lMapIndex, pos, d->GetEmpire()))
	{
		sys_err("InputDB::PlayerLoad : cannot find valid location %d x %d (name: %s)", pTab->x, pTab->y, pTab->name);
#ifdef ENABLE_GOHOME_IF_MAP_NOT_EXIST
		lMapIndex = EMPIRE_START_MAP(d->GetAccountTable().bEmpire);
		pos.x = EMPIRE_START_X(d->GetAccountTable().bEmpire);
		pos.y = EMPIRE_START_Y(d->GetAccountTable().bEmpire);
#else
		d->SetPhase(PHASE_CLOSE);
		return;
#endif
	}

	pTab->x = pos.x;
	pTab->y = pos.y;
	pTab->lMapIndex = lMapIndex;

	if (d->GetCharacter() || d->IsPhase(PHASE_GAME))
	{
		LPCHARACTER p = d->GetCharacter();
		sys_err("login state already has main state (character %s %p)", p->GetName(), get_pointer(p));
		return;
	}
	if (NULL != CHARACTER_MANAGER::Instance().FindPC(pTab->name))
	{
		sys_err("InputDB: PlayerLoad : %s already exist in game", pTab->name);
		return;
	}
	LPCHARACTER ch = CHARACTER_MANAGER::instance().CreateCharacter(pTab->name, pTab->id);

	ch->BindDesc(d);
	ch->SetPlayerProto(pTab);
	ch->SetEmpire(d->GetEmpire());

	d->BindCharacter(ch);

	{
		// P2P Login
		TPacketGGLogin p;

		p.bHeader = HEADER_GG_LOGIN;
		strlcpy(p.szName, ch->GetName(), sizeof(p.szName));
		p.dwPID = ch->GetPlayerID();
		p.bEmpire = ch->GetEmpire();
		p.lMapIndex = SECTREE_MANAGER::instance().GetMapIndex(ch->GetX(), ch->GetY());
		p.bChannel = g_bChannel;
#ifdef __MULTI_LANGUAGE_SYSTEM__
		p.bLanguage = d ? d->GetLanguage() : LOCALE_YMIR;
#endif

		P2P_MANAGER::instance().Send(&p, sizeof(TPacketGGLogin));

		char buf[51];
		snprintf(buf, sizeof(buf), "%s %lld %d %ld %d %d %d %d",
				inet_ntoa(ch->GetDesc()->GetAddr().sin_addr), ch->GetGold(), g_bChannel, ch->GetMapIndex(), ch->GetAlignment(), ch->GetPremiumPlayer(), ch->GetPremiumPlayerTimer());
		LogManager::instance().CharLog(ch, 0, "LOGIN", buf);

	}

	d->SetPhase(PHASE_LOADING);
	SECTREE_MANAGER::Instance().SendEntity(ch);
	ch->MainCharacterPacket();

	long lPublicMapIndex = lMapIndex >= 10000 ? lMapIndex / 10000 : lMapIndex;
	const TMapRegion * rMapRgn = SECTREE_MANAGER::instance().GetMapRegion(lPublicMapIndex);
	if( rMapRgn )
	{
		DESC_MANAGER::instance().SendClientPackageSDBToLoadMap( d, rMapRgn->strMapName.c_str() );
	}
	if (!map_allow_find(lPublicMapIndex))
	{
		sys_err("InputDB::PlayerLoad : entering %d map is not allowed here (name: %s, empire %u)",
				lMapIndex, pTab->name, d->GetEmpire());

		ch->SetWarpLocation(EMPIRE_START_MAP(d->GetEmpire()),
				EMPIRE_START_X(d->GetEmpire()) / 100,
				EMPIRE_START_Y(d->GetEmpire()) / 100);

		d->SetPhase(PHASE_CLOSE);
		return;
	}

	quest::CQuestManager::instance().BroadcastEventFlagOnLogin(ch);

	for (int i = 0; i < QUICKSLOT_MAX_NUM; ++i)
		ch->SetQuickslot(i, pTab->quickslot[i]);

	ch->PointsPacket();
	ch->SkillLevelPacket();

	sys_log(0, "InputDB: player_load %s %dx%dx%d LEVEL %d MOV_SPEED %d JOB %d ATG %d DFG %d GMLv %d",
			pTab->name,
			ch->GetX(), ch->GetY(), ch->GetZ(),
			ch->GetLevel(),
			ch->GetPoint(POINT_MOV_SPEED),
			ch->GetJob(),
			ch->GetPoint(POINT_ATT_GRADE),
			ch->GetPoint(POINT_DEF_GRADE),
			ch->GetGMLevel());

	ch->QuerySafeboxSize();

}
```

Verified by search that none of the helper calls in this function sends anything extra:
`CHARACTER_MANAGER::CreateCharacter` (`char_manager.cpp:97-200`),
`CHARACTER::SetPlayerProto` (`char.cpp:2233-2420`), `BindDesc`, `SetEmpire` and
`BindCharacter` contain no `Packet(` / `SetPhase(` call at all.

### 2.2 The exact send order in the loading phase

In write order, with the source line that produces each one:

| order | line | record | header byte | bytes | conditional? |
|---|---|---|---|---|---|
| 1 | `input_db.cpp:414` | `GC_PHASE` | **253** (`0xfd`, `packet.h:98`) | 2 | no |
| 2 | `input_db.cpp:415` | `GC_ENTITY` | **249** (`packet.h:3365`) | 3 + 28·n | no (n may be 0) |
| 3 | `input_db.cpp:416` | `GC_MAIN_CHARACTER` **or** `GC_MAIN_CHARACTER3_BGM` **or** `GC_MAIN_CHARACTER4_BGM_VOL` | **113** / **137** / **138** | 46 / 71 / 75 | **yes**, see 2.3 |
| 4 | `input_db.cpp:422` | `GC_HYBRIDCRYPT_SDB` (map resource stream) | **153** (`packet.h:214`) | 7 + stream | **yes**: `rMapRgn != NULL` and stream non-empty |
| 5 | `input_db.cpp:437` | `GC_CHAT` | **4** (`packet.h:104`) | variable | **yes**, one per active quest event flag |
| 6 | `input_db.cpp:439-440` | `GC_QUICKSLOT_ADD` | **28** (`packet.h:128`) | 4 each | **yes**, only for non-empty slots |
| 7 | `input_db.cpp:442` | `GC_CHARACTER_GOLD` | **224** (`packet.h:2952`) | 9 | no (`ENABLE_REMOVE_LIMIT_GOLD` is on) |
| 8 | `input_db.cpp:442` | `GC_CHARACTER_POINTS` | **16** (`packet.h:116`) | 2041 | no |
| 9 | `input_db.cpp:443` | `GC_SKILL_LEVEL` | **76** (`packet.h:161`) | 1531 | no |
| — | `input_db.cpp:455` | `QuerySafeboxSize()` — **sends nothing** | — | — | — |

**Records the brief named that the loading phase does NOT send:**

* **`GC_QUICKSLOT_DEL` (29)** — not sent. The loading loop only calls
  `CHARACTER::SetQuickslot`, and `DelQuickslot` (`char_quickslot.cpp:100-114`) is only
  reached from inside `SetQuickslot` when a *duplicate* slot already exists, which
  cannot happen on a fresh load because `m_quickslot` starts zeroed.
* **`GC_QUICKSLOT_SWAP` (30)** — not sent. `CHARACTER::SwapQuickslot`
  (`char_quickslot.cpp:116`) is only reached from the client handler in `CInputMain`,
  which is not installed in `PHASE_LOADING`.
* **`GC_ITEM_SET` (21)** — not sent by the loading burst. See 2.5.
* **`GC_LAND_LIST` (130)** — not sent by the loading burst. See section 5.

**Ordering guarantees.** Records 1-9 are all written from the same function on the same
thread into `DESC::m_lpOutputBuffer`, so their relative order is exact and
deterministic. Two of them are built by `BufferedPacket` + `Packet` pairs (2 and 4); see
2.4 and section 5 for the byte-level effect.

**Where the loading phase ends.** The next thing the client is expected to do is send
`HEADER_CG_ENTERGAME` (10, `packet.h:20`), dispatched at `input_login.cpp:1197-1199` to
`CInputLogin::Entergame` (`input_login.cpp:562`), which does `d->SetPhase(PHASE_GAME)` at
line 608 and then sends a completely different batch (CPVP list, land list, `GC_TIME`
106, `GC_CHANNEL` 121, greet, bonus info, ...). Nothing in `Entergame` belongs to the
loading phase.

### 2.3 Record 3, `MainCharacterPacket` — the three-way runtime branch

`server/server/game/char.cpp:1965-2029`:

```c
void CHARACTER::MainCharacterPacket()
{
#ifdef FIX_DUNGEON_MUSIC
	const unsigned mapIndex = GetMapIndex() < 10000 ? GetMapIndex() : GetMapIndex() / 10000;
#else
	const unsigned mapIndex = GetMapIndex();?	
#endif
	const BGMInfo& bgmInfo = CHARACTER_GetBGMInfo(mapIndex);

	// SUPPORT_BGM
	if (!bgmInfo.name.empty())
	{
		if (CHARACTER_IsBGMVolumeEnable())
		{
			sys_log(1, "bgm_info.play_bgm_vol(%d, name='%s', vol=%f)", mapIndex, bgmInfo.name.c_str(), bgmInfo.vol);
			TPacketGCMainCharacter4_BGM_VOL mainChrPacket;
			mainChrPacket.header = HEADER_GC_MAIN_CHARACTER4_BGM_VOL;
			mainChrPacket.dwVID = m_vid;
			mainChrPacket.wRaceNum = GetRaceNum();
			mainChrPacket.lx = GetX();
			mainChrPacket.ly = GetY();
			mainChrPacket.lz = GetZ();
			mainChrPacket.empire = GetDesc()->GetEmpire();
			mainChrPacket.skill_group = GetSkillGroup();
			strlcpy(mainChrPacket.szChrName, GetName(), sizeof(mainChrPacket.szChrName));

			mainChrPacket.fBGMVol = bgmInfo.vol;
			strlcpy(mainChrPacket.szBGMName, bgmInfo.name.c_str(), sizeof(mainChrPacket.szBGMName));
			GetDesc()->Packet(&mainChrPacket, sizeof(TPacketGCMainCharacter4_BGM_VOL));
		}
		else
		{
			sys_log(1, "bgm_info.play(%d, '%s')", mapIndex, bgmInfo.name.c_str());
			TPacketGCMainCharacter3_BGM mainChrPacket;
			mainChrPacket.header = HEADER_GC_MAIN_CHARACTER3_BGM;
			mainChrPacket.dwVID = m_vid;
			mainChrPacket.wRaceNum = GetRaceNum();
			mainChrPacket.lx = GetX();
			mainChrPacket.ly = GetY();
			mainChrPacket.lz = GetZ();
			mainChrPacket.empire = GetDesc()->GetEmpire();
			mainChrPacket.skill_group = GetSkillGroup();
			strlcpy(mainChrPacket.szChrName, GetName(), sizeof(mainChrPacket.szChrName));
			strlcpy(mainChrPacket.szBGMName, bgmInfo.name.c_str(), sizeof(mainChrPacket.szBGMName));
			GetDesc()->Packet(&mainChrPacket, sizeof(TPacketGCMainCharacter3_BGM));
		}
	}
	// END_OF_SUPPORT_BGM
	else
	{
		sys_log(0, "bgm_info.play(%d, DEFAULT_BGM_NAME)", mapIndex);

		TPacketGCMainCharacter pack;
		pack.header = HEADER_GC_MAIN_CHARACTER;
		pack.dwVID = m_vid;
		pack.wRaceNum = GetRaceNum();
		pack.lx = GetX();
		pack.ly = GetY();
		pack.lz = GetZ();
		pack.empire = GetDesc()->GetEmpire();
		pack.skill_group = GetSkillGroup();
		strlcpy(pack.szName, GetName(), sizeof(pack.szName));
		GetDesc()->Packet(&pack, sizeof(TPacketGCMainCharacter));
	}
}
```

**The `else` branch is the normal one.** The BGM table `gs_bgmInfoMap` is only ever
populated by `CHARACTER_AddBGMInfo` (`char.cpp:1937`), whose sole call site is the
quest-Lua binding `questlua_global.cpp:426`; `CHARACTER_IsBGMVolumeEnable()`
(`char.cpp:1959`) reads a flag only set by `questlua_global.cpp:408`. So the choice is a
**runtime** condition, not a build switch: 113 unless a quest script has already
registered a BGM name for the map (in which case 138 if volume is enabled, else 137).

`FIX_DUNGEON_MUSIC` **is** defined (`prodomodefines.h:108`), so the map index used for
the BGM lookup is `GetMapIndex()/10000` for dungeon maps. Note that the `#else` branch
at `char.cpp:1970` contains a stray `?` after `GetMapIndex();` — the file would not
compile without `FIX_DUNGEON_MUSIC`. Recorded because it is a latent build break, not
because it affects this deployment.

Field lists and widths, all measured on i386:

`TPacketGCMainCharacter`, `packet.h:1004-1013` — **46 bytes**:

| field | type | filled from |
|---|---|---|
| `header` | `BYTE` | `HEADER_GC_MAIN_CHARACTER` = 113 |
| `dwVID` | `DWORD` | `m_vid` |
| `wRaceNum` | `WORD` | `GetRaceNum()` |
| `szName` | `char[CHARACTER_NAME_MAX_LEN + 1]` = `char[25]` | `strlcpy` from `GetName()` |
| `lx` | `long` | `GetX()` |
| `ly` | `long` | `GetY()` |
| `lz` | `long` | `GetZ()` |
| `empire` | `BYTE` | `GetDesc()->GetEmpire()` |
| `skill_group` | `BYTE` | `GetSkillGroup()` |

`CHARACTER_NAME_MAX_LEN` is 24, a plain enumerator in `EMisc` at `common/length.h:15`.

`TPacketGCMainCharacter3_BGM`, `packet.h:1016-1031` — **71 bytes**: `header` `BYTE`,
`dwVID` `DWORD`, `wRaceNum` `WORD`, `szChrName` `char[25]`, `szBGMName` `char[25]`
(`MUSIC_NAME_LEN = 24`, an enumerator inside the struct at `packet.h:1018-1021`),
`lx`/`ly`/`lz` `long`, `empire` `BYTE`, `skill_group` `BYTE`.

`TPacketGCMainCharacter4_BGM_VOL`, `packet.h:1033-1049` — **75 bytes**: same as above
plus `float fBGMVol` between `szBGMName` and `lx`. `float` is 4 bytes on i386; this is
the one width on the path that a 32/64-bit reading would *not* have broken.

### 2.4 Records 2 and 4 — the two `BufferedPacket` + `Packet` pairs

`DESC::BufferedPacket` (`desc.cpp:386-395`) appends to `m_lpBufferedOutputBuffer`.
`DESC::Packet` (`desc.cpp:397-484`) then, at lines 423-428, detects the pending buffer,
concatenates header-record + payload, and encodes **one** contiguous block:

```c
		if (m_lpBufferedOutputBuffer)
		{
			buffer_write(m_lpBufferedOutputBuffer, c_pvData, iSize);
			c_pvData = buffer_read_peek(m_lpBufferedOutputBuffer);
			iSize = buffer_size(m_lpBufferedOutputBuffer);
		}
```

The buffer is cleared at line 479. So for `GC_ENTITY` and `GC_HYBRIDCRYPT_SDB` the
client sees a single continuous byte run, not two writes. TEA runs over the whole run
(once encrypted), so the encryption boundary is the same either way.

### 2.5 Records 6-9 in detail

#### Quickslots — `GC_QUICKSLOT_ADD` (28), 4 bytes

`input_db.cpp:439-440`:

```c
	for (int i = 0; i < QUICKSLOT_MAX_NUM; ++i)
		ch->SetQuickslot(i, pTab->quickslot[i]);
```

`QUICKSLOT_MAX_NUM` is 36 (`common/length.h:51`). The handler,
`CHARACTER::SetQuickslot` (`char_quickslot.cpp:45-98`):

```c
bool CHARACTER::SetQuickslot(BYTE pos, TQuickslot & rSlot)
{
	struct packet_quickslot_add pack_quickslot_add;

	if (pos >= QUICKSLOT_MAX_NUM)
		return false;

	if (rSlot.type >= QUICKSLOT_TYPE_MAX_NUM)
		return false;

	for (int i = 0; i < QUICKSLOT_MAX_NUM; ++i)
	{
		if (rSlot.type == 0)
			continue;
		else if (m_quickslot[i].type == rSlot.type && m_quickslot[i].pos == rSlot.pos)
			DelQuickslot(i);
	}

	TItemPos srcCell(INVENTORY, rSlot.pos);

	switch (rSlot.type)
	{
		case QUICKSLOT_TYPE_ITEM:
			if (false == srcCell.IsDefaultInventoryPosition() && false == srcCell.IsBeltInventoryPosition())
				return false;

			break;

		case QUICKSLOT_TYPE_SKILL:
			if ((int) rSlot.pos >= SKILL_MAX_NUM)
				return false;

			break;

		case QUICKSLOT_TYPE_COMMAND:
			break;

		default:
			return false;
	}

	m_quickslot[pos] = rSlot;

	if (GetDesc())
	{
		pack_quickslot_add.header	= HEADER_GC_QUICKSLOT_ADD;
		pack_quickslot_add.pos		= pos;
		pack_quickslot_add.slot		= m_quickslot[pos];

		GetDesc()->Packet(&pack_quickslot_add, sizeof(pack_quickslot_add));
	}

	return true;
}
```

`QUICKSLOT_TYPE_NONE = 0` is the first member of the anonymous enum at
`common/length.h:374-381`:

```c
enum
{
	QUICKSLOT_TYPE_NONE,
	QUICKSLOT_TYPE_ITEM,
	QUICKSLOT_TYPE_SKILL,
	QUICKSLOT_TYPE_COMMAND,
	QUICKSLOT_TYPE_MAX_NUM,
};
```

so an empty stored slot (`type == 0`) falls to `default: return false;` at
`char_quickslot.cpp:82-83` and **sends nothing**. Wire result: one
`GC_QUICKSLOT_ADD` per **non-empty** slot, in ascending slot order 0..35, with
`pos` = slot index and `slot` = the stored `{type, pos}` pair verbatim. No sorting, no
gaps filled, no summary record, no count.

`packet_quickslot_add`, `packet.h:1508-1513` — **4 bytes**: `header` `BYTE`,
`pos` `BYTE`, `slot` `TQuickslot`. `TQuickslot` (`common/tables.h:452-456`) is
`BYTE type; BYTE pos;` = 2 bytes. Raw bytes; no NUL requirement.

#### `GC_CHARACTER_GOLD` (224) then `GC_CHARACTER_POINTS` (16)

`char.cpp:2031-2087`, `CHARACTER::PointsPacket`. The gold record is emitted *first*:

```c
#ifdef ENABLE_REMOVE_LIMIT_GOLD
	TPacketGCGold packGold;
	packGold.header = HEADER_GC_CHARACTER_GOLD;
	packGold.gold = GetGold();
	GetDesc()->Packet(&packGold, sizeof(TPacketGCGold));
#endif

	GetDesc()->Packet(&pack, sizeof(TPacketGCPoints));
```

`ENABLE_REMOVE_LIMIT_GOLD` **is** defined (`prodomodefines.h:157`), so the
`GC_CHARACTER_GOLD` record is in the loading burst.
`TPacketGCGold` (`packet.h:2956-2960`) is `BYTE header; unsigned long long gold;` =
**9 bytes**.

`TPacketGCPoints`, `packet.h:1052-1056` — **2041 bytes**:

```c
typedef struct packet_points
{
	BYTE	header;
	long long		points[POINT_MAX_NUM];
} TPacketGCPoints;
```

`POINT_MAX_NUM = 255` (`common/length.h:82`, plain enumerator in `EMisc`). 255 × 8
`long long` is fixed-width and does not depend on the target, so a 64-bit reading would
not have broken this one.

Field-filling order, verbatim from `char.cpp:2040-2077`:

```c
	pack.points[POINT_LEVEL]		= GetLevel();
	pack.points[POINT_EXP]		= GetExp();
	pack.points[POINT_NEXT_EXP]		= GetNextExp();
	pack.points[POINT_HP]		= GetHP();
	pack.points[POINT_MAX_HP]		= GetMaxHP();
	pack.points[POINT_SP]		= GetSP();
	pack.points[POINT_MAX_SP]		= GetMaxSP();
	pack.points[POINT_GOLD]		= GetGold();
	pack.points[POINT_STAMINA]		= GetStamina();
	pack.points[POINT_MAX_STAMINA]	= GetMaxStamina();
#ifdef ENABLE_EXTEND_INVEN_SYSTEM
	pack.points[POINT_INVEN]		= Inven_Point();
#endif
#ifdef ENABLE_GAYA_SYSTEM
	pack.points[POINT_GAYA]			= GetGaya();
#endif
	for (int i = POINT_ST; i < POINT_MAX_NUM; ++i)
		pack.points[i] = GetPoint(i);
	
	
#ifdef __ENABLE_BIOLOGIST_RENEWAL_SYSTEM__
	pack.points[POINT_BIOLOGIST_STATE] = GetBiologistState();
	pack.points[POINT_BIOLOGIST_ITEMS_TAKEN] = GetBiologistItemsTaken();
	pack.points[POINT_BIOLOGIST_COMPLETED] = GetBiologistCompleted();
#endif

#ifdef __ENABLE_INVENTORY_PROTECTED_SYSTEM__
	pack.points[POINT_SECURED_STATE] = GetSecuredState();
	pack.points[POINT_SECURED_PASSWORD] = GetSecuredPassword();
#endif

#if defined(__CONQUEROR_LEVEL__)
	pack.points[POINT_CONQUEROR_LEVEL] = GetConquerorLevel();
	pack.points[POINT_CONQUEROR_EXP] = GetConquerorExp();
	pack.points[POINT_CONQUEROR_NEXT_EXP] = GetConquerorNextExp();

	pack.points[POINT_MOV_SPEED] = GetLimitPoint(POINT_MOV_SPEED);
#endif
```

**A second legacy Defect worth recording.** `TPacketGCPoints pack;` at `char.cpp:2036`
is an uninitialised automatic, and the fill loop starts at `POINT_ST`. Index values
from `enum EPointTypes` (`game/char.h:140-154`) give `POINT_NONE = 0`,
`POINT_LEVEL = 1`, `POINT_VOICE = 2`, …, `POINT_GOLD = 11`, `POINT_ST = 12`. The
explicit assignments cover 1, 3, 4, 5, 6, 7, 8, 9, 10, 11; the loop covers 12..254.
**`points[0]` and `points[2]` are never written**, so 16 bytes of stack garbage go out
in every `GC_CHARACTER_POINTS` record — including the two records the loading phase
sends. The Rewrite must zero the array; that is a recorded Divergence, not parity.
(Reachable by an honest client, so under the project rule it is a Defect.)

Every `#ifdef` on that path is on in this tree (section 6), so the effective set of
overwrites after the loop is: 145 (`POINT_INVEN`), 166/167/168 (biologist),
169-176 (sungma/conqueror block, from the `__CONQUEROR_LEVEL__` enum at `char.h:294-305`
plus the explicit four), 207 (`POINT_GAYA`), 208/209 (secured). `POINT_MOV_SPEED` (18)
is overwritten a second time with `GetLimitPoint(...)`.

#### `GC_SKILL_LEVEL` (76), 1531 bytes

`char_skill.cpp:171-181`:

```c
void CHARACTER::SkillLevelPacket()
{
	if (!GetDesc())
		return;

	TPacketGCSkillLevel pack;

	pack.bHeader = HEADER_GC_SKILL_LEVEL;
	thecore_memcpy(&pack.skills, m_pSkillLevels, sizeof(TPlayerSkill) * SKILL_MAX_NUM);
	GetDesc()->Packet(&pack, sizeof(TPacketGCSkillLevel));
}
```

`packet.h:1058-1062`:

```c
typedef struct packet_skill_level
{
	BYTE		bHeader;
	TPlayerSkill	skills[SKILL_MAX_NUM];
} TPacketGCSkillLevel;
```

`SKILL_MAX_NUM = 255` (`common/length.h:72`). `TPlayerSkill`
(`common/tables.h:466-471`):

```c
typedef struct SPlayerSkill
{
	BYTE	bMasterType;
	BYTE	bLevel;
	time_t	tNextRead;
} TPlayerSkill;
```

measured **6 bytes** on i386, because `time_t` is 4 there. **This is the one struct on
the whole path where a 64-bit reading would have silently changed the width** (it would
have become 10, and the record 2561 bytes). 1 + 255×6 = **1531**, matching the
measurement.

The whole array is a straight `thecore_memcpy` of `m_pSkillLevels`; no filtering, no
count field, no per-skill bounds work at send time. `tNextRead` goes on the wire as raw
little-endian bytes even though it is a server-side cooldown timestamp.

#### `QuerySafeboxSize` sends nothing

`char.cpp:7320-7331`:

```c
void CHARACTER::QuerySafeboxSize()
{
	if (m_iSafeboxSize == -1)
	{
		DBManager::instance().ReturnQuery(QID_SAFEBOX_SIZE,
				GetPlayerID(),
				NULL,
				"SELECT size FROM safebox%s WHERE account_id = %u",
				get_table_postfix(),
				GetDesc()->GetAccountTable().id);
	}
}
```

It is a database round trip against the game server's own MySQL link, not a send. The
answer is applied by `db.cpp:462-477` and never reaches the client. Note the
string-formatted SQL with an interpolated `get_table_postfix()` — the same pattern
`AGENTS.md` flags as a Defect elsewhere.

---

## 3. Record 249 — `GC_ENTITY`

### 3.1 The records

`packet.h:3364-3379` (inside the `pack(1)` region, and both structs are the only place
in the whole tree where the `struct`-tag form is spelled — see the sweep below):

```c
enum EntityHeader {
	HEADER_GC_ENTITY = 249,
};

using TPacketGCEntity = struct SPacketGCEntity
{
	BYTE bHeader;
	WORD wSize;
};
using TPacketEntityInfo = struct SPacketEntityInfo
{
	DWORD dwVID;
	DWORD dwRaceVNum;
	WORD wPart[CHR_EQUIPPART_NUM];
	LONG xPos, yPos;
};
```

`HEADER_GC_ENTITY` is a single-line enumerator with an explicit value, so **249** is
unambiguous; no `#define` or other definition form exists for it.

**Two separate searches, tag and alias, as the project rule requires:**

* `command grep -rn 'SPacketEntityInfo' -a ..` → only `game/packet.h:3373`.
* `command grep -rn 'TPacketEntityInfo' -a ..` → `game/packet.h:3373` and
  `game/sectree_manager.cpp:1719`.
* `command grep -rn 'SPacketGCEntity' -a ..` → only `game/packet.h:3368`.
* `command grep -rn 'TPacketGCEntity' -a ..` → `game/packet.h:3368` and
  `game/sectree_manager.cpp:1735`.
* `command grep -rn 'HEADER_GC_ENTITY' -a ..` → `game/packet.h:3365` and
  `game/sectree_manager.cpp:1736`.

So the record is defined once and produced once. No call site spells the struct tag;
the alias is the only spelling in use.

### 3.2 Widths

| struct | bytes | hand sum |
|---|---|---|
| `TPacketGCEntity` (fixed head) | **3** | 1 + 2 |
| `TPacketEntityInfo` (one element) | **28** | 4 + 4 + 2×`CHR_EQUIPPART_NUM` + 4 + 4 |

`CHR_EQUIPPART_NUM` is **6** because both `__SASH_SYSTEM__` and `__AURA_SYSTEM__` are
defined — `packet.h:869-882`:

```c
enum ECharacterEquipmentPart
{
	CHR_EQUIPPART_ARMOR,
	CHR_EQUIPPART_WEAPON,
	CHR_EQUIPPART_HEAD,
	CHR_EQUIPPART_HAIR,
#ifdef __SASH_SYSTEM__
	CHR_EQUIPPART_SASH,
#endif
#ifdef __AURA_SYSTEM__
	CHR_EQUIPPART_AURA,
#endif
	CHR_EQUIPPART_NUM,
};
```

So the six slots are ARMOR, WEAPON, HEAD, HAIR, SASH, AURA, in that index order. With
either switch off the element would be 24 bytes; that is a wire-visible build switch
and the Rewrite must match whichever it configures.

`LONG` is `long` = 4 bytes on i386 (`libthecore/typedef.h:17`).

**This is a variable-width record.** There is no count field and no sub-header. The
element count is implied by `wSize`: `n = (wSize - 3) / 28`. The frame adapter must
check `wSize >= 3` and `(wSize - 3) % 28 == 0`; the decoder must check the same. A
`wSize` of 3 means zero elements.

### 3.3 The builder

`server/server/game/sectree_manager.cpp:1697-1746`, verbatim:

```c
void SECTREE_MANAGER::SendEntity(const LPCHARACTER c_lpChar)
{
	if (c_lpChar == nullptr)
		return;

	const LPDESC c_lpDesc = c_lpChar->GetDesc();
	if (c_lpDesc == nullptr)
		return;

	TEMP_BUFFER TempBuffer;
	const DESC_MANAGER::DESC_SET& c_rDescSet = DESC_MANAGER::instance().GetClientSet();
	for (const LPDESC& it : c_rDescSet)
	{
		if (it == nullptr)
			continue;

		const LPCHARACTER c_lpEntityChar = it->GetCharacter();
		if (c_lpEntityChar == nullptr)
			continue;

		if (c_lpEntityChar->IsPC())
		{
			TPacketEntityInfo EntityInfo = {};
			EntityInfo.dwVID = c_lpEntityChar->GetVID();
			EntityInfo.dwRaceVNum = c_lpEntityChar->GetRaceNum();
			EntityInfo.wPart[CHR_EQUIPPART_ARMOR] = c_lpEntityChar->GetPart(PART_MAIN);
			EntityInfo.wPart[CHR_EQUIPPART_WEAPON] = c_lpEntityChar->GetPart(PART_WEAPON);
			EntityInfo.wPart[CHR_EQUIPPART_HEAD] = c_lpEntityChar->GetPart(PART_HEAD);
			EntityInfo.wPart[CHR_EQUIPPART_HAIR] = c_lpEntityChar->GetPart(PART_HAIR);
			EntityInfo.wPart[CHR_EQUIPPART_SASH] = c_lpEntityChar->GetPart(PART_SASH);
			EntityInfo.wPart[CHR_EQUIPPART_AURA] = c_lpEntityChar->GetPart(PART_AURA);
			EntityInfo.xPos = c_lpEntityChar->GetX();
			EntityInfo.yPos = c_lpEntityChar->GetY();

			TempBuffer.write(&EntityInfo, sizeof(EntityInfo));
		}
	}

	TPacketGCEntity Packet;
	Packet.bHeader = HEADER_GC_ENTITY;
	Packet.wSize = sizeof(Packet) + TempBuffer.size();

	if (TempBuffer.size())
	{
		c_lpDesc->BufferedPacket(&Packet, sizeof(Packet));
		c_lpDesc->Packet(TempBuffer.read_peek(), TempBuffer.size());
	}
	else
		c_lpDesc->Packet(&Packet, sizeof(Packet));
}
```

What is in the list, and what is not — three surprises:

1. **It is not a map entity list.** It iterates `DESC_MANAGER::instance().GetClientSet()`,
   which is **every descriptor the process has**, i.e. every connected client in this
   Channel. There is **no map filter, no distance filter, and no sectree involvement**,
   despite living in `sectree_manager.cpp`. A player entering the world receives a
   record naming every other logged-in player on the same Channel, on every map.
2. **Only players.** The `if (c_lpEntityChar->IsPC())` test excludes mobs, NPCs,
   items, buildings and private shops. The element struct has no `bType`/state byte, so
   the record is player-only by construction.
3. **The element order is not deterministic.** `DESC_SET` is
   `TR1_NS::unordered_set<LPDESC>` (`desc_manager.h:16-17`), so the iteration order
   depends on the hash of the pointer values. Two runs with the same population can
   produce the same set of elements in a different order. The Rewrite must not try to
   reproduce an order; it needs an agreed, deterministic order and a recorded
   Divergence if it differs, or the client may be order-sensitive.

`PART_MAIN`, `PART_WEAPON`, `PART_HEAD`, `PART_HAIR`, `PART_SASH`, `PART_AURA` are
enumerators of `enum EParts` at `common/length.h:383-390`, matching the six
`CHR_EQUIPPART_*` indices one for one.

The recipient is only the loading descriptor (`c_lpDesc`, the `DESC` of the character
being loaded). The list is **not** broadcast.

Feature gates: **none** on this path. `SendEntity` has no `#ifdef` in its body, and
`input_db.cpp:415` calls it unconditionally. The only conditional content is the
`__SASH_SYSTEM__` / `__AURA_SYSTEM__` width of `wPart[]` (3.2) and the
runtime `IsPC()` test.

---

## 4. Record 21 — there is no `ITEM_SET2`

`command grep -rn 'ItemSet2' -a --include=*.h --include=*.cpp .` over the whole legacy
tree returns **nothing**. Record 21 is:

`packet.h:122`: `HEADER_GC_ITEM_SET = 21,`

and the struct is `TPacketGCItemSet` (`packet.h:1427-1444`).

### 4.1 Full struct, measured width, fixed not variable

```c
typedef struct packet_item_set
{
	BYTE	header;
	TItemPos Cell;
	DWORD	vnum;
	WORD	count;
#ifdef ENABLE_REFINE_ELEMENT
	DWORD	dwRefineElement;
#endif
#ifdef __CHANGELOOK_SYSTEM__
	DWORD	transmutation;
#endif
	DWORD	flags;
	DWORD	anti_flags;
	bool	highlight;
	long	alSockets[ITEM_SOCKET_MAX_NUM];
	TPlayerItemAttribute aAttr[ITEM_ATTRIBUTE_MAX_NUM];
} TPacketGCItemSet;
```

`ENABLE_REFINE_ELEMENT` (`prodomodefines.h:28`) and `__CHANGELOOK_SYSTEM__`
(`prodomodefines.h:16`) are **both defined**, so both fields are present.
`ITEM_SOCKET_MAX_NUM` is **6** because `ENABLE_EXTENDED_SOCKETS` is defined
(`prodomodefines.h:76`; the alternative 3 is at `common/item_length.h:13-18`).
`ITEM_ATTRIBUTE_MAX_NUM` is **7** (`common/item_length.h:30`,
`ITEM_ATTRIBUTE_RARE_END` = 5 + 2).

`TItemPos` (`common/length.h:957-1057`, inside `#pragma pack(push, 1)`) is
`BYTE window_type; WORD cell;` = **3 bytes**.

`TPlayerItemAttribute` (`common/tables.h:426-430`) is `BYTE bType; short sValue;` =
**3 bytes**.

Measured: `TPacketGCItemSet` = **72 bytes**, matching the hand sum
1 + 3 + 4 + 2 + 4 + 4 + 4 + 4 + 1 + 6×4 + 7×3 = 72.

**It is a fixed-width record.** There is no `wSize`, no sub-header, no count, and no
appended array. One record carries one item at one cell. `ITEM_SOCKET_MAX_NUM` and
`ITEM_ATTRIBUTE_MAX_NUM` are the only things that can change its width, and both are
build switches.

### 4.2 The send site

`HEADER_GC_ITEM_SET` has exactly one producer in the whole tree
(`command grep -rn 'HEADER_GC_ITEM_SET' -a ..` → `char_item.cpp:572` and `packet.h:122`).
It is `CHARACTER::SetItem`, `char_item.cpp:567-593`:

```c
	if (GetDesc())
	{
		if (pItem)
		{
			TPacketGCItemSet pack;
			pack.header = HEADER_GC_ITEM_SET;
			pack.Cell = Cell;

			pack.count = pItem->GetCount();
#ifdef ENABLE_REFINE_ELEMENT
			pack.dwRefineElement = pItem->GetRefineElement();
#endif
#ifdef __CHANGELOOK_SYSTEM__
			pack.transmutation = pItem->GetTransmutation();
#endif
			pack.vnum = pItem->GetVnum();
			pack.flags = pItem->GetFlag();
			pack.anti_flags	= pItem->GetAntiFlag();
#if defined(__BL_ENABLE_PICKUP_ITEM_EFFECT__)
			pack.highlight = bHighlight;
#else
			pack.highlight = (Cell.window_type == DRAGON_SOUL_INVENTORY);
#endif

			thecore_memcpy(pack.alSockets, pItem->GetSockets(), sizeof(pack.alSockets));
			thecore_memcpy(pack.aAttr, pItem->GetAttributes(), sizeof(pack.aAttr));
			GetDesc()->Packet(&pack, sizeof(TPacketGCItemSet));
		}
		else
		{
			TPacketGCItemDelDeprecated pack;
			pack.header = HEADER_GC_ITEM_DEL;
			...
```

`__BL_ENABLE_PICKUP_ITEM_EFFECT__` is defined (`prodomodefines.h:23`), so
`highlight` is the `bHighlight` argument, not the Dragon Soul expression. Note every
`pack` field except `header` is assigned: there is no uninitialised-member problem
here.

### 4.3 `input_db.cpp:1451` is `ItemLoad`, not the record

The brief pointed at `input_db.cpp` around 1451 for this record. That line is the
**DB-peer arrival handler**, `CInputDB::ItemLoad` (`input_db.cpp:1451-1567`), and it
does not itself build a `TPacketGCItemSet`. It reads a count, walks
`TPlayerItem` records, and calls `item->AddToCharacter(ch, TItemPos(...))` or
`item->EquipTo(ch, p->pos)` (`input_db.cpp:1506-1532`), and *those* calls reach
`CHARACTER::SetItem`, which is the actual `GC_ITEM_SET` producer. The handler then
closes with two more sends:

```c
	ch->CheckMaximumPoints();
	ch->PointsPacket();

	ch->SetItemLoaded();
```

(`input_db.cpp:1563-1566`) — so a **second `GC_CHARACTER_GOLD` + `GC_CHARACTER_POINTS`
pair** follows the item records, and `IsItemLoaded()` (checked at
`input_db.cpp:1458`) makes the whole handler idempotent.

Timing: the DB server sends `HEADER_DG_ITEM_LOAD` immediately after
`PLAYER_LOAD_SUCCESS` (`db/ClientManagerPlayer.cpp:346-350`), so these records land
**after** the whole synchronous loading burst of section 2.2, but while the client is
still in `PHASE_LOADING` (the client has not sent `CG_ENTERGAME` yet). The order
relative to the burst is causal but the order *within* the item set follows
`TItemCacheSet` iteration in the DB server, not a slot order.

Items that fail to be placed go to the ground instead (`input_db.cpp:1543-1561`), and
`EQUIPMENT` items above the character's level are pushed to inventory
(`input_db.cpp:1519-1531`).

---

## 5. Map / land data at loading time

**`GC_LAND_LIST` (130) is not sent during the loading phase.** The only two producers
are both outside it.

`packet.h:2509-2521`:

```c
typedef struct
{
	DWORD	dwID;
	long	x, y;
	long	width, height;
	DWORD	dwGuildID;
} TLandPacketElement;

typedef struct packet_land_list
{
	BYTE	header;
	WORD	size;
} TPacketGCLandList;
```

`TLandPacketElement` measures **24 bytes** (6 × 4, all `DWORD`/`long`); the head
measures **3 bytes**. `HEADER_GC_LAND_LIST` is a plain enumerator, `packet.h:202`:
`= 130`.

Like `GC_ENTITY`, this is variable width with **no sub-header and no count**: the
element count is implied by `size`, `n = (size - 3) / 24`.

Producer A — `CManager::SendLandList`, `building.cpp:938-988`:

```c
void CManager::SendLandList(LPDESC d, long lMapIndex)
{
	TLandPacketElement e;

	TEMP_BUFFER buf;

	WORD wCount = 0;

	itertype(m_map_pkLand) it = m_map_pkLand.begin();

	while (it != m_map_pkLand.end())
	{
		CLand * pkLand = (it++)->second;
		const TLand & r = pkLand->GetData();

		if (r.lMapIndex != lMapIndex)
			continue;
		...
		buf.write(&e, sizeof(TLandPacketElement));
		++wCount;
	}

	sys_log(0, "SendLandList map %d count %u elem_size: %d", lMapIndex, wCount, buf.size());

	if (wCount != 0)
	{
		TPacketGCLandList p;

		p.header = HEADER_GC_LAND_LIST;
		p.size = sizeof(TPacketGCLandList) + buf.size();

		d->BufferedPacket(&p, sizeof(TPacketGCLandList));
		d->Packet(buf.read_peek(), buf.size());
	}
}
```

Its only caller is `server/server/game/input_login.cpp:676`, inside
`CInputLogin::Entergame`, which has already done `d->SetPhase(PHASE_GAME)` at line 608.
So it is a **`PHASE_GAME`** record, not a loading record. `wCount` is computed and
logged but never written to the wire — the count is implicit, and the record is not
sent at all when `wCount == 0`. The iteration order is `std::map` order, i.e. by
`dwID`, so that one *is* deterministic. Each land also triggers
`ch->SendGuildName(guild)` (`building.cpp:960-962`) before the land list.

Producer B — `CManager::UpdateLand`, `building.cpp:707-753`, is a live DB update that
sends one `size = sizeof(head) + sizeof(element) = 27` record to every descriptor whose
character is on `pTable->lMapIndex`, again preceded by `SendGuildName`.

**Feature gate:** none on the record or the send. `map_allow_find` (`config.cpp:174-183`)
gates which maps get lands registered in the first place (`building.cpp:221`, `248`,
`899`), and `MAP_ALLOW_LIMIT = 32` is a plain enumerator in
`prodomodefines.h:4-6`.

### 5.1 The one map record the loading phase *does* send

`GC_HYBRIDCRYPT_SDB` (**153**, `packet.h:214`) is a Prodomo-specific map-resource
stream, sent at `input_db.cpp:418-423`:

```c
	long lPublicMapIndex = lMapIndex >= 10000 ? lMapIndex / 10000 : lMapIndex;
	const TMapRegion * rMapRgn = SECTREE_MANAGER::instance().GetMapRegion(lPublicMapIndex);
	if( rMapRgn )
	{
		DESC_MANAGER::instance().SendClientPackageSDBToLoadMap( d, rMapRgn->strMapName.c_str() );
	}
```

`DESC_MANAGER::SendClientPackageSDBToLoadMap`, `desc_manager.cpp:552-572`:

```c
void DESC_MANAGER::SendClientPackageSDBToLoadMap( LPDESC desc, const char* pMapName )
{
	if( !desc )
	{
		return;
	}

	TPacketGCPackageSDB packet;
	{
		packet.bHeader      = HEADER_GC_HYBRIDCRYPT_SDB;
		if( !m_pPackageCrypt->GetRelatedMapSDBStreams( pMapName, &(packet.m_pDataSDBStream), packet.iStreamLen ) )
			return;
		if (test_server)
			sys_log(0, "[PackageCryptInfo] send to %s from map %s. (SDB len: %d)", desc->GetAccountTable().login, pMapName, packet.iStreamLen);
	}

	if( packet.iStreamLen > 0 )
	{
		desc->Packet( packet.GetStreamData(), packet.GetStreamSize());
	}
}
```

The wire layout is built by hand in `TPacketGCPackageSDB::GetStreamData`,
`packet.h:2691-2719`:

```cpp
	BYTE* GetStreamData()
	{
		if( m_pStream )
			delete[] m_pStream;

		uDynamicPacketSize =  GetStreamSize();

		m_pStream = new BYTE[ uDynamicPacketSize ];

		memcpy( m_pStream, &bHeader, 1 );
		memcpy( m_pStream+1, &uDynamicPacketSize, 2 );
		memcpy( m_pStream+3, &iStreamLen, 4 );

		if( iStreamLen > 0 )
			memcpy( m_pStream+7, m_pDataSDBStream, iStreamLen);

		return m_pStream;
	}

	BYTE	bHeader;
	WORD    uDynamicPacketSize; // 가변길이 동적 DynamicPacketHeader 규약을 따름 -_-;
	int		iStreamLen;
	BYTE*   m_pDataSDBStream;
```

So the frame is: `BYTE 153`, `WORD totalSize` (= 7 + `iStreamLen`), `int32 iStreamLen`,
then `iStreamLen` opaque bytes. **Two length fields, and they are redundant** —
`totalSize` is derivable from `iStreamLen`. The fixed part measures **7 bytes**; the
stream itself is a serialised per-map resource blob from
`CClientPackageCryptInfo::GetRelatedMapSDBStreams`
(`ClientPackageCryptInfo.cpp:206-224`), looked up by lower-cased map name, with no
`packet.h` description of its contents. This is the least portable part of the loading
phase: the Rewrite cannot produce it from the frozen C++ alone and will need either the
serialised blobs or a recorded Divergence.

Two runtime conditions guard it: `rMapRgn` must be non-NULL, and
`GetRelatedMapSDBStreams` must return true (map present **and** `vecSDBInfos` non-empty)
**and** `iStreamLen > 0`.

---

## 6. Every preprocessor guard on these paths, and whether it is on in this tree

`server/server/common/prodomodefines.h` is the only feature header. It is 201 lines
long and contains **no `#undef` and no `#if` around the feature block** other than the
one nested block at lines 171-175 (`ENABLE_GLOBAL_RANK` internals) and 177-190
(`__PREMIUM_PRIVATE_SHOP__` internals). Everything else is an unconditional `#define`,
so "defined in this tree" is simply "present in the file".

| guard | site | file:line | in this tree? | effect if flipped |
|---|---|---|---|---|
| `ENABLE_GOHOME_IF_MAP_NOT_EXIST` | `PlayerLoad` invalid-location branch | `input_db.cpp:327, 358-365` | **defined** (`prodomodefines.h:327` is the *use*; the define is in-tree — see note) | would `SetPhase(PHASE_CLOSE)` instead of warping home |
| `__MULTI_LANGUAGE_SYSTEM__` | `TPacketGGLogin.bLanguage` | `input_db.cpp:401-403` | **defined** (`:133`) | server-to-server only; no client wire effect |
| `FIX_DUNGEON_MUSIC` | `MainCharacterPacket` map index | `char.cpp:1967-1971` | **defined** (`:108`) | BGM lookup uses the raw map index; the `#else` branch has a stray `?` and would not compile |
| `ENABLE_REMOVE_LIMIT_GOLD` | extra `GC_CHARACTER_GOLD` before `GC_CHARACTER_POINTS` | `char.cpp:2079-2084` | **defined** (`:157`) | record 224 disappears from the loading burst |
| `ENABLE_EXTEND_INVEN_SYSTEM` | `points[POINT_INVEN]`, `INVENTORY_PAGE_COUNT` | `char.cpp:2050-2052`; `length.h:20-24` | **defined** (`:60`, again `:62`) | slot 145 unwritten; inventory pages 4 → 2 |
| `ENABLE_GAYA_SYSTEM` | `points[POINT_GAYA]` | `char.cpp:2053-2055` | **defined** (`:52`) | slot 207 left at the loop value |
| `__ENABLE_BIOLOGIST_RENEWAL_SYSTEM__` | `points[POINT_BIOLOGIST_*]` | `char.cpp:2060-2064` | **defined** (`:148`) | slots 166-168 left at the loop value |
| `__ENABLE_INVENTORY_PROTECTED_SYSTEM__` | `points[POINT_SECURED_*]`, `INVENTORY_PROTECTED_PASSWORD_MAX_LEN` | `char.cpp:2066-2069`; `length.h:35-37` | **defined** (`:169`) | slots 208-209 left at the loop value |
| `__CONQUEROR_LEVEL__` | `points[POINT_CONQUEROR_*]`, `POINT_MOV_SPEED` overwrite, `Entergame` `SetSungMaWill` | `char.cpp:2071-2077` | **defined** (`:53`) | 169-176 left at the loop value; `POINT_MOV_SPEED` not overwritten |
| `BONUS_PCT` | 30 auto-incremented `POINT_*` members after the conqueror block | `char.h:306-335` | **defined** (`:36`) | shifts every later `POINT_*` index |
| `__SASH_SYSTEM__` | `CHR_EQUIPPART_SASH`, so `CHR_EQUIPPART_NUM` | `packet.h:875-877` | **defined** (`:15`) | `TPacketEntityInfo` element 28 → 26 bytes |
| `__AURA_SYSTEM__` | `CHR_EQUIPPART_AURA`, so `CHR_EQUIPPART_NUM` | `packet.h:878-880` | **defined** (`:30`) | element 28 → 24 with both off; 26 with only this off |
| `__PREMIUM_PRIVATE_SHOP__` | `TAccountTable`/entity types, `POINT_PRIVATE_SHOP_UNLOCKED_SLOT` | `char.h`; `common/tables.h` | **defined** (`:176`) | not on this path |
| `ENABLE_EXTENDED_SOCKETS` | `ITEM_SOCKET_MAX_NUM` = 6 vs 3 | `common/item_length.h:13-18` | **defined** (`:76`) | `TPacketGCItemSet` 72 → 60 bytes |
| `ENABLE_REFINE_ELEMENT` | `TPacketGCItemSet.dwRefineElement` | `packet.h:1433-1435`; `char_item.cpp:576-578` | **defined** (`:28`) | item record 72 → 68 |
| `__CHANGELOOK_SYSTEM__` | `TPacketGCItemSet.transmutation` | `packet.h:1436-1438`; `char_item.cpp:579-581` | **defined** (`:16`) | item record −4 |
| `__BL_ENABLE_PICKUP_ITEM_EFFECT__` | `pack.highlight` source | `char_item.cpp:585-589` | **defined** (`:23`) | `highlight` becomes `(Cell.window_type == DRAGON_SOUL_INVENTORY)` |
| `LOCALE_STRING_RENEWAL` | `TPacketGCWhisper.bCanFormat` | `packet.h:998-1001` | **defined** (`:32`) | not on this path |
| `__WORLD_BOSS_EVENT__` | first `BroadcastEventFlagOnLogin` chat | `questmanager.cpp:1704-1709` | **defined** (`:201`) | one fewer conditional `GC_CHAT` |
| `ENABLE_SWITCHBOT` | `TPacketGCItemSet` unchanged; `ItemLoad` belt handling | `char_item.cpp:534-561`; `input_db.cpp:1513-1515` | **defined** (`:191`) | not a wire change to record 21 |
| `__EXTENDED_SAFEBOX__` | `SAFEBOX_MAX_PAGE_COUNT` 6, `QuerySafeboxSize` pages | `length.h:91-94`; `input_db.cpp:1146-1150` | **defined** (`:61`) | no loading-phase send either way |
| `FIX_SELECT_EMPIRE_PHASE` | select-screen order: `GC_EMPIRE`, then `SendLoginSuccessPacket`, then `SetPhase(PHASE_SELECT)` | `input_db.cpp:160-199` | **defined** (`:88`) | order becomes `SetPhase` first, then login success |
| `__7AND8TH_SKILLS__` | `SetSkillLevel` master caps | `char_skill.cpp:195-201` | **defined** (`:165`) | not on the send path |
| `_IMPROVED_PACKET_ENCRYPTION_` | whole TEA branch in `SetPhase` and `Packet` | `desc.cpp:433, 520, 528, 535` | **NOT defined anywhere in the tree** | would activate the DH2 path and make `KEY_AGREEMENT` live |

Note on `ENABLE_GOHOME_IF_MAP_NOT_EXIST`: it is `#define`d on `input_db.cpp:327`, i.e.
**in the .cpp, not in `prodomodefines.h`**. A whole-tree sweep
(`command grep -rn 'ENABLE_GOHOME_IF_MAP_NOT_EXIST' -a .`) shows exactly one definition
and one use, both on that line pair. It is the only feature switch on the loading path
defined outside the central header, which is easy to miss.

Switches that are **not** on the loading path, checked rather than assumed:
`LOCALE_STRING_RENEWAL` only touches `TPacketGCWhisper` (`packet.h:998`);
`ENABLE_UPDATE_LASTPLAY_REAL_TIME` (`input_login.cpp:599`) and `PRODOMO_HIDE_COSTUME`
(`input_login.cpp:611`) sit in `CInputLogin::Entergame`, i.e. `PHASE_GAME`;
`ENABLE_MOVE_CHANNEL` is in `char.h:999`, `input_db.cpp:2217`/`2431` and
`cmd_general.cpp:3643`; `__SEND_TARGET_INFO__` is in `input_main.cpp:66`,
`char.h:697` and `item_manager.cpp:914`. None of them changes a byte the loading
phase writes.

---

## 7. What is recorded here that is a Defect, a Quirk, or a Divergence candidate

Under the project rule ("a bug an honest client can trigger is a Defect unless the owner
names it a Quirk"):

1. **Defect** — `input_login.cpp:278` indexes `c_r.players[pinfo->index]` before the
   `PLAYER_PER_ACCOUNT` check at line 285. Already recorded in `AGENTS.md`. The Rewrite
   must bound-check; that is a Divergence, not parity.
2. **Defect** — `char.cpp:2036` leaves `points[0]` and `points[2]` uninitialised in
   every `GC_CHARACTER_POINTS` record. 16 bytes of stack garbage per record, twice
   during loading. The Rewrite must zero the array.
3. **Quirk candidate** — `GC_ENTITY` at loading is a list of every PC in the process,
   with no map filter, in `unordered_set` order. Not a bug an honest client triggers;
   it is a design the client evidently tolerates. But the order genuinely is not
   reproducible, so the Rewrite needs a deterministic order and a note.
4. **Divergence candidate** — closing an unknown frame is already recorded (ledger
   160.5); the loading phase has no separate entry.
5. **Not a Defect, but worth noting** — `HEADER_DG_PLAYER_LOAD_FAILED`
   (`input_db.cpp:1975`) sends nothing, so a refused load leaves the client on the
   select screen with no feedback. The Rewrite should pick a behaviour and record the
   difference.

## 8. Files consulted

`server/server/game/input_login.cpp`, `input_db.cpp`, `input.cpp`, `input.h`,
`packet.h`, `packet_info.cpp`, `char.cpp`, `char.h`, `char_skill.cpp`,
`char_quickslot.cpp`, `char_item.cpp`, `char_manager.cpp`, `sectree_manager.cpp`,
`sectree_manager.h`, `desc.cpp`, `desc_manager.cpp`, `desc_manager.h`,
`desc.h`, `desc_manager.h`, `building.cpp`, `building.h`, `config.cpp`,
`questmanager.cpp`, `questlua_global.cpp`, `db.cpp`, `ClientPackageCryptInfo.cpp`,
`ClientPackageCryptInfo.h`, `packet.h`; `server/server/common/prodomodefines.h`,
`length.h`, `tables.h`, `item_length.h`; `server/server/libthecore/typedef.h`;
`server/server/db/ClientManagerPlayer.cpp`, `ClientManager.cpp`.

Probe: `.scratch/ledger187/widths.cpp` (created for this research; it is the only file
this work created, and it contains no repository code).
