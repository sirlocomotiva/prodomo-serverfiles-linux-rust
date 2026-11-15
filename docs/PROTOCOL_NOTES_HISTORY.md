# Protocol notes: history

This page holds the parts of `docs/PROTOCOL_NOTES.md` that describe code and protocols the Rewrite
no longer has. They were moved here unchanged by ledger 682 (2026-11-15) and are not updated.
Nothing on this page is a rule or a description of today's tree:

- ADR-0001 (ledger 176) retired the DB-peer protocol and the game-to-game protocol. The DB records,
  the boot stream, `db-server`, M3 and the boot table loader audit below are about them.
- The module map and the module index are snapshots of 2026-09-26, from before ledgers 177 and 178.
  They name `game-server`, `db-server` and modules that were deleted; today's modules are in the
  crates' own `//!` docs.
- The coverage history and the mutation evidence are receipts of ledgers 162 to 168.

The client-side findings and the verification lessons that still apply are in
`docs/PROTOCOL_NOTES.md`; the rules are in `AGENTS.md`.

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
- `cg_whisper` covers header 19 `HEADER_CG_WHISPER` (0x13), the whisper request: a header, an explicit little-endian size word that counts the whole record, the raw 25-byte name field, and the raw text that follows it. The size is a floor and not an equality, so trailing bytes past it are legal and ignored, and a size under the 28-byte fixed part is its own refusal because the legacy closes the descriptor for it (`G/input_main.cpp:344-353`). The name and the text are C strings, so each stops at its first NUL, and the text is bounded by `CHAT_MAX_LEN` (512) as the legacy's `char buf[CHAT_MAX_LEN + 1]` is.
- `gc_whisper` covers header 34 `HEADER_GC_WHISPER` (0x22), the whisper answer: a header, an explicit little-endian size word that counts the whole record, the type byte, the raw 25-byte name field, the `bCanFormat` byte that `LOCALE_STRING_RENEWAL` adds (`G/input_main.cpp:631`), and the raw text that follows. The fixed part is 30 packed bytes, measured from a verbatim 32-bit compile under `#pragma pack(1)` with the repository's known-width controls; the `bool` is one byte, so the field is the legacy's own.
- `cg_messenger` covers header 67 `HEADER_CG_MESSENGER` (0x43), the friend add and remove: the 2-byte base (`TPacketCGMessenger`, `packet.h:1944-1977`), 4 more VID bytes for `ADD_BY_VID` (0), and a raw 24-byte name field for `ADD_BY_NAME` (1) and `REMOVE` (2); `INVITE_ANSWER` (3) and every unknown subheader consume the base alone, the analyzer's own default arm (`G/input_main.cpp:1225-1228`). The name arms read 24 bytes where the `REMOVE` struct carries 31 that legacy never reads, and the names are C strings read to the first NUL.
- `gc_messenger` covers header 74 `HEADER_GC_MESSENGER` (0x4a), the friend and team lists with the login, logout and remove-friend records: the 4-byte envelope whose size word counts it (`TPacketGCMessenger`), one `connected + length + name` entry per companion with no terminator, and the login, logout and `REMOVE_FRIEND` records as an envelope with a length byte and the name. The `INVITE` subheader (6) is never sent, so the codec declares the byte and produces no record of it.
- `gc_quest_info` covers header 81 `HEADER_GC_QUEST_INFO` (0x51), the quest window's record: the 8-byte `packet_quest_info` (with `c_index`, `__QUEST_RENEWAL__`), then one part per `flag` bit in bit order, with the whole record's size and zero-filled name fields where legacy writes 8 and uninitialised bytes (ledger 258).
- `cg_party_skill` covers the exact six-byte `TPacketCGPartyUseSkill` record.
- `cg_party` covers the five fixed-width party records of headers 72, 73, 74, 75, and 78, with explicit little-endian words, one shared error type, and separately named record and frame-payload widths so a wrong-width frame is rejected rather than misread; `cg_quickslot_add`, `cg_quickslot_del`, and `cg_quickslot_swap` cover the three quickslot records of headers 16, 17, and 18, where the add record is a counted run and the del/swap pair differ only in their declared slot byte.
- `cg_pick` covers the four fixed-width game-phase records that carry a single client choice and nothing else, at headers 31, 61, 66, and 114, where `CgQuestConfirm` is the only one with two fields and the only one whose `u8` precedes its `u32`, and where the three single-word records share one width and are therefore separated by their header rather than their length.
- `cg_login` covers the three CG records the handshake and login phases dispatch, at headers 206, 90, and 254, including two header-only records (`CgStateChecker`, header 206, and `CgPong`, header 254) and the one-byte `empire` selection of `CgEmpire`, header 90, which is range-checked by legacy session policy and so stays opaque across all 256 values; `CgStateChecker` is dispatched by the handshake phase's `else if` chain at `input.cpp:227` rather than a `case`, and `CgPong` is the only CG record dispatched in every client phase (handshake, auth, login, and game).
- `cg_micro` covers the four tiny game-phase records of headers 28, 29, 65, and 82, including the first zero-payload record (`CgWarp`, header 65) and the opaque single bytes of Position, ScriptAnswer, and Fishing.
- `cg_mark` covers the three fixed login-phase guild mark records of headers 100, 104, and 113, including the second header-only record (`CgMarkIdxList`, header 104) and the three client-asserted opaque words of the symbol CRC, whose server and client enumerators are named differently for the same value 113.
- `gc_mark` covers the three dynamic game-to-client guild mark records of headers 100, 102, and 133 (`TPacketGCMarkIDXList`, `TPacketGCMarkBlock`, `TPacketGCGuildSymbolData`). They are implemented but deliberately outside `gc_inventory`'s 134 rows: the mark downloader decodes them on its own path, not through the client's `PythonNetworkStream` decode table.
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

