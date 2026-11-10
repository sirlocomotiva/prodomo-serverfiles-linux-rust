//! The buff follower (`__ENABLE_SHAMAN_SYSTEM__`, `G/ShamanSystem.cpp`, `G/ItemUtils.cpp`):
//! the toggle item's summon, its follow, and the five buffs it keeps on its owner.
//!
//! Using the 69700 item toggles its socket-3 active bit (`OnUseToggleItem`,
//! `G/ItemUtils.cpp:177-196`): off goes on through `ActivateToggleItem` (`:58-102`), on
//! goes off through `DeactivateToggleItem` (`:104-132`). A stack past 1 unstacks first,
//! refusing with the no-room line when no base cell is free (`:61-76`); the group check
//! reads value 5, 0 on the owner's proto, and no second toggle exists, so it passes.
//! Activation
//! locks the item and summons (`SummonItem`, `G/ShamanSystem.cpp:556-581`); deactivation
//! unsummons and unlocks. The owner's proto reads no applies and no wear timer, so the
//! `ModifyPoints` and timer arms touch nothing. `FindToggleItem` scans the inventory;
//! only `OnUseToggleItem` reaches the follower here, as no login, drop or destroy path
//! calls it (the item load's `OnAfterCreatedItem` arm is next).
//!
//! The summon stands mob 20114 within 100 of the owner (`CShamanActor::Summon`,
//! `:93-141`), of the owner's empire with the name `Buff`, facing the owner's heading
//! turned half round, wearing 11899 with weapon 5109 (`UpdateEquipment`, `:143-150`) —
//! the parts the additional record names. The follow runs every 3 Pulses (the
//! `shamansystem_update_event`, `:50-68`, with the 400 ms period an event already slower
//! than it): a moved or dropped summon item unsummons, then a live owner is buffed
//! (`CShamanActor::Update`, `:161-172`) and the follower walks — running past 750 — to
//! 250 of the owner, re-standing 200 behind past 4500 (`UpdateFollowAI`, `:447-487`).
//!
//! `UseSkills` (`:392-416`) casts the first missing buff of the five, in index order,
//! one per tick past a 3-second stamp: 94 (`AFF_HOSIN`, `POINT_RESIST_NORMAL_DAMAGE`,
//! 1.3× level), 95 (`AFF_BOHO`, `POINT_MAX_HP`, 100× level), 96 (`AFF_GICHEON`,
//! `POINT_CRITICAL_PCT`, 1.3× level), 111 (`AFF_JEUNGRYEOK`, `POINT_ATT_GRADE`,
//! 8.75× level, premium), 110 (`AFF_KWAESOK`, `POINT_NORMAL_HIT_DAMAGE_BONUS`,
//! 0.3× level, premium, plus a same-flag `POINT_CASTING_SPEED` row of the bare level).
//! Each lands through `AddAffect` for the hour (`DEFAULT_SKILLS_TIME`) and is announced
//! to the view with `GC_AUTO_SHAMAN_SKILL` (`SendAutoShamanSkill`, `G/char.cpp:12745`).
//! The premium arms stay dark: 69699's use arm chats the success line but never trains
//! (`G/char_item.cpp:6944-6964`), so the `shaman_system.is_premium` flag is never set and
//! `HasPremium` never holds. Levels read the `shaman_system.skill_level{i}` quest flags,
//! clamped to 20-40 (`GetBuffLevel`); unset, all five are 20.
//!
//! The login sends `RECV_ShamanSkills` and `RECV_ShamanPremium` for a standing follower
//! (`SendAutoShamanInformations`, `G/char.cpp:12751-12757`, at `G/input_login.cpp:952`);
//! the TEST lines `GetBuffLevel` and `SetBuffLevel` chat stay out as Defects, as does the
//! Korean-commented quest-training stub the book path never reaches through a quest.
//! The purge spares the follower (`G/cmd_gm.cpp:843`) and the state machine skips it
//! (`G/char_state.cpp:518`); the duel, purge, mount-ride and observer arms around them
//! (which set nothing the follower reads) stay with their systems.
//!
//! Book training (`TrainSkill`, `UpgradeSkill`) and the quest arms
//! (`pc.get_buff_level`, `pc.learn_shaman_skill`) are next: no owner quest names either,
//! and the books 50494-50496/50510-50511 wait for the follower first.
//!
//! Everything here runs on the game thread only (ADR-0002).

use common::cfloat::f32_to_i32;
use common::vid::Vid;
use gamedata::item_kind::{ITEM_TOGGLE, TOGGLE_SHAMAN};
use world::character::number;

use super::view::EntityKey;
use super::GameState;
use crate::item_move::Mover;
use crate::sync_position::distance_approx;

/// The Pulses between two shaman updates: the `shamansystem_update_event` (`:50-68`)
/// re-arms in 3 Pulses; the 400 ms period (`m_dwUpdatePeriod`) never gates it, because
/// the event already fires slower than the period.
const SHAMAN_STATE_PULSES: u64 = 3;
/// `SHAMAN_MOB_VNUM`: the mob every follower stands as.
const SHAMAN_MOB: u32 = 20_114;
/// The name every follower stands with (`SummonItem`, `:566`).
const SHAMAN_NAME: &[u8] = b"Buff";
/// The weapon part `UpdateEquipment` dresses the follower in.
const SHAMAN_WEAPON_PART: u16 = 5109;
/// The armour part `UpdateEquipment` dresses the follower in.
const SHAMAN_ARMOUR_PART: u16 = 11_899;
/// `RESPAWN_DISTANCE`: past here the follower re-stands by the owner.
const RESPAWN_DISTANCE: i32 = 4500;
/// `START_FOLLOW_DISTANCE`: past here the follower follows the owner.
const START_FOLLOW_DISTANCE: i32 = 400;
/// `START_RUN_DISTANCE`: past here the follower runs instead of walking.
const START_RUN_DISTANCE: i32 = 750;
/// `APPROACH`: how far behind the owner a re-stand lands.
const APPROACH: f32 = 200.0;
/// The buffs' duration: `DEFAULT_SKILLS_TIME`, one hour.
const BUFF_SECONDS: i32 = 3600;
/// The seconds between two buff casts (`UseSkills`, `:413`).
const BUFF_COOLDOWN_SECONDS: u32 = 3;
/// The level an unset buff flag reads (`DEFAULT_SHAMAN_SKILLS_LEVEL`, `:24`).
const DEFAULT_BUFF_LEVEL: i32 = 20;
/// The most a buff flag reads before it clamps (`SKILL_MAX_LEVEL`, `C/length.h:75`).
const MAX_BUFF_LEVEL: i32 = 40;

/// A summoned buff follower standing in the world.
#[derive(Debug, Clone, Copy)]
pub(super) struct ShamanNpc {
    /// The owner that activated the summoning item.
    owner: Vid,
    /// The summoning item's id (`SummonItem`'s `itemId`).
    summon_item: u32,
    /// `m_dwLastSkillTime`: `get_global_time` past which the next buff may cast.
    next_buff_at: u32,
}

