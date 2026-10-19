-- The guilds (ledger section 230n).
--
-- Legacy keeps three tables (`guild`, `guild_grade`, `guild_member`, in
-- `legacy/sql/schema/player.sql:139-211`): the guild row with its skills blob, the 15 grade
-- rows, and the member rows keyed by player id. The Rewrite keeps the same three tables,
-- because the stored facts are the served facts: the world loads every guild at boot
-- (`CGuildManager::Initialize`, `G/guild_manager.cpp:212-243`) and serves creation,
-- membership and the member lists from memory.
--
-- The dungeon columns (`dungeon_ch`, `dungeon_map`, `dungeon_cooldown`) are not stored:
-- `__DUNGEON_FOR_GUILD__` is Meley's Lair (`sys.guild.dungeon`), which is not ported.
-- The name rule is the ported one: 2 to 12 ASCII letters and digits
-- (`check_name_alphabet` on the europe Locale,
-- `G/locale_service.cpp:317-337,1156`), unique regardless of case, so the check and the
-- unique index agree about what a name is.

CREATE TABLE guild (
    id serial PRIMARY KEY,
    -- `TGuildData::name`: the 12 legacy bytes, NUL-padded on the wire.
    name text NOT NULL CHECK (name ~ '^[A-Za-z0-9]{2,12}$'),
    -- `TGuildData::master_pid`: the leader's `player.id`.
    master_pid integer NOT NULL REFERENCES player (id) ON DELETE RESTRICT,
    -- `TGuildData::power`, read from the legacy `sp` column (`G/guild.cpp:624`).
    sp integer NOT NULL DEFAULT 1000 CHECK (sp >= 0),
    -- `TGuildData::level`, starting at 1.
    level smallint NOT NULL DEFAULT 1 CHECK (level >= 1),
    -- `TGuildData::exp`.
    exp integer NOT NULL DEFAULT 0 CHECK (exp >= 0),
    -- `TGuildData::skill_point`.
    skill_point smallint NOT NULL DEFAULT 0 CHECK (skill_point >= 0),
    -- `TGuildData::abySkill`: the 12 skill bytes (`GUILD_SKILL_COUNT`, `G/skill.h:200`).
    skills bytea NOT NULL DEFAULT '\x000000000000000000000000' CHECK (length(skills) = 12),
    -- `TGuildData::ladder_point`: `LoadGuildData` clamps a negative row to 0
    -- (`G/guild.cpp:624-628`), so the column never holds one.
    ladder_point integer NOT NULL DEFAULT 0 CHECK (ladder_point >= 0),
    -- `TGuildData::win`, `draw`, `loss`.
    win integer NOT NULL DEFAULT 0,
    draw integer NOT NULL DEFAULT 0,
    loss integer NOT NULL DEFAULT 0,
    -- `TGuildData::gold`.
    gold integer NOT NULL DEFAULT 0
);

-- The duplicate-name judgement (`CGuildManager::CreateGuild`, `G/guild_manager.cpp:95-113`)
-- reads case-insensitively: legacy's `latin1` `name = '...'` comparison folds case, and so
-- does this index.
CREATE UNIQUE INDEX guild_name_lower ON guild (lower(name));

CREATE TABLE guild_grade (
    guild_id integer NOT NULL REFERENCES guild (id) ON DELETE CASCADE,
    -- The grade, 1 to 15 (`GUILD_GRADE_COUNT`, `G/guild.h:11`).
    grade smallint NOT NULL CHECK (grade BETWEEN 1 AND 15),
    -- `TGuildGrade::grade_name`: the legacy row is `varchar(12)` but the load copies 9
    -- (`GUILD_GRADE_NAME_MAX_LEN + 1`, `G/guild.h:10`) into the packet struct, so only 9
    -- bytes ever reach the wire.
    name text NOT NULL CHECK (char_length(name) <= 9),
    -- `TGuildGrade::auth_flag`: the four `GUILD_AUTH_*` bits (`G/guild.h:95-100`).
    auth smallint NOT NULL DEFAULT 0 CHECK (auth BETWEEN 0 AND 15),
    PRIMARY KEY (guild_id, grade)
);

CREATE TABLE guild_member (
    -- `TGuildMember::pid`: one guild per player (`UNIQUE KEY pid` on the legacy table).
    pid integer NOT NULL PRIMARY KEY REFERENCES player (id) ON DELETE CASCADE,
    guild_id integer NOT NULL REFERENCES guild (id) ON DELETE CASCADE,
    -- `TGuildMember::grade`: 1 to 15.
    grade smallint NOT NULL CHECK (grade BETWEEN 1 AND 15),
    -- `TGuildMember::is_general`.
    is_general boolean NOT NULL DEFAULT false,
    -- `TGuildMember::offer_exp`.
    offer integer NOT NULL DEFAULT 0 CHECK (offer >= 0)
);

CREATE INDEX guild_member_guild ON guild_member (guild_id);
