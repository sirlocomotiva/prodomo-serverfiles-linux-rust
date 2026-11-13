//! The stone and the recipe arms of `UseItemEx` (`case USE_CREATE_STONE` and `case
//! USE_RECIPE`, `G/char_item.cpp:6866-6870`, `:6889-6943`).
//!
//! The stone scroll (the owner's 27999) gives a metin of `number(28000, 28013)` and
//! spends one of its stack. The roll still burns its draw when no proto of the
//! rolled vnum exists (`AutoGiveItem` answers null, `G/char_item.cpp:9083-9086`),
//! as none does on the owner's data: every owner metin starts at 28030, so the
//! scroll spends and gives nothing there.
//!
//! The recipe scroll (the owner's 50905-50907) brews value(0) out of value(1) of
//! value(2), value(3) of value(4), and one 50901 bottle, and is itself never spent.
//! A wanted count of 0 skips its source; a missing one chats `[LS;421]`, a short
//! stack `[LS;422]` with its name, a missing or empty bottle `[LS;423]`, and a
//! `number(1, 100)` past value(5) `[LS;424]`, each leaving what was already consumed
//! consumed. All three owner recipes carry value(5) 0, so the brew always fails
//! after consuming on the owner's data.
//!
//! `G/` = `server/server/game/`.

use common::vid::Vid;
use gamedata::item_kind::{ITEM_USE, USE_CREATE_STONE, USE_RECIPE};
use protocol::gc_actors::GcCharacterGoldChange;
use protocol::gc_chat::{CHAT_TYPE_INFO, CHAT_TYPE_TALKING};
use protocol::item_pos::ItemPos;
use world::character::{number, AutoGiven, MoveDone, RecipeTake};

use super::GameState;
use crate::chat_line::Arg;
use crate::item_move::{MoveItemRefused, MovedItems, Mover};

/// The stone roll's bounds (`:6867`).
const STONE_LOW: i32 = 28_000;
/// The stone roll's bounds (`:6867`).
const STONE_HIGH: i32 = 28_013;
/// The recipe's bottle (`:6919`).
const RECIPE_BOTTLE: u32 = 50_901;
/// The poison apple's vnum (`case 27996`, `G/char_item.cpp:4289`).
const POISON_APPLE: u32 = 27_996;
/// The mythical peaches (`case 71107` / `case 39032`, `:4811-4860`).
const MYTHICAL_PEACHES: [u32; 2] = [71_107, 39_032];
/// The peach's throttle flag (`mythical_peach.last_use_time`, `:4815,4852`).
const PEACH_LAST_USE_FLAG: &[u8] = b"mythical_peach.last_use_time";
/// The alignment ceiling the peach refuses at (`:4831-4835`).
const PEACH_MAX_ALIGNMENT: i32 = 200_000;
/// The hair colorants (`case 70201`-`case 70206`, `G/char_item.cpp:4937-4967`).
const HAIR_COLORANTS: [u32; 6] = [70_201, 70_202, 70_203, 70_204, 70_205, 70_206];
/// The colorant's throttle flag (`fix_bug.colorante`, `:4943,4962`).
const COLORANT_LAST_USE_FLAG: &[u8] = b"fix_bug.colorante";
/// The seconds between two colorant uses (`:4945`).
const COLORANT_WAIT_SECONDS: i64 = 3;
/// A hair past this one is a dyed one the colorant refuses (`:4951`).
const HAIR_DYE_CEILING: u16 = 1001;
/// The wonso bean, sugar and fruit (`case 50020`-`case 50022`, `:5165-5178`).
const WONSO_BEAN: u32 = 50_020;
/// The wonso sugar (`case 50021`).
const WONSO_SUGAR: u32 = 50_021;
/// The wonso fruit (`case 50022`).
const WONSO_FRUIT: u32 = 50_022;
/// The elk gold bar (`case ITEM_ELK_VNUM`, `G/char_item.cpp:5180-5191`).
const ELK_BAR: u32 = 50_026;
/// The stat-reset tonic (`case ITEM_GIVE_STAT_RESET_COUNT_VNUM`,
/// `G/unique_item.h:43`, `G/char_item.cpp:5110-5116`).
const STAT_TONIC: u32 = 70014;