/// One of the five buffs (`m_BuffSkills`, `:39-46`), in the order `UseSkills` reads them.
#[derive(Debug, Clone, Copy)]
struct BuffSkill {
    /// The skill's vnum, which is the affect kind too.
    vnum: u32,
    /// The affect flag the cast sets and `UseSkills` checks.
    flag: u32,
    /// The point the cast lands on.
    point: usize,
    /// The multiplier the cast scales the buff level by.
    factor: f32,
    /// Whether the cast needs the premium flag.
    premium: bool,
}

/// The five buffs (`m_BuffSkills`, `:39-46`), in `UseSkills`' index order.
const BUFF_SKILLS: [BuffSkill; 5] = [
    BuffSkill {
        vnum: 94,
        flag: crate::loading_phase::AFF_HOSIN,
        point: common::point_slot::POINT_RESIST_NORMAL_DAMAGE,
        factor: 1.3,
        premium: false,
    },
    BuffSkill {
        vnum: 95,
        flag: crate::loading_phase::AFF_BOHO,
        point: common::point_slot::POINT_MAX_HP,
        factor: 100.0,
        premium: false,
    },
    BuffSkill {
        vnum: 96,
        flag: crate::loading_phase::AFF_GICHEON,
        point: common::point_slot::POINT_CRITICAL_PCT,
        factor: 1.3,
        premium: false,
    },
    BuffSkill {
        vnum: 111,
        flag: crate::loading_phase::AFF_JEUNGRYEOK,
        point: common::point_slot::POINT_ATT_GRADE,
        factor: 8.75,
        premium: true,
    },
    BuffSkill {
        vnum: 110,
        flag: crate::loading_phase::AFF_KWAESOK,
        point: common::point_slot::POINT_NORMAL_HIT_DAMAGE_BONUS,
        factor: 0.3,
        premium: true,
    },
];

impl GameState {
    /// The toggle item at `at` of the character under `vid`, when the step is a toggle
    /// use: an item whose proto reads `ITEM_TOGGLE`/`TOGGLE_SHAMAN` (the owner's 69700).
    fn shaman_toggle_at(
        &self,
        vid: Vid,
        at: protocol::item_pos::ItemPos,
    ) -> Option<world::item::Item> {
        let character = self.characters.find_by_vid(vid).ok()?;
        let item = character.items().item_at(at)?.clone();
        let proto = self.protos.get(item.vnum)?;
        (proto.item_type == ITEM_TOGGLE && proto.sub_type == TOGGLE_SHAMAN).then_some(item)
    }

    /// Whether `CG_ITEM_USE` at `at` is a toggle use for the character under `vid`.
    pub(super) fn shaman_toggle_use(&self, vid: Vid, at: protocol::item_pos::ItemPos) -> bool {
        self.shaman_toggle_at(vid, at).is_some()
    }

    /// Run the shaman arms of `CG_ITEM_USE` at `at` for the character under `vid`:
    /// the toggle, then the buff-premium item. `None` when neither arm fits.
    pub(super) fn use_shaman_item(
        &mut self,
        vid: Vid,
        at: protocol::item_pos::ItemPos,
        actor: Mover,
    ) -> Option<Result<crate::item_move::MovedItems, crate::item_move::MoveItemRefused>> {
        if self.shaman_toggle_use(vid, at) {
            return Some(self.use_shaman_toggle(vid, at, actor));
        }
        self.shaman_premium_use(vid, at)
            .then(|| self.use_shaman_premium(vid, actor))
    }

