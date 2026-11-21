-- A character's skill levels (ledger section 230f).
--
-- Legacy keeps them in `TPlayerTable::skills[SKILL_MAX_NUM]` (`common/tables.h:612`), 255
-- packed `TPlayerSkill { BYTE bMasterType; BYTE bLevel; time_t tNextRead; }` records, and
-- writes the array as the `skills` blob of the player row. Here each nonzero slot is a row,
-- and a zero slot is no row, because legacy's empty slot is the zeroed record and the
-- loading burst sends the same zeros either way. The master type is derived from the level
-- (`SetSkillLevel`'s ladder, `G/char_skill.cpp:206-213`), so only the level and the read
-- time are stored.

CREATE TABLE player_skill (
    player_id integer NOT NULL REFERENCES player (id) ON DELETE CASCADE,
    -- The skill's vnum, below `SKILL_MAX_NUM` (255, `common/length.h:72`).
    vnum smallint NOT NULL CHECK (vnum BETWEEN 0 AND 254),
    -- `bLevel`, at most `SKILL_MAX_LEVEL` (40, `common/length.h:75`).
    level smallint NOT NULL CHECK (level BETWEEN 1 AND 40),
    -- `tNextRead`, a 32-bit `time_t`.
    next_read integer NOT NULL DEFAULT 0,
    PRIMARY KEY (player_id, vnum)
);
