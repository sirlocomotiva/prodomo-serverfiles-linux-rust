-- The unlocked shop slots a character without a stand grows (ledger section 497).
--
-- Legacy keeps it in `TPlayerTable.wPrivateShopUnlockedSlot`, a word the login
-- query (`D/ClientManagerPlayer.cpp:227,802`) loads whole and `CHARACTER::Save`
-- (`G/char.cpp:1632`) writes from `POINT_PRIVATE_SHOP_UNLOCKED_SLOT` (210). The
-- unlock refuses past `PRIVATE_SHOP_LOCKED_SLOT_MAX_NUM` (64, the window's second
-- half, `common/length.h:137`); a fresh row holds none.
ALTER TABLE player
ADD COLUMN private_shop_unlocked_slot smallint NOT NULL DEFAULT 0 CHECK (private_shop_unlocked_slot BETWEEN 0 AND 64);
