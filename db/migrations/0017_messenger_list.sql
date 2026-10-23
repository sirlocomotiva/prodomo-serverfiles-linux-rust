-- The messenger list (ledger section 248).
--
-- Legacy keeps one `messenger_list` table (`legacy/sql/schema/player.sql:593-607`): the
-- pair of the character names, one row per direction of a friendship
-- (`MessengerManager::AuthToAdd` calls `AddToList` twice, `G/messenger_manager.cpp:330-332`,
-- and each call inserts its own row, `G/messenger_manager.cpp:348-350`). The Rewrite keeps
-- the same rows, keyed by the same pair: the world loads one account's rows at its login
-- (`MessengerManager::Login`, `G/messenger_manager.cpp:12`), adds the pair when a request
-- is answered (`AuthToAdd` `:330-332`), and deletes both rows of the pair when a friend is
-- removed (`__FIX_DELETE_FRIEND_REFRESH__`'s own DELETE,
-- `G/messenger_manager.cpp:447-448`) or the character is deleted
-- (`ClientManagerPlayer.cpp:1541`).
--
-- Legacy's columns are `varchar(16)` with a comment that says '24 at maximum'. The stored
-- facts are the served facts here too, so the shape is the ported Name rule: 2 to 24 ASCII
-- letters and digits, the rule the world's own Name type carries and the other tables
-- check. The index `messenger_list_inverse` backs the cross-channel fan's inverse lookup,
-- which legacy walked in memory only.

CREATE TABLE messenger_list (
    account text NOT NULL CHECK (account ~ '^[A-Za-z0-9]{2,24}$'),
    companion text NOT NULL CHECK (companion ~ '^[A-Za-z0-9]{2,24}$'),
    PRIMARY KEY (account, companion)
);

CREATE INDEX messenger_list_inverse ON messenger_list (companion);
