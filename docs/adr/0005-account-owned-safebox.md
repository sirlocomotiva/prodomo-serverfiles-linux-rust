# The safebox and the mall belong to the account

Legacy keeps safebox and mall items in `player.item` with the **account** id in `owner_id`
(`db/ClientManager.cpp:790-814`, `window = SAFEBOX|MALL`), the same column that holds a character id
for every other window. It stores the safebox password in plain text in its own `safebox` table
(`safebox.password`), next to a size and a gold column the owner's build never reads
(`input_db.cpp:1135-1150` opens one page, or six with the premium or the large-safebox item,
whatever the size column says, and `CSafebox::Save` writes only the unused gold). The Rewrite keeps
the account as the owner, but in a column of its own, so every item row still has exactly one owner
of one type (ADR-0003).

## Considered options

- An `owner_id` that means an account for two windows: the foreign key to `player` cannot hold
  it, and nothing in the schema would say which kind of id a row carries.
- A safebox owned by each character: every character of an account would see a different
  safebox, which the Reference client and the owner's players do not expect.

## Consequences

- `item.account_id` references `account` and cascades on delete. A CHECK makes the owner follow
  the window: a ground row has neither owner, a `SAFEBOX` or `MALL` row has only `account_id`,
  and every other window has only `owner_id`. A deferrable unique key on
  `(account_id, window_type, pos)` keeps one item per safebox cell.
- A checkin or a checkout is one Transfer between a character and its account, and a move inside
  the safebox is one Transfer on the account's rows, each committed in one transaction before the
  client is told.
- The `safebox` table holds only the account id and an argon2id hash of the password. An
  account without a row or without a password opens with `000000`, as legacy's does.
- Changing the password creates the row when it is missing, and the old password must match
  exactly. Both are Divergences: legacy never creates the row outside the size change, so a
  fresh account could never change its password, and legacy compares the old password without
  case, which a hash cannot do.
- The size and gold columns are not carried over.
