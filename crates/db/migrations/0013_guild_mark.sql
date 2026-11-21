-- The guild marks and symbols (ledger section 230v).
--
-- Legacy keeps no SQL for either: the mark index lives in `mark/mark_index`
-- (`guild_id mark_id` lines, `G/MarkManager.cpp:57-73,75-94`), the pixels in five
-- 512x512 TGA files (`mark/mark_<i>.tga`, `:31-44,113-122`), and the symbols in
-- one binary file (`guild_symbol.tga`: count, then guild/size/bytes triples,
-- `:352-374`). The Rewrite keeps the same rows in the store: the mark id and the
-- 768 image bytes per guild, and the symbol bytes per guild. The TGA files and
-- the index file are gone; the world rebuilds its images from these rows at boot
-- and each upload commits through the descriptor, as guild creation judges,
-- stores, then commits.
--
-- The mark id is unique across guilds because `__AllocMarkID` hands out each id
-- once (`:176-190`); the image is exactly `16 * 12 * 4` bytes
-- (`TPacketCGMarkUpload::image`, `G/packet.h:2238-2243`). The symbol is 1 to
-- 65536 bytes: empty and oversized uploads close the connection
-- (`G/input_login.cpp:1007-1013`), and the `WORD size` counts its own 7 head
-- bytes, so 65536 is unreachable on the wire but expressible in the check.

CREATE TABLE guild_mark (
    -- The guild the mark belongs to.
    guild_id integer PRIMARY KEY REFERENCES guild (id) ON DELETE CASCADE,
    -- The mark id `__AllocMarkID` handed out, unique across guilds.
    mark_id integer NOT NULL UNIQUE CHECK (mark_id >= 0),
    -- The 768 uploaded image bytes (`16 * 12 * 4`).
    image bytea NOT NULL CHECK (octet_length(image) = 768)
);

CREATE TABLE guild_symbol (
    -- The guild the symbol belongs to.
    guild_id integer PRIMARY KEY REFERENCES guild (id) ON DELETE CASCADE,
    -- The uploaded symbol bytes.
    data bytea NOT NULL CHECK (octet_length(data) BETWEEN 1 AND 65536)
);