    /// Whether `CG_ITEM_USE` at `at` is the buff-premium item for the character under
    /// `vid`: an `ITEM_USE`/`USE_UNLOCK_SHAMAN` (the owner's 69699).
    pub(super) fn shaman_premium_use(&self, vid: Vid, at: protocol::item_pos::ItemPos) -> bool {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at).cloned())
            .and_then(|item| self.protos.get(item.vnum).cloned())
            .is_some_and(|proto| {
                proto.item_type == gamedata::item_kind::ITEM_USE
                    && proto.sub_type == gamedata::item_kind::USE_UNLOCK_SHAMAN
            })
    }

    /// `USE_UNLOCK_SHAMAN` (`G/char_item.cpp:6944-6964`): the success line, and the
    /// item kept — its training arm is commented out, so nothing else runs. The use
    /// answers as one that was never run, as the read's refusal does.
    pub(super) fn use_shaman_premium(
        &mut self,
        vid: Vid,
        actor: Mover,
    ) -> Result<crate::item_move::MovedItems, crate::item_move::MoveItemRefused> {
        use crate::item_move::MoveItemRefused;
        self.tell(
            vid,
            actor,
            protocol::gc_chat::CHAT_TYPE_INFO,
            b"You upgrade your buff successfully",
            &[],
        );
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let mut moved = crate::item_move::MovedItems::new(
            character.player_id(),
            vid.raw(),
            world::character::MoveDone {
                kind: world::character::MoveKind::Used,
                records: Vec::new(),
                changes: Vec::new(),
            },
            actor,
            &self.protos,
            &self.locale,
        );
        moved.points = character.points().cloned();
        moved.quickslots = Some(character.quickslots().clone());
        Ok(moved)
    }

    /// Whether the character under `vid` activates a toggle: the item's socket-3 bit
    /// (`ITEM_SOCKET_TOGGLE_ACTIVE`, `C/item_length.h:51`).
    pub(super) fn shaman_item_active(&self, vid: Vid, id: u32) -> bool {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item(id))
            .is_some_and(|item| {
                item.sockets[usize::try_from(common::constants::ITEM_SOCKET_TOGGLE_ACTIVE)
                    .unwrap_or(usize::MAX)]
                    != 0
            })
    }

    /// Whether the item `id` of the character under `vid` is an active toggle:
    /// `CItem::isLocked` as the toggle sets it (`ActivateToggleItem`, `:78`). The
    /// socket-3 bit stands for the lock, as the activation is the only writer.
    pub(super) fn shaman_locked(&self, vid: Vid, id: u32) -> bool {
        self.shaman_item_active(vid, id)
    }

    /// Whether the item at `at` of the character under `vid` is an active toggle: the
    /// `isLocked` check the destroy, drop, move, sell, check-in and refine paths run.
    pub(super) fn shaman_locked_at(&self, vid: Vid, at: protocol::item_pos::ItemPos) -> bool {
        self.shaman_toggle_at(vid, at)
            .is_some_and(|item| self.shaman_locked(vid, item.id))
    }

    /// The follower of the character under `vid` summoned by `id`, if one stands.
    fn summoned_shaman(&self, vid: Vid, id: u32) -> Option<u32> {
        self.shaman_npcs
            .iter()
            .find(|(_, stood)| stood.owner == vid && stood.summon_item == id)
            .map(|(&stood, _)| stood)
    }

    /// `CShamanActor::Summon` (`:93-141`) for the activated `item`, then the summon
    /// report (`UpdateInformations`) the activation answers with. A mob the spawn
    /// refuses stands nothing and stays silent, as legacy's null `SpawnMob` does past
    /// its `sys_err`.
    pub(super) fn shaman_summon(
        &mut self,
        vid: Vid,
        item: &world::item::Item,
        mover: Mover,
        records: &mut Vec<Vec<u8>>,
    ) {
        let (Some(place), Some(spot)) = (self.place_of(vid.raw()), self.spot_of(vid.raw())) else {
            return;
        };
        // `GetDungeon() == NULL && GetMapIndex() < 10000` (`:97`): in a dungeon, or on a
        // map past 10000, the follower stands on the owner. No dungeon is ported.
        let (dx, dy) = if place.1 < 10_000 {
            (
                number(&mut self.dice, -100, 100),
                number(&mut self.dice, -100, 100),
            )
        } else {
            (0, 0)
        };
        let rotation = self.bodies.get(&vid).map_or(0.0, |body| body.rotation);
        let at = (spot.x.saturating_add(dx), spot.y.saturating_add(dy));
        let Some(mut stood) = self.spawn_horse(place, SHAMAN_MOB, at, f32_to_i32(rotation + 180.0))
        else {
            return;
        };
        let stood_vid = stood.vid;
        stood.empire = mover.empire;
        stood.name = SHAMAN_NAME.to_vec();
        // `UpdateEquipment` (`:143-150`): weapon 5109, armour 11899. The parts land in
        // the additional record, as `EncodeInsertPacket` reads them (`G/char.cpp:1111`).
        stood.parts[0] = SHAMAN_ARMOUR_PART;
        stood.parts[1] = SHAMAN_WEAPON_PART;
        // `UpdatePacket` (`:148`): the neighbourhood learns the parts at once.
        let pulse = self.last_pulse;
        self.stand_runtime(place, vec![stood], pulse);
        self.send_shaman_update(stood_vid);
        self.shaman_npcs.insert(
            stood_vid,
            ShamanNpc {
                owner: vid,
                summon_item: item.id,
                next_buff_at: 0,
            },
        );
        self.push_shaman_informations(vid, mover, records);
    }

    /// The `GC_CHARACTER_UPDATE` the summon answers with (`UpdateEquipment`, `:148`):
    /// the standing parts, sent to the neighbourhood only, as the follower has no desc.
    fn send_shaman_update(&mut self, stood: u32) {
        let Some(npc) = self.npc_by_vid(stood) else {
            return;
        };
        let look = world::character::CharacterLook {
            parts: npc.parts,
            moving_speed: u8::try_from(super::mob_battle::npc_limit_speed(
                npc.fight.point(common::point_slot::POINT_MOV_SPEED),
            ))
            .unwrap_or(u8::MAX),
            attack_speed: u8::try_from(super::mob_battle::npc_limit_speed(
                npc.fight.point(common::point_slot::POINT_ATT_SPEED),
            ))
            .unwrap_or(u8::MAX),
            level: u32::from(npc.fight.level),
            conqueror_level: 0,
            refine_element_type: 0,
            mount_vnum: 0,
        };
        let mover = Mover {
            recently_fought: false,
            empire: npc.empire,
            language: 0,
            pk_mode: 0,
            affect_flags: npc.affect_flags,
            dead: npc.dead,
            alignment: 0,
            view_guild: (0, 0),
            premium: (0, 0),
        };
        let mut frame = Vec::new();
        crate::item_move::character_update(stood, &look, mover).encode_into(&mut frame);
        self.packet_around(EntityKey::Npc(stood), &frame, None);
    }

    /// `CShamanSystem::UnsummonItem` (`:665-677`) for `id`: the follower it summoned
    /// goes (the entry drop is `M2_DESTROY_CHARACTER`'s, through `destroy_npc`).
    pub(super) fn shaman_unsummon_item(&mut self, vid: Vid, id: u32) {
        if let Some(stood) = self.summoned_shaman(vid, id) {
            self.destroy_npc(stood);
        }
    }

    /// `CShamanSystem::GetByVID`'s drop (`DeletePet`'s twin): a destroyed follower
    /// drops its entry.
    pub(super) fn forget_shaman_npc(&mut self, stood: u32) {
        self.shaman_npcs.remove(&stood);
    }

    /// The leave's follower drop (`CHARACTER::Destroy`, `G/char.cpp:618-629`): every
    /// follower of the character under `vid` goes with it.
    pub(super) fn leave_shamans(&mut self, vid: Vid) {
        let stood: Vec<u32> = self
            .shaman_npcs
            .iter()
            .filter(|(_, stood)| stood.owner == vid)
            .map(|(&stood, _)| stood)
            .collect();
        for stood in stood {
            self.destroy_npc(stood);
        }
    }

    /// Whether the NPC under `vid` is a summoned buff follower (`IsAutoShaman`), which
    /// the state machine skips and `do_purge` spares.
    pub(super) fn is_shaman_npc(&self, vid: u32) -> bool {
        self.shaman_npcs.contains_key(&vid)
    }

    /// `CG_ITEM_USE` of the shaman toggle (`case ITEM_TOGGLE`, `G/char_item.cpp:7149-7152`):
    /// the toggle flips, the follower summons or goes, and `CheckMaximumPoints` and
    /// `UpdatePacket` run after it (`G/ItemUtils.cpp:99-101`, `:129-131`). A use the
    /// checks refuse answers nothing and changes nothing, as `OnUseToggleItem`'s false
    /// does (`:7150-7151`).
    pub(super) fn use_shaman_toggle(
        &mut self,
        vid: Vid,
        at: protocol::item_pos::ItemPos,
        actor: Mover,
    ) -> Result<crate::item_move::MovedItems, crate::item_move::MoveItemRefused> {
        use world::character::{use_toggle_item, Toggled};
        let group = self
            .shaman_toggle_at(vid, at)
            .and_then(|item| self.protos.get(item.vnum))
            .map_or(-1, |proto| {
                proto.values[usize::try_from(common::constants::ITEM_VALUE_TOGGLE_GROUP)
                    .unwrap_or(usize::MAX)]
            });
        // `FindToggleItem(ch, true, subType, group)` (`:187`): another active toggle of
        // the same subtype and group refuses. The owner's proto reads group 0 (value
        // 5) and no second toggle exists, so this never fires on the owner's data.
        let group_index =
            usize::try_from(common::constants::ITEM_VALUE_TOGGLE_GROUP).unwrap_or(usize::MAX);
        let active_index =
            usize::try_from(common::constants::ITEM_SOCKET_TOGGLE_ACTIVE).unwrap_or(usize::MAX);
        let group_taken = group != -1
            && self.characters.find_by_vid(vid).is_ok_and(|character| {
                (0..common::item_slots::INVENTORY_MAX_NUM).any(|cell| {
                    let pos = protocol::item_pos::ItemPos::new(
                        common::item_slots::EWindows::Inventory as u8,
                        cell,
                    );
                    character
                        .items()
                        .item_at(pos)
                        .filter(|item| item.pos != at)
                        .is_some_and(|item| {
                            self.protos.get(item.vnum).is_some_and(|proto| {
                                proto.item_type == ITEM_TOGGLE
                                    && proto.sub_type == TOGGLE_SHAMAN
                                    && proto.values.get(group_index).copied().unwrap_or(-1) == group
                            }) && item.sockets.get(active_index).copied().unwrap_or(0) != 0
                        })
                })
            });
        let mut flipped: Option<Toggled> = None;
        let mut moved = self.run_item_step(vid, actor, |items, ids, rules, gear, _protos| {
            use_toggle_item(items, ids, at, rules, gear, group_taken).map(|(done, toggled)| {
                flipped = Some(toggled);
                done
            })
        })?;
        let Some(flipped) = flipped else {
            return Ok(moved);
        };
        match flipped {
            Toggled::On { id } => {
                if let Some(item) = self
                    .characters
                    .find_by_vid(vid)
                    .ok()
                    .and_then(|character| character.items().item(id).cloned())
                {
                    self.shaman_summon(vid, &item, actor, &mut moved.records);
                }
            }
            Toggled::Off { id } => self.shaman_unsummon_item(vid, id),
        }
        // `CheckMaximumPoints` (`:100`, `:130`), then `UpdatePacket` (`:101`, `:131`):
        // the clamp's records and the update ride the answer; the update reaches the
        // view too, as `PacketAround` does.
        if let Some(points) = self.pc_points_mut(vid.raw()) {
            let clamped = points.check_maximum_points();
            let mut frames = crate::loading_phase::point_changes(&clamped, vid.raw());
            moved.records.append(&mut frames);
        }
        let flags = self.affect_flags_of(vid);
        if let Some(frame) = self.update_frame(vid, flags) {
            self.packet_around(super::view::EntityKey::Character(vid.raw()), &frame, None);
            moved.records.push(frame);
        }
        Ok(moved)
    }

    /// `CShamanActor::GetBuffLevel` (`:340-348`) for buff `index`: the
    /// `shaman_system.skill_level{i}` quest flag, clamped to 20-40. The TEST line
    /// legacy chats with every read is a Defect and stays out.
    fn buff_level(&self, pid: u32, index: usize) -> i32 {
        let flag = format!("shaman_system.skill_level{index}");
        let level = self
            .quests
            .as_ref()
            .and_then(|quests| quests.manager().quest_flag(pid, flag.as_bytes()))
            .unwrap_or(DEFAULT_BUFF_LEVEL);
        level.clamp(DEFAULT_BUFF_LEVEL, MAX_BUFF_LEVEL)
    }

    /// `CShamanActor::HasPremium` (`:386-389`): the `shaman_system.is_premium` quest
    /// flag is not 0. Never set: 69699's use arm chats the success line but never
    /// trains (`G/char_item.cpp:6944-6964`), so no ported path writes the flag.
    fn shaman_premium(&self, pid: u32) -> bool {
        self.quests
            .as_ref()
            .and_then(|quests| {
                quests
                    .manager()
                    .quest_flag(pid, b"shaman_system.is_premium")
            })
            .unwrap_or(0)
            != 0
    }

    /// `CShamanSystem::TrainSkill` (`:597-615`) for the book's skill vnum: a
    /// standing follower whose five hold the vnum trains it (`UpgradeSkill`,
    /// `:231-324`), and the read runs as training. Else the read falls on to the
    /// normal learn, as legacy's false does.
    pub(super) fn shaman_train_book(&mut self, vid: Vid, mover: Mover, vnum: u32) -> bool {
        let Some(stood) = self
            .shaman_npcs
            .iter()
            .filter(|(_, stood)| stood.owner == vid)
            .map(|(&stood, _)| stood)
            .min()
        else {
            return false;
        };
        let Some(index) = BUFF_SKILLS.iter().position(|skill| skill.vnum == vnum) else {
            return false;
        };
        // `IsTrainingSkill` (`:174-188`): a premium buff without the premium stands
        // over, and `TrainSkill` answers false.
        let skill = BUFF_SKILLS[index];
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        if skill.premium && !self.shaman_premium(pid) {
            return false;
        }
        self.upgrade_shaman_skill(vid, mover, stood, pid, index, vnum);
        true
    }

    /// `CShamanActor::UpgradeSkill` (`:231-324`) for buff `index`: the read-time gate
    /// (with the no-delay book) and the level-30 stone line, then the read body.
    /// The book's spend is the step's: the read ran, so the world consumes one.
    fn upgrade_shaman_skill(
        &mut self,
        vid: Vid,
        mover: Mover,
        stood: u32,
        pid: u32,
        index: usize,
        vnum: u32,
    ) {
        use super::pc_skill::AFFECT_SKILL_NO_BOOK_DELAY;
        let now = Self::global_time();
        let next = self
            .pc_skills
            .get(&vid.raw())
            .and_then(|state| {
                usize::try_from(vnum)
                    .ok()
                    .and_then(|vnum| state.next_read.get(vnum).copied())
            })
            .unwrap_or(0);
        let mut cont = now >= next;
        if !cont
            && self.affect_events.get(&vid).is_some_and(|event| {
                Self::has_book_affect(&event.affects, AFFECT_SKILL_NO_BOOK_DELAY)
            })
        {
            cont = true;
            let _gone = self.remove_pc_affect(vid, AFFECT_SKILL_NO_BOOK_DELAY);
        }
        if !cont {
            self.skill_learn_wait_more_time_message(vid, mover, next.saturating_sub(now));
            return;
        }
        if self.buff_level(pid, index) >= 30 {
            self.tell(
                vid,
                mover,
                protocol::gc_chat::CHAT_TYPE_INFO,
                b"You cannot train buffer skill with the book, you need to use special stone.",
                &[],
            );
            return;
        }
        self.read_shaman_book(vid, mover, stood, pid, index, vnum, now);
    }

    /// The read body of `UpgradeSkill`: the 65% read — a bonus-affect read always
    /// succeeds — counting reads past the level's books (`shaman_system.read_count{i}`)
    /// until the level climbs, the report follows, and the fresh buff casts at once
    /// (`UseSkill`). The failing read stamps the delay and chats its two lines.
    #[allow(clippy::too_many_arguments)]
    fn read_shaman_book(
        &mut self,
        vid: Vid,
        mover: Mover,
        stood: u32,
        pid: u32,
        index: usize,
        vnum: u32,
        now: u32,
    ) {
        use super::pc_skill::{AFFECT_SKILL_BOOK_BONUS, SKILLBOOK_DELAY_MAX, SKILLBOOK_DELAY_MIN};
        let delay = number(&mut self.dice, SKILLBOOK_DELAY_MIN, SKILLBOOK_DELAY_MAX);
        let stamp = now.saturating_add(u32::try_from(delay).unwrap_or(0));
        let read_flag = format!("shaman_system.read_count{index}");
        let read_count = self
            .quests
            .as_ref()
            .and_then(|quests| quests.manager().quest_flag(pid, read_flag.as_bytes()))
            .unwrap_or(0);
        let mut percent = 65;
        if self
            .affect_events
            .get(&vid)
            .is_some_and(|event| Self::has_book_affect(&event.affects, AFFECT_SKILL_BOOK_BONUS))
        {
            percent = 0;
            let _gone = self.remove_pc_affect(vid, AFFECT_SKILL_BOOK_BONUS);
        }
        // `number(1, 100) > percent` (`:282`): past the threshold the read works.
        if number(&mut self.dice, 1, 100) > percent {
            let need = self
                .buff_level(pid, index)
                .saturating_sub(DEFAULT_BUFF_LEVEL);
            if read_count >= need {
                let level = self.buff_level(pid, index).saturating_add(1);
                self.set_buff_level(pid, index, level);
                self.send_shaman_informations(vid, mover);
                if let Some(quests) = self.quests.as_mut() {
                    quests
                        .manager_mut()
                        .set_quest_flag(pid, read_flag.as_bytes(), 0);
                }
                self.stamp_skill_next_read(vid, vnum, stamp);
                self.tell(
                    vid,
                    mover,
                    protocol::gc_chat::CHAT_TYPE_INFO,
                    b"You finished your training with the Book successfully read.",
                    &[],
                );
                self.shaman_use_skill(vid, stood, index);
            } else {
                if let Some(quests) = self.quests.as_mut() {
                    quests.manager_mut().set_quest_flag(
                        pid,
                        read_flag.as_bytes(),
                        read_count.saturating_add(1),
                    );
                }
                self.tell(
                    vid,
                    mover,
                    protocol::gc_chat::CHAT_TYPE_INFO,
                    b"You need: %d more books to level up this skill.",
                    &[crate::chat_line::Arg::Int(i64::from(
                        need.saturating_sub(read_count),
                    ))],
                );
            }
            return;
        }
        self.stamp_skill_next_read(vid, vnum, stamp);
        self.tell(
            vid,
            mover,
            protocol::gc_chat::CHAT_TYPE_INFO,
            b"That did not work. Darn!",
            &[],
        );
        self.tell(
            vid,
            mover,
            protocol::gc_chat::CHAT_TYPE_INFO,
            b"Training failed. Please try again later.",
            &[],
        );
    }

    /// `CShamanActor::SetBuffLevel` (`:351-356`): the `shaman_system.skill_level{i}`
    /// quest flag takes `level`. The TEST chat stays out as a Defect.
    fn set_buff_level(&mut self, pid: u32, index: usize, level: i32) {
        let flag = format!("shaman_system.skill_level{index}");
        if let Some(quests) = self.quests.as_mut() {
            quests
                .manager_mut()
                .set_quest_flag(pid, flag.as_bytes(), level);
        }
    }

    /// `SetSkillNextReadTime(vnum, stamp)`: the book-climb delay for `vnum`.
    fn stamp_skill_next_read(&mut self, vid: Vid, vnum: u32, stamp: u32) {
        if vnum >= 255 {
            return;
        }
        self.pc_skill_state(vid.raw()).next_read[vnum as usize] = stamp;
        self.send_skill_levels(vid);
    }

    /// `CShamanActor::UseSkill` (`:392-400`) for buff `index`: the cast's 3-second
    /// stamp clears, and the owner's rows of the buff's vnum go, so the next tick
    /// re-casts at the fresh level. The buff tick never calls it.
    pub(super) fn shaman_use_skill(&mut self, owner: Vid, stood: u32, index: usize) {
        let Some(skill) = BUFF_SKILLS.get(index) else {
            return;
        };
        if let Some(entry) = self.shaman_npcs.get_mut(&stood) {
            entry.next_buff_at = 0;
        }
        let _gone = self.remove_pc_affect(owner, skill.vnum);
    }

    /// `CShamanActor::UseSkills` (`:402-416`) for the follower under `stood`: past its
    /// stamp, the first buff whose flag the owner lacks casts — one per tick — and
    /// the 3-second stamp lands. A premium buff without the premium stands over.
    fn use_shaman_skills(&mut self, stood: u32, owner: Vid, pid: u32) {
        let Some(entry) = self.shaman_npcs.get(&stood).copied() else {
            return;
        };
        let now = Self::global_time();
        if entry.next_buff_at >= now {
            return;
        }
        // `IsLoadedAffect` (`:404`): the load flag `LoadAffect` sets (`:655`). The
        // save restores the rows at admission, so every standing owner holds it.
        for (index, skill) in BUFF_SKILLS.iter().enumerate() {
            if skill.premium && !self.shaman_premium(pid) {
                continue;
            }
            if self.has_affect_flag(owner, skill.flag) {
                continue;
            }
            let level = self.buff_level(pid, index);
            // `static_cast<int>(level * factor)` (`:406`): the float product cast to an
            // `int`, truncating toward zero.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let value = (f32::from(i16::try_from(level).unwrap_or(0)) * skill.factor) as i32;
            self.add_pc_affect(
                owner,
                super::affect::Affect::buff(
                    skill.vnum,
                    skill.point,
                    value,
                    skill.flag,
                    BUFF_SECONDS,
                    0,
                ),
                false,
            );
            // `AFF_KWAESOK` (`:407-408`): the second same-flag row, on casting speed,
            // of the bare level. The shaman's own cast never lands it (the client's
            // 110 row carries its own `POINT_CASTING_SPEED` amount), so the twin is
            // the follower's own.
            if skill.flag == crate::loading_phase::AFF_KWAESOK {
                self.add_pc_affect(
                    owner,
                    super::affect::Affect::buff(
                        skill.vnum,
                        common::point_slot::POINT_CASTING_SPEED,
                        level,
                        skill.flag,
                        BUFF_SECONDS,
                        0,
                    ),
                    false,
                );
            }
            let mut record = Vec::new();
            protocol::gc_fields::GcShamanSkill::new(
                skill.vnum,
                stood,
                u8::try_from(level).unwrap_or(u8::MAX),
            )
            .encode_into(&mut record);
            self.packet_around(EntityKey::Npc(stood), &record, None);
            if let Some(entry) = self.shaman_npcs.get_mut(&stood) {
                entry.next_buff_at = now.saturating_add(BUFF_COOLDOWN_SECONDS);
            }
            return;
        }
    }

    /// Whether the player under `vid` holds affect flag `flag` (`IsAffectFlag`,
    /// `G/char_affect.cpp:886-889`): bit `flag - 1` of the two affect words.
    fn has_affect_flag(&self, vid: Vid, flag: u32) -> bool {
        let words = self.affect_flags_of(vid);
        let Some(bit) = flag.checked_sub(1) else {
            return false;
        };
        let Some(bit) = usize::try_from(bit).ok() else {
            return false;
        };
        bit < 64 && words[bit / 32] & (1 << (bit % 32)) != 0
    }

    /// `CShamanActor::UpdateInformations` (`:324-338`) for the owner under `vid`: the
    /// `RECV_ShamanSkills` line and the `RECV_ShamanPremium` line. Sent while the
    /// owner stands in `PHASE_GAME`, as legacy gates on the desc's phase.
    pub(super) fn send_shaman_informations(&self, vid: Vid, mover: Mover) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        let mut line = b"RECV_ShamanSkills ".to_vec();
        for (index, skill) in BUFF_SKILLS.iter().enumerate() {
            line.extend_from_slice(skill.vnum.to_string().as_bytes());
            line.push(b'|');
            line.extend_from_slice(self.buff_level(pid, index).to_string().as_bytes());
            line.push(b',');
        }
        line.pop();
        self.tell(vid, mover, protocol::gc_chat::CHAT_TYPE_COMMAND, &line, &[]);
        let premium = u8::from(self.shaman_premium(pid));
        let line = format!("RECV_ShamanPremium {premium}");
        self.tell(
            vid,
            mover,
            protocol::gc_chat::CHAT_TYPE_COMMAND,
            line.as_bytes(),
            &[],
        );
    }

    /// The two lines [`send_shaman_informations`] tells, appended to `records`: for
    /// the summon, whose answer carries them — the owner's outbox only carries
    /// view records, so a `tell` would never arrive.
    fn push_shaman_informations(&self, vid: Vid, mover: Mover, records: &mut Vec<Vec<u8>>) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        let mut line = b"RECV_ShamanSkills ".to_vec();
        for (index, skill) in BUFF_SKILLS.iter().enumerate() {
            line.extend_from_slice(skill.vnum.to_string().as_bytes());
            line.push(b'|');
            line.extend_from_slice(self.buff_level(pid, index).to_string().as_bytes());
            line.push(b',');
        }
        line.pop();
        records.push(crate::chat_line::chat_packet(
            mover.recipient(&self.locale),
            protocol::gc_chat::CHAT_TYPE_COMMAND,
            &line,
            &[],
        ));
        let premium = u8::from(self.shaman_premium(pid));
        let line = format!("RECV_ShamanPremium {premium}");
        records.push(crate::chat_line::chat_packet(
            mover.recipient(&self.locale),
            protocol::gc_chat::CHAT_TYPE_COMMAND,
            line.as_bytes(),
            &[],
        ));
    }

    /// `CShamanSystem::SendInformations` (`:631-641`): the first standing follower's
    /// report. Silent with none standing.
    pub(super) fn send_shaman_report(&self, vid: Vid) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        if self.shaman_npcs.values().any(|stood| stood.owner == vid) {
            self.send_shaman_informations(vid, mover);
        }
    }

    /// The shaman system's update event (`shamansystem_update_event`, `:50-68`): every
    /// 3 Pulses each follower runs `Update`, in VID order. A follower whose NPC is
    /// gone drops its entry, as `DeletePet`'s twin drops the actor.
    pub(super) fn run_shaman_ticks(&mut self, pulse: u64) {
        if pulse % SHAMAN_STATE_PULSES != 0 {
            return;
        }
        let mut stood: Vec<u32> = self.shaman_npcs.keys().copied().collect();
        stood.sort_unstable();
        for stood in stood {
            if self.npc_by_vid(stood).is_none() {
                self.shaman_npcs.remove(&stood);
            } else {
                self.idle_shaman(stood);
            }
        }
    }

    /// The follower's `Update` (`:161-172`) for the NPC under `stood`: a follower
    /// whose summon item is gone, dropped, or held by another goes; else a live
    /// owner is buffed, then the follow runs. A dead owner keeps the follow but
    /// gets no buff.
    fn idle_shaman(&mut self, stood: u32) {
        let Some(entry) = self.shaman_npcs.get(&stood).copied() else {
            return;
        };
        if !self.shaman_item_held(entry.owner, entry.summon_item) {
            self.destroy_npc(stood);
            return;
        }
        if !self.is_dead(entry.owner.raw()) {
            let pid = self
                .characters
                .find_by_vid(entry.owner)
                .map_or(0, world::character::Character::player_id);
            self.use_shaman_skills(stood, entry.owner, pid);
        }
        self.follow_shaman(stood, entry.owner);
    }

    /// Whether the summon item is still held by the owner (`FindByVID` and its
    /// owner, the pet twin's `:222`).
    fn shaman_item_held(&self, owner: Vid, id: u32) -> bool {
        self.characters
            .find_by_vid(owner)
            .ok()
            .and_then(|character| character.items().item(id))
            .is_some()
    }

    /// `UpdateFollowAI` (`:447-487`): past 4500 the follower re-stands 200 behind
    /// the owner's heading; past 400 it walks — running past 750 — to 250 of the
    /// owner, by the shared [`GameState::follow_actor`]; nearer it waits. A
    /// re-stand the sectree refuses answers false and the follow runs instead.
    fn follow_shaman(&mut self, stood: u32, owner: Vid) {
        let (Some(at), Some(spot)) = (self.npc_spot(stood), self.spot_of(owner.raw())) else {
            return;
        };
        let dist = distance_approx(at.0.saturating_sub(spot.x), at.1.saturating_sub(spot.y));
        if dist < START_FOLLOW_DISTANCE {
            return;
        }
        if dist >= RESPAWN_DISTANCE && self.show_shaman(stood, owner, spot) {
            return;
        }
        // Past 750 the follower runs, but only when it stands still: a moving
        // follower keeps its gait (`:471`, `!IsStateMove`).
        let moving = self
            .minds
            .get(stood)
            .is_some_and(|mind| mind.motion.dest != (at.0, at.1));
        if !moving {
            self.set_now_walking(stood, dist < START_RUN_DISTANCE);
            self.follow_actor(stood, owner);
            self.set_last_attacked(stood);
        }
    }

    /// The re-stand (`CHARACTER::Show`, `G/char.cpp:1847-1913`): 200 behind the
    /// owner's heading on the owner's map, written to the follower's
    /// last-attacked position. A point no sectree holds stands nothing and
    /// answers false, so the follow runs instead.
    fn show_shaman(&mut self, stood: u32, owner: Vid, spot: super::view::Spot) -> bool {
        let Some((channel, map)) = self.place_of(owner.raw()) else {
            return false;
        };
        let rotation = self.bodies.get(&owner).map_or(0.0, |body| body.rotation);
        let behind = rotation * core::f32::consts::PI / 180.0;
        let to = (
            spot.x.saturating_add(f32_to_i32(-APPROACH * behind.cos())),
            spot.y.saturating_add(f32_to_i32(-APPROACH * behind.sin())),
        );
        let Some(&(from_channel, from_map, at)) = self.npc_of.get(&stood) else {
            return false;
        };
        let Some(index) = self.maps.get_mut(&(channel, map)) else {
            return false;
        };
        let from_matches = self
            .npcs
            .get(&(from_channel, from_map))
            .is_some_and(|list| list.npcs.get(at).is_some_and(|npc| npc.vid == stood));
        if !from_matches {
            return false;
        }
        if from_map == map {
            let mut effects = Vec::new();
            if !index.show(
                EntityKey::Npc(stood),
                (to.0, to.1, 0),
                self.view_radius,
                &mut effects,
            ) {
                return false;
            }
            if let Some(list) = self.npcs.get_mut(&(channel, map)) {
                if let Some(npc) = std::sync::Arc::make_mut(list).npcs.get_mut(at) {
                    debug_assert_eq!(npc.vid, stood);
                    npc.x = to.0;
                    npc.y = to.1;
                    npc.z = 0;
                }
            }
            self.last_attacked_at(stood, to);
            self.deliver(&effects, None);
            return true;
        }
        if index.tree_at(to.0, to.1).is_none() {
            return false;
        }
        let Some(list) = self.npcs.get_mut(&(from_channel, from_map)) else {
            return false;
        };
        let list = std::sync::Arc::make_mut(list);
        let mut moved = list.npcs.swap_remove(at);
        if let Some(swap) = list.npcs.get(at) {
            self.npc_of.insert(swap.vid, (from_channel, from_map, at));
        }
        moved.x = to.0;
        moved.y = to.1;
        moved.z = 0;
        let mut effects = Vec::new();
        if let Some(index) = self.maps.get_mut(&(from_channel, from_map)) {
            index.remove(EntityKey::Npc(stood), &mut effects);
        }
        let list = self
            .npcs
            .entry((channel, map))
            .or_insert_with(|| std::sync::Arc::new(world::npc::MapNpcs::default()));
        let list = std::sync::Arc::make_mut(list);
        let at = list.npcs.len();
        list.npcs.push(moved);
        self.npc_of.insert(stood, (channel, map, at));
        if let Some(index) = self.maps.get_mut(&(channel, map)) {
            let _shown = index.show(
                EntityKey::Npc(stood),
                (to.0, to.1, 0),
                self.view_radius,
                &mut effects,
            );
        }
        self.last_attacked_at(stood, to);
        self.deliver(&effects, None);
        true
    }
}

