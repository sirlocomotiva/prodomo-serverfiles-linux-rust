-- The account's safebox premium expiry (`PREMIUM_SAFEBOX`, index 2, Ledger 735).
-- Legacy keeps it in `account` (`account.sql:47`): `safebox_expire`, a datetime
-- defaulting to `'0000-00-00 00:00:00'`. The auth query (`G/input_auth.cpp:139`)
-- reads `UNIX_TIMESTAMP(safebox_expire)` into `aiPremiumTimes[PREMIUM_SAFEBOX]`,
-- and `CInputDB::SafeboxLoad` (`G/input_db.cpp:1144-1147`) opens six pages
-- (`SAFEBOX_MAX_PAGE_COUNT * SAFEBOX_PAGE_SIZE = 54` rows) while it runs.
--
-- Stored as a non-negative unix-second count, like `exp_premium_expire`: 0 is
-- the never-bought case, already past like a legacy zero datetime.

ALTER TABLE account
ADD COLUMN safebox_expire bigint NOT NULL DEFAULT 0 CHECK (safebox_expire >= 0);
