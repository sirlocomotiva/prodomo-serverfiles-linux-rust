# 587. The 507 premium-shop arm (ledger 587)

Status: ready-for-agent

Ledger 586 ported main-switch `USE_AFFECT` (510-513) and left 507's arm out:

> 507's premium-shop arm waits on the account store.

This ticket is that arm: the 72355 use (`G/char_item.cpp:6835-6843`),
`SetPremiumPrivateShopBonus` (`G/char.cpp:12695-12742`), the account
`premium_privateshop_expire` column it writes
(`D/ClientManagerPrivateShop.cpp:2296-2350`), and the two readers: the build-time
`premium_time` copy (`G/private_shop_manager.cpp:171`) and the affect-event sync
(`G/char_affect.cpp:343-363`). Every flow below was read from the legacy source
(`G/` is `server/server/game/`, `D/` is `server/server/db/`, `C/` is
`server/server/common/`); re-check a line before citing it.

## Legacy flows

**The use arm** (`G/char_item.cpp:6835-6843`, behind `__PREMIUM_PRIVATE_SHOP__`,
which this tree defines — `common::features::PREMIUM_PRIVATE_SHOP` is true):
value(0) 507 never reaches the renewal loop or the held-kind check. The setter
runs on value(3); true consumes one of the stack and the use returns true, false
keeps the item and still returns true (no consume, no further arm).

**The setter** (`G/char.cpp:12695-12742`):
1. No descriptor → false, silent.
2. A held 507 row: the duration adds to its `lDuration`, clamped to
   `PRIVATE_SHOP_MAX_PREMIUM_TIME` (`C/length.h:139`, `3600*24*7` = 604800); when
   `MAX - lDuration < 60` the setter refuses with
   `LC_TEXT("You have just recently reached maximum premium time for your personal shop. ")`
   (verbatim: trailing space, `LC_TEXT` is `locale_find` with `LOCALE_YMIR`,
   which returns the input — `G/locale.cpp:71-89`) and the item is kept.
3. `AddAffect(507, POINT_NONE, 0, 0, duration, 0, true)` — `over`, so the old row
   is replaced.
4. `m_aiPremiumTimes[PREMIUM_PRIVATE_SHOP] = duration + time(0)` (index 7,
   `C/length.h:833-846`).
5. The DB write: `TPacketGDPrivateShopPremiumTimeUpdate{dwAID, dwPID, tPremiumTime=duration}`
   (`G/char.cpp:12718-1228`, struct at `C/tables.h:1415`).
6. `LC_TEXT("You have increased Premium Personal Shop duration.")` (verbatim, no
   trailing space), and when the duration equals the max also
   `LC_TEXT("You have reached maximum premium time for your personal shop. ")`
   (verbatim, trailing space).
7. `CharLog(0, "PRIVATE SHOP PREMIUM TIME", "+%u", duration)` — the operator log's,
   not the client's; the Rewrite has no char-log seat outside quests
   (`quests.rs:3486`), so `tracing::info!` like the empire change is enough.
8. Returns true.

