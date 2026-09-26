# Quests run on Lua 5.1, not Lua 5.0.3

Legacy quests are compiled by `qc` (`server/server/quest/qc.cpp`) into Lua 5.0.3 chunks. `mlua`
cannot host Lua 5.0, and embedding the 5.0.3 C sources would need `unsafe` FFI, which the workspace
forbids. Quests therefore run on `mlua`'s Lua 5.1, the closest supported version, which keeps most
5.0 idioms. `qc` is ported to Rust, and quest sources stay unchanged.

## Considered options

- Lua 5.0.3 through FFI: exact semantics, but requires `unsafe`.
- Lua 5.4: quest sources would need editing.

## Consequences

- Every quest in the Game data (`legacy/gamedata/locale/europe/quest`: 51 quest scripts and 10 Lua
  library files) must compile and load, and all 51 scripts are live, not only the 16 in the
  snapshot's `quest_list`. Any 5.0 idiom that 5.1 lacks gets a compatibility shim,
  never an edit to a quest source.
- The FreeBSD `qc` output in `quest/object/` is the reference the Rust `qc` is checked against.
