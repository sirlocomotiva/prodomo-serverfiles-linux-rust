# Game data

Every Game data file and table the legacy server reads, with the Rust reader that replaces it.
Hand-maintained. Paths are relative to `legacy/gamedata/` (files) or name a table imported from
`legacy/sql/gamedata/` (tables). A reader is `ported` when it reads the whole owner data set
byte-exact and a scenario exercises the system that uses it.

| id | path or table | read by (legacy) | used by | status | scenario | note |
|---|---|---|---|---|---|---|
| `data.proto.item` | `proto/item_proto.txt`, `proto/item_names.txt` | `D/ClientManagerBoot.cpp` (sent to the game at boot) | `sys.item.proto` | missing | | `PROTO_FROM_DB = 0`; name column in a legacy code page. |
| `data.proto.mob` | `proto/mob_proto.txt`, `proto/mob_names.txt` | `D/ClientManagerBoot.cpp` (sent to the game at boot) | `sys.mob.proto` | partial |  | Only the locale names the Name rules refuse are read (`gamedata::mob_names`, ledger 186). |
| `data.map.setting` | `locale/europe/map/*/Setting.txt` | `G/sectree_manager.cpp` | `sys.world.map` | missing | | 69 map directories. |
| `data.map.attr` | `locale/europe/map/*/server_attr` | `G/sectree_manager.cpp` | `sys.world.map` | missing | | Binary. |
| `data.map.town` | `locale/europe/map/*/Town.txt` | `G/sectree_manager.cpp` | `sys.world.map` | missing | | |
| `data.map.property` | `locale/europe/map/*/MapProperty.txt` | none | | unused | | No legacy source opens it (controls: `Town.txt`, `Setting.txt`, `server_attr` found in `G/sectree_manager.cpp`). Client-side file. |
| `data.map.regen` | `locale/europe/map/*/{regen,npc,stone,boss,dungeon}.txt` | `G/sectree_manager.cpp:740-770`, `G/regen.cpp` | `sys.world.regen` | missing | | |
| `data.map.index` | `locale/europe/forkedmapindex.txt` | `G/main.cpp`, `G/questlua_forked.cpp` | `sys.world.map` | missing | | |
| `data.motion` | `data/{pc,pc2,pc3,monster,monster2}/**` (`.msa`, `motlist.txt`) | `G/motion.cpp` | `sys.combat.core`, `sys.world.move` | missing | | |
| `data.regen.dungeon` | `data/dungeon/**`, `data/event/**` | `G/dungeon.cpp`, quests | `sys.dungeon.core` | missing | | |
| `data.mob.group` | `locale/europe/group.txt`, `group_group.txt` | `G/mob_manager.cpp` | `sys.world.regen` | missing | | No spaces in group names. |
| `data.drop.mob` | `locale/europe/mob_drop_item.txt` | `G/input_db.cpp` | `sys.combat.drop` | missing | | |
| `data.drop.common` | `locale/europe/common_drop_item.txt` | `G/input_db.cpp` | `sys.combat.drop` | missing | | |
| `data.drop.etc` | `locale/europe/etc_drop_item.txt` | `G/input_db.cpp` | `sys.combat.drop` | missing | | Read by vnum (`ENABLE_FIX_READ_ETC_DROP_ITEM_FILE_BY_VNUM`). |
| `data.drop.group` | `locale/europe/drop_item_group.txt` | `G/input_db.cpp` | `sys.combat.drop` | missing | | |
| `data.item.special_group` | `locale/europe/special_item_group.txt` | `G/item_manager_read_tables.cpp` | `sys.item.proto` | missing | | |
| `data.item.ori_to_new` | `locale/europe/ori_to_new_table.txt` | `G/input_db.cpp` | `sys.item.proto` | missing | | |
| `data.item.stack_attribute` | `locale/europe/item_stack_attribute.txt` | `G/item_stack_attribute.cpp` | `sys.item.stack_attribute` | missing | | |
| `data.item.blend` | `locale/europe/blend.json` | `G/blend_item.cpp` | `sys.item.blend` | missing | | |
| `data.skill.power` | `locale/europe/skill_power.txt` | `G/skill_power.cpp` | `sys.skill.core` | missing | | |
| `data.cube` | `locale/europe/cube.txt` | `G/cuberenewal.cpp` | `sys.custom.cube` | missing | | |
| `data.dragon_soul` | `locale/europe/dragon_soul_table.txt` | `G/dragon_soul_table.cpp` | `sys.custom.dragon_soul` | missing | | |
| `data.fishing` | `locale/europe/fishing.txt` | `G/fishing.cpp` | `sys.life.fishing` | missing | | |
| `data.gaya` | `locale/europe/gaya.txt` | `G/char_gaya.cpp` | `sys.custom.gaya` | missing | | |
| `data.daily_gift` | `locale/europe/daily_gift.txt` | `G/char_daily_gifts.cpp` | `sys.custom.daily_gift` | missing | | |
| `data.battle_pass` | `locale/europe/battlepass.json` | `G/BattlePassManager.cpp` | `sys.custom.battle_pass` | missing | | |
| `data.world_boss` | `locale/europe/world_boss.txt` | `G/input_db.cpp` | `sys.event.world_boss` | missing | | |
| `data.dungeon_info` | `locale/europe/dungeon_info.txt` | `G/dungeon_info.cpp` | `sys.dungeon.info` | missing | | |
| `data.ox` | `locale/europe/oxquiz.lua` | `G/questlua_oxevent.cpp` | `sys.event.ox` | missing | | |
| `data.lua.settings` | `locale/europe/settings.lua`, `locale/europe/translate.lua` | `G/questlua.cpp:737-790` (with `quest/questlib.lua` and `quest/locale.lua`) | `sys.quest.runtime` | missing | | |
| `data.lua.unread` | `locale/europe/BlueDragon.lua`, `locale/europe/monkey_dungeon.lua` | none | | unused | | Nothing opens them: no C++ path and no `dofile` in the quest tree (control: `settings.lua` found in `G/questlua.cpp`). `G/BlueDragon_Binder.cpp` reads a `BlueDragonSetting` global that is therefore never set, so legacy runs the Blue Dragon on its fallbacks. Owner to decide: load it (Divergence) or keep the fallbacks (parity). |
| `data.quest.sources` | `locale/europe/quest/**` | `qc`, `G/questmanager.cpp` | `sys.quest.runtime` | missing | | See `quests.md`. |
| `data.locale.strings` | `locale/europe/locale_string.txt`, `country/*/locale_string.txt`, `country/*/locale_quest.txt` | `G/locale_service.cpp:438-440`, `G/locale.cpp` | `sys.char.multi_language` | missing | | 11 languages. |
| `data.locale.names` | `country/*/{item_names,mob_names,skill_names}.txt`, `country/*/translate.lua` | `G/locale_service.cpp:535-537` | `sys.char.multi_language` | missing | | `country/*/pet_skill_names.txt` has no reader (control: `skill_names.txt` found). |
| `data.locale.charset` | `locale/europe/charset.txt` | none | | unused | | No legacy source opens it; every `charset` hit is the SQL connection charset. |
| `table.exp_table` | `common.exp_table` | `D/ClientManagerBoot.cpp` | `sys.char.points` | missing | | |
| `table.locale` | `common.locale` | `G/locale_service.cpp` | `sys.char.multi_language` | missing | | |
| `table.banword` | `player.banword` | `D/ClientManagerBoot.cpp` | `sys.char.chat` | partial |  | `gamedata::banword` rule; read from `player.sql` for the Name rules since ledger 186. Chat filtering waits for `sys.char.chat`. |
| `table.item_attr` | `player.item_attr`, `player.item_attr_rare` | `D/ClientManagerBoot.cpp` | `sys.item.attr` | partial | | `gamedata::item_attr` rule. |
| `table.land` | `player.land` | `D/ClientManagerBoot.cpp` | `sys.world.objects` | partial | | `gamedata::land` rule. |
| `table.object_proto` | `player.object_proto` | `D/ClientManagerBoot.cpp` | `sys.world.objects` | partial | | `gamedata::object_proto` rule. |
| `table.refine_proto` | `player.refine_proto` | `D/ClientManagerBoot.cpp` | `sys.item.refine` | partial | | `gamedata::refine` rule. |
| `table.shop` | `player.shop`, `player.shop_item` | `D/ClientManagerBoot.cpp` | `sys.npc.shop` | partial | | `gamedata::shop` rule. |
| `table.shopex` | `player.shopex`, `player.shopex_item` | `D/ClientManagerBoot.cpp` | `sys.npc.shop` | partial | | `gamedata::renewal_shop` rule. |
| `table.skill_proto` | `player.skill_proto` | `D/ClientManagerBoot.cpp` | `sys.skill.core` | partial | | `gamedata::skill` rule; `szName` is UTF-8. |