> **Corrected in ledger 184.** The three surplus `LOGIN3` bytes do not break the session. They are
  the upper bytes of the client's `DWORD` language, which is below 256, so each is a zero, and
  `CInputProcessor::Process` takes header 0 as a one-byte frame before it consults the packet-info
  table (`input.cpp:81-82`). The server consumes 66 bytes and then skips three. The Rewrite's
  framing has the same arm, and the auth scenario sends the client's 69 bytes. Section 162 also
  said `bLanguage` was never read; the `QID_AUTH_LOGIN` result refuses 0 (`NOLANG`) and 12 and up
  (`INVLANG`), and stores the rest as the account's language.

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

## DB records and boot

- Select a DB boot feature profile explicitly. Keep ordinary table records opaque until feature-dependent C++ record widths are verified; `TBanwordTable`, `TRefineTable`, optional `TEventTable`, optional `TMarketItemPrice`, and `TPacketUpdateHorseName` are source-fixed typed exceptions. `ENABLE_ITEMSHOP` data is a separate DB frame. `BootSnapshot` is caller-resolved and profile-bound; it must not query SQL, infer rows, or reuse request-specific hosts/admins. Horse-name lookups are injected and must not invent SQL results; missing rows are 25 zero bytes. Channel lookup is injected too: zero selectors are silent, valid no-match results are six zero bytes, and matches echo the request handle. Login-by-key and primary player-load adapters frame only already-resolved outcomes; they must not infer missing results, execute SQL, or mutate login state. The separate `player_index` boundary builds only the source-fixed login-by-key index query, preserves SQL NULLs, rejects out-of-range empire values, and its SQLx adapter uses a one-row bounded stream without implementing the legacy missing-row repair path. The primary login success payload is 362 bytes, the missing/invalid payload is empty, and an already-logged-in result preserves the caller's exact raw 31-byte record.
- *(From README.md.)* Select a DB boot feature profile explicitly. Keep ordinary table records opaque until their feature-dependent C++ record widths are verified; the source-fixed `TBanwordTable`, `TRefineTable`, optional `TEventTable`, optional `TMarketItemPrice`, and `TPacketUpdateHorseName` boundaries are the typed exceptions. `ENABLE_ITEMSHOP` data is a separate DB frame. Horse-name lookups must be injected; a missing row produces 25 zero bytes, while returned bytes are copied exactly. Channel lookup is also injected: a zero map/channel selector produces no frame, a valid request without a matching peer produces an all-zero six-byte result, and a match echoes the incoming DB handle. Login-by-key and primary player-load services are framing boundaries only: they must not infer missing results, execute SQL, or mutate login state. A resolved login success is 362 bytes, a missing/invalid result is empty, and an already-logged-in result preserves the caller-supplied raw 31-byte record; header 33 is not a primary key-login outcome. DB-peer identity must come from an external, generation-bound verification; the setup `bAuthServer` byte, frame handle, source prefix, login key, and account/player IDs are not authentication. The current policy authorizes only an explicitly verified `Auth`-role, base-only setup and emits an effect for caller application; it does not replace the legacy auth-pointer overwrite or authorize live setup.
- *(From README.md.)* `db-server::postfix` is the required allowlist before interpolating `TABLE_POSTFIX` into SQL. `db-server::player_index` builds the fixed login-by-key index query and rejects SQL NULL/out-of-range values; `db-server::player_index_sqlx` acquires at most one row, preserves NULLs, and keeps database failures distinct from an empty result. `db-server::banword_sqlx` accepts only the fixed banword query, preserves raw bytes and NULLs, and uses the pool's bounded retry-preserving stream. `db-server::quest`/`quest_sqlx` preserve the two exact quest query variants and the count-prefixed 106-byte response boundary. `db-server::event`/`event_sqlx` preserve the exact seven-column event query, positional timestamp extraction, raw type bytes, strict NULL/numeric handling, and bounded 85-byte sections without becoming a live cache. `db-server::item_attr`/`item_attr_sqlx` preserve the exact 18-column normal and 16-column rare item-attribute queries, raw NULL/byte cells, source order and duplicates, strict versus explicitly named legacy conversion, and the active 71-byte section boundary without becoming a live cache. `db-server::boot_snapshot` is caller-resolved and profile-bound; it does not query tables or choose a feature profile from bytes. `ENABLE_ITEMSHOP` stays in its separate frame.
- Validate `TABLE_POSTFIX` through `db-server::postfix` before any future SQL identifier interpolation. The postfix/query module is SQL-free and must not be described as a live loader. The separate `db-server::banword_sqlx` adapter accepts only the fixed banword query, preserves raw bytes and NULLs, and uses bounded retry-preserving streaming; it is not a live cache or boot service.
- The `db-server::event` and `db-server::event_sqlx` boundaries preserve the exact seven-column `event%s` query, positional `UNIX_TIMESTAMP` extraction, raw `type` bytes, strict numeric/NULL handling, source order, and bounded 85-byte sections. They are acquisition and encoding boundaries only; they do not select a boot profile, build a cache, compose a snapshot, or provide a live event service.

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
| `cg_messenger.rs` | The `HEADER_CG_MESSENGER` (0x43) friend add and remove records. |
| `cg_micro.rs` | Transport-free codecs for four tiny fixed-width CG game-phase records. |
| `cg_move.rs` | Explicit codec for the fixed legacy `TPacketCGMove` record. |
| `cg_name.rs` | Transport-free codecs for the three CG records that carry fixed `char[N+1]` strings. |
| `cg_use_skill.rs` | Explicit codec for the fixed legacy `TPacketCGUseSkill` record. |
| `cg_variable.rs` | Incremental framing for source-verified variable-length client packets. |
| `cg_vid.rs` | The two 5-byte records that are a header plus a 32-bit virtual id. |
| `cg_whisper.rs` | Transport-free codec for the legacy `TPacketCGWhisper` record. |
| `cg_wire.rs` | Safe framing for the legacy client-to-game stream. |
| `db_boot.rs` | Safe parser for the legacy version-6 database boot payload. |
| `db_market_price.rs` | Source-fixed premium-market item-price update codec. |
| `db_records.rs` | Legacy DB record codecs for the read-only login path. |
| `db_wire.rs` | Legacy DB peer framing. |
| `gc.rs` | Source-verified game-to-client authentication, roster, player-creation and |
| `gc_actors.rs` | The game-to-client actor, target, and main-character records. |
| `gc_fields.rs` | The game-to-client records that carry named scalar fields and raw fixed |
| `gc_inventory.rs` | Machine-readable inventory of the legacy game-to-client packet headers. |
| `gc_messenger.rs` | The `HEADER_GC_MESSENGER` (0x4a) friend list, login, logout and remove records. |
| `gc_nested.rs` | The game-to-client records that nest another record or carry a fixed array. |
| `gc_quest_info.rs` | `HEADER_GC_QUEST_INFO` (0x51): what the client's quest window shows of one quest. |
| `gc_small.rs` | The small fixed-width game-to-client records. |
| `gc_vid.rs` | The game-to-client records that carry one 32-bit value. |
| `gc_whisper.rs` | `HEADER_GC_WHISPER` (0x22): a private line, from one player to another. |
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
