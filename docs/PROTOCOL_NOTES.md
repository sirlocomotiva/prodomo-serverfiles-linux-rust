# Protocol and legacy-source notes

> Moved out of `AGENTS.md` and `README.md` on 2026-09-26 so that `AGENTS.md` can stay a short
> rule sheet. The text is verbatim except where it is marked **Corrected 2026-09-26** or
> **Superseded**, and except that a few line counts that go stale on every edit were dropped.
> Paragraphs taken from `README.md` are marked *(From README.md.)*. The authoritative status is
> `docs/REWRITE_LEDGER.md`; the current gaps and the plan are in `docs/STATUS.md`; the rules an
> agent must follow are in `AGENTS.md`.

## Contents

1. [Module map](#module-map)
2. [Coverage history (sections 162-168)](#coverage-history-sections-162-168)
3. [Verification lessons](#verification-lessons)
4. [Game-to-client module notes](#game-to-client-module-notes)
5. [Shared record types](#shared-record-types)
6. [Client-to-game record rules](#client-to-game-record-rules)
7. [Game-to-client login-flow records](#game-to-client-login-flow-records)
8. [Cross-direction header collisions](#cross-direction-header-collisions)
9. [DB records and boot](#db-records-and-boot)
10. [Legacy constants](#legacy-constants)
11. [Width probe and test values](#width-probe-and-test-values)
12. [Section 162 mutation evidence](#section-162-mutation-evidence)
13. [Module index](#module-index)

## Module map

### `protocol/`

Legacy client, game-to-client, game-to-game, and DB packet definitions/codecs.

- `cg_inventory`/`cg_wire` cover fixed client framing.
- `cg_account` covers the exact fixed login, login-by-key, character-select, character-delete, character-create, and enter-game records, including the ten-byte `TPacketCGPlayerDelete` request with one opaque index and eight raw private-code bytes and the 34-byte `TPacketCGPlayerCreate` request with one opaque index, the raw 25-byte name field, an explicit little-endian job word, and opaque shape and stat bytes.
- `cg_attack` covers the exact eight-byte `TPacketCGAttack` request with one opaque type byte, an explicit little-endian target `u32` VID, and two raw magic-cube CRC piece bytes.
- `cg_use_skill` covers the exact nine-byte `TPacketCGUseSkill` request with two explicit little-endian `u32` values, an opaque skill number and an opaque target VID.
- `cg_item_move` covers the exact nine-byte `TPacketCGItemMove` request with two reusable packed 3-byte positions, each an opaque window byte plus an explicit little-endian cell word, and an explicit little-endian count word.
- `cg_item_use` covers the exact four-byte `TPacketCGItemUse` request, a header plus one reusable packed 3-byte position, with no destination field and no stored header byte.
- `cg_item_use_to_item` covers the exact seven-byte `TPacketCGItemUseToItem` request, a header plus two reusable packed 3-byte positions, reusing the same position type.
- `cg_safebox` covers the three safebox and mall item-transfer records sharing one 8-byte active-profile layout: header 69 `HEADER_CG_MALL_CHECKOUT` (0x45) and header 71 `HEADER_CG_SAFEBOX_CHECKOUT` (0x47) both use `TPacketCGSafeboxCheckout`, and header 70 `HEADER_CG_SAFEBOX_CHECKIN` (0x46) uses `TPacketCGSafeboxCheckin`, the only one of the three that moves an item INTO a container. Layout is `[header][container_pos little-endian u32][window_type][cell little-endian u16]`, a 7-byte framed payload, reusing the shared `CgItemPos`. Like the inbound handshake record, which also stores its header because it accepts two values, this record stores its header; it is the second such record and the first with three legal header values, and the stored `SafeBoxKind` enum is closed, so no unencodable state exists. The width is 8 bytes with `__EXTENDED_SAFEBOX__` and 5 without it, but that is a compile-time constant with one `#define` per tree and no `#undef` or build override, so unlike the DB boot feature profile it cannot be selected at run time and the 5-byte shape is unreachable; the codec implements only the 8-byte shape. The `u32` is an opaque flat container cell index and `window_type` is opaque, because the legacy server never clamps the index and never range-checks the window. It reproduces no gameplay policy, no container capacity or grid geometry, and no caller defect.
- `cg_item_destroy` covers header 21 `HEADER_CG_ITEM_DESTROY` (0x15), the item-destroy request, which is a header plus one reusable packed 3-byte position, so 4 bytes and a 3-byte framed payload with no stored header byte. It is a SEPARATE record type even though `TPacketCGItemUse` at packet.h:599-603 is byte-for-byte the same 4-byte shape and is already implemented as `cg_item_use`: two types make passing a header-21 record to a header-11 call site a compile error, whereas one type with a `kind` field would let it compile and fail on the wire. The window byte is read and range-checked by `CHARACTER::DestroyItem` but stays opaque here, and it must: the stock client wrapper declares `TItemPos Cell;` and assigns only `Cell.cell`, so the client constructor default makes the window byte permanently 1, and the server's `GetItem` returns `INVENTORY` and `EQUIPMENT` from the SAME array with the SAME bound. Header 21 collides across directions with an inbound game-to-client record that the server calls `HEADER_GC_ITEM_SET` (packet.h:122) but the client calls `HEADER_GC_ITEM_SET2` (client Packet.h:117), so name-based pairing across the two trees would match the wrong records; `char_item.cpp:7439` also dereferences an item freed at `:7438`.
- `cg_safebox_move` covers header 77 `HEADER_CG_SAFEBOX_ITEM_MOVE` (0x4d), the safebox and mall in-container move, which reuses `sizeof(TPacketCGItemMove)` and so shares the exact nine-byte layout of `cg_item_move` while remaining a SEPARATE record type: the server's `CInputMain::SafeboxItemMove` handler reads only the two cell words and never reads either window byte, whereas the header-13 `CInputMain::ItemMove` handler passes both whole `TItemPos` values so the window is resolved, and the two trees do not even agree on the field names (`Cell`/`CellTo`/`count` against `pos`/`change_pos`/`num`). Because header 77 is a single encodable value it stores no header byte, matching the rule that a stored header is only needed when more than one value must round-trip. The 16-bit count is preserved at full width and is never truncated, which is the codec-level counterpart of the confirmed legacy defect at `input_main.cpp:2472`, where that `WORD` is narrowed into the `BYTE` count parameter of `CSafebox::MoveItem` and a transmitted 256 therefore becomes 0, which `safebox.cpp:211-212` then reads as "move the whole stack".
- `cg_item_give` covers the exact nine-byte `TPacketCGGiveItem` request with an explicit little-endian target `u32` VID, one reusable packed 3-byte position, and an opaque count byte that the legacy server never reads, so the five Rust item records share one packed position layout.
- `cg_variable` covers source-verified chat, whisper, SyncPosition, active-profile MyShop, FishEvent, Shop, and Messenger variable prefixes plus the typed SyncPosition element slice.
- `cg_party_skill` covers the exact six-byte `TPacketCGPartyUseSkill` record.
- `cg_party` covers the five fixed-width party records of headers 72, 73, 74, 75, and 78, with explicit little-endian words, one shared error type, and separately named record and frame-payload widths so a wrong-width frame is rejected rather than misread; `cg_quickslot_add`, `cg_quickslot_del`, and `cg_quickslot_swap` cover the three quickslot records of headers 16, 17, and 18, where the add record is a counted run and the del/swap pair differ only in their declared slot byte.
- `cg_pick` covers the four fixed-width game-phase records that carry a single client choice and nothing else, at headers 31, 61, 66, and 114, where `CgQuestConfirm` is the only one with two fields and the only one whose `u8` precedes its `u32`, and where the three single-word records share one width and are therefore separated by their header rather than their length.
- `cg_login` covers the three CG records the handshake and login phases dispatch, at headers 206, 90, and 254, including two header-only records (`CgStateChecker`, header 206, and `CgPong`, header 254) and the one-byte `empire` selection of `CgEmpire`, header 90, which is range-checked by legacy session policy and so stays opaque across all 256 values; `CgStateChecker` is dispatched by the handshake phase's `else if` chain at `input.cpp:227` rather than a `case`, and `CgPong` is the only CG record dispatched in every client phase (handshake, auth, login, and game).
- `cg_micro` covers the four tiny game-phase records of headers 28, 29, 65, and 82, including the first zero-payload record (`CgWarp`, header 65) and the opaque single bytes of Position, ScriptAnswer, and Fishing.
- `cg_mark` covers the three fixed login-phase guild mark records of headers 100, 104, and 113, including the second header-only record (`CgMarkIdxList`, header 104) and the three client-asserted opaque words of the symbol CRC, whose server and client enumerators are named differently for the same value 113.
- `cg_name` covers the three fixed-string-buffer records (`SPacketCGChangeName` 106, `SPacketCGWhisperDetails` 239, `SPacketCGInventoryProtected` 144) with raw `char[N+1]` storage and no NUL-aware accessor.
- `cg_sash` covers `TPacketSash` 230 together with the 3-byte `ItemPos`.
- `cg_client_version` covers the exact 67-byte client-version report with source header variants `0xfd` and `0xf1`.
- `cg_shoot` covers the exact two-byte `TPacketCGShoot` record without interpreting its opaque type byte.
- `cg_move` covers the exact 16-byte `TPacketCGMove` record with explicit little-endian signed coordinates and opaque function/argument/rotation/time bytes.
- `gc_inventory` is the machine-readable game-to-client table of all 134 records the client registers for decoding, pairing each client name with the server enumerator name for the same wire byte, and records framing plus implementation status.
- `gg_inventory` is the machine-readable game-to-game table of all 36 legacy registrations with hand-verified packed widths and record shape.
- `gg` covers the pure four-byte `TPacketGGSetup` record, the pure nine-byte `TPacketGGFindPosition` record, the pure thirteen-byte `TPacketGGWarpCharacter` record with explicit little-endian fields and signed coordinates, and the pure thirteen-byte `TPacketGGGuildWarMapIndex` record with header 15, two little-endian `u32` guild IDs, and a signed `i32` map index.
- `gc_small` covers the 18 fixed game-to-client records of 1-4 bytes and `gc_vid` covers the 20 records of 5-7 bytes, both keyed by wire byte.
- `gc_fields` covers the 18 game-to-client records with named scalar fields and raw fixed `char` arrays, through 14 structural types, where `GcNamed` backs the 30-byte `ITEM_OWNERSHIP` (31), `PARTY_ADD` (78), and `CHANGE_NAME` (107) records and `GcTwoWord` backs the 9-byte `OWNERSHIP` (62), `PARTY_LINK` (91), and `PARTY_UNLINK` (92) records, with the header byte as the only thing separating the members of each group; all 18 widths there are machine-derived by compiling the verbatim packed legacy bodies with `i686-linux-gnu-g++-12 -static` under `#pragma pack(1)`, which is required because `gcc -m32` has no installation candidate on this host and a non-static 32-bit link compiles but cannot run without `/lib/ld-linux.so.2`; that probe runs two controls every time, re-measuring five already-settled widths and a packed two-`long` struct that must be 8 bytes, and the `long`-dependent records were only settled by that control (`TPacketGCQuestConfirm` is 74 not 78, `TPacketGCTargetUpdate` is 13 not 21 and writes `long lX, lY;` on one line, `TPacketGCPoints` is 2041, `TPacketGCGold` is 9, and `QWORD` is `unsigned long` at 4 bytes despite the name).
- `gc` covers the pure fixed seven-byte `TPacketGCBindUDP` record with opaque raw x86 `sockaddr_in` field words and explicit little-endian field conversion, the transport-free one-byte `GcTimeSync` acknowledgement (`0xfc`) with exact length/header validation, the transport-free fixed six-byte `GcAuthSuccess` record (`0x96`) with an opaque little-endian `u32` login key and raw result byte, the transport-free fixed five-byte `GcLoginKey` record (`0x76`) with an opaque little-endian `u32` login key, and the transport-free fixed ten-byte `GcLoginFailure` record (`0x07`) with raw status bytes through the first NUL and fail-closed missing-NUL decoding, and the transport-free active-profile 357-byte `GcLoginSuccess` record (`0x20`) with four 70-byte player summaries, four little-endian guild IDs, four raw 13-byte guild-name fields, and trailing handle/random-key words, the transport-free active-profile 72-byte `GcPlayerCreateSuccess` record (`0x08`) with an opaque account-character index and the shared 70-byte player summary, the transport-free declared two-byte `GcCreateFailure` record (`0x09`) with an opaque type byte, the transport-free two-byte `GcPlayerDeleteSuccess` record (`0x0a`) with an opaque account index, and the transport-free header-only one-byte `GcPlayerDeleteWrongSocialId` record (`0x0b`).
- The twelve small-and-wide client records of Section 162 are covered by `cg_header_only` (`CgText`, `CgEnvanterBlack`), `cg_vid` (`CgOnClick`, `CgTargetInfoLoad`), `cg_fly_target` (one 13-byte body under `HEADER_CG_FLY_TARGETING` and `HEADER_CG_ADD_FLY_TARGETING`), `cg_arg_u8` (seven opaque one- and two-byte argument records, all 256 values round-tripping, with `CG_REFINE_ELEMENT_CLOSE = 255` exported as a semantic constant and never as a validation rule), `cg_refine` (the 3-byte `CgRefine` and the 6-byte `Attr67AddData` inside an 8-byte `CgAttr67Add`), `cg_cube_renewal`, `cg_guild_answer` (a raw 13-byte guild name with no NUL requirement), `cg_exchange` (the active 14-byte `u64` profile, with the inactive 10-byte `DWORD` width recorded as `CG_EXCHANGE_DWORD_PROFILE_SIZE`), `cg_hack` (header plus a raw `[u8; 256]`), `cg_login3` (the server's 66-byte profile, rejecting the client's 69-byte shape as a named `CgLogin3Error::ClientWidth`), `cg_change_look` (10 bytes, reusing the shared `ItemPos`), and `cg_gaya_system` (6 bytes, with the header-241 first-wins collision against `TPacketCGClientVersion2` exposed as `CG_GAYA_SYSTEM_SHADOWED_BY`).
- `gc_actors` covers the 14 fixed-size actor, target, and main-character game-to-client records, keyed by wire byte, with machine-derived packed widths, canonical non-caller-mutable headers, raw preserved name arrays, and no stored header field.
- `tea` covers the pure 8-byte/16-byte legacy block codec with explicit little-endian words, aligned slice helpers, owned zero-padding with logical/wire lengths, and a bounded incremental ciphertext-tail decoder.
- `db_wire`/`db_records` cover the tested DB peer subset, including the 67-byte login-by-key request, 31-byte already-logged-in record, nine-byte `TPacketGDRemoveAffect` request, exact 25-byte `TPacketGDAddAffect` request, the variable-size `HEADER_DG_AFFECT_LOAD` response payload with an exact 8-byte player/count prefix followed by 21-byte affect elements, source-fixed market-price, event-table, refine-table, and horse-name records.
- `db_boot` covers the 24-byte boot request, version-6 response parser/encoder, caller-supplied response frame, and typed banword/refine/event/market section boundaries.
- `protocol/src/gc_nested.rs`: the twelve fixed-size game-to-client records that embed a nested
  body or a fixed-length array (bytes 17, 65, 69, 71, 76, 79, 126, 134, 135, 141, 180, 221) plus
  the three public nested bodies `GcSkill` (6), `GcAffectElement` (21), and `GcCubeRenewalDate`
  (169). Every width is machine-derived from a verbatim packed i686 compile with positive controls.
  The module is a pure protocol boundary: it does not send, route, authorize, or mutate session
  state.

Protocol modules that the old `AGENTS.md` text never named (first line of each module's `//!` doc,
added 2026-09-26):

- `cg_dragon_soul`: Transport-free codec for the client-to-server dragon-soul refine request.
- `cg_handshake`: Explicit client-to-game handshake and time-sync record codec.
- `cg_item_drop`: Explicit codec for the fixed legacy `TPacketCGItemDrop` record.
- `cg_item_drop2`: Explicit codec for the fixed legacy `TPacketCGItemDrop2` record.
- `cg_item_pickup`: Explicit codec for the fixed legacy `TPacketCGItemPickup` record.
- `cg_quest_text`: Transport-free codecs for the two 66-byte client quest-text requests.
- `db_market_price`: Source-fixed premium-market item-price update codec.
- `item_pos`: The legacy `TItemPos`, shared by every record that embeds one.

### Other crates

- `common/`: shared config, logging, enums, constants, and legacy table/domain types.
- `net/`: Tokio transport helpers. The generic length-prefixed `ReadBuffer` is not the legacy client framing; tested DB and fixed-client transport boundaries feed conservative read-only accept loops.
- `db/`: SQLx/MySQL pool and database foundation, including retry-preserving bounded query streaming.
- `world/`: spatial model, characters, events, and bounded combat/monster rules.
- `quest/`: quest/Lua scaffold; not a working legacy quest runtime.
- `game-server/`: game process scaffold; fixed keepalive/pong boundaries, pure source-verified SyncPosition, handshake, and heartbeat/PONG reducers, raw GC effect projections, a transport-free `ClientLifecycle` adapter, the transport-free `AccountPlayerSession` reducer, and a source-derived transport-free `DescriptorCrypto` adapter are present. The account/player reducer models one descriptor's pending login/player requests, seeded correlations, normalized key material, account/player bindings, source-ordered effects, explicit key-installation policy and retry helpers, C-string status handling, and explicit `EnterGame` phase-operation errors without SQL, live packets, key installation, world construction, or gameplay. Its local `Login` phase is not a lifecycle-phase check: the adapter must first observe `ClientLifecycle::phase() == ClientPhase::Login` (not Auth, Handshake, or Close) and run admission checks before `on_login`. The lifecycle adapter models handshake-to-Login/Auth, explicit post-handshake Select/Loading/Game/Dead transitions, old/new encryption-boundary metadata, and terminal close behavior. Close effects are control-only because the legacy descriptor suppresses packets after entering `PHASE_CLOSE`. `protocol::cg_variable` remains a pure framing boundary. The descriptor adapter derives the source Myevan/client keys, retains aligned-input tails, and pads output units, but it is not a socket or phase analyzer. Descriptor/session integration, variable-session dispatch, transactional lifecycle coupling, and live gameplay dispatch remain incomplete.
- `db-server/`: DB process scaffold; typed peer frames, boot requests, the source-fixed horse-name and login-by-key request boundaries, the validation-only `HEADER_GD_REMOVE_AFFECT` and `HEADER_GD_ADD_AFFECT` request classifications, the public strict active-x86 `HEADER_GD_SETUP` decoder and typed `DbRequest::Setup` validation, pure primary login/player response adapters, the exact channel-lookup request/result exchange, SQL-free allowlisted `TABLE_POSTFIX` refine-query and event-query boundaries, bounded SQLx banword/player-index/quest/event acquisition adapters, caller-resolved `BootSnapshot` composition, source-verified metadata codecs, the transport-free fail-closed DB-peer identity/authorization policy, and the transport-free injected-time/persistence cache core are validated. `BootResponseAdapter` can encode/write a caller-supplied immutable payload with an explicit profile and limit, and the SQL-free `DbRequestService` can dispatch injected horse-name and optional channel-resolution seams. Validated setup, login, and player paths remain `NotReady` or unintegrated until SQL, cache, peer-registry, authentication, and state dependencies exist. The cache core is not wired into manager maps or shutdown. Query routing, live SQL table loading, live peer registries, production boot response service, cache persistence integration, and complete dispatch remain incomplete.
- `tools/`: auxiliary tools that are not automatically part of the locked workspace test run.

The complete per-file list is in the [module index](#module-index) at the end of this file.

## Coverage history (sections 162-168)

Section 168 closes the actor, target, and main-character game-to-client records. It adds
`protocol/src/gc_actors.rs` with 14 fixed-size codecs, brings
`protocol/src/gc_inventory.rs` to **96 of 134 implemented** game-to-client rows, and leaves 38.
Three of the 14 are client decode entries the checked-in server never sends (bytes 15, 72, and
117); the other 11 are live pairs whose client and server structs agree on every field offset. The
client and server header enumerations are not name-compatible, and reading either side's name as
if it belonged to the other selects the wrong record. Every width is machine-derived from an
`i686` compile of the struct under the active gate set, never hand-summed.

Section 162 closes the client-to-server codec inventory. Twelve new modules implement the twenty
records left pending by section 161, which brings `protocol/src/cg_inventory.rs` to **91 of 92
implemented**. The remaining row is not pending work: `HEADER_CG_KEY_AGREEMENT = 0xfb` has both its
`Set()` call (`server/server/game/packet_info.cpp:190-192`) and its struct
(`server/server/game/packet.h:2721-2737`) behind `#ifdef _IMPROVED_PACKET_ENCRYPTION_`, and the
only `#define` of that macro in the entire `server/` and `client/` tree is
`client/Client/EterBase/ServiceDefs.h:4`, where it is commented out. **The CG inventory is complete
for the active build.**

The section also corrects a broken metric. The old CG count matched header names as *substrings*,
which reported `HEADER_CG_CL` as implemented purely because `HEADER_CG_CLIENT_VERSION` contains
that text, and which hid a real `HEADER_CG_GAYA_SYSTEM` gap. Match whole tokens instead:
`\bHEADER_CG_X\b` over comment-stripped `protocol/src/cg_*.rs` excluding `cg_inventory.rs`.
`HEADER_CG_ITEM_SELL` and `HEADER_CG_DUNGEON` are declared constants with no `Set()` row in
`CPacketInfoCG` and are outside the backlog entirely.

Section 162 records three legacy defects that per-record auditing cannot see. `LOGIN3` is 66 bytes
on the server and 69 on the client, because the language field is `BYTE bLanguage`
(`packet.h:486-488`) versus `DWORD bLanguage` (`client/.../Packet.h:442-444`); the client writes
three surplus bytes that the framing layer reads as the next header, so the codec models the
server's 66 bytes and rejects the 69-byte form as a named `CgLogin3Error::ClientWidth` rather than
silently accepting a shape the legacy server cannot speak. `GAYA_SYSTEM` is worse: header 241 is
`HEADER_CG_GAYA_SYSTEM` in one enum line and `HEADER_CG_CLIENT_VERSION2 = 0xf1` in another, and
`CPacketInfo::Set` is first-wins (`packet_info.cpp:18-21`), so framing consumes
`sizeof(TPacketCGClientVersion2)` = 67 bytes while the live `CInputMain::GayaSystemSend` handler
reads 6. Header 111 is also `HEADER_CG_LOGIN3` inbound and the live 6-byte `HEADER_GC_WALK_MODE`
outbound; the two directions use separate tables, so this is not a collision, but a
direction-blind header audit will report it as one.

> **Superseded by section 168.** The table and the "82 of 134" bullet below are the
> section 163-167 baseline. After section 168 added `gc_actors` (14 fixed-size records),
> game-to-client coverage is **96 of 134**, the 38 missing rows split into 19 fixed-size and 19
> dynamic-size records, and **71** registered records have no codec in total (1 dead CG, 32 GG,
> 38 GC). `protocol/src/gc_inventory.rs` asserts 96.

Sections 163 through 167 establish the honest baseline and correct three method defects.
**The protocol layer is not complete: 95 records still have no codec.**

| direction | registered | implemented | missing |
|---|---|---|---|
| client to game | 92 | 91 | 1 (dead, `_IMPROVED_PACKET_ENCRYPTION_`) |
| game to game | 36 | 4 | 32 |
| game to client | 134 | 82 | 52 |

- The game-to-client registration table is the **client's**, in
  `client/Client/UserInterface/PythonNetworkStream.cpp` (134 `CNetworkPacketHeaderMap::Set` calls,
  115 static and 19 dynamic). `server/server/game/packet_info.cpp` registers only inbound records
  and must never be used to measure game-to-client coverage.
- Game-to-client coverage is **82 of 134**, reached by `gc_small` (18 records of 1-4 bytes),
  `gc_vid` (20 records of 5-7 bytes), `gc_fields` (18 records with named fields), and `gc_nested`
  (12 records that embed a nested body or a fixed-length array), per sections 164, 165, 166, and
  167. The 52 remaining rows split into 33 fixed-size and 19 dynamic-size records. Two wrong answers came first, from the same lesson: a
  substring metric over-counts `HEADER_GC_CHARACTER_DEL` and `HEADER_GC_TIME` by matching inside
  longer names, and a client-name-keyed whole-token metric under-counts because three implemented
  records are coded under their **server** name (byte 6, 32, and 252 are `LOGIN_SUCCESS3`/
  `LOGIN_SUCCESS4`/`HANDSHAKE_OK` on the client but `LOGIN_SUCCESS`/
  `LOGIN_SUCCESS_NEWSLOT`/`TIME_SYNC` on the server). **Key every coverage and implementation
  metric on the wire byte, never on an identifier name.** `gc_inventory` now stores the flag per
  byte and has a regression test that fails if a renamed row is reverted.

## Verification lessons

- **A name-keyed enum diff across the two trees is unsound.** The four `HEADER_GC_*` names whose
  values differ between server and client (`SKILL_LEVEL`, `MAIN_CHARACTER`, `REFINE_INFORMATION`,
  `ITEM_SET`) are the same record generations under opposite naming conventions: the server suffixes
  the legacy generation with `_OLD`, the client suffixes the modern one with `_NEW`. Byte values and
  packed layouts are identical when paired correctly. Pair header tables **by byte value, never by
  name**, and check every value in both trees. Value-keyed pairing resolves 20 name differences
  across the 134 rows; 5 client rows have no server enumerator at all.
- The client's `CNetworkPacketHeaderMap::Set` is **last-wins**; the game server's `CPacketInfo::Set`
  is **first-wins**. The policies only diverge if a byte is registered twice, which does not occur
  in the client table.
- The **DB server consumes no GG record**: `server/server/db` has no `HEADER_GG_` enumerator, no
  `TPacketGG` type, and does not include the game packet header. The Rust `db-server` needs no GG
  decoder. GG framing is `[u8 header][body]` with no length prefix and no handle, which is a
  different socket and shape from the DB peer's `[u8 header][u32 handle][u32 size]`; never share
  the two tables. No `HEADER_GG_` token appears anywhere in the client tree.
- Of the 32 missing GG records, 20 are fixed-width scalar-only, 3 embed a nested struct, and 5 are
  variable-length prefix-plus-tail (for those the registered size is a minimum, not a frame
  length). `HEADER_GG_GUILD` is the hardest: a sub-header switch with a 519-byte nested record, a
  bare 4-byte `int`, and a zero-length default. `HEADER_GG_PRIVATE_SHOP_ITEM_SEARCH_RESULT` uses a
  **16-bit** length word where the other variable records use 32-bit.
- **A self-consistent codec with a wrong layout passes every self-consistent test.** Section 167
  implemented `GcCubeRenewalDate` and measured it at the correct 169 bytes, yet the layout was
  wrong. `TInfoDateCubeRenewal` declares `vnum_material_N; count_material_N;` **in pairs**, so
  material `i` sits at byte `17 + 8 * i` with its count at `21 + 8 * i`, not as two adjacent
  blocks. The decoder and the encoder were wrong in *mirrored* ways, so width checks, published
  constants, and mirror-image round trips all passed. Only golden bytes caught it, and
  `the_cube_material_pairs_are_interleaved_on_the_wire` now pins the exact layout including that
  byte 21 holds `count_material[0]` rather than `vnum_material[1]`. **A round trip is not an
  independent witness: it only proves the encoder and decoder agree, not that either matches the
  source.** Publish a source-order claim, or golden bytes, for every record whose fields are not
  simply the obvious sequence.
- **A `#define`-only constant search reports a false absence.** All twelve Section 167 headers are
  multiline anonymous-enum members (`HEADER_GC_WARP                                        = 65,`),
  not `#define`s. Search all four forms: `#define NAME value`, `const T NAME = value`, a
  multiline enum member, and a single-line `enum { NAME = value }`, with a positive control for
  each.
  **Corrected 2026-09-26:** the four forms above are not enough. Two more enum-member forms defeat a search
  for a literal `NAME = n`: an **expression-valued** member such as
  `ITEM_ATTRIBUTE_MAX_NUM = ITEM_ATTRIBUTE_RARE_END` (`server/server/common/item_length.h:30`),
  and an **auto-increment** member with no `=` at all, such as `CHR_EQUIPPART_NUM,`
  (`server/server/game/packet.h:881`), whose value depends on which gated members precede it.
  Resolve both by evaluating the enum under the active gate set, not by pattern matching.
- **The client and server name the same wire byte differently, and the pairing is not
  mechanical.** Byte 17 is `HEADER_GC_PLAYER_POINT_CHANGE` in the client and
  `HEADER_GC_CHARACTER_POINT_CHANGE` in the server (`server/server/game/packet.h:117`). For
  skill level the *generations* swap: the client's `SKILL_LEVEL_NEW` (76) is the server's plain
  `SKILL_LEVEL` (76), and the client's plain `SKILL_LEVEL` (72) is the server's
  `SKILL_LEVEL_OLD` (72). `gc.rs` publishes `HEADER_GC_CHARACTER_POINT_CHANGE` and
  `HEADER_GC_SKILL_LEVEL_SERVER` as aliases so the trees can be compared by name without a
  false second record appearing. **Pair by byte, never by name, and keep both names in the tree.**
- **`time_t` is 4 bytes on the legacy target, and it is load-bearing.** `TPlayerSkill` is
  `{ BYTE bMasterType; BYTE bLevel; time_t tNextRead; }`, so an entry is **6** bytes, not 10. The
  255-entry live byte-76 table is therefore **1531** bytes, not 2551. A 64-bit mental model
  silently doubles it. Likewise `TPacketGCPointChange::header` is a 4-byte `int`, not a byte, so
  that record is 25 bytes and carries `header: i32` with no single-byte header check.
- **A client-only array bound is not automatically a shared constant.**
  `PARTY_AFFECT_SLOT_MAX_NUM` (7) exists only in `client/.../Packet.h`; the server has no
  definition and writes the literal `7`. `DAILY_GIFT_WEEK_DAYS` (7) is genuinely shared. Record
  which case applies.
- **An equivalent mutant proves only that the tests cannot tell two reinterpreting reads apart.**
  Two Section 167 survivors replaced `i32_at(raw, 12)` with `u32_at(raw, 12) as i32`. Rust defines
  `u32 as i32` as a wrapping two's-complement bit reinterpretation, so those are identical for
  every input; this was confirmed over 200,012 values including all boundaries. **That says
  nothing about a *converting* read**, which is a different defect, so four boundary tests were
  added that walk `i32::MIN`/`i64::MIN` through the signed fields. Seven new `abs`, saturating-fold,
  and wrapping-division mutants were then caught. **When a survivor is an equivalent mutant, ask
  what class of real defect it hides you from, and test that class.**
- **Compile-error kills are weak evidence and should be read, not accepted.**
  `HEADER_GC_SKILL_LEVEL` fails with `E0425` because the module deliberately imports only the live
  constant, and a changed array length fails with `E0308` in the test literals. Both are
  name- and type-level rejections. Report them separately from semantic kills.
- **Never audit this tree with `grep --include=*.h`.** The glob is case-sensitive and the client's
  key feature header is `client/Client/UserInterface/LOCALE_INC.H` with an uppercase extension, so
  the flag silently skips it. This produced a false negative that made Section 162 record the
  `LOGIN3` 69-byte client profile as conditional; it is unconditional, because
  `ENABLE_MULTI_LANGUAGE_SYSTEM` is defined at `LOCALE_INC.H:110` and the client's Visual Studio
  projects set no `ENABLE_*` macro at all. Use both `*.h` and `*.H`, or read files in code with
  `Path.read_text(errors="replace")`. Every negative search needs a positive and a negative control.
- **Do not publish automatically computed packed widths.** A `#pragma pack(1)` width calculator that
  evaluates real `#ifdef` gates and extracts only top-level members matched 3 of 8 hand-summed
  ground-truth widths; every failure was constant resolution, not the field walk
  (`ITEM_ATTRIBUTE_MAX_NUM` is an enum alias resolving to 4 instead of 7, under-counting
  `TPacketGCItemSet` as 63 against a true 72). `protocol/src/gc_inventory.rs` therefore has no
  size column on purpose. **Section 166 supersedes the "hand-summed" half of this rule:** the 18
  `gc_fields` widths are machine-derived, because a hand sum and the true width disagreed on
  records containing a C++ `long`. Prefer the i686 probe described in the compatibility rules;
  keep a hand sum only as the value you are checking the probe against. The `gg_inventory` table
  is still hand-summed and should be re-measured the same way.

## Game-to-client module notes

- `protocol/src/gc_small.rs` implements the 18 smallest fixed-width game-to-client
  records: 1-4 packed bytes with a single opaque `BYTE` body. Three legacy hazards are documented
  there. The **server reuses `CG`-prefixed struct names for outbound records** —
  `TPacketCGSafeboxSize` carries both `HEADER_GC_SAFEBOX_SIZE` and `HEADER_GC_MALL_OPEN`, and
  `TPacketCGSafeboxWrongPassword` carries `HEADER_GC_SAFEBOX_WRONG_PASSWORD`, so a `TPacketGC*`
  search in the server tree misses them. **Field names differ while the wire does not**:
  `TPacketGCQuickSlotSwap` is `{header; pos; change_pos}` on the client and `{header; pos;
  pos_to}` on the server. **Bytes 73, 112, and 213 have no server producer at all** — zero
  whole-token hits in `server/server` and no server enumerator — so `HEADER_GC_SKILL_COOLTIME_END`,
  `HEADER_GC_CHANGE_SKILL_GROUP`, and `HEADER_GC_UNK_213` are real client decode entries with no
  producer; treat them as server reachability questions, not porting work.
- `TPacketGCDragonSoulChangeAttrResult` is `{ BYTE header; bool result; }`. Under `pack(1)` the
  `bool` is one byte, so the body is a raw `u8`, never a Rust `bool`; the module has no `bool`.
- Nine structurally identical 2-byte records share one `GcHeaderAndByte` type with a
  caller-supplied header. The header is the only thing separating them and the body is opaque in
  all nine, so a shared shape is honest and nine near-duplicate Rust structs would imply nine
  legacy structs that do not exist.
- `protocol/src/gc_vid.rs` implements 20 game-to-client records of 5 to 7 packed bytes. Three
  legacy field widths depend on the 32-bit target set by
  `server/server/premake5.lua:12` (`architecture "x86"`): C++ `long` is 4 bytes, `time_t` is 4
  bytes with no `_TIME_BITS=64` anywhere, and a C++ `bool` is 1 byte. A 64-bit reading of any of
  them silently breaks framing, so cite the `x86` setting rather than assume it.
- `protocol/src/gc_fields.rs` (39 unit tests, plus seven public wiring tests) implements
  the 18 game-to-client records that carry named scalar fields and raw fixed `char` arrays, through
  14 types: `GcNamed` for the 30-byte `ITEM_OWNERSHIP`/`PARTY_ADD`/`CHANGE_NAME` records, and
  `GcTwoWord` for the 9-byte `OWNERSHIP`/`PARTY_LINK`/`PARTY_UNLINK` records, with the header byte
  as the only difference inside each group. The other twelve records are `PLAYER_POINTS` (2041),
  `PVP` (10), `AUTO_SHAMAN_SKILL` (10), `PICKUP_ITEM_SC` (9), `CREATE_FLY` (10), `TARGET_UPDATE`
  (13), `LOVER_INFO` (27), `SHOP_SIGN` (38), `QUEST_CONFIRM` (74), `SPECIFIC_EFFECT` (133),
  `CHARACTER_GOLD` (9), and the shared `GcPoints` array. Every width is machine-derived, see the
  compatibility rules. Three legacy facts the module pins: `party.cpp:729` sends the unlink record
  through the link struct with only the header changed; `char.cpp:12745` initialises the shaman
  record positionally, which confirms its field order; and `char.cpp:5045` takes
  `struct packet_motion *`, so the server spells the motion record as a plain struct, not a
  `TPacketGC*` typedef. `GcPoints.points` is `long long` and is `i64` while `GcGold.gold` is
  `unsigned long long` and is `u64`, and one test checks `i64::MIN` against `u64::MAX` so the
  distinction cannot be lost.
- `TPacketGCItemDel` (bytes 86 `HEADER_GC_SAFEBOX_DEL` and 129 `HEADER_GC_MALL_DEL`) is
  `#if defined(__EXTENDED_SAFEBOX__)` a `DWORD pos`, else a `BYTE pos`. The macro **is** defined
  on both sides (`prodomodefines.h:61`, `LOCALE_INC.H:35`), so the active record is 5 bytes. This
  is a direct consequence of the `--include=*.h` defect: the client macro lives in the
  uppercase-extension header, so a `*.h` search reports the record as 2 bytes.
- `HEADER_GC_SAFEBOX_DEL` and `HEADER_GC_MALL_DEL` are one struct sent from one ternary at
  `safebox.cpp:117-122`. The header is the only thing separating them.
- `HEADER_GC_FISHING` (89) is 7 bytes for every sub-header, but `info` has **two** meanings:
  a character VID for `START`/`STOP`/`REACT`/`SUCCESS`/`FAIL`, and a fish item vnum for `FISH`
  (`fishing.cpp:406-428` vs `:501-503`; the client branches at
  `PythonNetworkStreamPhaseGame.cpp:4399-4401`). `gc_vid` keeps `info` raw and exposes
  `actor_vid()` and `fish_vnum()`, each returning `None` for the sub-header where that reading is
  wrong. `GcFishingSubheader` has an explicit `Unknown(u8)` arm because the legacy `switch` has
  no `default`. `dir` is a `u8`, not an `i8`; the client computes `float(dir) * 5.0f`.
- `SPacketGCDragonSoulRefine` (209) is a C++ **class** with a user-supplied constructor, not a
  typedef struct, and it is still 1 + 1 + 3 = 5 bytes. A struct extractor that only understands
  the `} NAME;` typedef form misses it and misses the server's `struct packet_position`; handle
  both `} NAME;` and `struct NAME { ... };`.
- Four records in `gc_vid` are the same length with different field **order**
  (`SEPCIAL_EFFECT` puts the `BYTE` first, reversing the 6-byte shared shape). They keep separate
  types with byte-exact round trips, and the wiring test asserts that reading one frame as another
  type fails on the header, so a future merge cannot pass on length alone.
- **Corrected 2026-09-26:** **eight** game-to-client bytes have no server producer, not five: 18, 73, 84,
  112, and 213, plus 15, 72, and 117 from section 168 (`protocol/src/gc_actors.rs:60-98`).
  The server declares byte 15 as `HEADER_GC_MAIN_CHARACTER_OLD` and byte 72 as
  `HEADER_GC_SKILL_LEVEL_OLD` and never sends either; the live records are byte 113 and byte
  76. Byte 117 has no server enumerator at all. The client registers and decodes all eight, so
  their framing is real, but they are unreachable in the checked-in server and are better read
  as server reachability questions than as porting work.

## Shared record types

Records that share a body keep their header. `CgFlyTarget` is one 13-byte record under
`HEADER_CG_FLY_TARGETING = 0x33` and `HEADER_CG_ADD_FLY_TARGETING = 0x35`; the header is the only
thing separating the two actions, so `new` takes an explicit `CgFlyTargetHeader` with no default.
`TItemPos` now has four consumers (`cg_sash`, `cg_dragon_soul`, `cg_exchange`, `cg_change_look`).
Fixed `char[N]` fields stay raw bytes: the 13-byte guild name, the 256-byte `HACK` buffer, and the
`LOGIN3` 31-byte and 17-byte credentials carry no `&str`, no `CStr`, and no NUL validation.
All 256 `u8` values round-trip through the seven opaque-argument records; handler switches,
sentinels such as `CG_REFINE_ELEMENT_CLOSE = 255`, and range checks are not framing constraints.

- The `cg_name` records carry `char[N+1]` fields as **raw fixed storage with no NUL-aware accessor**. `CgChangeName::name` and `CgWhisperDetails::name` are 25 bytes each (`CHARACTER_NAME_MAX_LEN + 1`), and `CgInventoryProtected::current_password` and `replacement_password` are 7 bytes each (`INVENTORY_PROTECTED_PASSWORD_MAX_LEN + 1`). Every legacy consumer reads these as C strings without bounding the read — `check_name` opens with `strlen`, `CInputMain::WhisperDetails` tests `if (!*name)` and then passes the pointer to `FindPC` and `P2P_MANAGER::Find` — so a client that fills the buffer with non-NUL bytes makes the legacy server read past the record. That is a C++ robustness follow-up, not a codec rule: adding a NUL requirement would reject records the legacy server accepts. Do not add a `&str` accessor, and do not add a `check_name` equivalent to the whisper path, which the legacy handler never had. `CgInventoryProtected::activated` is a C++ `bool`, so it decodes as `!= 0` and every nonzero byte is `true`.
- A `u8` at offset 1 is **not automatically an opaque byte**. `CgInventoryProtected::by_sub_header` and `CgSash::subheader` are named enumerations that their handlers `switch` on immediately, so they are sub-typed; `CgEmpire::empire` is also a `u8` at offset 1 and is *not* switched on, because its handler compares it against `EMPIRE_MAX_NUM` later. The struct cannot tell you which kind you have — read the handler. None of the three is validated at the codec, and all 256 values round-trip, because the legacy `default:` arms discard unknown values silently.
- `TItemPos` is **3 bytes**: `BYTE window_type` then `WORD cell`, the only definition being the `struct SItemPos` in `server/server/common/length.h`. `sizeof(TPacketSash)` is 23 and `1+1+1+4+1+3+4+4+4 = 23`; a 4-byte reading would contradict the registration. `CG_SASH_WIRE_SIZE` is written as arithmetic over `ItemPos::WIRE_SIZE` so the two cannot drift. Do not merge `CgSash::window` with `ItemPos::window_type`: `char.cpp:10422` sets the former from `m_bSashCombination` while `char.cpp:10415-10417` sets the latter to `INVENTORY`, and the two are unrelated decisions.
- `TItemPos` now lives in `protocol::item_pos`, not in any one record. It was defined in `cg_sash`, then `cg_dragon_soul` needed the same type, so the local definition moved out and `cg_sash` re-exports it with `pub use`. `ItemPos::decode` reports `ItemPosError`; `cg_sash` keeps its public error API through a **total** `From<ItemPosError> for CgSashError`. A nested fixed-width field must be handed **its own width**, never the remaining tail: `ItemPos::decode(&bytes[at..at + ItemPos::WIRE_SIZE])`, not `ItemPos::decode(&bytes[at..])`, because `decode` validates an exact 3-byte slice and the tail form is 45 bytes on the first slot. `decode_at` and `decode_frame_at` exist for offset reads that validate nothing, since the enclosing record has already checked its own width.
- The two 66-byte quest text records are **equal by coincidence**. `TPacketCGQuestInputString` declares a literal `char msg[64+1]` and `SPacketCGRequestEventQuest` declares `char szName[QUEST_NAME_MAX_NUM + 1]`, where `QUEST_NAME_MAX_NUM` is 64 at `common/length.h:43`. Nothing in the source ties them together, so change that constant and the two records diverge with no compile error; `cg_quest_text` defines one shared `CG_QUEST_TEXT_FIELD_SIZE` and `the_two_records_share_the_field_width` is the test that will fail when it moves. Their handlers also disagree on safety: `QuestInputString` uses a bounded `strlcpy` into a local `char msg[65]`, while `RequestEventQuest` hands the raw pointer to `quest::CQuestManager::instance()` as a `const char *` with no length bound. Both fields stay `[u8; 65]`; a NUL check is a **legacy C++ finding to record, not a rewrite requirement to enforce**, and a stricter codec would change who can start a quest.
- The dragon-soul sub-header enum is a **naming** divergence, not a wire one, and this is categorically different from `CG_CHANGE_LANGUAGE` where the bytes genuinely differ. `EPacketCGDragonSoulSubHeaderType` has the same name and the same 13 numeric values in both trees, but values 2, 3 and 4 are `DO_REFINE_GRADE`/`DO_REFINE_STEP`/`DO_REFINE_STRENGTH` on the server and `DO_UPGRADE`/`DO_IMPROVEMENT`/`DO_REFINE` on the client. The bytes are identical and only the vocabulary differs, so **no `DS_SUB_HEADER_*` constant is re-exported**; a Rust name would have to pick one tree's vocabulary and hide the divergence. The byte is an opaque `u8`. The server handles 0..=4 and 12; 5..=11 are `REFINE_FAIL`/`REFINE_SUCCEED` values that the inner `switch` silently drops because it has no `default:` arm, so all 256 values are representable and none are policed. `DS_SUB_HEADER_REFINE_ALL` (12) reads only `ItemGrid[0].cell` and `ItemGrid[1].cell`; which slots are load-bearing is session policy.
- `TPacketSash` is **bidirectional** and its sub-header enum is shared. `SASH_SUBHEADER_CG_REFINED` is a **server-to-client** value with a misleading `CG_` prefix: it is 4 because it continues from `SASH_SUBHEADER_GC_REMOVED`, and both trees use it only for `HEADER_GC_SASH`. The CG domain is 0..=3 (`CLOSE`, `ADD`, `REMOVE`, `REFINE`), and `CInputMain::Sash`'s `default: break;` silently drops 4 and above. The explicit `= 0` on `SASH_SUBHEADER_CG_CLOSE` restarts numbering after the GC half and is load-bearing. Only the CG direction is implemented; `HEADER_GC_SASH` belongs to the GC set.

## Client-to-game record rules

- The dead `protocol/src/cg.rs`, which held 91 such `#[repr(C)]` declarations, was deleted in Section 146. It was never registered, never compiled, and every field in it is re-derivable from `packet.h`, which is the designated oracle. The seven record names it was the only Rust-side mention of are listed in that section; four of them, the `TPacketCGPrivateShop*` family, were a genuine remaining gap and were closed in Section 148, but they were **not** a new boundary class: it is a third sub-header-routed two-byte prefix for the framing layer that already exists, structurally identical to the shop and fish-event arms in `cg_variable`. There is no wire-supplied extra length anywhere in it; every variable sub-record computes its length from a compile-time `sizeof`, and only sub-header 0 `BUILD` is count-derived, which is the MyShop rule already implemented. `HEADER_CG_PRIVATE_SHOP = 236` is already in `LEGACY_CG_VARIABLE_HEADERS` and already has a correct `cg_inventory` row with `base_size: 2`. Implemented as a third sub-header-routed two-byte prefix in `cg_variable`: 22 sub-header constants, 21 constant extension sizes, and one count-derived `BUILD` rule reading `wItemCount` little-endian at frame offset 41. **Ten** sub-headers extend by zero, including `STATE_UPDATE` (9), which is enumerated at `packet.h:1146` but has no case arm, and every unrecognised sub-header takes the same `default:` path. Never invent a payload for either. The 22 sub-header values sit at wire offset 1, after the 236 byte at offset 0, so they are a field inside the frame and not a header space: the four-bucket collision taxonomy does **not** apply to them, and 236 itself is collision-free. The server gates the family with `__PREMIUM_PRIVATE_SHOP__` and the client with `ENABLE_PREMIUM_PRIVATE_SHOP`; do not unify the two names. One legacy defect here is serious enough to name: `CInputMain::PrivateShopItemCheckin` is declared `int` but returns `false` at `input_main.cpp:4722`, `:4728`, `:4734`, and `:4740`, so `iExtraLen` becomes 0, only the two base bytes are consumed, and the remaining 17 bytes of the 19-byte sub-record are re-parsed as the next CG frame. Three of those paths are ordinary actions: an Item-Shop, an equipped, or a locked item offered for sale. The Rust framing layer must model the declared 21-byte size and must never mirror that return value. See ledger Section 148 for the full table, the receipt, and the corrections it forced. `tools/packet_compare` is preserved but cannot be built: it requires `clap`, which appears in neither `Cargo.lock` nor the local registry cache, and it is not a workspace member, so its `use protocol::cg::*` is now a dangling import on a crate that never compiled.
- The inbound CG handshake codec is exact 13 bytes total (one header plus 12 payload bytes), accepts only `0xff` and `0xfc`, and is not a live descriptor. The fixed party-use-skill codec is also protocol-only; it does not resolve parties or execute heal/warp behavior. The pure reducer models signed 32-bit timing arithmetic, the 32-retry initial limit, auth/login phase selection, and resync acknowledgement; `handshake_wire` projects effects to raw GC records without enabling TEA or sockets. `ClientSession` must still preserve its existing Close and TEA barriers.
- *(From README.md.)* The inbound CG handshake codec is exact 13 bytes total (one header plus 12 payload bytes), accepts only `0xff` and `0xfc`, and is not a live descriptor. The transport-free dispatch adapter preserves Close-first and phase/boundary precedence, returns explicit `UnsupportedHeader` errors for non-handshake headers, and never turns those errors into a close. The pure reducer models signed 32-bit timing arithmetic, the 32-retry initial limit, auth/login phase selection, and resync acknowledgement without enabling TEA. `DecryptedLegacyTea` is a caller assertion, not proof of socket decryption. `ClientSession` must still preserve its existing Close and TEA barriers.
- The safebox and mall transfer records are exactly 8 bytes with a 7-byte framed payload, and their three headers 69, 70, and 71 all collide across directions with `HEADER_GC_ADD_FLY_TARGETING`, `HEADER_GC_CREATE_FLY`, and `HEADER_GC_FLY_TARGETING`; the client registers those three inbound as fixed 10- and 17-byte records, so no header-value table may be shared between directions. `window_type` is a source cell for header 70 and a destination cell for headers 69 and 71, and the record must not be typed as a direction the wire does not carry. Both handlers omit the `IsSecured()`, `IsExchanging()`, and `GetCount() <= 0` checks that sibling item paths apply, neither rejects an equipped item, and both detach an item before the operation that must re-home it while discarding its result. The codec preserves all of that as transport behavior and applies none of it.
- The inbound `CgPlayerDelete` record is exactly `[0x05][index:u8][private_code:[u8;8]]`. Every index and code byte remains opaque at this boundary. The decoder requires the complete ten-byte fixed record before checking header 5, and the frame adapter requires exactly nine payload bytes. It does not require a NUL, interpret text, compare a social ID, enforce the four-slot policy, build a DB request, run deletion, mutate an account/session, or frame a live descriptor stream.
- The inbound `CgPlayerCreate` record is exactly 34 bytes: header `4`, one opaque `u8` index, the raw 25-byte `name[CHARACTER_NAME_MAX_LEN + 1]` field, an explicitly little-endian `u16` job word, and opaque `shape`, `Con`, `Int`, `Str`, and `Dex` bytes. The decoder requires the complete 34-byte fixed record before checking header 4, and the frame adapter requires exactly 33 payload bytes. The codec requires no NUL, does not require or produce UTF-8, does not trim or interpret text, does not narrow or validate the job/race, and does not validate or derive statistics. Slot ownership, account state, name policy, locale/banword checks, DB `TPlayerCreatePacket` construction, SQL, roster and GC routing, and UI callbacks stay above this boundary. Legacy `strncpy(name, src, 24)` leaves the twenty-fifth name byte indeterminate, and the legacy handler treats the name as a C string; neither defect lets a safe codec synthesize bytes or apply policy.
- The inbound `CgAttack` record is exactly 8 bytes: header `2`, one opaque `u8` type, a little-endian `u32` target VID, and two raw magic-cube CRC piece bytes. The frame adapter requires exactly 7 payload bytes, and both decoders check the exact length before reading fields or checking the header. The codec requires no nonzero VID, performs no victim lookup, and does not interpret the type byte as a skill, apply the magic-cube XOR table, or enforce hit-rate limits. The legacy `AssembleCRCMagicCube` runs before any target check, `CheckSkillHitCount` counts before range and PvP checks, `IsValidProcessCRC` and `IsValidFileCRC` have no call site, and the client narrows its `UINT` motion index into the type byte; all of those stay documented legacy defects, not codec behavior.
- The inbound `CgUseSkill` record is exactly 9 bytes: header `52`, a little-endian `u32` skill number, and a little-endian `u32` target VID. The frame adapter requires exactly 8 payload bytes, and both decoders check the exact length before reading fields or checking the header. Zero is valid for both fields: the client declares `SendUseSkillPacket(DWORD dwSkillIndex, DWORD dwTargetVID=0)` and two active call sites omit the target, while the charge path deliberately sends a zero target first. The client also sends a skill-slot index in the target field on its active-toggle path, so the field must stay opaque. The legacy handler forwards an unresolved VID without a null check, and the downstream path deducts HP or SP and records cooldown before it validates the target; those are documented legacy defects, not codec behavior.
- The inbound `CgItemMove` record is exactly 9 bytes: header `13`, a source position, a destination position, and a count. Each position is a packed 3-byte `CgItemPos` holding an opaque `window_type` byte and an explicit little-endian `cell` word; the count is an explicit little-endian word. The frame adapter requires exactly 8 payload bytes, and both decoders check the exact length before reading fields or checking the header. Zero is valid for the count, because zero means "move the whole stack", and an identical source and destination is not rejected, because the legacy handler has no `Cell == DestCell` check. The `window_type` byte must stay opaque: `enum EWindows` at `length.h:657-675` inserts `ATTR67_ADD`, `AURA_REFINE`, and `SWITCHBOT` behind build flags, which shifts the numeric values of `BELT_INVENTORY` and `GROUND`. The same 9-byte struct also carries safebox header `77`, so the strict header check is the only thing separating the two records; a future safebox codec must reuse `CgItemMove` and `CgItemPos` with a different header. The legacy path mutates derived state before validation, discards the `EquipItem`, `SetItem`, `CreateItem`, and `AddToCharacter` results, can destroy the source item mid-merge, and returns `true` unconditionally; all of those stay documented legacy defects, not codec behavior.
- The inbound `CgItemUse` record is exactly 4 bytes: header `11` and one packed 3-byte position. The frame adapter requires exactly 3 payload bytes, and both decoders check the exact length before reading fields or checking the header. The record carries **one** position and must not gain a destination: `char.h:1182` declares `UseItem(TItemPos Cell, TItemPos DestCell = NPOS)`, and `NPOS` is the compile-time constant `(RESERVED_WINDOW, WORD_MAX)` at `length.h:1059`, so the second position is never wire data for this header. That default is also always *invalid*, because `SItemPos::IsValidItemPosition` rejects `RESERVED_WINDOW` at `length.h:977-978`; that single fact makes every target-required `UseItemEx` branch unreachable from header 11. The codec stores no header field, since it only ever accepts and writes header 11. The window byte stays opaque and cell `0xffff` is a normal client value. The numeric value 11 is reused in three other directions — game-to-client wrong-social-id, game-to-game messenger-remove, and game-to-database quest-save — so no header lookup table may be shared across directions. Inventory and window policy, timing and rate limits, item types, affects, skills, quests, gold, Dragon-Soul, rewards, and item-state mutation all stay outside this record codec.
- The inbound `CgItemUseToItem` record is exactly 7 bytes: header `60` and two packed 3-byte positions, source at 1..4 and target at 4..7. The frame adapter requires exactly 6 payload bytes, and both decoders check the exact length before reading fields or the header. It is the **only** inbound record that passes a real destination position into `CHARACTER::UseItem`, which is why it reaches the 22 destination-dependent branches in `UseItemEx` that the single-position record can never reach (21 dereference the target; `ITEM_HAIR` at `char_item.cpp:2837` is the 22nd and never reads it). The item-move record also carries a second position, but that one reaches `MoveItem`. Unlike headers 11 and 22, header 60 has no cross-direction collision: it is the only `HEADER_CG_*` equal to 60 and no `HEADER_GC_*` equals 60, and it is not a variable header. The codec stores no header field, must not compare or reject equal source and target positions (the legacy path has no such guard), and must not interpret either window byte. Two recorded legacy defects make the window byte's meaning unsafe to infer: `char_item.cpp:5243` and `:2837` validate the whole target position and then discard the window byte by looking the item up by cell in the main inventory, and the refine-element branches at `:3712` discard their helper's return value and report success for a target the helper silently rejects.
- The inbound `CgItemGive` record is exactly 9 bytes: header `83`, an explicit little-endian target `u32` VID at 1..5, one packed 3-byte position at 5..8, and an opaque count byte at 8. The frame adapter requires exactly 8 payload bytes, and both decoders check the exact length before reading fields or checking the header. The count byte is a **dead wire field**: it is declared at `packet.h:2325` and read nowhere on this path, so the transferred amount is chosen entirely by the receiver and the only possible outcomes are zero units, exactly one unit, or the whole stack. Preserve it byte-for-byte anyway, because it is physically present in every frame the client sends, and never derive a transfer policy from it. This is **not** a player-to-player give: `CanReceiveItem` returns `false` for any player destination at `char_item.cpp:9255-9256`, and the destination can therefore never be another character. Header 83 **does** collide across directions: `HEADER_GC_PARTY_PARAMETER` is also 83 and is only 2 bytes, and the client registers that two-byte record for inbound 83, so no header table may be shared between directions. The legacy path has no ownership check and no same-map check; its only locality test is a 2-D distance comparison. Keep the client uninitialized-send defect, the server bare cast, and the discarded `bool` above the codec.
- `protocol::cg_item_destroy` is the exact four-byte header-21 record, a packed `TItemPos` and nothing else. Header 21 collides cross-direction and is a **live** two-sizes collision: the server calls it `HEADER_GC_ITEM_SET` at `packet.h:122`, the client calls it `HEADER_GC_ITEM_SET2` at `Packet.h:117`, and the client registers its own `packet_set_item2` at `PythonNetworkStream.cpp:71-72`. Do not merge or rename across the two trees. Three legacy facts are recorded in the module and must not be reproduced or normalized: `CHARACTER::DestroyItem` frees the item at `char_item.cpp:7438` and then dereferences it at `:7439` on **every** successful destroy; `GetItem` at `char_item.cpp:254-305` returns `INVENTORY` and `EQUIPMENT` from the same `pItems` array under the same bound at `:269`, so those two window bytes alias; and the client wrapper default-constructs `TItemPos`, so it transmits window byte 1 permanently. The container claim depends on citing `CHARACTER::IsValidItemPosition` at `char_item.cpp:10015`, **not** `SItemPos::IsValidItemPosition` at `length.h:973`, which returns false for `SAFEBOX`/`MALL` and would make the claim false.
- `protocol::cg_item_drop` is the exact eight-byte header-12 record, a packed `TItemPos` plus an opaque `DWORD gold`, and its framed payload is seven bytes. `gold` is the **sole** discriminator at `input_main.cpp:1036` between dropping currency and dropping an item, so the codec must keep the full 32-bit word and must not bound it: the only server-side bound is a comparison against the character's own balance inside `CHARACTER::DropGold`. Do not reproduce the legacy signed narrowing there, where `DropGold` takes a signed `INT` (`char.h:1265`, active because `ENABLE_REMOVE_LIMIT_GOLD` is defined at `prodomodefines.h:157`) and so rejects every wire value at or above `0x80000000` at `char_item.cpp:7556`. Header 12 is **not** interchangeable with header 20: `char.h:1190` defaults `bCount` to zero, `char_item.cpp:7498-7499` turns a zero into the whole stack, and header 12 therefore cannot drop a partial stack at all. Header 12 is also **not** free of cross-direction collision: `packet.h:112` declares `HEADER_GC_ATTACK = 12`, but the collision is dead, because the server has no `TPacketGCAttack` and no send site, and the client has no such constant, skips index 12 in both its enum and its receive map, and would reject an inbound 12 in `CheckPacket` at `PythonNetworkStream.cpp:536-543`.
- *(From README.md.)* The inbound `CgItemDrop2` codec is exactly 10 bytes: header `20`, one packed 3-byte position, an opaque `u32` currency word, and an opaque `u16` amount word, with no stored header byte. It is header 12 plus a trailing `WORD count`, so it is a different operation, not a re-spelling: `ItemDrop` at `input_main.cpp:1024-1040` calls `DropItem(Cell)` with one argument, `ItemDrop2` at `:1042-1051` calls `DropItem(Cell, count)` with two, and header 20 is the only one of the two that can split a partial stack at `char_item.cpp:7510-7530`. Keep `gold` an unvalidated `u32`: the client hardcodes it to `0` on the item path at `PythonNetworkStreamPhaseGameItem.cpp:863`, so the server's `gold > 0` discriminator at `input_main.cpp:1047` is reachable only through the separate `SendGoldDropPacketNew` entry point, and the client truncates `count` from `DWORD` to `WORD` at `:665` with no range check. Keep `count` an unvalidated `u16`: a transmitted zero and a transmitted over-large value both mean "whole stack" through `char_item.cpp:7498-7499`, and the `if (bCount == 0)` guard at `char_item.cpp:7512-7517` is unreachable, so no zero-triggered rewrite belongs here. The window byte's meaning is not fixed by the wire: the `EWindows` enum at `length.h:657-676` conditionally inserts three enumerators at `:665-673`, making `BELT_INVENTORY` 9 with them on and 6 with them off.
- *(From README.md.)* The inbound drop codecs share one packed 3-byte position type, and header 20 is a cross-direction trap: the server sends `HEADER_GC_ITEM_DEL = 20` while the client's only 20 is `HEADER_GC_ITEM_SET`, so the client has no item-delete handler to receive it. See the four-bucket collision taxonomy.
- The CG quickslot family is three separate fixed records: `protocol::cg_quickslot_add` is header 16, `[0x10][pos][slot type][slot pos]`, 4 bytes; `protocol::cg_quickslot_del` is header 17, `[0x11][pos]`, 2 bytes; `protocol::cg_quickslot_swap` is header 18, `[0x12][pos][change_pos]`, 3 bytes. `protocol::TQuickslot` in `protocol/src/lib.rs` is the compliant explicit primitive the add codec uses. The del and swap codecs deliberately add **no** 36-slot bound, no type-enum check, and no requirement that the two swap indices differ: `CHARACTER::SwapQuickslot` accepts a self-swap, and the bounds live in `char_quickslot.cpp` above framing. A self-swap and any `pos` 0..255 are therefore legal at the record boundary. `decode_frame` in both new modules checks a **payload** width, not the full-record width, because a `ClientFrame` payload excludes the header byte; sharing one `check_exact` between `decode` and `decode_frame` is a defect that real tests caught.
- The legacy client declares **two disagreeing** quickslot capacities four lines apart: `QUICKSLOT_MAX_COUNT = 32` at `client/Client/UserInterface/Packet.h:258` (from 4x8) and `QUICKSLOT_MAX_NUM = 36` at `:260`. The slot array is sized by the 36 constant at `PythonPlayer.h:134`, and `QUICKSLOT_MAX_NUM` has six uses in `PythonPlayer.cpp` against exactly one use of the 32. So client slots 32..35 are populated, drawn, moved, and deletable, and only `RequestDeleteGlobalQuickSlot` refuses them. Do not infer "32 slots" or "36 slots" for a client; a script can place any value 0..255 in either swap index, because `RequestMoveGlobalQuickSlotToLocalQuickSlot` bounds neither and `LocalQuickSlotIndexToGlobalQuickSlotIndex` is unbounded. The server's `Analyze` closes the phase when the character is null, so the missing `if (!ch)` in the two new handlers is a style inconsistency, not a reachable null dereference.
- When claiming a legacy struct is "declared but never used", sweep the **struct tag** and the typedef alias as two separate searches. Call sites overwhelmingly spell the tag, so a single-name sweep that returns one hit is the signature of this error. It produced a false "GC 17 is dead" claim in this repository once already.
- **Superseded 2026-09-26.** Header 27 is implemented as `protocol::cg_exchange` (the active
  14-byte `u64` profile), and header 22 `HEADER_CG_ITEM_SELL` has no `Set()` row in
  `CPacketInfoCG`, so it is outside the backlog. The original note follows.
  Next CG headers after this family are **both feature-gated**: header 22 `HEADER_CG_ITEM_SELL` sits inside `#ifdef ENABLE_SELL_ITEM` so the value names nothing without that define, and header 27 `TPacketCGExchange` has `#ifdef ENABLE_REMOVE_LIMIT_GOLD unsigned long long arg1; #else DWORD arg1; #endif` making it four bytes wider in one profile, plus a `sub_header`. Either needs an explicitly stated feature profile before a faithful codec.
- `protocol::cg_item_pickup` is the exact five-byte header-15 record, `[0x0f][vid: u32 little-endian]`, with a four-byte framed payload. It is the only record in this family that carries **no** `TItemPos`: `vid` is a ground-item virtual ID, not a window and cell, so the codec must never grow one. `vid` is an opaque `u32` with no sentinel and no validation, because `CHARACTER::PickupItem` (`char_item.cpp:7972-8161`) uses its `dwVID` parameter exactly once, at `:7974`, in `ITEM_MANAGER::FindByVID`, which is a `std::map::find` at `item_manager.cpp:664-672`; there is no `-1`, no `0`, no `INVALID_VID`, no range test, and no signed cast on that path, so `0` and `0xFFFFFFFF` are ordinary wire values. Header 15 is **collision-free inside the CG direction** under the four-bucket taxonomy: both trees spell it with the same struct tag `command_item_pickup`, the same members, and the same five-byte width, and no second CG record carries 15 in either tree. The three other value-15 enumerators are different direction families with different decoders and constrain nothing: `HEADER_GC_MAIN_CHARACTER_OLD` (`packet.h:115`) is a dead enumerator with no struct, no send site, and no consumer, whose client-side namesake `HEADER_GC_MAIN_CHARACTER` is a live 45-byte `TPacketGCMainCharacter` (`Packet.h:1429-1437`); `HEADER_GG_GUILD_WAR_ZONE_MAP_INDEX` (`packet.h:244`) is the 13-byte GG record; and `HEADER_GD_EMPIRE_SELECT` (`common/tables.h:32`) is in the handle-and-length DB-peer framing. The trees spell the typedef differently, `TPacketCGItemPickup` (`packet.h:662`) versus `TPacketCGItemPickUp` (`Packet.h:547`); do not unify them, but cite both. Three client defects are documented and must not be reproduced or compensated for: the Python wrappers at `PythonNetworkStreamModule.cpp:929/934` and `PythonPlayerModule.cpp:843/847` widen a signed `int` into the `DWORD` parameter, and `PyTuple_GetInteger` (`ScriptLib/PythonUtils.cpp:137`) never checks `PyLong_AsLong`, so `0xFFFFFFFF` is script-reachable. That is a client defect, not a server sentinel, and it is the specific reason the codec must not add a "non-zero vid" rule. The legacy handler is `void` and discards `PickupItem`'s `bool`, so this boundary models no success or failure result, and the legacy length guarantee comes from `input.cpp:92-93` rather than from the handler.
- The client Python wrappers are not uniformly safe, and three of them transmit uninitialized stack values. `PythonNetworkStreamModule.cpp` has thirteen arity `switch (PyTuple_Size(...))` statements; ten of the twelve that have a `default:` arm return `Py_BuildException()` from it and are safe, but `netSendGiveItemPacket` breaks at `:963-964` and falls through to `:968`, `netSendSafeboxCheckoutPacket` breaks at `:1376-1377` and falls through to `:1381`, and `netSendSafeboxCheckinPacket` has **no `default:` arm at all** and falls through from `:1329-1347` to `:1350`. The last is the worst form. Do not treat an absent `default:` as safe. Classify a wrapper only by extracting its `default` arm and closing its switch by brace matching, never by a line window, because one `default:` label at `:1376` is otherwise counted against two switches.
- Client variable framing is a pure protocol boundary. Header `0x08` SyncPosition has a 3-byte full-frame prefix and 12-byte elements; retain all declared elements, keep the gameplay cap of 16 out of the protocol decoder, and keep TEA/descriptor/session integration separate. Header `0xdb` FishEvent consumes a 2-byte prefix, extends box-use to 5 bytes and shape-add to 3 bytes, and leaves unknown subheaders at the 2-byte legacy prefix. Header `0x32` Shop consumes a 2-byte prefix; END/unknown subheaders consume no extension, BUY adds 2 bytes, SELL adds 1 byte, and SELL2 adds the active x86 four-byte `TfckOFF` (including its uninitialized padding byte). No shop action belongs in the framing layer. The pure game policy caps only processed elements and models owner, interval, displacement, close, and self-send effects through injected ports.
- *(From README.md.)* Client variable framing is a pure, source-verified boundary. Header `0x08` SyncPosition has a 3-byte full-frame prefix and 12-byte elements; keep every declared element, do not enforce the gameplay cap of 16 in the protocol decoder, and keep TEA/descriptor/session integration separate. The pure game policy caps only the first 16 processed elements, preserves the full declared consumption, and models the legacy owner, interval, displacement, descriptor-close, and self-send effects through injected ports. Messenger header `0x43` has a 2-byte base; ADD_BY_VID adds a packed 4-byte VID, ADD_BY_NAME and REMOVE add a raw 24-byte name field, and subheader 3 or unknown values consume only the base. This framing slice does not resolve players or invoke messenger actions. Private shop header `236` is the third sub-header-routed two-byte prefix in the same layer: 22 sub-header constants, 21 constant extension sizes, and one count-derived `BUILD` rule that reads `wItemCount` little-endian at frame offset 41. Ten sub-headers extend by zero, including `STATE_UPDATE` (9), which is enumerated but never dispatched, and unrecognised sub-headers take the same `default:` path; never give either a payload. The sub-header values live at wire offset 1, so they are not a header space and the collision taxonomy does not apply to them. This slice frames private-shop sub-records and nothing more: it parses no sub-record field, resolves no shop or owner, and runs no buy, check-in, checkout, price-change, or search logic.
- *(From README.md.)* Header 77 collides across directions with the five-byte inbound `HEADER_GC_PARTY_INVITE`, so a header-keyed table would need two sizes for one value and the directions must never share one.

## Game-to-client login-flow records

- The outbound `GcTimeSync` acknowledgement is exactly `[0xfc]` and carries no timestamp or payload. It is separate from the 13-byte inbound `CG_TIME_SYNC` handshake record, the 5-byte header-106 `GC_TIME` record, the one-byte heartbeat ping, retry state, client phase dispatch, TEA, and sockets. The `protocol::gc` codec is transport-free.
- The outbound `GcAuthSuccess` record is exactly `[0x96][login_key:u32 little-endian][result:u8]`, with no padding or length field. Both fields remain opaque; `result` is not restricted to zero or one. It is separate from `GC_LOGIN_SUCCESS`, `GC_LOGIN_FAILURE`, and `GC_LOGIN_KEY`. The codec does not validate authentication, install keys, encrypt or decrypt, frame a stream, reconnect, or mutate session state.
- The `GcLoginKey` record is exactly `[0x76][login_key:u32 little-endian]`, with no padding or length field. The key is opaque; zero and high-bit values are valid wire values. The checked-in server has no active send or receive path for header 118, while the client registers and receives this fixed record. The codec does not generate, validate, or install keys, add transport framing or TEA, or mutate session state.
- The `GcLoginFailure` record is exactly `[0x07][status bytes][NUL]`, 10 bytes total with 8 logical status bytes. Status remains arbitrary bytes; encoding stops at the first NUL or eight bytes and zero-fills the unused suffix, while decoding requires a NUL and ignores bytes after the first one. The codec does not map status meanings, authenticate, frame transport, or mutate session state.
- The active `GcLoginSuccess` record is exactly 357 bytes: header `0x20`, four 70-byte active-profile `TSimplePlayer` summaries, four little-endian `u32` guild IDs, four raw 13-byte guild-name fields, then little-endian `handle` and `random_key`. It accepts only that active feature profile; the 329-byte no-feature shape and legacy header-6 three-slot shape remain separate and rejected. Player names, dummy bytes, and guild names stay raw fixed storage. The codec does not authenticate an account, validate IDs or names, resolve guilds, select a player, frame transport, install keys, or mutate session state.
- The active `GcPlayerCreateSuccess` codec is exactly 72 bytes: header `0x08`, one opaque account-character index, and the shared 70-byte active-profile player summary. The declared `GcCreateFailure` codec is exactly `[0x09][type:u8]`. The create-success index stays opaque at the record boundary, and the exact create-failure decoder rejects the legacy one-byte and wrong-type ten-byte sends. Neither codec runs SQL, selects or creates a player, classifies a failure, repairs a producer, frames transport, invokes UI, or mutates session state.
- The `GcPlayerDeleteSuccess` record is exactly `[0x0a][account_index:u8]`. Every `u8` index is preserved, but this codec does not reproduce the unchecked game-side or client-side four-slot indexing. `GcPlayerDeleteWrongSocialId` is the distinct exact one-byte record `[0x0b]`; that legacy name does not classify every DB failure mapped to the header. The client's static header-10 entry understates the consumed success record as one byte, but this exact codec follows the two-byte server send and client receiver. Neither delete codec runs the deletion flow, SQL/cache work, roster/UI mutation, transport framing, encryption, or session handling.
- *(From README.md.)* The declared `GcCreateFailure` codec is exactly two bytes: header `0x09` and one opaque `bType`. Both server/client header aliases resolve to 9, and every `u8` value is preserved. The exact decoder rejects the legacy header-only `[0x09]` send and the zero-filled wrong-`TPacketGCLoginFailure` ten-byte frame `[0x09, 0x00 x9]` as source framing defects; it does not normalize TEA padding, classify the type, run SQL, invoke UI callbacks, repair producer sends, frame transport, or mutate session state.
- GC header 17 (`TPacketGCPointChange`, `struct packet_point_change`, 25 bytes) is **live**, not dead: it is sent from `char.cpp:2093-2116` and `char.cpp:4805-4818`, both unconditional, plus a site at `:1834-1840` behind `__FIX_UPDATE_PLAYTIME_AND_ITEMS__`. Its first member is `int header` at `packet.h:1066`, not `BYTE`, so under `pack(1)` the server writes four little-endian bytes `11 00 00 00`. **A future GC 17 codec must not copy the one-byte-header convention from the CG 16/17/18 siblings.** `TPacketGCChangeSpeed` / `struct packet_change_speed` at header 18 is genuinely dead, verified in both spellings, though the client still registers it inbound. Values 17 and 18 are both bucket 2, colliding but not two-sizes; CG 17 is 2 bytes against GC 17's 25 and GG 17's 6, and CG 18 is 3 against GG 18's 2. No header table may mix CG 17 with GC 17 or CG 18 with GC 18.

## Cross-direction header collisions

- Cross-direction header collisions are not a single category, and the distinction decides how much a codec may assume. Sort every header into one of **four** buckets and record which one applies. *Live two-sizes*: the value names a different struct of a different width on the two trees, so a shared table is actively wrong. Headers 21 and 77 are in this bucket; 21 is the `packet_item_set`/`packet_set_item2` drift above, and 77 is the safebox/mall pair where two server structs and three client declarations share three values. *Colliding but not two-sizes*: the value is reused across directions while the width stays consistent, as for 11, 13, 15, 22, 69, 70, 71, and 83. *Width-equal semantic collision*: the value is reused across directions, the widths match, and the two records are **unrelated or opposite** operations. This is the only bucket in which the client **cannot** fail closed, because the header is already a known size, and the only one where width agreement is a **coincidence of layout** rather than evidence of a shared record. Never report one of these as safe merely because the widths match. Header 20 is the confirmed case: the server sends `HEADER_GC_ITEM_DEL` at `packet.h:121` as a bare `struct TPacketGCItemDelDeprecated` at `packet.h:1085-1099`, set at `char_item.cpp:598` and sized at `:610`, while the client has **no** `HEADER_GC_ITEM_DEL` at all and its only 20 is `HEADER_GC_ITEM_SET` at `Packet.h:116`, registered at `PythonNetworkStream.cpp:71` and dispatched to the item-**set** handler at `PythonNetworkStreamPhaseGame.cpp:342`. The two declarations are field-for-field identical, so an item-delete record would be executed by the item-set handler. Whether the widths match **exactly** is not determined, because the client `ITEM_SOCKET_SLOT_MAX_NUM` is itself 3 or 6 at `GameType.h:550-552` and only the server is confirmed x86 at `premake5.lua:12`; a width match means a silent wrong operation, and a width mismatch means `CheckPacket` drops the connection at `PythonNetworkStream.cpp:537-543`. Note that Section 145 had classified header 20 as a live two-sizes collision, which was wrong on both counts and is corrected here. *Nominally colliding, server-enum-only*: the enum value exists but no struct, no send site, and no client consumer exist for it, so it imposes no second size; 12 is the only header confirmed in this bucket, and the client's own unused `TPacketGCAttack` at `Packet.h:2102-2108` is what would change that if a live GC 12 were ever implemented. Headers 52 and 60 are free. A value can also appear outside `packet.h`: the game-to-DB constants live in `common/tables.h`, so header 20 is a fourth `HEADER_GD_GUILD_EXP_UPDATE` at `tables.h:39` and 20 is a `HEADER_GG_LOGIN_PING` at `packet.h:249`. Use targeted `HEADER_(CG|GC|GD|DG|GG)` patterns, never a bare `HEADER_[A-Z]_` one, and give every negative result a known-positive control. Never share a header table between directions, and never infer a width from the fact that two directions use the same number.

## DB records and boot

- Select a DB boot feature profile explicitly. Keep ordinary table records opaque until feature-dependent C++ record widths are verified; `TBanwordTable`, `TRefineTable`, optional `TEventTable`, optional `TMarketItemPrice`, and `TPacketUpdateHorseName` are source-fixed typed exceptions. `ENABLE_ITEMSHOP` data is a separate DB frame. `BootSnapshot` is caller-resolved and profile-bound; it must not query SQL, infer rows, or reuse request-specific hosts/admins. Horse-name lookups are injected and must not invent SQL results; missing rows are 25 zero bytes. Channel lookup is injected too: zero selectors are silent, valid no-match results are six zero bytes, and matches echo the request handle. Login-by-key and primary player-load adapters frame only already-resolved outcomes; they must not infer missing results, execute SQL, or mutate login state. The separate `player_index` boundary builds only the source-fixed login-by-key index query, preserves SQL NULLs, rejects out-of-range empire values, and its SQLx adapter uses a one-row bounded stream without implementing the legacy missing-row repair path. The primary login success payload is 362 bytes, the missing/invalid payload is empty, and an already-logged-in result preserves the caller's exact raw 31-byte record.
- *(From README.md.)* Select a DB boot feature profile explicitly. Keep ordinary table records opaque until their feature-dependent C++ record widths are verified; the source-fixed `TBanwordTable`, `TRefineTable`, optional `TEventTable`, optional `TMarketItemPrice`, and `TPacketUpdateHorseName` boundaries are the typed exceptions. `ENABLE_ITEMSHOP` data is a separate DB frame. Horse-name lookups must be injected; a missing row produces 25 zero bytes, while returned bytes are copied exactly. Channel lookup is also injected: a zero map/channel selector produces no frame, a valid request without a matching peer produces an all-zero six-byte result, and a match echoes the incoming DB handle. Login-by-key and primary player-load services are framing boundaries only: they must not infer missing results, execute SQL, or mutate login state. A resolved login success is 362 bytes, a missing/invalid result is empty, and an already-logged-in result preserves the caller-supplied raw 31-byte record; header 33 is not a primary key-login outcome. DB-peer identity must come from an external, generation-bound verification; the setup `bAuthServer` byte, frame handle, source prefix, login key, and account/player IDs are not authentication. The current policy authorizes only an explicitly verified `Auth`-role, base-only setup and emits an effect for caller application; it does not replace the legacy auth-pointer overwrite or authorize live setup.
- *(From README.md.)* `db-server::postfix` is the required allowlist before interpolating `TABLE_POSTFIX` into SQL. `db-server::player_index` builds the fixed login-by-key index query and rejects SQL NULL/out-of-range values; `db-server::player_index_sqlx` acquires at most one row, preserves NULLs, and keeps database failures distinct from an empty result. `db-server::banword_sqlx` accepts only the fixed banword query, preserves raw bytes and NULLs, and uses the pool's bounded retry-preserving stream. `db-server::quest`/`quest_sqlx` preserve the two exact quest query variants and the count-prefixed 106-byte response boundary. `db-server::event`/`event_sqlx` preserve the exact seven-column event query, positional timestamp extraction, raw type bytes, strict NULL/numeric handling, and bounded 85-byte sections without becoming a live cache. `db-server::item_attr`/`item_attr_sqlx` preserve the exact 18-column normal and 16-column rare item-attribute queries, raw NULL/byte cells, source order and duplicates, strict versus explicitly named legacy conversion, and the active 71-byte section boundary without becoming a live cache. `db-server::boot_snapshot` is caller-resolved and profile-bound; it does not query tables or choose a feature profile from bytes. `ENABLE_ITEMSHOP` stays in its separate frame.
- Validate `TABLE_POSTFIX` through `db-server::postfix` before any future SQL identifier interpolation. The postfix/query module is SQL-free and must not be described as a live loader. The separate `db-server::banword_sqlx` adapter accepts only the fixed banword query, preserves raw bytes and NULLs, and uses bounded retry-preserving streaming; it is not a live cache or boot service.
- The `db-server::event` and `db-server::event_sqlx` boundaries preserve the exact seven-column `event%s` query, positional `UNIX_TIMESTAMP` extraction, raw `type` bytes, strict numeric/NULL handling, source order, and bounded 85-byte sections. They are acquisition and encoding boundaries only; they do not select a boot profile, build a cache, compose a snapshot, or provide a live event service.

## Legacy constants

- Resolve a legacy array-dimension constant across **all four** definition forms: `#define NAME n`, `const T NAME = n;`, a bare member line inside an anonymous enum, and a single-line named `enum { NAME = n };`. A `#define`-only search reports a symbol that is used 143 times as absent from the repository. `CHARACTER_NAME_MAX_LEN` is 24 and is defined only as a bare anonymous-enum member at `server/server/common/length.h:15` and `client/Client/UserInterface/StdAfx.h:43`; `SHOP_SIGN_MAX_LEN` is 32; `MAX_EFFECT_FILE_NAME` is 128; `POINT_MAX_NUM` is 255. Five further constants are genuinely undefined anywhere in `server/` or `client/`: `ITEM_ATTRIBUTE_MAX_NUM`, `ITEM_ATTRIBUTE_SLOT_MAX_NUM`, `PRIVATE_SHOP_HOST_ITEM_MAX_NUM`, `CHR_EQUIPPART_NUM`, and `ITEM_APPLY_RANDOM_SLOT_MAX_NUM`. Any record whose array dimension is one of those five has no source-derivable width; do not invent one. Use the `metin2-verify-absence-source-sweep` skill before reporting any such absence.
  **Corrected 2026-09-26:** only **one** of those five is undefined. `ITEM_ATTRIBUTE_MAX_NUM` is 7
  (`server/server/common/item_length.h:30`, expression-valued). `ITEM_ATTRIBUTE_SLOT_MAX_NUM` is 7
  (`client/Client/UserInterface/GameType.h:566`, expression-valued). `PRIVATE_SHOP_HOST_ITEM_MAX_NUM`
  is 128, from 8 x 8 x 2 (`server/server/common/length.h:136`, `client/Client/UserInterface/Packet.h:290`).
  `CHR_EQUIPPART_NUM` is an auto-increment member (`server/server/game/packet.h:881`,
  `client/Client/UserInterface/Packet.h:1209`): 6 when `__SASH_SYSTEM__` and `__AURA_SYSTEM__` are
  both defined, as they are in `server/server/common/prodomodefines.h`. Only
  `ITEM_APPLY_RANDOM_SLOT_MAX_NUM` has no definition, and every use of it is inside
  `#ifdef ENABLE_PRIVATE_SHOP_APPLY_RANDOM`, which is commented out in both trees
  (`client/Client/UserInterface/LOCALE_INC.H:139`, `server/server/common/prodomodefines.h:188`).
  The searches that called the
  other four undefined looked only for a literal `= n`; see the correction under
  [Verification lessons](#verification-lessons).

## Width probe and test values

- Measure a packed legacy width by compiling the verbatim struct body for `i686`, never by summing field widths, and always run a positive control that proves the probe is 32-bit. Search the whole server tree for struct definitions: restricting to `server/server/game/packet.h` wrongly reported five of the 18 `gc_fields` records as client-only, because `packet_motion` and others live under different names or in other headers. A field name does not determine its width: `TPacketGCShamanUseSkill::dwLevel` is a `BYTE`, so that record is 10 bytes and not 13. A header byte can be the only difference between two records, as in `party.cpp:729`, which sends the unlink record as a `TPacketGCPartyLink` with the header changed.
- Make decode round-trip tests use values with distinct halves. A value like `u16::MAX` is byte-symmetric, so a big-endian read returns the same value and the test cannot catch the bug. Three `gc_fields` records were originally tested on the encode side only, which left eight real coverage gaps that a two-round mutation sweep exposed; a sweep whose mutants are all compile errors proves nothing, and a sweep that applies each mutation on top of the previous one reports catches that belong to the wrong mutant. Apply every mutation to the pristine file, assert the mutated text differs, and re-run any survivor after refactoring the tests.

## Section 162 mutation evidence

Gate and sweep evidence: 38 mutations across the 13 new or changed codec files, **38 caught, zero
survivors, zero equivalent mutants**, with every file restored byte-for-byte by SHA-256. The five
strict-Clippy findings were fixed by refactoring, never by suppression; the two `.expect()` calls
inside `CgHack::decode` and `decode_frame` became a pre-sized array plus `copy_from_slice`, which
removed a real panic point instead of documenting it.

## Module index

Generated 2026-09-26 from the first `//!` line of each file. "No module doc" means the file
has none. Regenerate this list instead of editing it by hand.

### `protocol`

| file | first doc line |
|---|---|
| `cg_account.rs` | Explicit codecs for the fixed client-to-game account-phase records. |
| `cg_arg_u8.rs` | Seven small records whose payload is one to five **opaque** `u8` arguments. |
| `cg_attack.rs` | Explicit codec for the fixed legacy `TPacketCGAttack` record. |
| `cg_change_look.rs` | The 10-byte `ChangeLook` record: a header, a sub-header, a cost, a slot, and |
| `cg_client_version.rs` | Explicit codec for the fixed legacy client-version report. |
| `cg_cube_renewal.rs` | The 14-byte `CubeRenewalSend` record. |
| `cg_dragon_soul.rs` | Transport-free codec for the client-to-server dragon-soul refine request. |
| `cg_exchange.rs` | The 14-byte `Exchange` record. |
| `cg_fly_target.rs` | The 13-byte fly-targeting record, which arrives under **two** headers. |
| `cg_gaya_system.rs` | The 6-byte `GayaSystemSend` record -- and the one place where the legacy |
| `cg_guild_answer.rs` | The 14-byte `AnswerMakeGuild` record: a header and a raw 13-byte guild name. |
| `cg_hack.rs` | The 257-byte `Hack` record: a header and a raw 256-byte anti-cheat report. |
| `cg_handshake.rs` | Explicit client-to-game handshake and time-sync record codec. |
| `cg_header_only.rs` | The two 1-byte client records that carry no payload at all. |
| `cg_inventory.rs` | Machine-readable inventory of the legacy client-to-game packet headers. |
| `cg_item_destroy.rs` | Explicit codec for the fixed legacy `TPacketCGItemDestroy` record. |
| `cg_item_drop.rs` | Explicit codec for the fixed legacy `TPacketCGItemDrop` record. |
| `cg_item_drop2.rs` | Explicit codec for the fixed legacy `TPacketCGItemDrop2` record. |
| `cg_item_give.rs` | Explicit codec for the fixed legacy `TPacketCGGiveItem` record. |
| `cg_item_move.rs` | Explicit codec for the fixed legacy `TPacketCGItemMove` record. |
| `cg_item_pickup.rs` | Explicit codec for the fixed legacy `TPacketCGItemPickup` record. |
| `cg_item_use.rs` | Explicit codec for the fixed legacy `TPacketCGItemUse` record. |
| `cg_item_use_to_item.rs` | Explicit codec for the fixed legacy `TPacketCGItemUseToItem` record. |
| `cg_login.rs` | Transport-free codecs for the three CG records that the handshake and login |
| `cg_login3.rs` | The 66-byte `Login3` record -- and the one live client/server incompatibility |
| `cg_mark.rs` | Transport-free codecs for three small fixed-width CG **login-phase** records. |
| `cg_micro.rs` | Transport-free codecs for four tiny fixed-width CG game-phase records. |
| `cg_move.rs` | Explicit codec for the fixed legacy `TPacketCGMove` record. |
| `cg_name.rs` | Transport-free codecs for the three CG records that carry fixed `char[N+1]` |
| `cg_party.rs` | Transport-free codecs for the five remaining legacy CG party records. |
| `cg_party_skill.rs` | Explicit codec for the fixed legacy `TPacketCGPartyUseSkill` record. |
| `cg_pick.rs` | Explicit codecs for the four fixed-width game-phase records that carry a |
| `cg_quest_text.rs` | Transport-free codecs for the two 66-byte client quest-text requests. |
| `cg_quickslot_add.rs` | Explicit codec for the fixed legacy `TPacketCGQuickslotAdd` record. |
| `cg_quickslot_del.rs` | Transport-free codec for the legacy `TPacketCGQuickslotDel` record. |
| `cg_quickslot_swap.rs` | Transport-free codec for the legacy `TPacketCGQuickslotSwap` record. |
| `cg_refine.rs` | The two item-refinement request records. |
| `cg_safebox.rs` | Transport-free codec for the legacy safebox and mall item-transfer records. |
| `cg_safebox_move.rs` | Explicit codec for the fixed legacy `TPacketCGItemMove` record as it is |
| `cg_sash.rs` | Transport-free codec for the CG direction of `TPacketSash`. |
| `cg_shoot.rs` | Explicit codec for the fixed legacy `TPacketCGShoot` record. |
| `cg_use_skill.rs` | Explicit codec for the fixed legacy `TPacketCGUseSkill` record. |
| `cg_variable.rs` | Incremental framing for source-verified variable-length client packets. |
| `cg_vid.rs` | The two 5-byte records that are a header plus a 32-bit virtual id. |
| `cg_wire.rs` | Safe framing for the legacy client-to-game stream. |
| `db_boot.rs` | Safe parser for the legacy version-6 database boot payload. |
| `db_market_price.rs` | Source-fixed premium-market item-price update codec. |
| `db_records.rs` | Legacy DB record codecs for the read-only login path. |
| `db_wire.rs` | Legacy DB peer framing. |
| `gc.rs` | Source-verified game-to-client authentication, roster, player-creation and |
| `gc_actors.rs` | The game-to-client actor, target, and main-character records. |
| `gc_fields.rs` | The game-to-client records that carry named scalar fields and raw fixed |
| `gc_inventory.rs` | Machine-readable inventory of the legacy game-to-client packet headers. |
| `gc_nested.rs` | The game-to-client records that nest another record or carry a fixed array. |
| `gc_small.rs` | The small fixed-width game-to-client records. |
| `gc_vid.rs` | The game-to-client records that carry one 32-bit value. |
| `gg.rs` | Explicit codecs for fixed game-to-game setup, lookup, warp, and map-index |
| `gg_inventory.rs` | Machine-readable inventory of the legacy game-to-game packet headers. |
| `item_pos.rs` | The legacy `TItemPos`, shared by every record that embeds one. |
| `lib.rs` | Metin2 packet protocol definitions. |
| `tea.rs` | Pure legacy TEA block codec. |

### `common`

| file | first doc line |
|---|---|
| `config.rs` | Configuration file parser for Metin2 server |
| `constants.rs` | Game constants ported from C++ length.h and item_length.h |
| `enums.rs` | Game enums ported from C++ length.h and item_length.h |
| `features.rs` | Feature flags documentation ported from C++ prodomodefines.h |
| `lib.rs` | Metin2 shared types and utilities |
| `logging.rs` | *No module doc.* |
| `tables.rs` | Data structures ported from `server/server/common/tables.h`. |
| `vid.rs` | Virtual ID (VID) system for network entity identification. |

### `net`

| file | first doc line |
|---|---|
| `buffer.rs` | Generic length-prefixed buffer management for network I/O. |
| `client_transport.rs` | Tokio transport boundary for fixed legacy client-to-game frames. |
| `db_transport.rs` | Tokio transport boundary for the legacy DB peer protocol. |
| `lib.rs` | Metin2 networking layer |

### `db`

| file | first doc line |
|---|---|
| `lib.rs` | Metin2 database layer |
| `pool.rs` | Connection pool management for `MySQL` databases. |

### `world`

| file | first doc line |
|---|---|
| `event.rs` | Deterministic synchronous events driven by caller-supplied pulses. |
| `lib.rs` | Metin2 game world and map management |
| `map.rs` | Map identity and explicit sector topology. |
| `sector.rs` | Legacy-compatible world coordinate conversion and sector keys. |
| `spatial.rs` | Deterministic entity membership across configured map sectors. |
| `character/combat.rs` | *No module doc.* |
| `character/manager.rs` | *No module doc.* |
| `character/mod.rs` | Bounded runtime character state and lifecycle management. |
| `character/model.rs` | *No module doc.* |
| `character/state.rs` | *No module doc.* |
| `character/combat/pk.rs` | *No module doc.* |
| `event/queue.rs` | *No module doc.* |
| `event/queue/process.rs` | *No module doc.* |

### `quest`

| file | first doc line |
|---|---|
| `lib.rs` | Metin2 Lua quest system |

### `game-server`

| file | first doc line |
|---|---|
| `account_player.rs` | Transport-free account and player state reduction for one client descriptor. |
| `account_player_router.rs` | Transport-free account/player routing beside the lifecycle adapter. |
| `client_session.rs` | Isolated client session boundary for fixed legacy game-server frames. |
| `descriptor_crypto.rs` | Source-derived legacy descriptor cipher boundary. |
| `game_loop.rs` | Deterministic pulse planning and synchronous game-loop ownership. |
| `game_loop_messages.rs` | Typed messages crossing between Tokio and the synchronous game loop. |
| `handshake.rs` | Pure state reduction for the source-verified CG handshake boundary. |
| `handshake_dispatch.rs` | Transport-free dispatch from a decoded handshake frame to client lifecycle state. |
| `handshake_wire.rs` | Pure wire projection for handshake reducer effects. |
| `heartbeat.rs` | Pure heartbeat and PONG liveness reduction. |
| `heartbeat_wire.rs` | Pure wire projection for heartbeat effects. |
| `lib.rs` | Game-server scheduling and asynchronous boundary modules. |
| `lifecycle.rs` | Transport-free client lifecycle state adapter. |
| `main.rs` | Metin2 game server binary |
| `sync_position.rs` | Pure policy for the legacy `HEADER_CG_SYNC_POSITION` gameplay action. |
| `game_loop/pulse_planner.rs` | Deterministic accumulated-deadline pulse planning. |

### `db-server`

| file | first doc line |
|---|---|
| `banword.rs` | Source-verified, SQL-free `banword` table loading. |
| `banword_sqlx.rs` | `SQLx` acquisition adapter for the source-fixed `banword` table. |
| `boot.rs` | Explicit, caller-owned boot response composition. |
| `boot_composition.rs` | Pure, profile-bound registration of already-loaded boot sections. |
| `boot_snapshot.rs` | SQL-free composition of a caller-resolved legacy DB boot snapshot. |
| `cache.rs` | Transport-free keyed cache core. |
| `event.rs` | Source-verified, SQL-free `event` table loading. |
| `event_sqlx.rs` | `SQLx` acquisition adapter for the source-fixed event boot table. |
| `item_attr.rs` | Pure, SQL-free conversion of the active legacy item-attribute tables. |
| `item_attr_sqlx.rs` | `SQLx` acquisition for the source-fixed `item_attr` and `item_attr_rare` |
| `item_proto.rs` | SQL-free query and conversion boundary for the active `item_proto` table. |
| `item_proto_sqlx.rs` | Bounded `SQLx` acquisition for the source-fixed active `item_proto` table. |
| `land.rs` | Pure, SQL-free conversion of the active legacy `land` query to a boot section. |
| `land_sqlx.rs` | `SQLx` acquisition adapter for the source-fixed `land` boot table. |
| `lib.rs` | Database server process support. |
| `login.rs` | SQL-free response framing for the primary login-by-key outcome. |
| `main.rs` | Metin2 database cache server binary |
| `market_price.rs` | Source-verified, SQL-free optional premium private-shop market-price loading. |
| `market_price_sqlx.rs` | `SQLx` acquisition adapter for the optional premium private-shop price table. |
| `mob_proto.rs` | SQL-free query and row-decoding boundary for the active `mob_proto` table. |
| `mob_proto_sqlx.rs` | Row-count-bounded `SQLx` acquisition for the source-fixed active `mob_proto` table. |
| `monarch.rs` | SQL-free acquisition and conversion for the legacy Monarch query. |
| `monarch_sqlx.rs` | Row-count-bounded `SQLx` acquisition for the source-fixed Monarch query. |
| `monarch_state.rs` | Immutable, transport-free boot state for Monarch values. |
| `object.rs` | Pure, SQL-free conversion of the source-fixed `object` boot table. |
| `object_proto.rs` | Pure, SQL-free conversion of the source-fixed `object_proto` boot table. |
| `object_proto_sqlx.rs` | `SQLx` acquisition adapter for the source-fixed `object_proto` boot table. |
| `object_sqlx.rs` | `SQLx` acquisition adapter for the source-fixed `object` boot table. |
| `peer_policy.rs` | Transport-free DB peer authentication and authorization policy. |
| `player.rs` | SQL-free primary player-load response composition. |
| `player_index.rs` | SQL-free `player_index` query and row boundary for login-by-key. |
| `player_index_sqlx.rs` | `SQLx` acquisition adapter for the source-fixed `player_index` query. |
| `postfix.rs` | Strict, SQL-free `TABLE_POSTFIX` and fixed boot-table query boundaries. |
| `quest.rs` | Source-verified, SQL-free quest-load query and row boundary. |
| `quest_sqlx.rs` | `SQLx` acquisition adapter for the source-verified quest-load boundary. |
| `refine.rs` | Pure, SQL-free conversion of refine query rows to a boot section. |
| `refine_sqlx.rs` | `SQLx` acquisition adapter for the source-fixed `refine_proto` boot table. |
| `renewal_shop.rs` | SQL-free conversion of the active legacy renewal-shop boot table. |
| `renewal_shop_sqlx.rs` | `SQLx` acquisition adapter for the source-verified renewal-shop boot table. |
| `service.rs` | Injected DB request execution boundary. |
| `session.rs` | Isolated DB peer session boundary. |
| `setup.rs` | Strict, SQL-free decoding of one legacy `HEADER_GD_SETUP` payload, plus the auth-mode base-only receive policy. The record layout and encoder live in `protocol/src/db_setup.rs`. |
| `shop.rs` | SQL-free base `TShopTable` boot boundary. |
| `shop_sqlx.rs` | `SQLx` acquisition adapter for the source-fixed base shop boot table. |
| `skill.rs` | SQL-free conversion of the active legacy `skill_proto` rows to a boot section. |
| `skill_sqlx.rs` | `SQLx` acquisition adapter for the source-fixed `skill` boot table. |

## M3: the legacy game server has no boot-ready gate, and four framing defects

Read-only audit of the game half of the DB peer link, for ledger section 172.
Every claim below was checked against the checked-in source with a positive
control, not inferred from names.

### The bootstrap sequence

`CLIENT_DESC::SetPhase(PHASE_DBCLIENT)` in
`server/server/game/desc_client.cpp:126-226` is the **only** place that writes
`HEADER_GD_BOOT` and `HEADER_GD_SETUP`. Both are appended in that one call,
boot first (`desc_client.cpp:141`) then setup (`desc_client.cpp:223`), and both
carry handle `0`. `ClientManager.cpp:1395,1405,1431` answers setup with
`HEADER_DG_MAP_LOCATIONS` then `HEADER_DG_P2P`, all at handle `0`. There is no
handle echo anywhere on the bootstrap path: `Boot` and `MapLocations` are not
even passed the handle, because `CInputDB::Process` drops the frame length at
`input_db.cpp:2273` and never passes the handle down either.

`CInputDB::Process` (`input_db.cpp:2250-2288`) frames
`[u8 header][u32 handle][u32 size]`, all little-endian with a nine-byte prefix.
This matches the Rust `db_wire` format exactly.

### Divergence 1: the boot-ready gate does not exist in legacy

This is the headline finding, and it is the one intentional behavioral
divergence in M3.

| fact | evidence |
|---|---|
| the client port binds before the DB connector is created | `main.cpp:671` binds, `main.cpp:697` creates `db_clientdesc` |
| `TryConnect()`'s result is discarded | `main.cpp:840-856` calls it as a statement |
| `AcceptDesc()` runs unconditionally every pass | `main.cpp:841`, `desc_manager.cpp:150-198` |
| no readiness flag exists anywhere | whole-tree search for `g_bBoot`, `BootComplete`, `BootFinish`, `bBooted`, `IsBoot`, `boot_ok`, `g_boot` returns **no output**; the positive control `g_bNoMoreClient` returns 3 hits |
| a new descriptor goes straight to handshake | `desc.cpp:242-243` sets `PHASE_HANDSHAKE`; `input.cpp:209-277` gates only on `guild_mark_server` and a dead branch |

So legacy accepts, handshakes, and logs players in while the DB link is still
retrying. `g_bNoMoreClient` is **not** a readiness flag: it is written only at
`main.cpp:449` and `cmd_general.cpp:229` and read only at `desc_client.cpp:306`.

The Rust `BootReadyGate` closes the gap and re-closes when the link drops.

### Divergence 2: `bSentBoot` means BOOT is never resent

`desc_client.cpp:132` guards the boot send with a function-local
`static bool bSentBoot`. There is no reset path anywhere in the tree. `GD_SETUP`
has no such guard, so **setup is resent on every reconnect and boot never
is**. After a DB restart the game keeps serving the tables from the original
boot and never re-requests the item-ID range. The Rust client resends both.

### Divergence 3: an unknown DB header is skipped, not fatal

`input_db.cpp:2273-2280` logs an unknown header and still consumes
`9 + iSize`, leaving the connection open and continuing to parse. The Rust
client closes the link instead.

### Defect: the frame length is narrowed to a signed `int`

`input_db.cpp:2255` declares `int iSize;` and `:2264` assigns
`*((DWORD*)(c_pData+5))`. A wire length at or above `0x8000_0000` becomes
negative, the guard at `:2268` passes, and `:2279-2281` move the pointer
**backward** while increasing the remaining byte count. That is an unbounded
loop and an out-of-bounds read on a trusted-peer link. The Rust side reads
`u32` and checks a payload limit before allocating.

### Defect: `map_allow_copy` writes one value too many

`server/server/game/config.cpp:274-287` stores the map index at line 282 and
only then tests the bound at line 284. The sole caller is `desc_client.cpp:158`
with `size = MAP_ALLOW_LIMIT = 32` writing into `TPacketGDSetup.alMaps[32]`,
which on the packed layout **is** `dwLoginCount` at offset 149. Thirty-three or
more map ids in `map_allow.txt` therefore silently corrupt the login count, and
`ClientManager.cpp:1436` then walks that many records off the end of the setup
payload. This is a legacy-server defect, not a codec defect. The Rust
`DbClientConfig::base` drops the extra entry.

### Audit correction: `input_db.h` and `game/db.h` do not say what the names suggest

- There is no `server/server/game/input_db.h`. `CInputDB` is declared in
  `server/server/game/input.h:179-275`.
- `server/server/game/db.h` is the **SQL** wrapper (`DBManager`, `AccountDB`),
  not the peer client. The game-to-DB peer is `CLIENT_DESC` in
  `game/desc_client.{h,cpp}`.
- There is no `CDBManager` anywhere in the tree. The DB server's own class is
  `CClientManager` in `server/server/db/ClientManager.cpp`.
- `HEADER_DG_BOOT` is 43 and `HEADER_DG_MAP_LOCATIONS` is `0xfe` (254). These
  were measured by compiling the verbatim anonymous enum from
  `common/tables.h:14-337` after the verbatim `common/prodomodefines.h`, not by
  summing.
- `0xfe` is also `HEADER_CG_PONG` (`game/packet.h:9`) and `HEADER_GC_BINDUDP`
  (`game/packet.h:99`). Three unrelated tables, one byte.

### Fourteen declared `DG` headers legacy never handles

These reach `default: return (-1)` at `input_db.cpp:2243`:
`DG_DIRECT_ENTER` 55, `DG_PARTY_HEAL_USE` 64, `DG_ITEM_ID_RANGE` 91,
`DG_GUILD_TIME_UPDATE` 99, `DG_ITEMSHOP` 76, `DG_BREAK_MARRIAGE` 159,
`DG_ELECT_MONARCH` 160, `DG_CANDIDACY` 161, `DG_COME_TO_VOTE` 164,
`DG_RMCANDIDACY` 165, `DG_SETMONARCH` 166, `DG_RMMONARCH` 167,
`DG_RESULT_CHARGE_CASH` 179, `DG_RANKGLOBAL_ADD_POINT` 200.

`DG_ITEMSHOP` (76) is **live**: `ENABLE_ITEMSHOP` is defined at
`prodomodefines.h:153` and the DB sends that frame at the end of every boot
(`db/ClientManager.cpp:639-641`). The Rust game client currently reports it as
an unknown header and closes the link, so that divergence is load-bearing: an
`ENABLE_ITEMSHOP` build profile must add it before the game client can be
pointed at a real DB server. The checked-in profile is empty-snapshot only.

### Not measured by the audit

All struct widths in the audit are hand sums for the 32-bit target
(`TMapLocation` 146, `TPacketGDBoot` 24, `TPacketGDSetup` 154, the 14 boot
sections). The Rust side does not rely on those sums: `TMapLocation` and the
boot section widths were already verified by i686 compilation in earlier
ledger sections, and `protocol::db_setup` re-exports the 154- and 100-byte
widths that the existing tested decoder had already fixed.

### Note on the client listener: the gate is a Rust-only concept

`main.cpp:841` calls `AcceptDesc()` on every pass of the main loop, and
`desc_manager.cpp:150-198` accepts unconditionally. There is no readiness flag
anywhere in the tree. The Rust process therefore differs in two ways, both
recorded in ledger 173:

1. `run_accept_loop` checks `boot_gate.admit()` **before** it spawns a handler
   task, so an unbooted server does not allocate a per-connection task for every
   arriving client;
2. the gate re-closes when the DB link drops, which legacy never does. Legacy
   keeps serving from the tables it loaded at the first boot, and because of
   `static bool bSentBoot` it never re-requests them.

The first client that arrives in the same instant boot completes is served
rather than dropped. The gate counts both outcomes so the refusal is visible in
the log instead of silent.

### `DbClientConfig` carries no peer list yet

`GD_SETUP` has a `dwLoginCount` plus an array of 100-byte
`TPacketLoginOnSetup` records describing every *other* connected login
(`desc_client.cpp:165-186`). The Rust `DbLinkConfig.logins` is empty because
there is no connected-peer registry, so the encoded count is zero. That is a
correct encoding of "no other peers" and a wrong picture of a live channel: once
several game servers share a DB, each must report the others, or the DB's
per-server login bookkeeping will be empty.


## Boot table loader audit (section 175)

A source-by-source check of every boot table against
`server/server/db/ClientManagerBoot.cpp` and `ClientManagerPrivateShop.cpp`.
The loader was already source-derived; this records what the check confirmed
and the one real divergence it found.

### Confirmed: all fourteen statements, the wire order, and the collection kinds

- Every statement matches character for character, including the legacy
  double space in `refine_proto` before `vnum3` and the `enable='YES'` filter
  on `land`. The consolidated check is
  `boot_loader::tests::every_boot_statement_matches_the_legacy_loader`.
- `refine_proto` has **no** `ORDER BY` in legacy, so the rows arrive in whatever
  order MySQL returns. Adding one would be a behavior change, not a cleanup.
- `event` is sorted by **`start`**, not by `id`.
- `item_attr_rare` selects fourteen columns, not the normal table's eighteen.
- `shop`, `shopex`, and `banword` take no `TABLE_POSTFIX`.
- The section order in `BootSectionKind` matches the `QUERY_BOOT` wire order,
  with `shopex` at position four, not at the end.
- Only `shop` and `object` are keyed collections in legacy. `shop` is a
  `std::map<int, ...>` so shops are grouped by vnum and emitted in ascending
  signed order; `object` is a `std::map<DWORD, ...>` keyed by `dwID`, so a
  duplicate id keeps the **first** row and the count is the map size, not the
  row count. Every other table is a `std::vector` and keeps source order and
  duplicates. The Rust `object` module already reproduces the map policy with
  `BTreeMap::entry(..).or_insert(..)`.
- The market price is averaged per vnum over its row group, summed with
  wrapping arithmetic, and emitted in ascending vnum order. Already correct.
- The empty active boot is 415 bytes and the empty minimal boot is 403. Both are
  already pinned by `boot_snapshot.rs` and both reconcile with the legacy
  `dwPacketSize` formula at `ClientManager.cpp:448-480`.

### Divergence: the mob and item tables are read from SQL, and legacy by default does not

This is the one real finding, and it is a **configuration** divergence rather
than a code defect.

`ClientManager.h:17` defines `ENABLE_PROTO_FROM_DB`, and `InitializeTables`
then branches on a **runtime** flag:

```cpp
// ClientManagerBoot.cpp:18-30
#ifdef ENABLE_PROTO_FROM_DB
    if (!(bIsProtoReadFromDB?InitializeMobTableFromDB():InitializeMobTable()))
#else
    if (!InitializeMobTable())
#endif
```

`bIsProtoReadFromDB` is initialized to `false` at `ClientManager.cpp:71` and is
only set when the config key `PROTO_FROM_DB` is present and non-zero
(`ClientManager.cpp:251-256`). So on a **default** legacy deployment:

- `mob_proto` and `item_proto` are read from the **text files** `mob_proto.txt`
  and `item_proto.txt` via `cCsvTable` (`ClientManagerBoot.cpp:220-298` and
  `522-654`), not from the database.
- Only `PROTO_FROM_DB=1` selects the SQL statements this rewrite implements.

The wire format is the same either way: `QUERY_BOOT` always sends
`sizeof(TMobTable) * m_vec_mobTable.size()` records, so the client cannot tell
which source was used. The **data** can differ, though, and silently: a
deployment whose text files and SQL tables have drifted will boot a different
world on each path.

The Rust `BootTableLoader` reads both tables from `SQL_PLAYER` unconditionally.
That is the only option available here, because **neither text file is checked
into this repository** and there is no Rust text-table reader. The consequence
to state plainly: this rewrite matches the legacy `PROTO_FROM_DB=1` behavior,
and a legacy deployment running the default `PROTO_FROM_DB=0` is being served
from SQL data instead of its text files. That is a deliberate, recorded
divergence, not an oversight, and it is not detectable from the wire.

`ENABLE_AUTODETECT_VNUMRANGE` is defined at `ClientManagerBoot.cpp:1372`, inside
the `ENABLE_PROTO_FROM_DB` block, so the `item_proto` statement selects 34
columns and **not** `vnum_range`. The Rust `ITEM_PROTO_QUERY_TEMPLATE` matches
that 34-column form. If a build ever drops that `#define`, the statement grows
a `vnum_range` column and the whole item table shifts by one cell.

### One mutation that was ineffective, and why it matters

Four statement mutations were planned to check the consolidated test. Three
were killed on the first run. The fourth, changing the `event` ordering from
`start` to `id`, **survived** — not because of a test gap but because the
replacement string matched a **doc comment** at `event.rs:7` before the
`EVENT_QUERY_SUFFIX` constant at line 42, so the mutation never reached the code.
Retargeting the constant killed it immediately.

The lesson generalizes: a text replacement that is expected to change behavior
must be confirmed to have landed on the *executable* text. Matching a comment
produces a green run that proves nothing.
