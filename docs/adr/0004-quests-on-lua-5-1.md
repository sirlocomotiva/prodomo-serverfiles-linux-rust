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

## Amendments

- **ADR-0006** refines this decision. 50 of the 51 scripts are live: `_basic/change_empire.lua`
  is not Lua, and legacy's own `qc` refuses it.
- **2026-11-05 (owner, ledger 401).** A quest API name that no Lua file of the Game data
  references is `unused`, not ported. It is ported when a quest needs it.
- **2026-11-15 (owner, ledger 680).** The owner had `_basic/change_empire.lua` fixed. The stray
  `end` on line 132, which closed the `if ret == 1` chain before its `elseif ret == 4`, is
  removed: the nine bytes `\t\t\t\tend\r\n` at offset 3368, and no other byte (3,590 bytes,
  sha256 `bcc3cf2c…c34230`, to 3,581 bytes, sha256 `09cde998…b80672`). All 51 scripts load and
  the quest runs. This is the one edit to a quest source, and it was the owner's to make: the
  rule stands for every other file, and a source that does not load is a question for the
  owner.
- **2026-11-15 (owner, ADR-0008, ledger 681).** The owner's rule that mistakes are fixed, not
  ported, reaches quest sources in one narrow form. A source may be edited when a mistake stops
  a script: it does not compile, or a path always raises before its end. The edit is the
  smallest that lets the path run, every other byte stays, `legacy/README.md` lists the file,
  and the ledger gives the bytes and both hashes, as the entry above does. What a working path
  says, gives or costs is never edited; that is content, and content is the owner's.