#[cfg(test)]
mod tests {
    use common::item_slots::EWindows;
    use protocol::item_pos::ItemPos;
    use tokio::sync::mpsc::UnboundedReceiver;
    use world::item::{Item, ItemIdRange};

    use super::super::fixtures::{a_world, open_cells, region, TestClock, PLACE};
    use super::super::GameState;
    use super::*;
    use crate::item_move::Mover;

    /// The owner's toggle.
    const TOGGLE: u32 = 69_700;
    /// The owner.
    const KEEPER: u32 = 1;

    const fn mover() -> Mover {
        Mover {
            recently_fought: false,
            empire: 1,
            language: 0,
            pk_mode: 2,
            affect_flags: [0; 2],
            dead: false,
            alignment: 0,
            view_guild: (0, 0),
            premium: (0, 0),
        }
    }

    /// A world hosting [`PLACE`] with the owner's mob protos, so follower 20114
    /// spawns, and the owner entered at `at` holding `items`.
    fn a_shaman_world(
        clock: &TestClock,
        at: (i32, i32),
        items: &[(ItemPos, Item)],
    ) -> (GameState, UnboundedReceiver<Vec<u8>>) {
        let mut world = a_world(clock);
        let proto_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto");
        let respawner = world::npc::Respawner::new(
            std::sync::Arc::new(
                gamedata::mob_proto::MobProtos::load(&proto_dir).expect("the mob protos load"),
            ),
            std::sync::Arc::new(gamedata::mob_group::MobGroups::default()),
            std::sync::Arc::new(
                gamedata::mob_locale_names::MobLocaleNames::parse(b"VNUM\tNAME\n").unwrap(),
            ),
            world::npc::NpcVids::starting_at(world::npc::FIRST_NPC_VID),
        );
        world.install_respawner(respawner, open_cells());
        world
            .install_item_ids(ItemIdRange::new(1, 1_000_000, 1).expect("a range"))
            .expect("the ids install");
        world.stand_npcs(
            PLACE.0,
            &region(),
            world::npc::MapNpcs {
                npcs: Vec::new(),
                positions: Vec::new(),
                regens: Vec::new(),
            },
        );
        let inbox = enter_on_with(&mut world, at, items);
        (world, inbox)
    }

