-- Package C5 (ledger 738): convert the two remaining free-text columns from `text` to
-- `bytea` with byte-length checks, per ADR-0003 ("Free text is stored and relayed as the
-- exact bytes the client sent (`bytea`) and is never transcoded").
--
-- Neither `0011_guild.sql` nor `0012_guild_comment.sql` is edited (applied migrations stay
-- immutable). Existing UTF-8 rows convert losslessly via `convert_to(..., 'UTF8')`.

ALTER TABLE guild_grade
    DROP CONSTRAINT guild_grade_name_check,
    ALTER COLUMN name TYPE bytea USING convert_to(name, 'UTF8'),
    ADD CONSTRAINT guild_grade_name_check CHECK (octet_length(name) <= 9);

ALTER TABLE guild_comment
    DROP CONSTRAINT guild_comment_content_check,
    ALTER COLUMN content TYPE bytea USING convert_to(content, 'UTF8'),
    ADD CONSTRAINT guild_comment_content_check CHECK (octet_length(content) BETWEEN 1 AND 50);