impl GameState {
    /// Whether the item at `at` is a stone scroll (`USE_CREATE_STONE`).
    pub(super) fn create_stone_at(&self, vid: Vid, at: ItemPos) -> bool {
        self.use_subtype_at(vid, at) == Some(USE_CREATE_STONE)
    }

    /// Whether the item at `at` is a recipe scroll (`USE_RECIPE`).
    pub(super) fn recipe_at(&self, vid: Vid, at: ItemPos) -> bool {
        self.use_subtype_at(vid, at) == Some(USE_RECIPE)
    }

    /// Whether the item at `at` is the poison apple (`case 27996`,
    /// `G/char_item.cpp:4289-4292`).
    pub(super) fn poison_apple_at(&self, vid: Vid, at: ItemPos) -> bool {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .is_some_and(|item| item.vnum == POISON_APPLE)
    }

    /// The poison apple (`case 27996`, `G/char_item.cpp:4289-4292`): one is spent,
    /// then the eater is poisoned with no attacker (`AttackedByPoison(NULL)`).
    pub(super) fn use_poison_apple(
        &mut self,
        vid: Vid,
        at: ItemPos,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let moved = self.run_item_step(vid, actor, |items, _ids, rules, gear, protos| {
            let apple = world::character::use_source(items, at)?;
            let gear = gear.ok_or(world::character::MoveRefused::NotPorted(
                world::character::Unported::Equipment,
            ))?;
            let proto = protos
                .get(apple.vnum)
                .ok_or(world::character::MoveRefused::UnknownVnum(apple.vnum))?;
            world::character::use_checks(&apple, proto, rules, gear.points)?;
            world::character::spend_one(items, &apple, proto)
        })?;
        self.poison_pc(vid, None);
        Ok(moved)
    }

    /// Whether the item at `at` is a mythical peach (`case 71107` / `case 39032`,
    /// `G/char_item.cpp:4811-4860`).
    pub(super) fn mythical_peach_at(&self, vid: Vid, at: ItemPos) -> bool {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .is_some_and(|item| MYTHICAL_PEACHES.contains(&item.vnum))
    }

