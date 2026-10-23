-- The affects a relogin restores (ledger section 255).
--
-- Legacy keeps a player's timed affects in the `affect` table and restores them at
-- login (`LoadAffect`, `G/char_affect.cpp:573-666`): one row per `(dwPID, bType,
-- bApplyOn, lApplyValue)`, written by `QUERY_ADD_AFFECT`
-- (`server/server/db/ClientManagerPlayer.cpp:1563-1585`).
--
-- Here the table holds only the silence (`AFFECT_BLOCK_CHAT`, 223), the one affect
-- whose duration a relogin must keep: a silence survives death and its minutes are
-- the sentence, while the revive-invisible is no-save and the fight's poison, stun
-- and slow die with the moment. Each row carries its own full element, so the table
-- is ready when a later affect needs its row; a command that holds no affect the
-- table knows deletes that character's rows, as legacy's no-save rules do.

CREATE TABLE player_affect (
    player_id integer NOT NULL REFERENCES player (id) ON DELETE CASCADE,
    -- `bType`: the affect's kind, such as 223 for the silence.
    affect_type integer NOT NULL,
    -- `bApplyOn`: the point the affect applies to, 0 for none.
    apply_on integer NOT NULL,
    -- `lApplyValue`: what the affect added to its point.
    apply_value integer NOT NULL,
    -- `dwFlag`: the affect flag it sets, numbered from 1, or 0 for none.
    flag integer NOT NULL,
    -- `lDuration`: the remaining ticks, which a restore loads unchanged.
    duration integer NOT NULL,
    -- `lSPCost`: the spell points each tick spends.
    sp_cost integer NOT NULL,
    PRIMARY KEY (player_id, affect_type, apply_on, apply_value)
);
