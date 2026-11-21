-- The account's fish-mind premium expiry (ledger section 650).
--
-- Legacy keeps it in `account` (`account.sql:49`): `fish_mind_expire`, a datetime
-- defaulting to `2025-01-01 22:00:00`. The auth query reads it as
-- `UNIX_TIMESTAMP(...)` into `aiPremiumTimes[PREMIUM_FISH_MIND]`
-- (`G/input_auth.cpp:141`), and `GetPremiumRemainSeconds` answers
-- `expiry - get_global_time()` (`G/char.cpp:8091-8097`). Here it is a unix time,
-- like `private_shop_premium_expire`: 0 is the never-bought case, already past
-- like a stale legacy default.
ALTER TABLE account
ADD COLUMN fish_mind_expire bigint NOT NULL DEFAULT 0 CHECK (fish_mind_expire >= 0);
