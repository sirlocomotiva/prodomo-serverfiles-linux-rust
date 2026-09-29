-- A character's cell key, made deferrable (ledger section 214).
--
-- `0005_items.sql` made `(owner_id, window_type, pos)` unique with a partial unique index, and a
-- unique index is checked row by row as each statement changes a row. Equipping an item onto a
-- worn one is a swap (`CHARACTER::SwapItem`, `char_item.cpp:8164-8264`): the carried item takes
-- the wear cell while the worn one takes the carried item's old cell. Written as the two row
-- moves it is, the first move lands on a cell the second row still holds, so an index checked
-- per row refuses a change whose end state is perfectly legal. Only a constraint can be deferred;
-- an index cannot.
--
-- The constraint is not partial, and it does not need to be. A unique constraint treats NULLs as
-- distinct (PostgreSQL's default, `NULLS DISTINCT`), so two ground rows -- `owner_id IS NULL`, and
-- `window_type` and `pos` are `NOT NULL` -- never conflict, which is exactly what the old
-- `WHERE owner_id IS NOT NULL` said. The name is kept, so the insert path's classification by
-- constraint name (`db::items::classify_insert`) and its SQLSTATE `23505` are unchanged.
--
-- `INITIALLY IMMEDIATE`: every statement outside a move keeps the per-row check it had.
-- `db::items::apply_row_changes` defers it for its own transaction, checks the cells it moved
-- before it commits, and lets the commit re-check as the backstop.

DROP INDEX item_owner_cell_key;

ALTER TABLE item
    ADD CONSTRAINT item_owner_cell_key UNIQUE (owner_id, window_type, pos)
    DEFERRABLE INITIALLY IMMEDIATE;