    /// The mythical peach (`case 71107` / `case 39032`, `G/char_item.cpp:4811-4860`):
    /// past the use's checks, the throttle refuses within `value(1)` hours of the flag
    /// (the `[#Unk]` line on a live Channel, `[LS;1034]` on the test Channel), the
    /// ceiling refuses at 200000, else `value(0) * 10` alignment lands (clamped to the
    /// ceiling), one peach spends, the flag stamps now, and `[LS;383]` talks with
    /// `[LS;384]` naming the gain.
    pub(super) fn use_mythical_peach(
        &mut self,
        vid: Vid,
        at: ItemPos,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let peach = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at).cloned());
        let proto = peach
            .as_ref()
            .and_then(|item| self.protos.get(item.vnum).cloned());
        let Some(proto) = proto else {
            return Err(MoveItemRefused::NoSuchCharacter { vid });
        };
        if peach.is_none() {
            return Err(MoveItemRefused::NoSuchCharacter { vid });
        }
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        let now = Self::global_time();
        let last = self
            .quests
            .as_ref()
            .and_then(|quests| quests.manager().quest_flag(pid, PEACH_LAST_USE_FLAG))
            .unwrap_or(0);
        let wait = i64::from(proto.values[1]) * 60 * 60;
        if i64::from(now).saturating_sub(i64::from(last)) < wait {
            if self.test_server {
                self.tell(vid, actor, CHAT_TYPE_INFO, b"[LS;1034]", &[]);
            } else {
                self.tell(
                    vid,
                    actor,
                    CHAT_TYPE_INFO,
                    b"[#Unk]You cannot use this item yet.",
                    &[],
                );
            }
            return self.nothing_used(vid, actor);
        }
        let shown = self.real_alignment_of(vid.raw());
        if shown == PEACH_MAX_ALIGNMENT {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"[#Unk]You already have your rank at the maximum.",
                &[],
            );
            return self.nothing_used(vid, actor);
        }
        let mut val = proto.values[0];
        if PEACH_MAX_ALIGNMENT.saturating_sub(shown) < val.saturating_mul(10) {
            val = PEACH_MAX_ALIGNMENT.saturating_sub(shown) / 10;
        }
        self.update_alignment(vid, val.saturating_mul(10));
        let moved = self.run_item_step(vid, actor, |items, _ids, rules, gear, protos| {
            let peach = world::character::use_source(items, at)?;
            let gear = gear.ok_or(world::character::MoveRefused::NotPorted(
                world::character::Unported::Equipment,
            ))?;
            let proto = protos
                .get(peach.vnum)
                .ok_or(world::character::MoveRefused::UnknownVnum(peach.vnum))?;
            world::character::use_checks(&peach, proto, rules, gear.points)?;
            world::character::spend_one(items, &peach, proto)
        })?;
        if let Some(quests) = self.quests.as_mut() {
            quests.manager_mut().set_quest_flag(
                pid,
                PEACH_LAST_USE_FLAG,
                i32::try_from(now).unwrap_or(i32::MAX),
            );
        }
        self.tell(vid, actor, CHAT_TYPE_TALKING, b"[LS;383]", &[]);
        let mut moved = moved;
        moved.records.push(crate::chat_line::chat_packet(
            actor.recipient(&self.locale),
            CHAT_TYPE_INFO,
            b"[LS;384;%d]",
            &[crate::chat_line::Arg::Int(i64::from(val))],
        ));
        Ok(moved)
    }

    /// Whether the item at `at` is a hair colorant (`case 70201`-`case 70206`,
    /// `G/char_item.cpp:4937-4967`).
    pub(super) fn hair_colorant_at(&self, vid: Vid, at: ItemPos) -> bool {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .is_some_and(|item| HAIR_COLORANTS.contains(&item.vnum))
    }

    /// A hair colorant (`case 70201`-`case 70206`, `G/char_item.cpp:4937-4967`): past
    /// the use's checks, the throttle refuses within 3 seconds of
    /// `fix_bug.colorante`, a hair at or past 1001 refuses as a dyed one, else the
    /// hair becomes the vnum minus 70201, the neighbourhood sees the update, and the
    /// flag stamps now. The colorant is never spent (`:4959` is commented out).
    pub(super) fn use_hair_colorant(
        &mut self,
        vid: Vid,
        at: ItemPos,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let vnum = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .map(|item| item.vnum);
        let Some(vnum) = vnum else {
            return Err(MoveItemRefused::NoSuchCharacter { vid });
        };
        let moved = self.run_item_step(vid, actor, |items, _ids, rules, gear, protos| {
            let colorant = world::character::use_source(items, at)?;
            let gear = gear.ok_or(world::character::MoveRefused::NotPorted(
                world::character::Unported::Equipment,
            ))?;
            let proto = protos
                .get(colorant.vnum)
                .ok_or(world::character::MoveRefused::UnknownVnum(colorant.vnum))?;
            world::character::use_checks(&colorant, proto, rules, gear.points)?;
            Ok(MoveDone {
                kind: world::character::MoveKind::Used,
                records: Vec::new(),
                changes: Vec::new(),
            })
        })?;
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        let now = Self::global_time();
        let last = self
            .quests
            .as_ref()
            .and_then(|quests| quests.manager().quest_flag(pid, COLORANT_LAST_USE_FLAG))
            .unwrap_or(0);
        if i64::from(now).saturating_sub(i64::from(last)) < COLORANT_WAIT_SECONDS {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You have to wait 3 seconds to use this item again.",
                &[],
            );
            return self.nothing_used(vid, actor);
        }
        let hair = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .map(|points| points.parts()[common::enums::EParts::Hair as usize]);
        if hair.is_some_and(|hair| hair >= HAIR_DYE_CEILING) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot dye or bleach your current Hairstyle.",
                &[],
            );
            return self.nothing_used(vid, actor);
        }
        let dye = u16::try_from(vnum.saturating_sub(HAIR_COLORANTS[0])).unwrap_or(u16::MAX);
        if let Ok(character) = self.characters.find_by_vid_mut(vid) {
            if let Some(points) = character.items_and_points_mut().1 {
                points.set_part(common::enums::EParts::Hair, dye);
            }
        }
        if let Some(quests) = self.quests.as_mut() {
            quests.manager_mut().set_quest_flag(
                pid,
                COLORANT_LAST_USE_FLAG,
                i32::try_from(now).unwrap_or(i32::MAX),
            );
        }
        let flags = self.affect_flags_of(vid);
        if let Some(frame) = self.update_frame(vid, flags) {
            let me = crate::game_state::view::EntityKey::Character(vid.raw());
            self.packet_around(me, &frame, None);
        }
        Ok(moved)
    }

    /// Whether the item at `at` is a wonso bean, sugar or fruit (`case 50020`-
    /// `case 50022`, `G/char_item.cpp:5165-5178`).
    pub(super) fn wonso_at(&self, vid: Vid, at: ItemPos) -> bool {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .is_some_and(|item| matches!(item.vnum, WONSO_BEAN | WONSO_SUGAR | WONSO_FRUIT))
    }

    /// The wonso bean, sugar and fruit (`case 50020`-`case 50022`,
    /// `G/char_item.cpp:5165-5178`): past the use's checks, the bean fills the hit
    /// points, the sugar the spirit, and the fruit the stamina, each by the gap to
    /// its maximum, and one is spent.
    pub(super) fn use_wonso(
        &mut self,
        vid: Vid,
        at: ItemPos,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let vnum = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .map(|item| item.vnum);
        let kind = match vnum {
            Some(WONSO_BEAN) => common::point_slot::POINT_HP,
            Some(WONSO_SUGAR) => common::point_slot::POINT_SP,
            Some(WONSO_FRUIT) => common::point_slot::POINT_STAMINA,
            _ => return Err(MoveItemRefused::NoSuchCharacter { vid }),
        };
        let (now, max) = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .map_or((0, 0), |points| match kind {
                common::point_slot::POINT_HP => (points.hp(), points.max_hp()),
                common::point_slot::POINT_SP => (points.sp(), points.max_sp()),
                _ => (points.stamina(), points.max_stamina()),
            });
        let mut moved = self.run_item_step(vid, actor, |items, _ids, rules, gear, protos| {
            let wonso = world::character::use_source(items, at)?;
            let gear = gear.ok_or(world::character::MoveRefused::NotPorted(
                world::character::Unported::Equipment,
            ))?;
            let proto = protos
                .get(wonso.vnum)
                .ok_or(world::character::MoveRefused::UnknownVnum(wonso.vnum))?;
            world::character::use_checks(&wonso, proto, rules, gear.points)?;
            world::character::spend_one(items, &wonso, proto)
        })?;
        let gap = max.saturating_sub(now);
        if let Ok(character) = self.characters.find_by_vid_mut(vid) {
            if let Some(points) = character.items_and_points_mut().1 {
                if let Ok(records) = points.point_change(kind, gap, true, false) {
                    let done = MoveDone {
                        kind: world::character::MoveKind::Used,
                        records: records
                            .into_iter()
                            .map(world::character::MoveRecord::Point)
                            .collect(),
                        changes: Vec::new(),
                    };
                    let packed = MovedItems::new(
                        moved.owner_id,
                        vid.raw(),
                        done,
                        actor,
                        &self.protos,
                        &self.locale,
                    );
                    moved.records.extend(packed.records);
                }
            }
        }
        Ok(moved)
    }

    /// Whether the item at `at` is the stat-reset tonic (`case
    /// ITEM_GIVE_STAT_RESET_COUNT_VNUM`, `G/char_item.cpp:5110-5116`).
    pub(super) fn stat_tonic_at(&self, vid: Vid, at: ItemPos) -> bool {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .is_some_and(|item| item.vnum == STAT_TONIC)
    }

    /// The stat-reset tonic (`case ITEM_GIVE_STAT_RESET_COUNT_VNUM`,
    /// `G/char_item.cpp:5110-5116`): past the use's checks, the reset count rises
    /// by 1 through the shared counter arm (its change record rides, amount 0: legacy
    /// calls `PointChange` with both flags false), and one tonic spends.
    pub(super) fn use_stat_tonic(
        &mut self,
        vid: Vid,
        at: ItemPos,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let mut moved = self.run_item_step(vid, actor, |items, _ids, rules, gear, protos| {
            let tonic = world::character::use_source(items, at)?;
            let gear = gear.ok_or(world::character::MoveRefused::NotPorted(
                world::character::Unported::Equipment,
            ))?;
            let proto = protos
                .get(tonic.vnum)
                .ok_or(world::character::MoveRefused::UnknownVnum(tonic.vnum))?;
            world::character::use_checks(&tonic, proto, rules, gear.points)?;
            world::character::spend_one(items, &tonic, proto)
        })?;
        if let Ok(character) = self.characters.find_by_vid_mut(vid) {
            if let Some(points) = character.items_and_points_mut().1 {
                if let Ok(records) =
                    points.point_change(common::point_slot::POINT_STAT_RESET_COUNT, 1, false, false)
                {
                    let done = MoveDone {
                        kind: world::character::MoveKind::Used,
                        records: records
                            .into_iter()
                            .map(world::character::MoveRecord::Point)
                            .collect(),
                        changes: Vec::new(),
                    };
                    let packed = MovedItems::new(
                        moved.owner_id,
                        vid.raw(),
                        done,
                        actor,
                        &self.protos,
                        &self.locale,
                    );
                    moved.records.extend(packed.records);
                    moved.points = self
                        .characters
                        .find_by_vid(vid)
                        .ok()
                        .and_then(|character| character.points().cloned());
                }
            }
        }
        Ok(moved)
    }

    /// Whether the item at `at` is the elk gold bar (`case ITEM_ELK_VNUM`,
    /// `G/char_item.cpp:5180-5191`).
    pub(super) fn elk_bar_at(&self, vid: Vid, at: ItemPos) -> bool {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .is_some_and(|item| item.vnum == ELK_BAR)
    }

    /// The elk gold bar (`case ITEM_ELK_VNUM`, `G/char_item.cpp:5180-5191`): past the
    /// use's checks, the whole bar leaves (it is removed, not spent), `[LS;1269]` names
    /// the socket-0 gold, and it pays through `ChangeGold`, whose compiled-in arm the
    /// build takes (`ENABLE_REMOVE_LIMIT_GOLD` is defined): past the cap the bar is
    /// still consumed, the gold refuses, and only the line lands.
    pub(super) fn use_elk_bar(
        &mut self,
        vid: Vid,
        at: ItemPos,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let bar = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at).cloned());
        let Some(bar) = bar else {
            return Err(MoveItemRefused::NoSuchCharacter { vid });
        };
        let pays = u64::try_from(bar.sockets[0].max(0)).unwrap_or(u64::MAX);
        let held = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::gold);
        let left = held
            .checked_add(pays)
            .filter(|left| *left < world::character::GOLD_MAX_MAX);
        let mut moved = self.run_item_step(vid, actor, |items, _ids, rules, gear, protos| {
            let elk = world::character::use_source(items, at)?;
            let gear = gear.ok_or(world::character::MoveRefused::NotPorted(
                world::character::Unported::Equipment,
            ))?;
            let proto = protos
                .get(elk.vnum)
                .ok_or(world::character::MoveRefused::UnknownVnum(elk.vnum))?;
            world::character::use_checks(&elk, proto, rules, gear.points)?;
            let (done, _) =
                world::character::destroy_item(items, at, rules.questing, rules.secured)?;
            Ok(done)
        })?;
        self.tell(
            vid,
            actor,
            CHAT_TYPE_INFO,
            b"[LS;1269;%d]",
            &[Arg::Int(i64::from(bar.sockets[0]))],
        );
        if let Some(left) = left {
            if let Ok(character) = self.characters.find_by_vid_mut(vid) {
                character.set_gold(left);
            }
            moved.gold = Some(crate::item_move::gold_delta(held, left));
            let mut frame = Vec::new();
            GcCharacterGoldChange::new(vid.raw(), i64::try_from(pays).unwrap_or(i64::MAX), left)
                .encode_into(&mut frame);
            moved.records.push(frame);
        }
        Ok(moved)
    }

    /// The sub type of the `ITEM_USE` at `at`, or `None` for anything else.
    fn use_subtype_at(&self, vid: Vid, at: ItemPos) -> Option<i32> {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .and_then(|item| self.protos.get(item.vnum))
            .filter(|proto| proto.item_type == ITEM_USE)
            .map(|proto| proto.sub_type)
    }

    /// Run one stone-scroll use (`case USE_CREATE_STONE`, `G/char_item.cpp:6866-6870`):
    /// the checks, the roll, the give, and the spend, in legacy's order.
    pub(super) fn use_create_stone(
        &mut self,
        vid: Vid,
        at: ItemPos,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        if self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .is_some_and(world::character::Points::is_secured)
        {
            return Err(MoveItemRefused::Refused(
                world::character::MoveRefused::Secured,
            ));
        }
        let (scroll, item_type) = self.scroll_vnum_and_type(vid, at);
        self.pass_on_use_item(vid, actor, scroll, item_type);
        self.run_item_step(vid, actor, |items, _ids, rules, gear, _protos| {
            world::character::check_use_scroll(items, at, rules, gear)
        })?;
        let rolled = number(&mut self.dice, STONE_LOW, STONE_HIGH);
        let mut gifts = MoveDone {
            kind: world::character::MoveKind::Used,
            records: Vec::new(),
            changes: Vec::new(),
        };
        if let Ok(vnum) = u32::try_from(rolled) {
            let questing = self.quest_running(vid);
            if let Some((outcome, done)) = self.auto_give(vid, (vnum, 1), -1, questing) {
                if let AutoGiven::Unplaced(item) = outcome {
                    self.lay_auto_given(vid, item);
                }
                gifts.records.extend(done.records);
                gifts.changes.extend(done.changes);
            }
        }
        let mut moved = self.run_item_step(vid, actor, |items, _ids, rules, gear, _protos| {
            world::character::consume_use_scroll(items, at, rules, gear)
        })?;
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        let packed = MovedItems::new(pid, vid.raw(), gifts, actor, &self.protos, &self.locale);
        let mut records = packed.records;
        records.append(&mut moved.records);
        moved.records = records;
        let mut changes = packed.changes;
        changes.append(&mut moved.changes);
        moved.changes = changes;
        Ok(moved)
    }

    /// The used scroll's vnum and item type, or 0 and -1 when it is gone.
    fn scroll_vnum_and_type(&self, vid: Vid, at: ItemPos) -> (u32, i32) {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .map_or((0, -1), |item| {
                (
                    item.vnum,
                    self.protos
                        .get(item.vnum)
                        .map_or(-1, |proto| proto.item_type),
                )
            })
    }

    /// Run one recipe use (`case USE_RECIPE`, `G/char_item.cpp:6889-6943`): the scroll's
    /// checks, the sources, the bottle, the roll, and the give. The scroll is never spent.
    pub(super) fn use_recipe(
        &mut self,
        vid: Vid,
        at: ItemPos,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        if self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .is_some_and(world::character::Points::is_secured)
        {
            return Err(MoveItemRefused::Refused(
                world::character::MoveRefused::Secured,
            ));
        }
        let (scroll, item_type) = self.scroll_vnum_and_type(vid, at);
        self.pass_on_use_item(vid, actor, scroll, item_type);
        let values = self.protos.get(scroll).map_or([0; 6], |proto| proto.values);
        let wanted = [
            (
                u32::try_from(values[1].max(0)).unwrap_or(u32::MAX),
                u32::try_from(values[2].max(0)).unwrap_or(u32::MAX),
            ),
            (
                u32::try_from(values[3].max(0)).unwrap_or(u32::MAX),
                u32::try_from(values[4].max(0)).unwrap_or(u32::MAX),
            ),
        ];
        let mut stopped: Option<(u32, RecipeTake)> = None;
        let mut bottled: Option<RecipeTake> = None;
        let mut moved = self.run_item_step(vid, actor, |items, _ids, rules, gear, protos| {
            world::character::check_use_scroll(items, at, rules, gear)?;
            let mut spent = MoveDone {
                kind: world::character::MoveKind::Used,
                records: Vec::new(),
                changes: Vec::new(),
            };
            for (vnum, needed) in wanted {
                match world::character::take_recipe_source(items, protos, vnum, needed) {
                    Ok(done) => {
                        spent.records.extend(done.records);
                        spent.changes.extend(done.changes);
                    }
                    Err(take) => {
                        stopped = Some((vnum, take));
                        return Ok(spent);
                    }
                }
            }
            match world::character::take_recipe_source(items, protos, RECIPE_BOTTLE, 1) {
                Ok(done) => {
                    spent.records.extend(done.records);
                    spent.changes.extend(done.changes);
                }
                Err(take) => {
                    bottled = Some(take);
                }
            }
            Ok(spent)
        })?;
        if let Some((vnum, take)) = stopped {
            if take == RecipeTake::Short {
                let name = self.item_get_name(vnum);
                self.tell(
                    vid,
                    actor,
                    CHAT_TYPE_INFO,
                    b"[LS;422;%s]",
                    &[Arg::Text(&name)],
                );
            } else {
                self.tell(vid, actor, CHAT_TYPE_INFO, b"[LS;421]", &[]);
            }
            return Ok(moved);
        }
        if bottled.is_some() {
            self.tell(vid, actor, CHAT_TYPE_INFO, b"[LS;423]", &[]);
            return Ok(moved);
        }
        if number(&mut self.dice, 1, 100) > values[5] {
            self.tell(vid, actor, CHAT_TYPE_INFO, b"[LS;424]", &[]);
            return Ok(moved);
        }
        let made = u32::try_from(values[0].max(0)).unwrap_or(0);
        let questing = self.quest_running(vid);
        if let Some((outcome, done)) = self.auto_give(vid, (made, 1), -1, questing) {
            if let AutoGiven::Unplaced(item) = outcome {
                self.lay_auto_given(vid, item);
            }
            let pid = self
                .characters
                .find_by_vid(vid)
                .map_or(0, world::character::Character::player_id);
            let packed = MovedItems::new(pid, vid.raw(), done, actor, &self.protos, &self.locale);
            moved.records.extend(packed.records);
            moved.changes.extend(packed.changes);
        }
        Ok(moved)
    }
}
