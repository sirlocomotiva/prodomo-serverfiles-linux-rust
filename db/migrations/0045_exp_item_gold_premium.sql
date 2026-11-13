-- The account's EXP, ITEM and GOLD premium expiries (ledger section 651).
-- Legacy keeps them in `account` (`account.sql:45-46, 51`): `silver_expire`,
-- `gold_expire` and `money_drop_rate_expire`, datetimes defaulting to
-- `2025-01-01 22:00:00`. The names lie: the auth query reads
-- `UNIX_TIMESTAMP(silver_expire)` into `aiPremiumTimes[PREMIUM_EXP]`,
-- `UNIX_TIMESTAMP(gold_expire)` into `aiPremiumTimes[PREMIUM_ITEM]`, and
-- `UNIX_TIMESTAMP(money_drop_rate_expire)` into `aiPremiumTimes[PREMIUM_GOLD]`
-- (`G/input_auth.cpp:137-143`, in the db row order `G/db.cpp:347-353`), and
-- `GetPremiumRemainSeconds` answers `expiry - get_global_time()`
-- (`G/char.cpp:8091-8097`). Here they are unix times named for what they hold,
-- like `fish_mind_expire`: 0 is the never-bought case, already past like a
-- stale legacy default.
ALTER TABLE account
ADD COLUMN exp_premium_expire bigint NOT NULL DEFAULT 0 CHECK (exp_premium_expire >= 0),
ADD COLUMN item_premium_expire bigint NOT NULL DEFAULT 0 CHECK (item_premium_expire >= 0),
ADD COLUMN gold_premium_expire bigint NOT NULL DEFAULT 0 CHECK (gold_premium_expire >= 0);
