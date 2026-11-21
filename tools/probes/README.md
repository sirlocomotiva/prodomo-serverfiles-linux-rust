# Width and enumeration probes

Each file here compiles legacy struct or enum bodies copied verbatim from `server/server`, so the
compiler gives a packed width or an enumerator's value instead of a hand sum. `AGENTS.md`
("VERIFICATION AND SEARCH RULES") has the rules a probe must follow. The directory name is the
ledger section that wrote the probe.

| probe | measures | cited by |
|---|---|---|
| `187/root-probe/probe.cpp` | the chat, entity, channel, time, gold and skill-level records; `mutant.cpp` is its negative control (a wrong width must fail to compile) | `crates/protocol/src/gc_chat.rs`, `gc_entity.rs`, `gc_npc_position.rs` |
| `187/point_enum.cpp` | `EPointTypes` as the compiler numbers it | `crates/common/src/point_slot.rs` |
| `187/widths.cpp`, `187/probe.cpp`, `187/probe/` | the working probes of ledger 187: the login, character-add, main-character, points and land-list records | ledger 187 |
| `188/chat_enum.cpp` | `EChatType` as the compiler numbers it | ledger 188 |
| `193/width.cpp` | the item-window records, on the host with a 4-byte stand-in for `long` | ledger 193 |
| `194/slot_space.cpp` | the `EMisc2` inventory slot space and the `EWindows` numbering | `crates/common/src/item_slots.rs` |
| `475/land.cpp` | `TPacketGCLandList` and its element | `crates/protocol/src/gc_land_list.rs` |
| `738/gc_widths.cpp` | all 145 game-to-client record headers and dynamic sub-structs compiled directly from `server/server/game/packet.h` for i686 (Package C8) | `crates/protocol/src/gc_inventory.rs` (`probed_i686_widths_match_every_rust_codec`) |

The probes up to 475 were built with `g++ -m32` or on the host. A new probe is built for the legacy
target with the cross compiler (installed since ledger 680):

```bash
mkdir -p .tmp && i686-linux-gnu-g++-12 -static -o .tmp/probe \
  tools/probes/NNN/name.cpp && .tmp/probe
```

Every run must also measure a packed two-`long` struct, which must be 8 bytes, and at least one
width that is already known. The binary goes to the ignored `.tmp/`, not to `/tmp`; only
the `.cpp` files and this page are tracked.

Package C8 of `docs/STATUS.md` probes every game-to-client record the Rewrite sends; its probe goes
in a directory named after its ledger section.
