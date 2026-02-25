# DB SERVER KNOWLEDGE BASE

## OVERVIEW
Database server subsystem; manages persistence, player data, and cross-server sync.

## WHERE TO LOOK
| Task | Location | Notes |
|------|----------|-------|
| Server entry | `Main.cpp` / `Main.h` | DB server bootstrap |
| Client manager | `ClientManager*.cpp` | player/account data flows |
| DB core | `DBManager*`, `Cache*` | DB operations + caching |
| Guild/party | `GuildManager*`, `Marriage*` | social data handling |
| Networking | `Peer*`, `NetBase*` | DB peer connections |
| Utils | `CsvReader*`, `Config*` | config + data parsing |
| Flags/privs | `PrivManager*`, `ItemAwardManager*` | privilege + award data |
| Shared definitions | `QID.h`, `grid.*` | quest IDs + grid helpers |

## CONVENTIONS
- PCH via `stdafx.h/.cpp`.
- Build config via `premake5.lua` and `Makefile`.

## UNIQUE STYLES
- Entry file is `Main.cpp` (capitalized).
