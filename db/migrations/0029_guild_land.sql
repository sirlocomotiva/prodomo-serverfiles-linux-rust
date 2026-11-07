-- The guild land owners (ledger section 416).
--
-- Legacy's `land.guild_id` (`legacy/sql/schema/player.sql`), which `CLand::SetOwner`
-- writes through `HEADER_GD_UPDATE_LAND` (`G/building.cpp:605-623`) and the DB server
-- persists. The land rows themselves are Game data (`table.land`, the owner's
-- `player.sql` dump); only the current owner changes at runtime, so only it is stored.
-- A row exists only while a guild owns the land; an unowned land has no row, as a
-- `guild_id` of 0 does.
CREATE TABLE guild_land (
    land_id integer PRIMARY KEY CHECK (land_id > 0),
    guild_id integer NOT NULL REFERENCES guild (id) ON DELETE CASCADE CHECK (guild_id > 0)
);
