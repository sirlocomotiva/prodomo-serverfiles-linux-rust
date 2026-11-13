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
use protocol::gc_chat::{CHAT_TYPE_COMMAND, CHAT_TYPE_INFO, CHAT_TYPE_TALKING};
use protocol::gc_fields::GcCreateFly;
use protocol::gc_vid::GcSpecialEffect;
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
/// The firecracker (`case 50107`, `G/char_item.cpp:5118-5140`).
const FIRECRACKER: u32 = 50_107;
/// The sky fireworks (`case 50100`-`case 50106`, `G/char_item.cpp:4351-4360`).
const SKY_FIREWORKS: [u32; 7] = [50_100, 50_101, 50_102, 50_103, 50_104, 50_105, 50_106];
/// The random sky firework (`case 71013`, `G/char_item.cpp:4346-4349`).
const RANDOM_FIREWORK: u32 = 71_013;
/// `FLY_FIREWORK1` (`G/char.h:96`): 7, counted from `FLY_NONE`.
const FLY_FIREWORK1: u8 = 7;
/// `FLY_FIREWORK6` (`:101`): 12.
const FLY_FIREWORK6: u8 = 12;
/// The spinning top (`case 50108`, `G/char_item.cpp:5142-5162`).
const SPINNING_TOP: u32 = 50_108;
/// The elk gold bar (`case ITEM_ELK_VNUM`, `G/char_item.cpp:5180-5191`).
const ELK_BAR: u32 = 50_026;
/// `SE_CHINA_FIREWORK` (`common/length.h:857`): 9, counted from `SE_NONE`.
const SE_CHINA_FIREWORK: u8 = 9;
/// `SE_SPIN_TOP` (`:858`): 10.
const SE_SPIN_TOP: u8 = 10;
/// `[LS;1205]`: the arena-map and fighting refusals both tell it.
const FIREWORK_REFUSAL: &[u8] = b"[LS;1205]";
/// The stat-reset tonic (`case ITEM_GIVE_STAT_RESET_COUNT_VNUM`,
/// `G/unique_item.h:43`, `G/char_item.cpp:5110-5116`).
const STAT_TONIC: u32 = 70014;
/// The stone detectors (`case 27989` / `case 76006`, `G/char_item.cpp:4226-4292`).
const STONE_DETECTORS: [u32; 2] = [27_989, 76_006];
/// The uses a detector survives (`:4278`): socket 0 past 5 breaks it.
const DETECTOR_USES: i32 = 6;
/// The near band's ceiling (`:4262`): under 10000 walks 2.
const DETECT_NEAR: u32 = 10_000;
/// The middle band's ceiling (`:4263`): under 70000 walks 1, else 3.
const DETECT_MID: u32 = 70_000;
/// The event-stone detector (`case 50004`, `G/char_item.cpp:4007-4225`).
const EVENT_DETECTOR: u32 = 50_004;
/// The arming clamp (`:4018`): the buried point is drawn within 700 of the caller.
const EVENT_BURY_RANGE: i32 = 700;
/// The arming tries (`G/sectree_manager.cpp:890-937`): 100 draws per region.
const EVENT_BURY_TRIES: i32 = 100;
/// The find's distance (`:4031`): under 1000 the stone is found.
const EVENT_FIND: f32 = 1000.0;
/// The near band's ceiling (`:4179`): under 20000 walks 1.
const EVENT_NEAR: f32 = 20_000.0;
/// The middle band's ceiling (`:4181`): under 70000 walks 2, else 3.
const EVENT_MID: f32 = 70_000.0;
/// The tries an event detector survives (`:4187`): socket 0 past 9 burns out.
const EVENT_TRIES: i32 = 10;
/// The burnout's consolation (`:4191`): a 27002.
const EVENT_CONSOLATION: u32 = 27_002;
/// The event-stone prizes (`:4047-4108`): vnum, count, weight, by socket-0 band.
/// The `info_7` table names 27004 twice (the second row should be 27002) and the
/// three tables sum to 100/89/93, so a high roll in the upper bands wins nothing;
/// both ride as written.
const EVENT_PRIZES_4: [(u32, u16, i32); 15] = [
    (27_001, 10, 0),
    (27_004, 10, 0),
    (27_002, 10, 0),
    (27_005, 10, 0),
    (27_100, 1, 0),
    (27_103, 1, 0),
    (27_101, 1, 0),
    (27_104, 1, 0),
    (27_999, 1, 25),
    (25_040, 1, 0),
    (27_410, 1, 0),
    (27_600, 1, 0),
    (25_100, 1, 15),
    (50_001, 1, 10),
    (50_003, 1, 50),
];
const EVENT_PRIZES_7: [(u32, u16, i32); 15] = [
    (27_001, 10, 1),
    (27_004, 10, 1),
    (27_004, 10, 9),
    (27_005, 10, 9),
    (27_100, 1, 5),
    (27_103, 1, 5),
    (27_101, 1, 10),
    (27_104, 1, 10),
    (27_999, 1, 14),
    (25_040, 1, 5),
    (27_410, 1, 5),
    (27_600, 1, 5),
    (25_100, 1, 5),
    (50_001, 1, 0),
    (50_003, 1, 5),
];
const EVENT_PRIZES_10: [(u32, u16, i32); 15] = [
    (27_001, 10, 8),
    (27_004, 10, 6),
    (27_002, 10, 12),
    (27_005, 10, 12),
    (27_100, 1, 9),
    (27_103, 1, 9),
    (27_101, 1, 10),
    (27_104, 1, 10),
    (27_999, 1, 12),
    (25_040, 1, 4),
    (27_410, 1, 0),
    (27_600, 1, 0),
    (25_100, 1, 0),
    (50_001, 1, 0),
    (50_003, 1, 1),
];

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

    /// Whether the item at `at` is a sky firework (`case 50100`-`case 50106` and
    /// `case 71013`, `G/char_item.cpp:4346-4360`).
    pub(super) fn sky_firework_at(&self, vid: Vid, at: ItemPos) -> bool {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .is_some_and(|item| item.vnum == RANDOM_FIREWORK || SKY_FIREWORKS.contains(&item.vnum))
    }

    /// The sky fireworks (`case 50100`-`case 50106` and `case 71013`,
    /// `G/char_item.cpp:4346-4360`): past the use's checks the fly goes around from
    /// the lighter to herself (`CreateFly`, `G/char_battle.cpp:129-139`), and one
    /// firework spends. The kind is the vnum less 50100 plus `FLY_FIREWORK1` (7), so
    /// 50100-50105 fly 7-12 and 50106 flies 13 (`FLY_FIREWORK_CHRISTMAS`); 71013
    /// rolls `number(7, 12)` instead. No gate guards these arms.
    pub(super) fn use_sky_firework(
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
            .map_or(0, |item| item.vnum);
        let fly = if vnum == RANDOM_FIREWORK {
            u8::try_from(number(
                &mut self.dice,
                i32::from(FLY_FIREWORK1),
                i32::from(FLY_FIREWORK6),
            ))
            .unwrap_or(FLY_FIREWORK1)
        } else {
            u8::try_from(
                vnum.wrapping_sub(50_100)
                    .wrapping_add(u32::from(FLY_FIREWORK1)),
            )
            .unwrap_or(FLY_FIREWORK1)
        };
        let moved = self.run_item_step(vid, actor, |items, _ids, rules, gear, protos| {
            let firework = world::character::use_source(items, at)?;
            let gear = gear.ok_or(world::character::MoveRefused::NotPorted(
                world::character::Unported::Equipment,
            ))?;
            let proto = protos
                .get(firework.vnum)
                .ok_or(world::character::MoveRefused::UnknownVnum(firework.vnum))?;
            world::character::use_checks(&firework, proto, rules, gear.points)?;
            world::character::spend_one(items, &firework, proto)
        })?;
        let mut record = Vec::new();
        GcCreateFly::new(fly, vid.raw(), vid.raw()).encode_into(&mut record);
        self.packet_around(super::view::EntityKey::Character(vid.raw()), &record, None);
        Ok(moved)
    }

    /// Whether the item at `at` is a stone detector (`case 27989` / `case 76006`,
    /// `G/char_item.cpp:4226-4292`).
    pub(super) fn stone_detector_at(&self, vid: Vid, at: ItemPos) -> bool {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .is_some_and(|item| STONE_DETECTORS.contains(&item.vnum))
    }

    /// The stone detector (`case 27989` / `case 76006`, `G/char_item.cpp:4226-4292`):
    /// socket 0 counts the uses up by 1, then the nearest live stone on the map is
    /// found (`FFindStone`, `:65-80`, over the map's characters; a stone at distance
    /// 0 is skipped, and the seed is the lowest VID, as the `std::map` iterates by
    /// key). With one, the caller hears `StoneDetect <vid> <val> <degree>` as a
    /// command line, where `val` is 2 under 10000, 1 under 70000, else 3, and the
    /// degree is `GetDegreeFromPositionXY` of the caller to the stone; with none,
    /// `[LS;1461]`. At socket 0 past 5 the line `StoneDetect <vid> 0 0` tells and
    /// the whole detector leaves (`RemoveItem`, the stack with it).
    /// The detector's answer for `spot`: the nearest live stone of `stones` names
    /// its val and degree (`:4240-4272`). The seed is the first stone in VID order;
    /// only a nearer stone at a nonzero distance replaces it, in legacy's order.
    fn detector_answer(stones: &[(u32, i32, i32)], spot: super::view::Spot) -> Option<(u32, i32)> {
        let mut stones = stones.to_vec();
        stones.sort_by_key(|(vid, _, _)| *vid);
        let mut target = stones.first().map(|(stone, _, _)| *stone);
        let mut max = u32::MAX;
        for (stone, x, y) in &stones {
            let dist =
                common::cfloat::f32_to_u32(super::motion::distance_sqrt(spot.x - x, spot.y - y));
            if dist != 0 && max > dist {
                max = dist;
                target = Some(*stone);
            }
        }
        let stone = target?;
        let (tx, ty) = stones
            .iter()
            .find_map(|(vid, x, y)| (*vid == stone).then_some((*x, *y)))
            .unwrap_or((spot.x, spot.y));
        let val = if max < DETECT_NEAR {
            2
        } else if max < DETECT_MID {
            1
        } else {
            3
        };
        let degree =
            common::cfloat::f32_to_i32(super::motion::rotation_to_xy(tx - spot.x, ty - spot.y));
        Some((val, degree))
    }

    pub(super) fn use_stone_detector(
        &mut self,
        vid: Vid,
        at: ItemPos,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let place = self.place_of(vid.raw());
        let spot = self.spot_of(vid.raw());
        let held = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at));
        let Some(held) = held else {
            return Err(MoveItemRefused::NoSuchCharacter { vid });
        };
        let (id, uses) = (held.id, held.sockets[0].saturating_add(1));
        let mut stones: Vec<(u32, i32, i32)> = Vec::new();
        if let Some((channel, map)) = place {
            if let Some(list) = self.npcs.get(&(channel, map)) {
                for npc in &list.npcs {
                    if npc.dead || npc.char_type != gamedata::mob_proto::CHAR_TYPE_STONE {
                        continue;
                    }
                    stones.push((npc.vid, npc.x, npc.y));
                }
            }
        }
        let target = spot.and_then(|spot| Self::detector_answer(&stones, spot));
        let moved = self.run_item_step(vid, actor, |items, _ids, rules, gear, protos| {
            let detector = world::character::use_source(items, at)?;
            let gear = gear.ok_or(world::character::MoveRefused::NotPorted(
                world::character::Unported::Equipment,
            ))?;
            let proto = protos
                .get(detector.vnum)
                .ok_or(world::character::MoveRefused::UnknownVnum(detector.vnum))?;
            world::character::use_checks(&detector, proto, rules, gear.points)?;
            if uses >= DETECTOR_USES {
                let (done, _) =
                    world::character::destroy_item(items, at, rules.questing, rules.secured)?;
                return Ok(done);
            }
            Ok(world::character::set_item_socket(items, id, (0, uses)))
        })?;
        if uses >= DETECTOR_USES {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_COMMAND,
                b"StoneDetect %u 0 0",
                &[Arg::Int(i64::from(vid.raw()))],
            );
            return Ok(moved);
        }
        match target {
            Some((val, degree)) => {
                self.tell(
                    vid,
                    actor,
                    CHAT_TYPE_COMMAND,
                    b"StoneDetect %u %d %d",
                    &[
                        Arg::Int(i64::from(vid.raw())),
                        Arg::Int(i64::from(val)),
                        Arg::Int(i64::from(degree)),
                    ],
                );
            }
            None => {
                self.tell(vid, actor, CHAT_TYPE_INFO, b"[LS;1461]", &[]);
            }
        }
        Ok(moved)
    }

    /// Whether the item at `at` is the event-stone detector (`case 50004`,
    /// `G/char_item.cpp:4007-4225`).
    pub(super) fn event_detector_at(&self, vid: Vid, at: ItemPos) -> bool {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .is_some_and(|item| item.vnum == EVENT_DETECTOR)
    }

    /// Bury the event stone for the detector's first use
    /// (`GetRandomLocation(map, pos, 700)`, `G/sectree_manager.cpp:876-940`): up to
    /// 100 draws over the channel's region, each clamped within 700 of the caller;
    /// the first movable one wins. `None` when the map stood no region or no
    /// draw lands movable.
    fn bury_event_stone(&mut self, channel: u8, map: i32, at: (i32, i32)) -> Option<(i32, i32)> {
        let region = self.regens.region_of(channel, map)?;
        for _ in 0..EVENT_BURY_TRIES {
            let mut x = number(&mut self.dice, region.sx + 50, region.ex - 50);
            let mut y = number(&mut self.dice, region.sy + 50, region.ey - 50);
            let dx = (x - at.0).abs();
            if dx > EVENT_BURY_RANGE {
                x = if x < at.0 {
                    at.0 - EVENT_BURY_RANGE
                } else {
                    at.0 + EVENT_BURY_RANGE
                };
            }
            let dy = (y - at.1).abs();
            if dy > EVENT_BURY_RANGE {
                y = if y < at.1 {
                    at.1 - EVENT_BURY_RANGE
                } else {
                    at.1 + EVENT_BURY_RANGE
                };
            }
            if self.is_movable(map, x, y) {
                return Some((x, y));
            }
        }
        None
    }

    /// The event-stone prize for socket-0 `uses` and `roll` (`:4113-4150`): the
    /// band's table (4 at 4 or less, 7 at 7 or less, else 10), walked down while
    /// the roll exceeds each nonzero weight. `None` when the roll falls past the
    /// last weight — the upper tables sum below 100 — or names the 50001 lotto
    /// row, whose DB insert has no Rewrite table (the kill lotto's hook likewise
    /// holds the draw and grants nothing).
    fn event_prize(uses: i32, mut roll: i32) -> Option<(u32, u16)> {
        let table = if uses <= 4 {
            EVENT_PRIZES_4
        } else if uses <= 7 {
            EVENT_PRIZES_7
        } else {
            EVENT_PRIZES_10
        };
        for (vnum, count, weight) in table {
            if weight == 0 {
                continue;
            }
            if roll <= weight {
                return if vnum == 50_001 {
                    None
                } else {
                    Some((vnum, count))
                };
            }
            roll -= weight;
        }
        None
    }

    /// The event-stone detector (`case 50004`, `G/char_item.cpp:4007-4225`): the
    /// first use buries the stone at a random movable point within 700 (`[LS;154]`
    /// when none lands) and stamps sockets 0-2; later uses count socket 0 up.
    /// Within 1000 of the buried point the stone is found (`[LS;165]`), the prize
    /// table pays, the `StoneDetect <vid> 0 0` line goes around, and the detector
    /// leaves. Else past 9 tries the detector burns out (`[LS;176]`, a 27002
    /// consolation, and the detector leaves); otherwise the caller hears
    /// `StoneDetect <vid> <dist> <degree>` with 1 under 20000, 2 under 70000,
    /// else 3.
    pub(super) fn use_event_detector(
        &mut self,
        vid: Vid,
        at: ItemPos,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let held = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at));
        let Some(held) = held else {
            return Err(MoveItemRefused::NoSuchCharacter { vid });
        };
        let (id, sockets) = (held.id, held.sockets);
        let Some(place) = self.place_of(vid.raw()) else {
            return Err(MoveItemRefused::NoSuchCharacter { vid });
        };
        let Some(spot) = self.spot_of(vid.raw()) else {
            return Err(MoveItemRefused::NoSuchCharacter { vid });
        };
        let armed = sockets[0] != 0;
        let buried = if armed {
            Some((sockets[1], sockets[2]))
        } else {
            self.bury_event_stone(place.0, place.1, (spot.x, spot.y))
        };
        let Some((bx, by)) = buried else {
            self.tell(vid, actor, CHAT_TYPE_INFO, b"[LS;154]", &[]);
            return self.arena_item_refused(vid, actor);
        };
        let uses = if armed {
            sockets[0].saturating_add(1)
        } else {
            1
        };
        let distance = super::motion::distance_sqrt(spot.x - bx, spot.y - by);
        if distance < EVENT_FIND {
            return self.find_event_stone(vid, at, actor, uses);
        }
        if uses >= EVENT_TRIES {
            return self.burn_out_event_detector(vid, at, actor);
        }
        let dist = if distance < EVENT_NEAR {
            1
        } else if distance < EVENT_MID {
            2
        } else {
            3
        };
        let degree =
            common::cfloat::f32_to_i32(super::motion::rotation_to_xy(bx - spot.x, by - spot.y));
        let first = !armed;
        let moved = self.run_item_step(vid, actor, |items, _ids, rules, gear, protos| {
            let detector = world::character::use_source(items, at)?;
            let gear = gear.ok_or(world::character::MoveRefused::NotPorted(
                world::character::Unported::Equipment,
            ))?;
            let proto = protos
                .get(detector.vnum)
                .ok_or(world::character::MoveRefused::UnknownVnum(detector.vnum))?;
            world::character::use_checks(&detector, proto, rules, gear.points)?;
            let mut done = world::character::set_item_socket(items, id, (0, uses));
            if first {
                let x = world::character::set_item_socket(items, id, (1, bx));
                let y = world::character::set_item_socket(items, id, (2, by));
                done.records.extend(x.records);
                done.changes.extend(x.changes);
                done.records.extend(y.records);
                done.changes.extend(y.changes);
            }
            Ok(done)
        })?;
        self.tell(
            vid,
            actor,
            CHAT_TYPE_COMMAND,
            b"StoneDetect %u %d %d",
            &[
                Arg::Int(i64::from(vid.raw())),
                Arg::Int(i64::from(dist)),
                Arg::Int(i64::from(degree)),
            ],
        );
        Ok(moved)
    }

    /// The find (`:4031-4177`): `[LS;165]`, the prize, the `0 0` line around, and
    /// the detector leaves.
    fn find_event_stone(
        &mut self,
        vid: Vid,
        at: ItemPos,
        actor: Mover,
        uses: i32,
    ) -> Result<MovedItems, MoveItemRefused> {
        let roll = number(&mut self.dice, 1, 100);
        let prize = Self::event_prize(uses, roll);
        let questing = self.quest_running(vid);
        let mut moved = self.run_item_step(vid, actor, |items, _ids, rules, gear, protos| {
            let detector = world::character::use_source(items, at)?;
            let gear = gear.ok_or(world::character::MoveRefused::NotPorted(
                world::character::Unported::Equipment,
            ))?;
            let proto = protos
                .get(detector.vnum)
                .ok_or(world::character::MoveRefused::UnknownVnum(detector.vnum))?;
            world::character::use_checks(&detector, proto, rules, gear.points)?;
            let (done, _) =
                world::character::destroy_item(items, at, rules.questing, rules.secured)?;
            Ok(done)
        })?;
        if let Some((vnum, count)) = prize {
            if let Some((outcome, done)) = self.auto_give(vid, (vnum, count), 0, questing) {
                if let AutoGiven::Unplaced(item) = outcome {
                    self.lay_auto_given(vid, item);
                }
                let packed = MovedItems::new(
                    moved.owner_id,
                    vid.raw(),
                    done,
                    actor,
                    &self.protos,
                    &self.locale,
                );
                moved.records.extend(packed.records);
                moved.changes.extend(packed.changes);
            }
        }
        self.tell(vid, actor, CHAT_TYPE_INFO, b"[LS;165]", &[]);
        let line = crate::chat_line::chat_packet(
            actor.recipient(&self.locale),
            CHAT_TYPE_COMMAND,
            b"StoneDetect %u 0 0",
            &[Arg::Int(i64::from(vid.raw()))],
        );
        self.packet_around(super::view::EntityKey::Character(vid.raw()), &line, None);
        Ok(moved)
    }

    /// The burnout (`:4187-4193`): `[LS;176]`, a 27002 consolation, and the
    /// detector leaves.
    fn burn_out_event_detector(
        &mut self,
        vid: Vid,
        at: ItemPos,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let questing = self.quest_running(vid);
        let mut moved = self.run_item_step(vid, actor, |items, _ids, rules, gear, protos| {
            let detector = world::character::use_source(items, at)?;
            let gear = gear.ok_or(world::character::MoveRefused::NotPorted(
                world::character::Unported::Equipment,
            ))?;
            let proto = protos
                .get(detector.vnum)
                .ok_or(world::character::MoveRefused::UnknownVnum(detector.vnum))?;
            world::character::use_checks(&detector, proto, rules, gear.points)?;
            let (done, _) =
                world::character::destroy_item(items, at, rules.questing, rules.secured)?;
            Ok(done)
        })?;
        if let Some((outcome, done)) = self.auto_give(vid, (EVENT_CONSOLATION, 1), 0, questing) {
            if let AutoGiven::Unplaced(item) = outcome {
                self.lay_auto_given(vid, item);
            }
            let packed = MovedItems::new(
                moved.owner_id,
                vid.raw(),
                done,
                actor,
                &self.protos,
                &self.locale,
            );
            moved.records.extend(packed.records);
            moved.changes.extend(packed.changes);
        }
        self.tell(vid, actor, CHAT_TYPE_INFO, b"[LS;176]", &[]);
        Ok(moved)
    }

    /// Whether the item at `at` is a firework (`case 50107` / `case 50108`,
    /// `G/char_item.cpp:5118-5162`): the firecracker or the spinning top.
    pub(super) fn firework_at(&self, vid: Vid, at: ItemPos) -> bool {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .is_some_and(|item| item.vnum == FIRECRACKER || item.vnum == SPINNING_TOP)
    }

    /// The fireworks (`case 50107` / `case 50108`, `G/char_item.cpp:5118-5162`): an
    /// arena map refuses with `[LS;1205]`, then a consent-duel fight with the
    /// `no_potions_on_pvp` flag up refuses with the same line (the vnums are not on
    /// the allowed list, as `IsAllowedPotionOnPVP` names only blue potions). Past the
    /// gates and the use's checks the effect goes around (`SE_CHINA_FIREWORK` 9 for
    /// the firecracker, `SE_SPIN_TOP` 10 for the top) and one firework spends. The
    /// `ENABLE_FIREWORK_STUN` arm is commented out at the head of `UseItemEx`
    /// (`:2716`), so no row lands.
    pub(super) fn use_firework(
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
            .map_or(0, |item| item.vnum);
        if self
            .place_of(vid.raw())
            .is_some_and(|(_, map)| self.is_arena_map(map))
        {
            self.tell(vid, actor, CHAT_TYPE_INFO, FIREWORK_REFUSAL, &[]);
            return self.arena_item_refused(vid, actor);
        }
        let fighting = self
            .characters
            .find_by_vid(vid)
            .ok()
            .is_some_and(|character| self.pvp_is_fighting(character.player_id()));
        let no_potions = self
            .quests
            .as_ref()
            .is_some_and(|quests| quests.manager().event_flag(b"no_potions_on_pvp") != 0);
        if no_potions && fighting {
            self.tell(vid, actor, CHAT_TYPE_INFO, FIREWORK_REFUSAL, &[]);
            return self.arena_item_refused(vid, actor);
        }
        let effect = if vnum == SPINNING_TOP {
            SE_SPIN_TOP
        } else {
            SE_CHINA_FIREWORK
        };
        let moved = self.run_item_step(vid, actor, |items, _ids, rules, gear, protos| {
            let firework = world::character::use_source(items, at)?;
            let gear = gear.ok_or(world::character::MoveRefused::NotPorted(
                world::character::Unported::Equipment,
            ))?;
            let proto = protos
                .get(firework.vnum)
                .ok_or(world::character::MoveRefused::UnknownVnum(firework.vnum))?;
            world::character::use_checks(&firework, proto, rules, gear.points)?;
            world::character::spend_one(items, &firework, proto)
        })?;
        let mut record = Vec::new();
        GcSpecialEffect {
            effect_type: effect,
            vid: vid.raw(),
        }
        .encode_into(&mut record);
        self.packet_around(super::view::EntityKey::Character(vid.raw()), &record, None);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The prize walk at 1 use (the 4-band): roll 1 skips the eight zero rows and
    /// lands the 27999 stone at weight 25; roll 26 lands the 25100 at weight 15;
    /// roll 100 lands the last row, 50003.
    #[test]
    fn the_low_band_pays_stone_map_and_bundle_in_order() {
        assert_eq!(GameState::event_prize(1, 1), Some((27_999, 1)));
        assert_eq!(GameState::event_prize(4, 26), Some((25_100, 1)));
        assert_eq!(GameState::event_prize(4, 100), Some((50_003, 1)));
    }

    /// The 50001 lotto row pays nothing: at 1 use roll 41..=50 names it (25 + 15
    /// walked off), and the arm holds the draw with no table to write.
    #[test]
    fn the_lotto_row_holds_its_draw() {
        assert_eq!(GameState::event_prize(1, 41), None);
        assert_eq!(GameState::event_prize(1, 50), None);
    }

    /// The upper tables sum below 100, so a roll past their last weight wins
    /// nothing: roll 100 at 8 uses (the 10-band sums 93) and at 5 uses (the
    /// 7-band sums 89) both fall through.
    #[test]
    fn a_roll_past_the_table_wins_nothing() {
        assert_eq!(GameState::event_prize(8, 100), None);
        assert_eq!(GameState::event_prize(5, 100), None);
        assert_eq!(GameState::event_prize(8, 93), Some((50_003, 1)));
    }
}
