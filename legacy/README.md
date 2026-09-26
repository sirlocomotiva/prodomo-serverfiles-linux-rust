# Legacy deployment snapshot

What the owner's FreeBSD deployment (`/root/pd2_game`) and its MySQL 5.6.51 database held, cut
down to what the Rewrite reads. Every file here is Game data or configuration that the Legacy
server opens. Nothing here is built or run.

## Layout

| path | what it is |
|---|---|
| `config/db/conf.txt` | Legacy DB server config. `PROTO_FROM_DB = 0`, `LOCALE = latin1`, `PLAYER_ID_START = 100`, `ITEM_ID_RANGE = 100000000 200000000`. |
| `config/auth/CONFIG` | Auth process, port 30001. |
| `config/chN/coreM/CONFIG` | One per legacy Core. Each Channel's map set is the union of its Cores' `MAP_ALLOW` lists. |
| `gamedata/proto/` | `item_proto.txt`, `mob_proto.txt`, `item_names.txt`, `mob_names.txt`: the files the Legacy DB server loaded from its working directory. They are the authoritative protos. |
| `gamedata/data/` | Files the source opens by `data/...` paths: motion files (`.msa`, `motlist.txt`) and dungeon and event regen files. |
| `gamedata/locale/europe/` | The Locale base path: 69 map directories, quest sources, drop and group tables, and per-language strings under `country/` (cz de en es fr hu it pl pt ro tr). |
| `sql/schema/` | `mysqldump --no-data` of `account`, `common`, and `player` (68 tables), with `AUTO_INCREMENT` counters stripped. The datadir had no log database. |
| `sql/gamedata/` | Row data for the Game data tables only, dumped byte-for-byte (`--default-character-set=binary`): `common.exp_table`, `common.locale`, and `player.banword`, `item_attr`, `item_attr_rare`, `land`, `object_proto`, `refine_proto`, `shop`, `shop_item`, `shopex`, `shopex_item`, and `skill_proto`. |

Credentials in `config/` are replaced by `REDACTED`. The store starts fresh (ADR-0003), so no
account, player, item, guild, quest-flag, or log rows are kept. `land.guild_id` is reset to 0,
because one plot had a runtime owner.

### Topology

| Channel | Core | port | `MAP_ALLOW` entries |
|---|---|---|---|
| 1 | core1, core2 | 30003, 30005 | 30, 27 |
| 2 | core1, core2 | 30007, 30009 | 27, 22 |
| 3 | core1, core2 | 30011, 30013 | 27, 22 |
| 4 | core1, core2 | 30015, 30017 | 27, 22 |
| 99 | core99 | 30019 | 29 |

The DB server listened on 30000 and auth on 30001. Each Core's P2P port is its port plus one.

## Quests

`gamedata/locale/europe/quest` has 51 quest scripts (`_basic` 35, `event` 13, `rank` 2, `dungeon`
1) and 10 Lua library files. All 51 are live in the Rewrite. The snapshot's `quest_list` compiles
only 16 of them (the 13 event quests, the 2 rank quests, and `dungeoninfo.lua`), which looks like a
partial rebuild: without `_basic` nobody could create a guild. `object/` is the FreeBSD `qc` output
for those 16, kept as a reference for the Rust `qc` port. No script uses a `define`, so the
`pre_qc.py` preprocessor was a no-op and is removed.

## Encodings

Leave every file's bytes as they are. The proto name column is Korean in a legacy code page,
several files use CRLF line endings, and `skill_proto.szName` is the only UTF-8 column in the Game
data tables.

## What was removed

| removed | why |
|---|---|
| The raw MySQL datadir | Replaced by `sql/`. It also held the account row and the MySQL user table. |
| `start.py`, `stop.py`, `clear.py`, `gen.py`, `gen_settings.py`, `daemon.sh`, `prodomo.sh`, `ipfw.rules`, and the `*.list` files | FreeBSD process management for the legacy layout, which ADR-0002 replaces. `gen_settings.py` also held plaintext credentials. |
| Per-Core symlinks | Pointed at binaries, logs, and `/root` paths absent from the snapshot. |
| `share/conf/{BANIP,CMD,CRC,VERSION,state_user_count}` | All five were empty. |
| `*.gr2`, `*.dds`, `*.msm`, `*.mse`, `*.mdatr`, `monster.rar` under `data/` | Client models and textures. The source never opens them. |
| `quest/qc`, `pre_qc/`, `pre_qc.py`, `make.sh`, `map/find_map.*` | A FreeBSD binary, a no-op preprocessor with its output, and developer helpers. |
| `chan/mark/`, `reward_global/`, `reward_solo/`, `locale/europe/gaya/`, `stage/`, `special_item_group_vnum.txt` | Runtime state written by the running server (guild marks, per-player reward and Gaya files) or nothing reads it (`stage/`). The fresh store starts without it. |
| `share/conf/{item,mob}_{proto,names}.txt` | A second proto set that no legacy process opened (1,385 item and 18 mob vnums the loaded set lacks). The owner chose the loaded set. |