**The DB write** (`D/ClientManagerPrivateShop.cpp:2296-2350`):
1. `tNewPremiumTime = tPremiumTime + time(0)`.
2. `UPDATE account SET premium_privateshop_expire = FROM_UNIXTIME(tNewPremiumTime) WHERE id`.
3. For each of the account's four `player_index` slots: a cached private shop of
   that PID retimes to `tNewPremiumTime` (`UpdatePremiumTime`, `D/PrivateShop.cpp:31-44`,
   which also rewrites the shop row's `premium_time`).

**The load** (`G/input_auth.cpp:145`, `G/db.cpp:342-356`, `D/ClientManagerPlayer.cpp:307`,
`G/char.cpp:2386`): the auth query reads `UNIX_TIMESTAMP(premium_privateshop_expire)`
into `aiPremiumTimes[7]`, copied through login data and the player table into
`m_aiPremiumTimes[7]`. Positive controls that the other six premium columns are
out of scope: `char_battle.cpp:2495` (EXP), `:562` (AUTOLOOT), `:588` (GOLD) and
`char_manager.cpp:965` (ITEM) read them, and none is ported — this ticket adds
only the private-shop index.

**Reader 1 — the build** (`G/private_shop_manager.cpp:171`): the stored shop row's
`tPremiumTime = GetPremiumRemainSeconds(7) + time(0)`, i.e. the account expiry
verbatim (`remain = expiry - now`, so `remain + now = expiry`). The Rewrite's
build writes `premium_time: now` (`private_shop.rs:907`) — that becomes the
expiry. Reachability: `premium_time > time(0)` is the respawn gate
(`D/ClientManagerPrivateShop.cpp:2672-2679`) and `premium_time < time(0)` is the
despawn gate (`:2411`); the Rewrite has no respawn and no despawn sweep
(positive control: no `load_shop` caller outside tests, no premium tick in
`private_shop.rs`), so the value is stored but unread — record that in the note,
do not build the sweep.

**Reader 2 — the affect sync** (`G/char_affect.cpp:343-363`, first in `ProcessAffect`):
for `i` in `0..=PREMIUM_MAX_NUM`, a held row of kind `500+i` with
`GetPremiumRemainSeconds(i) < 0` is removed, else its `lDuration` is rewritten to
`remain + 1`. Only index 7 is in scope here (the others have no stored expiry, so
their remain would read 0 — a whole-array port would rewrite durations it must
not touch). The Rewrite's seat is `run_affect_event` (`affect.rs:833-946`):
before the guild-skill strip (which is where legacy's order puts it — the sync
runs before the countdown, and the strip runs before the drain), a held 507 row
reads `expiry - wall_now()`: negative removes it (uncompute + remove frame, like
the strip), else its `remaining` rewrites to `remain + 1`. `left_of` already
reads `remaining` for non-realtime kinds, and 507 is not realtime, so no tick
change is needed. Note the `0..=` off-by-one reads index 9 (`PREMIUM_MAX_NUM`),
whose `GetPremiumRemainSeconds` returns 0 (`G/char.cpp:8091-8097`): a held 509
row's duration rewrites to 1 every tick — a Defect, not reproduced; do not loop
past 7.

**The affect is never saved**: `IS_NO_SAVE_AFFECT` returns true for 500-509
(`G/char_affect.cpp:43-58`), and `SaveAffect` skips those (`:468-491`). The
Rewrite's `player_affect` save must exclude 507 (check its filter; if it saves
by kind range, 507 must fall outside). On relog the row is gone but the account
expiry stays — legacy behaviour, keep it.

## Design

- `db/migrations/0043_private_shop_premium.sql`: `ALTER TABLE account ADD COLUMN
  private_shop_premium_expire bigint NOT NULL DEFAULT 0 CHECK (private_shop_premium_expire >= 0)`.
  A unix time, like `private_shop.premium_time` (bigint), not a timestamptz: the
  arithmetic is `expiry - now` in seconds on both sides.
- `db/src/accounts.rs`: `set_private_shop_premium_expire(store, id, expire)` and
  `private_shop_premium_expire(store, id)` (the load), following `set_language`'s
  shape (`NoSuchAccountId` when no row). The multi-shop retime (step 3 of the DB
  write) is one `UPDATE private_shop SET premium_time = $ WHERE owner_id IN
  (SELECT id FROM player WHERE account_id = $)` — check `player`'s account
  column name first; legacy retimes only cached shops, a store detail, while the
  Rewrite retimes rows, which is what it has.
- The world holds the expiry per online character: `premium_shop_expiry: HashMap<Vid, i64>`
  on `GameState` (a `Character` has no account link — positive control:
  `world/src/character/model.rs` names no account — so the map keys on `Vid`
  like `affect_events` does). Seeded at admission: `Loaded` gains
  `private_shop_premium_expire: i64`, read by the descriptor before
  `enter_world_with_items` (as it reads the lobby for `quest_account`,
  `main.rs:2560-2580`); cleared on `leave_world_by` (as `affect_events` is).
