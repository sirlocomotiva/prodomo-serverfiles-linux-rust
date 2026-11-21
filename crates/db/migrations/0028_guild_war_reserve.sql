-- The guild-war reservations and bets (ledger section 408).
--
-- Legacy keeps `guild_war_reservation` and `guild_war_bet`
-- (`legacy/sql/schema/player.sql:229-269`): a battle or flag accept inserts the
-- reservation (`db/GuildManager.cpp:1062-1068`), bets land beside it
-- (`:1286-1293`), and the boot loads the unstarted rows and the started ones with
-- no winner yet (`:896-897`), marking every loaded row unstarted. The Rewrite keeps
-- the same two tables with the same columns the boot reads, because the stored facts
-- are the served facts: the world loads the same rows at boot and serves the bet
-- and observer chats from memory.
--
-- The poly configs (`POLY_POWER`, `POLY_HANDICAP`) are absent from the owner's
-- deployment (`legacy/config/db/conf.txt` names no `POLY_*` key), so the power and
-- handicap evaluate to 0 on it: `CPoly::Eval` on an empty token base returns
-- `save[iSp-1]` with `iSp` 0 — an out-of-bounds stack read — whose value the reserve
-- then stores. A Defect: the bounds read is undefined, so the Rewrite stores 0,
-- which is what the row holds when the stack happens to be zeroed.

CREATE TABLE guild_war_reservation (
    id serial PRIMARY KEY,
    -- `guild1`, `guild2`: the pair, sorted low first (`db/GuildManager.cpp:991-992`).
    guild1 integer NOT NULL REFERENCES guild (id) ON DELETE CASCADE,
    guild2 integer NOT NULL REFERENCES guild (id) ON DELETE CASCADE,
    CHECK (guild1 < guild2),
    -- `time`: when the war starts, 180 seconds after the accept on europe
    -- (`GetGuildWarReserveSeconds`, `db/GuildManager.cpp:42-48`).
    starts_at integer NOT NULL CHECK (starts_at >= 0),
    -- `type`: battle or flag (`GUILD_WAR_TYPE_BATTLE`, `GUILD_WAR_TYPE_FLAG`).
    war_type smallint NOT NULL CHECK (war_type IN (1, 2)),
    -- `warprice`, `initscore`: the type's price and initial score
    -- (`KOR_aGuildWarInfo`, `G/guild_war.cpp:332-343`).
    war_price integer NOT NULL DEFAULT 0,
    initial_score integer NOT NULL DEFAULT 0,
    -- `started`: the countdown's end flips it (`db/GuildManager.cpp:1107`).
    started boolean NOT NULL DEFAULT false,
    -- `bet_from`, `bet_to`: the bet totals (`:1304-1309`).
    bet_from integer NOT NULL DEFAULT 0 CHECK (bet_from >= 0),
    bet_to integer NOT NULL DEFAULT 0 CHECK (bet_to >= 0),
    -- `winner`, `result1`, `result2`: the war's end writes them (`:1409`).
    winner integer NOT NULL DEFAULT -1,
    result1 integer NOT NULL DEFAULT 0,
    result2 integer NOT NULL DEFAULT 0,
    -- `power1`, `power2`, `handicap`: the poly evaluations, 0 on the owner's
    -- deployment (see above).
    power1 integer NOT NULL DEFAULT 0,
    power2 integer NOT NULL DEFAULT 0,
    handicap integer NOT NULL DEFAULT 0
);

-- One bet per login per war (`PRIMARY KEY (war_id, login)` on the legacy table).
CREATE TABLE guild_war_bet (
    war_id integer NOT NULL REFERENCES guild_war_reservation (id) ON DELETE CASCADE,
    -- The bettor's account login, 2 to 16 bytes (`account.login`).
    login text NOT NULL CHECK (char_length(login) BETWEEN 2 AND 16),
    -- `gold`: the stake.
    gold integer NOT NULL CHECK (gold >= 0),
    -- `guild`: the backed guild, one of the reservation's pair.
    guild integer NOT NULL REFERENCES guild (id) ON DELETE CASCADE,
    PRIMARY KEY (war_id, login)
);
