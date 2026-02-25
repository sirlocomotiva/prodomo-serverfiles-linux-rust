# GAME SERVER KNOWLEDGE BASE

## OVERVIEW
Primary gameplay server; handles character state, quests, combat, items, and networking.

## WHERE TO LOOK
| Task | Location | Notes |
|------|----------|-------|
| Server entry | `main.cpp` | game server bootstrap |
| Character logic | `char_*.cpp` | combat, items, skills, state |
| Input handlers | `input_*.cpp` | login, main, db, p2p, auth |
| Quest bindings | `questlua_*.cpp` | Lua quest APIs |
| Items | `item*.cpp` | item managers, refine, attributes |
| Networking | `desc*`, `p2p*`, `packet*` | connections + packet flow |
| Systems | `battle*`, `dungeon*`, `guild*`, `party*`, `shop*` | gameplay subsystems |
| Locale/text | `locale*`, `locale_service*` | locale tables + text conversion |
| Data files | `*.inc`, `*.py` | build locale strings and data |

## CONVENTIONS
- PCH via `stdafx.h/.cpp`.
- Build config via `premake5.lua` and `Makefile` (gmake).

## ANTI-PATTERNS
- `char_battle.cpp`: do not delete `m_dwKillerPID = 0` reset line.
- `text_file_loader.cpp` and `group_text_parse_tree.cpp`: group names must not contain spaces.

## UNIQUE STYLES
- Uses `premake5.lua` to define project links and platform-specific libs.
- `minilzo.c` and `lzodefs.h` are embedded compression sources (treat as third-party).