- `world/src/character/equip.rs`: the 507 exclusion in `use_special_kind`
  (`:1143-1144`) becomes its own flag, answering a new `MoveKind::ShopPremium`
  (new variant in `world/src/character/item_move.rs`, next to `Timed`), so the
  step consumes one of the stack and the land hook runs. The `AFFECT_PREMIUM_PRIVATE_SHOP_VALUE`
  const stays as the classifier's test.
- The refusal (setter step 2) is a guard in `use_item_guards`
  (`game_state.rs:4273-4312`), after the timed guard: `!self.shop_premium_use_allowed(vid, at, actor)`
  answers `arena_item_refused` (told, kept — the setter told and returned false,
  and the arm consumed nothing). It reads the held 507 row's `remaining` and
  refuses when `604800 - remaining < 60`. The world finds the row through
  `affect_events`, as `holds_affect_on` does (`pc_skill.rs:2010-2017`).
- The land runs in `land_item_use_kind` (`game_state.rs:4454-4476`), which needs
  the account: thread `account: &QuestAccount` through it (it already reaches
  `use_item`, `game_state.rs:4326-4331`, and `run_quest_used` shows the shape).
  `land_shop_premium(vid, vnum, account)` in `pc_skill.rs`, next to
  `land_timed_affect`:
  1. The proto/vnum gates (ITEM_USE, USE_AFFECT, value(0) 507 — the mirror of
     `land_timed_affect`'s 507 exclusion, which stays).
  2. `duration = min(value(3) + held_remaining, 604800)` (held 0 when no row).
  3. `add_pc_affect(507, POINT_NONE, 0, 0, duration, 0, over=true)`.
  4. `expiry = duration + private_now()`; the map stores it; `moved` carries it
     for the descriptor (see below).
  5. The two `tell`s, verbatim, `CHAT_TYPE_INFO`; the `tracing::info!` char-log.
- The store: `MovedItems` gains `shop_premium: Option<(u32, i64)>` (account id,
  expiry), set by the land hook from `account.id`. The descriptor stores it in
  the ITEM_USE handler (`main.rs:1300-1342`), next to the `climbed` store and
  before `finish_item_step` (a failure closes without a half-written step, as
  the comment says). `account.id == 0` (no account table) stores nothing — the
  setter's no-descriptor arm is unreachable here (a use needs a descriptor), but
  a 0 id must not write row 0.
- The build (`private_shop.rs:907`): `premium_time: now` becomes the held expiry
  (`self.premium_shop_expiry.get(&vid).copied().unwrap_or(0)`). `unwrap_or(0)`
  is the never-bought case: legacy reads a fresh `CURRENT_TIMESTAMP` expiry
  (already past), which is equally dead — record the form difference in the note.
- The affect sync: in `run_affect_event`, before `strip_peacetime_guild_affects`,
  `sync_shop_premium_affect(vid)`: a held 507 row with `expiry - wall_now() < 0`
  goes (uncompute + remove frame), else `remaining = remain + 1`. No event
  rescheduling: the row's presence already keeps the event alive through `again`.

## Done when

1. World test: a 507 use consumes one and answers `ShopPremium` (fixture: the
   owner's 72355, `USE_AFFECT` 507/0/0/604800).
2. Game-state tests: the land (507 row with value(3)'s duration, `over` — a
   second land replaces, durations add clamped to 604800), the near-max refusal
   (`604800 - remaining < 60` tells and keeps), the max line at exactly 604800.
3. The build test: a build with a held expiry stores it as `premium_time`.
4. The sync test: a held 507 row past expiry is removed on the tick; a live one
   rewrites to `remain + 1`.
5. A scripted-client scenario in `prodomo/tests/parity.rs`: Alpha uses 72355,
   one of two goes, the 507 `GC_AFFECT_ADD` lands with 604800 s, the increased
   line tells, the account row reads the expiry, and a second use while held
   tells the recently-reached line with the row kept (seed the held row by a
   first maxed use: value(3) is 604800, so the first land already maxes — the
   second use refuses).
6. Every gate passes; there is a ledger section; `cg.game.item_use`'s note grows
   a Ledger 587 line (status stays `partial`); `sys.trade.private_shop` stays
   `ported` (no row change — the premium column is shopless scope).