    /// [`fixtures::enter`], with `items` the entrant holds.
    fn enter_on_with(
        world: &mut GameState,
        at: (i32, i32),
        items: &[(ItemPos, Item)],
    ) -> UnboundedReceiver<Vec<u8>> {
        let (tx, mut inbox) = tokio::sync::mpsc::unbounded_channel();
        let name = format!("P{KEEPER}");
        world
            .enter_world_with_items(
                "",
                Vid::new(KEEPER),
                KEEPER,
                &name,
                items,
                crate::client_registry::ClientOutbox::new(tx),
            )
            .expect("the player enters");
        while inbox.try_recv().is_ok() {}
        world
            .characters
            .find_by_vid_mut(Vid::new(KEEPER))
            .expect("the player is in the world")
            .set_points(Some(super::super::fixtures::points(12_345)));
        let card = crate::loading_phase::PcCard {
            name,
            job: 0,
            empire: 1,
            level: 10,
            conqueror_level: 0,
            language: 0,
            pk_mode: crate::loading_phase::PK_MODE_PROTECT,
            skill_group: 0,
            alignment: 0,
            voice: 0,
            premium: 0,
            premium_time: 0,
        };
        let _shown = world.place_body(
            Vid::new(KEEPER),
            crate::game_loop_messages::EnterPlace {
                channel: PLACE.0,
                map: PLACE.1,
                x: at.0,
                y: at.1,
                z: 0,
            },
            card,
        );
        inbox
    }

