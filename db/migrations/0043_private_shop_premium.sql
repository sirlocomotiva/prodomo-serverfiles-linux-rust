-- The account's premium private-shop expiry (ledger section 587).
--
-- Legacy keeps it in `account` (`account.sql:52`): `premium_privateshop_expire`,
-- a datetime defaulting to `CURRENT_TIMESTAMP`. The auth query reads it as
-- `UNIX_TIMESTAMP(...)` into `aiPremiumTimes[PREMIUM_PRIVATE_SHOP]`
-- (`G/input_auth.cpp:145`), and the 507 use rewrites it to
-- `duration + time(0)` (`D/ClientManagerPrivateShop.cpp:2345`). Here it is a
-- unix time, like `private_shop.premium_time`: the arithmetic on both sides is
-- `expiry - now` in seconds. 0 is the never-bought case, already past like a
-- fresh legacy `CURRENT_TIMESTAMP`.
ALTER TABLE account
ADD COLUMN private_shop_premium_expire bigint NOT NULL DEFAULT 0 CHECK (private_shop_premium_expire >= 0);
