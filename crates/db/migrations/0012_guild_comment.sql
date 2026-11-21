-- The guild comment boards (ledger section 230q).
--
-- Legacy keeps one `guild_comment` table (`legacy/sql/schema/player.sql:161-178`): the
-- row id, the guild, the poster's name, the notice flag (the content's first byte is
-- `!`, `G/guild.cpp:1067-1068`), the 50 content bytes, and the post time. The Rewrite
-- keeps the same rows, keyed by the same id: `RefreshCommentForce` orders `notice DESC,
-- id DESC` and takes 12 (`G/guild.cpp:1104`), and the delete names the id
-- (`G/guild.cpp:1082-1086`).
--
-- The name rides the wire's 25 field bytes and the content its 51, as the load holds
-- them; the checks agree about both. The post time is kept but never served: no record
-- carries it, and no gate reads it.

CREATE TABLE guild_comment (
    id serial PRIMARY KEY,
    -- The guild the board belongs to.
    guild_id integer NOT NULL REFERENCES guild (id) ON DELETE CASCADE,
    -- The poster's name: the 24 legacy bytes (`varchar(24)`).
    name text NOT NULL CHECK (char_length(name) <= 24),
    -- Whether the content opened with `!` (`G/guild.cpp:1068`).
    notice boolean NOT NULL DEFAULT false,
    -- The 50 content bytes (`GUILD_COMMENT_MAX_LEN`, `G/guild.h:13`).
    content text NOT NULL CHECK (char_length(content) BETWEEN 1 AND 50),
    -- The post time (`NOW()` at the insert, `G/guild.cpp:1068`).
    posted_at timestamptz NOT NULL DEFAULT now()
);

-- `RefreshCommentForce`'s order (`notice DESC, id DESC` on one guild, `G/guild.cpp:1104`).
CREATE INDEX guild_comment_board ON guild_comment (guild_id, notice DESC, id DESC);
