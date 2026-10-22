-- A character's quest flags (ledger section 244).
--
-- Legacy keeps a quest's flags in the quest manager's own per-character record
-- (`quest::PC`, `G/questpc.h:98-149`): a map from a name to a value, and a flag whose
-- value is zero is no flag at all (`quest::PC::SetFlag`, `G/questpc.cpp:55-72`). A quest's
-- state is one of those flags: `SetCurrentQuestStateName` writes
-- `<quest>.__status` (`G/questpc.cpp:129-132`), and the load rebuilds the quest list from
-- the flags that end in `.__status` (`quest::PC::Build`, `G/questpc.cpp:664-679`).
--
-- The save writes the flags to the database as one row per flag
-- (`quest::PC::Save`, `G/questpc.cpp:519-590`): the character, the quest's name, the
-- flag's state and the value, split at the first dot. The load reads them back and sets
-- them again (`CInputDB::QuestLoad`, `G/input_db.cpp:1079-1101`), so a flag outlives the
-- session that set it.
--
-- Here the whole name is one column: the split the legacy does at the first dot is a
-- property of its table, not of the flag, and the name is what the flag answers to.

CREATE TABLE quest_flag (
    player_id integer NOT NULL REFERENCES player (id) ON DELETE CASCADE,
    -- The flag's whole name, the quest's name and its state (`quest::PC::m_FlagMap`). It is
    -- free text, so it is stored as the exact bytes (ADR-0003).
    name bytea NOT NULL,
    -- The flag's value (`TQuestTable::lValue`). A flag whose value is zero is no flag, so
    -- the value is never zero here.
    value integer NOT NULL CHECK (value <> 0),
    PRIMARY KEY (player_id, name)
);