    fn drained(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    fn toggle_item(id: u32) -> Item {
        Item::new(id, TOGGLE)
    }

    fn inv(cell: u16) -> ItemPos {
        ItemPos::new(EWindows::Inventory as u8, cell)
    }

    /// Using the toggle stands 20114 within 100 with the name `Buff`, the owner's
    /// empire, and weapon 5109 over armour 11899; using it again unsummons. The
    /// purge spares the follower and the state machine skips it.
    #[test]
    fn the_toggle_summons_and_unsummons_its_follower() {
        let clock = TestClock::default();
        clock.set(1_000);
        let (mut world, mut inbox) =
            a_shaman_world(&clock, (3200, 3200), &[(inv(0), toggle_item(7))]);
        let _ = drained(&mut inbox);
        assert!(world.shaman_npcs.is_empty());
        let account = crate::game_loop_messages::QuestAccount::default();
        world
            .use_item(Vid::new(KEEPER), inv(0), mover(), &account)
            .expect("the toggle flips on");
        assert_eq!(world.shaman_npcs.len(), 1);
        let stood = *world.shaman_npcs.keys().next().expect("the follower");
        let follower = world
            .npc_by_vid(stood)
            .expect("the follower stands")
            .clone();
        assert_eq!(follower.vnum, SHAMAN_MOB);
        assert_eq!(follower.name, b"Buff");
        assert_eq!(follower.empire, 1);
        assert_eq!((follower.parts[0], follower.parts[1]), (11899, 5109));
        assert!((follower.x - 3200).abs() <= 100);
        assert!((follower.y - 3200).abs() <= 100);
        assert!(world.is_shaman_npc(stood));
        // The socket flipped, and the toggle locks.
        let kept = world
            .characters
            .find_by_vid(Vid::new(KEEPER))
            .expect("the owner")
            .items()
            .item(7)
            .cloned()
            .expect("the toggle");
        assert_eq!(kept.sockets[3], 1);
        assert!(world.shaman_locked_at(Vid::new(KEEPER), inv(0)));
        // The purge spares it.
        world.purge_around(Vid::new(KEEPER), false, false);
        assert!(
            world.npc_by_vid(stood).is_some(),
            "the purge spares the follower"
        );
        // The second use unsummons it.
        world
            .use_item(Vid::new(KEEPER), inv(0), mover(), &account)
            .expect("the toggle flips off");
        assert!(world.shaman_npcs.is_empty());
        assert!(world.npc_by_vid(stood).is_none());
    }

    /// A tick past the follower's stamp casts the first missing buff — 94, the
    /// resist — for the hour, announced with `GC_AUTO_SHAMAN_SKILL`; the next tick
    /// casts the next. A dead owner is not buffed.
    #[test]
    fn the_follow_ticks_cast_the_missing_buffs_in_order() {
        let clock = TestClock::default();
        clock.set(1_000);
        let (mut world, mut inbox) =
            a_shaman_world(&clock, (3200, 3200), &[(inv(0), toggle_item(7))]);
        let _ = drained(&mut inbox);
        let account = crate::game_loop_messages::QuestAccount::default();
        world
            .use_item(Vid::new(KEEPER), inv(0), mover(), &account)
            .expect("the toggle flips on");
        let stood = *world.shaman_npcs.keys().next().expect("the follower");
        world.run_shaman_ticks(SHAMAN_STATE_PULSES);
        let kinds: Vec<u32> = world
            .affect_events
            .get(&Vid::new(KEEPER))
            .map(|event| event.affects.iter().map(|affect| affect.kind).collect())
            .unwrap_or_default();
        assert_eq!(kinds, [94], "the first missing buff casts");
        let cast = world
            .affect_events
            .get(&Vid::new(KEEPER))
            .and_then(|event| event.affects.iter().find(|affect| affect.kind == 94))
            .expect("the 94 row");
        assert_eq!(
            (cast.apply_on, cast.value, cast.flag, cast.remaining),
            (
                u8::try_from(common::point_slot::POINT_RESIST_NORMAL_DAMAGE).unwrap_or(u8::MAX),
                26,
                crate::loading_phase::AFF_HOSIN,
                3600
            )
        );
        // The stamp blocks the next tick: one cast per 3 real seconds.
        world.run_shaman_ticks(2 * SHAMAN_STATE_PULSES);
        let kinds: Vec<u32> = world
            .affect_events
            .get(&Vid::new(KEEPER))
            .map(|event| event.affects.iter().map(|affect| affect.kind).collect())
            .unwrap_or_default();
        assert_eq!(kinds, [94], "the 3 s stamp holds the next cast");
        // Past the stamp, the next missing buff casts. `shaman_use_skill`
        // clears the owner's 94 row too, so the tick re-casts 94.
        world.shaman_use_skill(Vid::new(KEEPER), stood, 0);
        world.run_shaman_ticks(3 * SHAMAN_STATE_PULSES);
        let kinds: Vec<u32> = world
            .affect_events
            .get(&Vid::new(KEEPER))
            .map(|event| {
                let mut kinds: Vec<u32> = event.affects.iter().map(|affect| affect.kind).collect();
                kinds.sort_unstable();
                kinds
            })
            .unwrap_or_default();
        assert_eq!(
            kinds,
            [94],
            "the freed stamp re-casts the first missing buff"
        );
        // A dead owner is not buffed.
        world.pc_combat.entry(KEEPER).or_default().dead = true;
        world.run_shaman_ticks(4 * SHAMAN_STATE_PULSES);
        let kinds: Vec<u32> = world
            .affect_events
            .get(&Vid::new(KEEPER))
            .map(|event| event.affects.iter().map(|affect| affect.kind).collect())
            .unwrap_or_default();
        assert_eq!(kinds, [94], "death keeps the follow but casts nothing");
    }

    /// Past 4500 the follower re-stands 200 behind the owner; past 750 it runs,
    /// past 400 it walks; a dropped summon item unsummons it.
    #[test]
    fn a_far_follower_stands_by_its_owner_and_a_dropped_item_unsummons_it() {
        let clock = TestClock::default();
        clock.set(1_000);
        let (mut world, mut inbox) =
            a_shaman_world(&clock, (3200, 3200), &[(inv(0), toggle_item(7))]);
        let _ = drained(&mut inbox);
        let account = crate::game_loop_messages::QuestAccount::default();
        world
            .use_item(Vid::new(KEEPER), inv(0), mover(), &account)
            .expect("the toggle flips on");
        let stood = *world.shaman_npcs.keys().next().expect("the follower");
        // Past 4500: the re-stand lands 200 behind the owner.
        world.npc_move(stood, 3200 + 5000, 3200);
        world.run_shaman_ticks(SHAMAN_STATE_PULSES);
        let follower = world
            .npc_by_vid(stood)
            .expect("the follower stands")
            .clone();
        assert_eq!((follower.x, follower.y), (3000, 3200));
        // Past 750: the follower runs toward the owner.
        world.npc_move(stood, 3200 + 1000, 3200);
        world.wake_minds();
        world.run_shaman_ticks(2 * SHAMAN_STATE_PULSES);
        let walking = world.minds.get(stood).expect("the follower moves").walking;
        assert!(!walking, "past 750 it runs");
        // Past 400: the follower walks. The run above left it moving, and a
        // moving follower keeps its gait, so its mind first arrives.
        world.npc_move(stood, 3200 + 500, 3200);
        if let Some(mind) = world.minds.get_mut(stood) {
            mind.motion.dest = (3200 + 500, 3200);
        }
        world.wake_minds();
        world.run_shaman_ticks(3 * SHAMAN_STATE_PULSES);
        let walking = world.minds.get(stood).expect("the follower moves").walking;
        assert!(walking, "past 400 it walks");
        // A dropped summon item unsummons it.
        let held = world
            .characters
            .find_by_vid(Vid::new(KEEPER))
            .expect("the owner")
            .items()
            .item(7)
            .cloned()
            .expect("the toggle");
        world
            .characters
            .find_by_vid_mut(Vid::new(KEEPER))
            .expect("the owner")
            .items_mut()
            .remove(held.pos, &held)
            .expect("the drop");
        world.run_shaman_ticks(4 * SHAMAN_STATE_PULSES);
        assert!(world.shaman_npcs.is_empty());
        assert!(world.npc_by_vid(stood).is_none());
    }

    /// The premium item chats the success line and is kept; the premium flag is
    /// never set, so the premium buffs stay dark.
    #[test]
    fn the_premium_item_chats_and_keeps() {
        let clock = TestClock::default();
        clock.set(1_000);
        let mut premium = Item::new(9, 69_699);
        premium.count = 1;
        let (mut world, mut inbox) = a_shaman_world(&clock, (3200, 3200), &[(inv(1), premium)]);
        let _ = drained(&mut inbox);
        let account = crate::game_loop_messages::QuestAccount::default();
        world
            .use_item(Vid::new(KEEPER), inv(1), mover(), &account)
            .expect("the premium use answers");
        let kept = world
            .characters
            .find_by_vid(Vid::new(KEEPER))
            .expect("the owner")
            .items()
            .item(9)
            .cloned()
            .expect("the premium item is kept");
        assert_eq!(kept.count, 1);
        assert!(!world.shaman_premium(KEEPER));
        let frames = drained(&mut inbox);
        assert!(
            frames.iter().any(|frame| frame
                .windows(b"You upgrade your buff successfully".len())
                .any(|window| window == b"You upgrade your buff successfully")),
            "the success line is chatted"
        );
    }
}
