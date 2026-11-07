-- The building objects (ledger section 416).
--
-- Legacy's `object` table (`legacy/sql/schema/player.sql`), which `RequestCreateObject`
-- and `RequestDeleteObject` write (`G/building.cpp:500-600`) and the DB server reads at
-- boot (`D/ClientManagerBoot.cpp`). The coordinates are absolute (legacy adds the map
-- base before storing, `G/building.cpp:516-517`). The id is the `dwID` the client
-- never sees but the delete half names, so it stays a 32-bit integer.
CREATE TABLE building_object (
    id integer PRIMARY KEY CHECK (id > 0),
    land_id integer NOT NULL CHECK (land_id > 0),
    vnum integer NOT NULL CHECK (vnum > 0),
    map_index integer NOT NULL,
    x integer NOT NULL,
    y integer NOT NULL,
    x_rot real NOT NULL,
    y_rot real NOT NULL,
    z_rot real NOT NULL,
    life integer NOT NULL
);
