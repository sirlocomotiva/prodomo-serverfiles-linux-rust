//! The experience half of `CHARACTER::Reward` (`G/char_battle.cpp:751-1003`): the damage
//! map, `DistributeExp`, the live `GiveExp`, and the PC-attacker arms (alignment, the
//! quest kill, the kill recovery) up to the kill recovery (`:789-830`); then `RewardGold`
//! (`:558-749`) and the item half (`:833-990`), whose hits wait for D5.
//!
//! # What legacy does
//!
//! `Damage` fills `m_map_kDamage` (attacker VID to damage) while the hit takes any
//! (`:2407-2423`); `Dead` of a monster calls `Reward`, whose `DistributeExp` builds a
//! damage table from the map in VID order, skipping attackers that left, are NPCs, or
//! stand beyond 5000 (`DISTANCE_APPROX`). The top damager takes 20% of the monster's
//! experience plus its share of the rest, and the rest splits by share; every grant
//! runs the level-delta table and the bonus terms of the live `GiveExp` (`:2567-2700`,
//! the `#else` arm: `ENABLE_NEWEXP_CALCULATION` is never defined). The most-attacked
//! character, not the last hitter, is `Reward`'s attacker: it gains alignment, its
//! quests see the kill, and a 10% draw may heal it.
//!
//! # What this build does
//!
//! The damage map is `NpcCombat.damage`, filled by `after_damage` before the recovery
//! starts. `dead_npc` calls `reward_exp` after the stun cancel and before `GC_DEAD`.
//! Party attackers group into one row per party, after the solos; there is no stone link
//! (OQ11), no priv manager, no premium, no mounts, no marriage, and no ring of exp (its
//! group does not exist). The devil-tower arm is dead on a hostile map (no hosted map is
//! in 660000-669999), `AdjustExpByLevel` is a no-op at level 99 and below, and the tower
//! of the instant `POINT_EXP` percent is always 0 (Defect D3). What is not ported is a
//! hook, and every hook is named where its term would run.
//!
//! Two deliberate differences from legacy:
//!
//! - The experience math runs in `i64`: legacy's `int` products can wrap negative on a
//!   boss kill with stacked bonuses and LOSE experience (Defect D1). Identical results
//!   without the overflow.
//! - An attacker must stand on the same Channel and map: legacy checks only the
//!   distance, which would grant experience across maps (Defect D2). Nothing else
//!   widens the gate.
//!
//! Everything here runs on the game thread only (ADR-0002).

use super::view::EntityKey;
use super::GameState;
use crate::game_loop_messages::GroundPlace;
use crate::sync_position::distance_approx;
use common::cfloat::{f32_to_i32, f32_to_u32, i32_to_f32, u32_to_f32};
use common::levels;
use common::point_slot as point;
use common::vid::Vid;
use protocol::gc_fields::GcCreateFly;
use std::sync::Arc;
use world::character::{number, Dice, FLY_EXP, FLY_HP_SMALL, FLY_SP_SMALL, SUNGMA_WILL_MAPS};
use world::character::{DropAsk, DropMade, DropRefused};
use world::item::Item;

/// `ITEM_WHITEDAY_ROSE` and `ITEM_WHITEDAY_CANDY` (`G/unique_item.h:62-63`).
const WHITEDAY_ITEMS: [u32; 2] = [50_031, 50_032];
/// The fixed `iRandRange`: the precedence Defect at `G/item_manager.cpp:906-911` parses as
/// `(100 + priv + worn) ? 4000000 : 0`, which is always 4000000 (the owner's custom
/// Item-Drop Boost Event never raised it). The Rewrite fixes the parse but the priv manager
/// does not exist yet, so the rate stays 4000000 with this cite.
const DROP_RAND_RANGE: i32 = 4_000_000;
/// `GetThreeSkillLevelAdjust`'s bands (`G/item_manager.cpp:1336-1350`): the divisor for a
/// mob of the level.
const THREE_SKILL_BANDS: [(u8, i32); 6] =
    [(40, 32), (45, 16), (50, 8), (55, 4), (60, 2), (u8::MAX, 1)];
/// The `xmas_sock`/`moon_drop`/`hexegonal_drop` per-rank kill divisors
/// (`G/item_manager.cpp:1617-1631`, `:1645-1659`, `:1671-1685`): PAWN to KING.
const EVENT_SOCK_DIVISORS: [i32; 6] = [2000, 1000, 300, 50, 0, 0];
/// The `mars_drop` per-rank multipliers (`G/item_manager.cpp:1889-1896`): PAWN to KING.
const MARS_MULTIPLIERS: [i32; 6] = [50, 30, 5, 1, 0, 0];
/// The `medal_part_drop` items (`G/item_manager.cpp:1857`).
const MEDAL_PARTS: [u32; 5] = [30265, 30266, 30267, 30268, 30269];

/// `UNIQUE_ITEM_LARBOR_MEDAL` (`G/unique_item.h:35`): worn, it adds 20% experience.
const LARBOR_MEDAL: u32 = 70_004;
/// `UNIQUE_ITEM_DOUBLE_EXP` (`G/unique_item.h:36`): worn, it adds 50% experience.
const DOUBLE_EXP: u32 = 70_005;
/// `UNIQUE_ITEM_FASTER_ALIGNMENT_UP_BY_KILL` (`G/unique_item.h:31`): worn, a kill that
/// raises a negative alignment raises it by 14 instead of 7.
const FASTER_ALIGNMENT_UP_BY_KILL: u32 = 70_051;
/// `DistributeExp`'s reach (`G/char_battle.cpp:2848`): attackers beyond it earn nothing.
const EXP_RANGE: i32 = 5000;
/// `UpdateAlignment`'s clamp (`G/char_battle.cpp:3486`).
const ALIGNMENT_CLAMP: i32 = 200_000;

/// One row of `DistributeExp`'s damage table (`TDamageInfo`, `G/char_battle.cpp:2769`):
/// a solo attacker, or a party by its leader PID, and the damage dealt. Solos row in
/// VID order; parties append after them, in first-appearance order (legacy's
/// `std::map<LPPARTY>` order is pointer order, nondeterministic across runs — the port's
/// order is deterministic, which is noted, not a Divergence row).
enum DamageRow {
    /// `pAttacker` is set.
    Solo { attacker: Vid, damage: i32 },
    /// `pParty` is set.
    Party {
        leader: common::CharacterId,
        damage: i32,
    },
}

impl DamageRow {
    /// `iDam`.
    fn damage(&self) -> i32 {
        match *self {
            DamageRow::Solo { damage, .. } | DamageRow::Party { damage, .. } => damage,
        }
    }
}

impl GameState {
    /// The experience half of `Reward` (`G/char_battle.cpp:789-830`) for the monster under
    /// `vid`: distributes its experience, then runs the attacker's alignment, quest kill
    /// and kill recovery. Returns the attacker `DistributeExp` named (the most-attacked
    /// character), for the gold and item halves. The damage map stays: the item half's
    /// owner list reads it, and legacy clears it only after the placement (`:992`; the
    /// `:2895` clear is commented out).
    pub(super) fn reward_exp(&mut self, vid: u32) -> Option<Vid> {
        let attacker = self.distribute_exp(vid)?;
        let npc = self.npc_by_vid(vid)?;
        let (level, race) = (npc.fight.level, u32::from(npc.race));
        self.raise_alignment(vid, attacker, level);
        self.raise_quest_kill(vid, attacker, race);
        self.kill_recovery(vid, attacker);
        Some(attacker)
    }

    /// `DistributeExp` (`G/char_battle.cpp:2818-2985`): the most-attacked character, after
    /// every in-range attacker took its share. `DistributeHP` is a no-op without a
    /// dungeon (`:2438-2442`); a son feeds half its exp to its stone first (`:2899-2906`).
    fn distribute_exp(&mut self, vid: u32) -> Option<Vid> {
        let npc = self.npc_by_vid(vid)?;
        let exp = npc.exp;
        if exp == 0 {
            return None;
        }
        let at = self
            .spot_of(vid)
            .map_or((npc.x, npc.y), |spot| (spot.x, spot.y));
        // `place_of` reads the bodies, which hold no NPC: the NPC's own map is in `npc_of`.
        let place = self
            .npc_of
            .get(&vid)
            .map(|&(channel, map, _)| (channel, map));
        let level = npc.fight.level;
        let damage = self.damage_entries(vid);
        let (rows, total, most) = self.build_damage_rows(vid, &damage, at, place);
        let most = most.map(|(attacker, _)| attacker);
        // `SetExp(0)` runs before the empty returns (`:2893-2897`): the experience is
        // consumed even when nobody takes any.
        if let Some(npc) = self.npc_mut(vid) {
            npc.exp = 0;
        }
        if total == 0 {
            return None;
        }
        let exp = self.stone_feed(vid, exp);
        if rows.is_empty() {
            return None;
        }
        let most = most?;
        // The top row is the first maximum in table order (strict `>` scan from
        // `begin()`), which a grouped table need not share with the most-attacked
        // character.
        let mut top = 0;
        for (index, row) in rows.iter().enumerate() {
            if row.damage() > rows[top].damage() {
                top = index;
            }
        }
        let top_damage = rows[top].damage();
        let bonus = exp / 5;
        let rest = exp - bonus;
        let share = |damage: i32| {
            let f = i32_to_f32(damage) / i32_to_f32(total);
            f32_to_i32(u32_to_f32(rest) * f)
        };
        let first = i64::from(bonus) + i64::from(share(top_damage));
        self.distribute_row(vid, level, at, &rows[top], first);
        let f = i32_to_f32(top_damage) / i32_to_f32(total);
        if f > 1.0 {
            tracing::error!(victim = vid, f, "DistributeExp percent over 1.0");
        }
        // Legacy compares the `float` to 1.0 exactly (`fPercent == 1.0f`); the bit compare
        // keeps that exactness without the lint.
        if f.to_bits() == 1.0_f32.to_bits() {
            return Some(most);
        }
        // `di->Clear()` skips the top row in the second walk.
        for (index, row) in rows.iter().enumerate() {
            if index == top {
                continue;
            }
            let share = i64::from(share(row.damage()));
            self.distribute_row(vid, level, at, row, share);
        }
        Some(most)
    }

    /// `m_map_kDamage` under `vid` as (attacker VID, total) in VID order.
    fn damage_entries(&self, vid: u32) -> Vec<(u32, i32)> {
        self.npc_combat
            .get(&vid)
            .map(|combat| {
                combat
                    .damage
                    .iter()
                    .map(|(&vid, row)| (vid, row.total))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `DistributeExp`'s damage-table walk (`G/char_battle.cpp:2839-2904`): the rows, the
    /// total, and the most-attacked character with its damage. The walk skips attackers
    /// that left, are NPCs, stand on another Channel or map (D2: legacy checks only
    /// the distance), or stand beyond 5000 of `at`.
    fn build_damage_rows(
        &self,
        _victim: u32,
        damage: &[(u32, i32)],
        at: (i32, i32),
        place: Option<(u8, i32)>,
    ) -> (Vec<DamageRow>, i32, Option<(Vid, i32)>) {
        let mut rows: Vec<DamageRow> = Vec::new();
        let mut party_damage: Vec<(common::CharacterId, i32)> = Vec::new();
        let mut total = 0_i32;
        let mut most: Option<(Vid, i32)> = None;
        // The map iterates in VID order (`std::map`), and the table keeps it.
        for &(raw, dealt) in damage {
            let attacker = Vid::new(raw);
            let Some(body) = self.bodies.get(&attacker) else {
                continue;
            };
            // A NPC VID in the map is skipped (`IsNPC()`).
            if self.npc_by_vid(raw).is_some() || Some((body.channel, body.map)) != place {
                continue;
            }
            let Some(spot) = self.spot_of(raw) else {
                continue;
            };
            if distance_approx(spot.x.saturating_sub(at.0), spot.y.saturating_sub(at.1)) > EXP_RANGE
            {
                continue;
            }
            total = total.saturating_add(dealt);
            // The first strict maximum (`!pkChrMostAttacked || iDam > iMostDam`), over
            // CHARACTERS, not rows: the return.
            if most.is_none_or(|(_, held)| dealt > held) {
                most = Some((attacker, dealt));
            }
            // A party attacker accumulates into its party's row (`:2871-2885`); a solo
            // rows at once (`:2886-2896`).
            if let Some(leader) = self.party_of_vid(attacker) {
                if let Some(entry) = party_damage.iter_mut().find(|(held, _)| *held == leader) {
                    entry.1 = entry.1.saturating_add(dealt);
                } else {
                    party_damage.push((leader, dealt));
                }
            } else {
                rows.push(DamageRow::Solo {
                    attacker,
                    damage: dealt,
                });
            }
        }
        // The party rows append after the solos (`:2899-2904`).
        for (leader, damage) in party_damage {
            rows.push(DamageRow::Party { leader, damage });
        }
        (rows, total, most)
    }

    /// `TDamageInfo::Distribute` (`G/char_battle.cpp:2782-2815`): a solo row feeds the
    /// live `GiveExp`; a party row feeds `distribute_party_row`.
    fn distribute_row(
        &mut self,
        victim: u32,
        victim_level: u8,
        at: (i32, i32),
        row: &DamageRow,
        exp: i64,
    ) {
        match *row {
            DamageRow::Solo { attacker, .. } => {
                self.give_exp(victim, attacker, victim_level, exp);
            }
            DamageRow::Party { leader, .. } => {
                self.distribute_party_row(victim, victim_level, at, leader, exp);
            }
        }
    }

    /// The party arm of `TDamageInfo::Distribute` (`G/char_battle.cpp:2788-2814`): the
    /// near-leader bonus multiplier, then the totaler and distributor over the online
    /// members in victim range. The test-server log arm is not taken. The centralize
    /// arm reads `m_pkChrExpCentralize`, which no setter ever sets (230j finding), so
    /// it is dead and is not ported.
    fn distribute_party_row(
        &mut self,
        victim: u32,
        victim_level: u8,
        (x, y): (i32, i32),
        leader: common::CharacterId,
        exp: i64,
    ) {
        // `iExp * (100 + GetExpBonusPercent()) / 100` when the victim stands near the
        // leader (`:2791-2792`); the math runs in `i64` (D1).
        let mut exp = exp;
        if self.party_victim_near_leader(leader, x, y) {
            let percent = self.party_exp_bonus_percent(leader);
            exp = exp * i64::from(100 + percent) / 100;
        }
        let members = self.party_members_near_victim(leader, x, y);
        if members.is_empty() {
            return;
        }
        let victim = Vid::new(victim);
        if self.party_mode(leader) == super::party::DISTRIBUTION_PARITY {
            // `m_iMemberCount` is never 0 here (the walk found members), so no max.
            let each = exp / i64::try_from(members.len()).unwrap_or(1).max(1);
            for (member, _) in members {
                self.give_exp(victim.raw(), member, victim_level, each);
            }
            return;
        }
        // `FPartyTotaler` (`:2704-2722`) and `FPartyDistributor` (`:2724-2767`): each
        // member's share is `(DWORD)(exp * (float)NP / total)`.
        let total: i64 = members
            .iter()
            .map(|(_, level)| i64::from(levels::party_exp_np(*level)))
            .sum();
        if total <= 0 {
            return;
        }
        let total_f = i32_to_f32(i32::try_from(total.min(i64::from(i32::MAX))).unwrap_or(0));
        // `exp` fits `u32` on every real kill; a wider value would have wrapped legacy's
        // `DWORD` multiply, so the narrow keeps the defined half.
        let exp_f = u32_to_f32(u32::try_from(exp.max(0)).unwrap_or(u32::MAX));
        for (member, level) in members {
            let share = f32_to_u32(exp_f * u32_to_f32(levels::party_exp_np(level)) / total_f);
            self.give_exp(victim.raw(), member, victim_level, i64::from(share));
        }
    }

    /// `DistributeExp`'s stone arm (`G/char_battle.cpp:2899-2906`): a son feeds `exp >> 1`
    /// to its live stone and splits the rest. A dead or missing stone reads as none
    /// (D-new: legacy would read freed memory).
    fn stone_feed(&mut self, vid: u32, exp: u32) -> u32 {
        let stone = self
            .npc_by_vid(vid)
            .and_then(|npc| npc.stone)
            .filter(|stone| self.npc_by_vid(*stone).is_some_and(|stone| !stone.dead));
        let Some(stone) = stone else {
            return exp;
        };
        let half = exp >> 1;
        if let Some(stone) = self.npc_mut(stone) {
            stone.exp = stone.exp.saturating_add(half);
        }
        exp - half
    }

    /// The live `GiveExp` (`G/char_battle.cpp:2567-2700`): the level-delta table and the
    /// bonus terms on `exp`, granted to `to` for a kill of a level-`from_level` monster
    /// under `from`, with the exp orb around the monster. The math runs in `i64` (D1);
    /// each term truncates exactly where legacy's `int` division truncates.
    fn give_exp(&mut self, from: u32, to: Vid, from_level: u8, exp: i64) {
        let Ok(character) = self.characters.find_by_vid(to) else {
            return;
        };
        let Some(points) = character.points() else {
            return;
        };
        let level = points.level();
        let (exp_double, ramadan, mall, next_exp) = (
            points.get_point(point::POINT_EXP_DOUBLE_BONUS),
            points.get_point(point::POINT_RAMADAN_CANDY_BONUS_EXP),
            points.get_point(point::POINT_MALL_EXPBONUS),
            levels::next_exp(level),
        );
        let mut exp = levels::exp_level_delta_share(level, from_level, exp);
        // No priv manager: `PRIV_EXP_PCT` is 0 (hook).
        if self.wears_unique(to, LARBOR_MEDAL) {
            exp += exp * 20 / 100;
        }
        // No hosted map is in 660000-669999 (the devil tower is not hosted): the arm is
        // dead and is not ported.
        if exp_double != 0 && number(&mut self.dice, 1, 100) <= exp_double {
            exp += exp * 30 / 100;
        }
        if self.wears_unique(to, DOUBLE_EXP) {
            exp += exp * 50 / 100;
        }
        // No mount system (hook): both mount arms run 0. No premium (hook): `PREMIUM_EXP`
        // is 0. The ring of exp is dead: `special_item_group.txt` has no group 10000.
        // No marriage (hook): `GetMarriageBonus` is 0.
        exp += exp * i64::from(ramadan) / 100;
        exp += exp * i64::from(mall) / 100;
        // The instant `POINT_EXP` percent is always 0 (D3, dead; not ported).
        exp = exp * i64::from(self.mob_exp_rate()) / 100;
        exp = exp.min(next_exp / 10);
        // `AdjustExpByLevel` is a no-op at level 99 and below (not ported).
        let conqueror = points.conqueror_level();
        let map = self.place_of(to.raw()).map_or(0, |(_, map)| map);
        if conqueror > 0 && is_new_world_map(map) {
            // The conqueror grant (`POINT_CONQUEROR_EXP`) waits for its level-up chain
            // (hook): a conqueror on a new-world map takes nothing here. Legacy's
            // `__CONQUEROR_LEVEL__` arm (`:2654-2673`).
            return;
        }
        self.gain_pc_exp(to, exp);
        let mut orb = Vec::new();
        GcCreateFly::new(FLY_EXP, from, to.raw()).encode_into(&mut orb);
        self.packet_around(EntityKey::Npc(from), &orb, None);
    }

    /// `CHARACTER_MANAGER::GetMobExpRate` (`G/char_manager.cpp:1035-1058`): 100, or the
    /// `mob_exp` event flag while a quest set it (`G/questmanager.cpp:1374-1377`). No
    /// premium time exists, so the buyer rate never applies.
    fn mob_exp_rate(&self) -> i32 {
        self.quests
            .as_ref()
            .map(|quests| quests.manager().event_flag(b"mob_exp"))
            .filter(|&rate| rate != 0)
            .unwrap_or(100)
    }

    /// `CHARACTER_MANAGER::GetMobDamageRate` (`G/char_manager.cpp:980-983`): 100, or the
    /// `mob_dam` event flag while a quest set it (`G/questmanager.cpp:1354-1357`).
    /// `BONUS_PTC` is a typo legacy never defines, so the `RESIST_COMBAT` factor of
    /// `G/battle.cpp:314-315` never applies and the rate is a plain `int`.
    pub(super) fn mob_damage_rate(&self) -> i32 {
        self.quests
            .as_ref()
            .map(|quests| quests.manager().event_flag(b"mob_dam"))
            .filter(|&rate| rate != 0)
            .unwrap_or(100)
    }

    /// `CHARACTER_MANAGER::GetUserDamageRate` (`G/char_manager.cpp:1060-1070`): 100, or
    /// the `user_dam` event flag while a quest set it (`G/questmanager.cpp:1362-1365`).
    /// No premium time exists, so the buyer rate never applies.
    pub(super) fn user_damage_rate(&self) -> i32 {
        self.quests
            .as_ref()
            .map(|quests| quests.manager().event_flag(b"user_dam"))
            .filter(|&rate| rate != 0)
            .unwrap_or(100)
    }

    /// Whether the player under `vid` wears unique item `vnum`
    /// (`IsEquipUniqueItem`, `G/char_item.cpp:9519-9540`).
    pub(super) fn wears_unique(&self, vid: Vid, vnum: u32) -> bool {
        use common::enums::EWearPositions;
        use world::character::Equipment;
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return false;
        };
        let equipment = Equipment::of(character.items(), &self.protos);
        [EWearPositions::Unique1, EWearPositions::Unique2]
            .into_iter()
            .filter_map(|wear| equipment.wear(wear as u16))
            .any(|worn| worn.item.vnum == vnum)
    }

    /// The alignment arm (`G/char_battle.cpp:794-807`): a kill of a monster at most 10
    /// levels below raises the killer's alignment, and a crossing of a /10 boundary
    /// sends the update (`UpdateAlignment`, `G/char_battle.cpp:3477-3494`). The value
    /// lives on the body's card until `sys.char.pk` stores it. The `ALIGNMENT_BONUS`
    /// affects wait for their system (hook).
    fn raise_alignment(&mut self, _victim: u32, attacker: Vid, victim_level: u8) {
        let Some(body) = self.bodies.get(&attacker) else {
            return;
        };
        let (body_level, alignment) = (body.card.level, body.card.alignment);
        if i32::from(victim_level) - i32::from(body_level) < -10 {
            return;
        }
        let amount = if alignment < 0 {
            if self.wears_unique(attacker, FASTER_ALIGNMENT_UP_BY_KILL) {
                14
            } else {
                7
            }
        } else {
            2
        };
        // Both legacy's `int / 10` and Rust's `/` truncate toward zero, so `-8 / 10` is 0
        // on both sides and the quotient compares directly.
        let before = alignment / 10;
        let after = (alignment + amount).clamp(-ALIGNMENT_CLAMP, ALIGNMENT_CLAMP);
        if let Some(body) = self.bodies.get_mut(&attacker) {
            body.card.alignment = after;
        }
        // `m_iAlignment == m_iRealAlignment` always holds here (no second value is
        // tracked), so `bShow` is true and only the /10 crossing sends.
        if before != after / 10 {
            let flags = self.affect_flags_of(attacker);
            self.send_affect_update(attacker, flags);
        }
    }

    /// The quest kill (`G/char_battle.cpp:810-811`): `SetQuestNPCID` then
    /// `CQuestManager::Kill` (`G/questmanager.cpp:574-611`). The kill log is log-only.
    pub(super) fn raise_quest_kill(&mut self, victim: u32, attacker: Vid, race: u32) {
        let Some(mover) = self.own_mover(attacker) else {
            return;
        };
        let Some((channel, map)) = self.place_of(attacker.raw()) else {
            return;
        };
        let (x, y) = self
            .spot_of(attacker.raw())
            .map_or((0, 0), |spot| (spot.x, spot.y));
        let place = GroundPlace { channel, map, x, y };
        for record in self.quest_kill(attacker, victim, race, place, mover) {
            let _sent = self.write_to_client(attacker, record);
        }
    }

    /// The kill recovery (`G/char_battle.cpp:814-829`): a 10% draw heals the killer by
    /// its kill-recovery points, each with its orb from the monster.
    fn kill_recovery(&mut self, victim: u32, attacker: Vid) {
        if number(&mut self.dice, 0, 9) != 0 {
            return;
        }
        let Ok(character) = self.characters.find_by_vid(attacker) else {
            return;
        };
        let Some(points) = character.points() else {
            return;
        };
        let (hp_recover, sp_recover, hp_max, sp_max) = (
            points.get_point(point::POINT_KILL_HP_RECOVERY),
            points.get_point(point::POINT_KILL_SP_RECOVER),
            points.max_hp(),
            points.max_sp(),
        );
        if hp_recover != 0 {
            let heal = hp_max * hp_recover / 100;
            self.change_pc_point(attacker, point::POINT_HP, heal, false);
            let mut orb = Vec::new();
            GcCreateFly::new(FLY_HP_SMALL, victim, attacker.raw()).encode_into(&mut orb);
            self.packet_around(EntityKey::Npc(victim), &orb, None);
        }
        if sp_recover != 0 {
            let heal = sp_max * sp_recover / 100;
            self.change_pc_point(attacker, point::POINT_SP, heal, false);
            let mut orb = Vec::new();
            GcCreateFly::new(FLY_SP_SMALL, victim, attacker.raw()).encode_into(&mut orb);
            self.packet_around(EntityKey::Npc(victim), &orb, None);
        }
    }

    /// The gold half of `Reward`: `RewardGold` (`G/char_battle.cpp:558-749`) for the monster
    /// under `vid`, killed by `attacker`. Without autoloot (no premium, and unique group
    /// 10011 does not exist) `GetMovablePosition` around the monster must succeed, else no
    /// gold falls (`:568-569`); the chance is the rank's gold percent through the mall bonus,
    /// the gold drop rate (100: no premium, no China check), and the level-delta table (the
    /// boss table from rank BOSS up, with NO stone exclusion, `:599-603`).
    ///
    /// Every pile is laid with the monster's z (legacy's `pos.z` is uninitialised stack, a
    /// Defect) and a fixed 300 s destroy event. A pile whose point has no sectree is dropped
    /// unseen, its draws kept (OQ10). `SendMoneyLog` is log-only here.
    pub(super) fn reward_gold(&mut self, vid: u32, attacker: Vid) {
        use common::enums::EMobRank;
        use gamedata::mob_proto::CHAR_TYPE_STONE;
        let Some(npc) = self.npc_by_vid(vid) else {
            return;
        };
        let (x, y, z) = (npc.x, npc.y, npc.z);
        let (rank, level, gold) = (npc.fight.rank, npc.fight.level, npc.fight.gold_range);
        let stone = npc.char_type == CHAR_TYPE_STONE;
        let Some(&(channel, map, _)) = self.npc_of.get(&vid) else {
            return;
        };
        // No autoloot (hook): the movable point must exist.
        if self.movable_position(map, x, y).is_none() {
            return;
        }
        let Ok(character) = self.characters.find_by_vid(attacker) else {
            return;
        };
        let Some(points) = character.points() else {
            return;
        };
        let (killer_level, mall, double) = (
            points.level(),
            points.get_point(point::POINT_MALL_GOLDBONUS),
            points.get_point(point::POINT_GOLD_DOUBLE_BONUS),
        );
        let mut percent = levels::GOLD_PERCENT_BY_RANK
            .get(usize::from(rank))
            .copied()
            .unwrap_or(0);
        // No priv manager: `PRIV_GOLD_DROP` is 0 (hook). No premium gold (hook).
        percent += percent * mall / 100;
        percent = percent.min(100);
        let boss = rank >= EMobRank::Boss as u8;
        let chance = levels::gold_level_delta_share(killer_level, level, percent, boss);
        if number(&mut self.dice, 1, 100) > chance {
            return;
        }
        // The owner's largest gold range is 700000; anything past `i32::MAX` saturates.
        let low = i32::try_from(gold[0]).unwrap_or(i32::MAX);
        let high = i32::try_from(gold[1]).unwrap_or(i32::MAX);
        let mut multiplier = 1;
        if 1 == number(&mut self.dice, 1, 50_000) {
            multiplier *= 10;
        } else if 1 == number(&mut self.dice, 1, 10_000) {
            multiplier *= 5;
        }
        if double != 0 && number(&mut self.dice, 1, 100) <= double {
            multiplier *= 2;
        }
        // No priv manager: `PRIV_GOLD10_DROP` is 0, so the ten-drop chance is 1 (hook).
        let gold_max = gold[1];
        let wide = boss && !stone && gold_max != 0;
        let piles = Self::draw_gold_piles(&mut self.dice, wide, boss, low, high, &mut multiplier);
        let mut total = 0_i64;
        for pile in piles {
            // The boss scatter is ±28 cells of 23; the common scatter ±7 cells of 20.
            let (dx, dy) = if wide {
                (
                    (number(&mut self.dice, -14, 14) + number(&mut self.dice, -14, 14)) * 23,
                    (number(&mut self.dice, -14, 14) + number(&mut self.dice, -14, 14)) * 23,
                )
            } else {
                (
                    number(&mut self.dice, -7, 7) * 20,
                    number(&mut self.dice, -7, 7) * 20,
                )
            };
            let Some(px) = x.checked_add(dx) else {
                continue;
            };
            let Some(py) = y.checked_add(dy) else {
                continue;
            };
            self.lay_monster_gold(channel, map, (px, py, z), pile);
            total += i64::from(pile);
        }
        tracing::debug!(victim = vid, total, "monster gold paid");
    }

    /// `RewardGold`'s split (`G/char_battle.cpp:612-749`): the boss many-pile draw, the
    /// 1-in-100 ten piles, or the single roll split 1..3 (a boss under 3 gold splits
    /// 3..10 unless the quotient is 0). Zero piles are dropped. The boss arm's 1-in-100
    /// folds into `multiplier`.
    fn draw_gold_piles(
        dice: &mut dyn Dice,
        wide: bool,
        boss: bool,
        low: i32,
        high: i32,
        multiplier: &mut i32,
    ) -> Vec<i32> {
        let mut piles: Vec<i32> = Vec::new();
        if wide {
            if 1 == number(dice, 1, 100) {
                *multiplier *= 10;
            }
            let split = number(dice, 25, 35);
            for _ in 0..split {
                let pile = number(dice, low, high) / split * *multiplier;
                if pile != 0 {
                    piles.push(pile);
                }
            }
        } else if 1 == number(dice, 1, 100) {
            for _ in 0..10 {
                let pile = number(dice, low, high) * *multiplier;
                if pile != 0 {
                    piles.push(pile);
                }
            }
        } else {
            let gold = number(dice, low, high) * *multiplier;
            let mut split = 1;
            if gold >= 3 {
                split = number(dice, 1, 3);
            } else if boss {
                split = number(dice, 3, 10);
                if gold / split == 0 {
                    split = 1;
                }
            }
            if gold != 0 {
                for _ in 0..split {
                    piles.push(gold / split);
                }
            }
        }
        piles
    }

    /// Lay one monster gold pile of `gold` at `at` on `channel`'s `map`: the ELK pile
    /// `CreateItem(1, gold)` makes, unnumbered and rowless like a player drop's
    /// (`G/item_manager.cpp:618-694`, `G/char_item.cpp:7554`), with no owner and the fixed
    /// 300 s destroy event. A point with no sectree drops the pile unseen, its draws kept
    /// (OQ10: legacy's `AddToGround` fails the same way, `:549-584`, but keeps the failed
    /// item for 300 s where nothing can observe it).
    fn lay_monster_gold(&mut self, channel: u8, map: i32, at: (i32, i32, i32), gold: i32) {
        use gamedata::item_kind::ITEM_ELK;
        use world::character::GroundItem;
        let Some(vnum) = self
            .protos
            .rows()
            .iter()
            .find(|row| row.item_type == ITEM_ELK)
            .map(|row| row.vnum)
        else {
            return;
        };
        let key = (channel, map);
        let Some(index) = self.maps.get(&key) else {
            return;
        };
        if index.tree_at(at.0, at.1).is_none() {
            return;
        }
        let vid = self.next_ground_vid;
        self.next_ground_vid = self.next_ground_vid.wrapping_add(1).max(1);
        let ground = GroundItem {
            vid,
            item: world::item::Item {
                id: world::item::NO_ITEM,
                vnum,
                count: u16::try_from(gold.clamp(0, i32::from(u16::MAX))).unwrap_or(u16::MAX),
                flags: 0,
                anti_flags: 0,
                sockets: [0; world::item::SOCKETS],
                attributes: [protocol::gc_item_window::ItemAttribute::new(0, 0);
                    world::item::ATTRIBUTES],
                size: 1,
                pos: protocol::item_pos::ItemPos::new(0, 0),
                refine_element: 0,
                transmutation: 0,
            },
            x: at.0,
            y: at.1,
            last_owner: 0,
        };
        let (order, stored) = crate::item_move::StoreOrder::laid();
        // A monster pile has no row to store: the order is told at once.
        order.stored();
        self.ground.insert(
            vid,
            super::Lying {
                ground,
                channel,
                map,
                expires: self.last_pulse.saturating_add(super::MONSTER_DROP_LIFETIME),
                owner: None,
                stored,
            },
        );
        let radius = self.view_radius;
        let mut effects = Vec::new();
        if let Some(index) = self.maps.get_mut(&key) {
            index.add_to_ground(EntityKey::Ground(vid), at, radius, &mut effects);
        }
        self.deliver(&effects, None);
    }

    /// `Reward`'s item half (`G/char_battle.cpp:833-990`): the multi-farm gate, the movable
    /// point, `CreateDropItem`'s draws, and the placement. Every character is treated as
    /// allowed (the gate's GG flow has no Rewrite equivalent; a Divergence while it waits
    /// for the owner's call). The point's z is the monster's (it stands still while dead,
    /// and `GetMovablePosition` rewrites x/y only).
    ///
    /// D5: each hit is a made item, laid at the point with `MONSTER_DROP_LIFETIME`; the
    /// owner list, the `SetOwnership` calls, and every `number(-7, 7)` draw run in legacy's
    /// order. No hosted kill reaches a hit.
    pub(super) fn reward_drops(&mut self, vid: u32, attacker: Vid) {
        let hits = self.create_drop_item(vid, attacker);
        // Zero items: nothing but the damage-map clear (the empty `size() == 0` arm,
        // `:867-869`, still reaches `:992`).
        if hits.is_empty() {
            if let Some(combat) = self.npc_combat.get_mut(&vid) {
                combat.damage.clear();
            }
            return;
        }
        let Some(npc) = self.npc_by_vid(vid) else {
            return;
        };
        let (x, y, z) = (npc.x, npc.y, npc.z);
        let Some(&(channel, map, _)) = self.npc_of.get(&vid) else {
            return;
        };
        // The movable point is drawn from the monster's own x/y, before `RewardGold` drew
        // anything: `Reward` takes it at `:843-846`, `RewardGold` overwrites its own copy.
        let Some((mut px, mut py)) = self.movable_position(map, x, y) else {
            return;
        };
        if hits.len() == 1 {
            // One item (`:870-890`): the attacker's ownership, then the two draws for a `pos`
            // nobody uses.
            let mut one = hits;
            let item = one.pop().expect("one hit");
            let ground = self.lay_drop_hit(channel, map, (px, py, z), item);
            if let Some(ground) = ground {
                self.set_drop_owner(ground, attacker);
            }
            px = x.saturating_add(number(&mut self.dice, -7, 7) * 20);
            py = y.saturating_add(number(&mut self.dice, -7, 7) * 20);
            let _ = (px, py);
            if let Some(combat) = self.npc_combat.get_mut(&vid) {
                combat.damage.clear();
            }
            return;
        }
        // Several (`:892-989`): the qualifying owners in order, or no owner when the list
        // is empty; round-robin from the vector's back, two draws per item for the next
        // point.
        let owners = self.drop_owner_list(vid);
        let mut next = 0;
        for item in hits.into_iter().rev() {
            let owner = if owners.is_empty() {
                None
            } else {
                let owner = owners[next % owners.len()];
                next += 1;
                Some(owner)
            };
            let ground = self.lay_drop_hit(channel, map, (px, py, z), item);
            if let (Some(ground), Some(owner)) = (ground, owner) {
                self.set_drop_owner(ground, owner);
            }
            px = x.saturating_add(number(&mut self.dice, -7, 7) * 20);
            py = y.saturating_add(number(&mut self.dice, -7, 7) * 20);
        }
        // The damage map clears after the placement (`:992`); the no-movable-point and
        // missing-map returns above skip it, as legacy's early returns do.
        if let Some(combat) = self.npc_combat.get_mut(&vid) {
            combat.damage.clear();
        }
    }

    /// The several-item owner list (`G/char_battle.cpp:898-923`): every VID in the damage
    /// map that dealt damage and is still online, in VID order, kept while
    /// `damage * 10 >= total`. The queue pops the greatest damage first and breaks ties by
    /// pointer (a Defect): the Rewrite breaks them by lower VID first, the same order the
    /// exp half already uses.
    pub(super) fn drop_owner_list(&self, vid: u32) -> Vec<Vid> {
        let Some(combat) = self.npc_combat.get(&vid) else {
            return Vec::new();
        };
        let mut entries: Vec<(u32, i32)> = combat
            .damage
            .iter()
            .filter(|(&damager, dealt)| {
                dealt.total > 0 && self.characters.find_by_vid(Vid::new(damager)).is_ok()
            })
            .map(|(&damager, dealt)| (damager, dealt.total))
            .collect();
        let total: i32 = entries.iter().map(|&(_, dealt)| dealt).sum();
        // Greatest damage first, ties by lower VID: the pop order of the queue with the
        // Defect's pointer tie-break replaced.
        entries.sort_by(|&(a_vid, a_dealt), &(b_vid, b_dealt)| {
            b_dealt.cmp(&a_dealt).then_with(|| a_vid.cmp(&b_vid))
        });
        entries
            .into_iter()
            .take_while(|&(_, dealt)| dealt.saturating_mul(10) >= total)
            .map(|(damager, _)| Vid::new(damager))
            .collect()
    }

    /// `SetOwnership(ch)` on the ground item under `ground` (`G/item.cpp:1733-1775`): the
    /// character's store id and 25-byte name, for the header's default 10 s (which the
    /// `<= 10` rule turns into 30). No pickup-affect system exists, so the
    /// `CanPickupDirectly` hook is always false and the grounding always runs.
    fn set_drop_owner(&mut self, ground: u32, owner: Vid) {
        let Ok(character) = self.characters.find_by_vid(owner) else {
            return;
        };
        let pid = character.player_id();
        let Some(body) = self.bodies.get(&owner) else {
            return;
        };
        let name = crate::loading_phase::name_field(&body.card.name);
        self.set_ground_ownership(ground, Some((pid, name)), 10);
    }

    /// Lay one made drop item `item` at `at` on `channel`'s `map`: `AddToGround` with the
    /// fixed 300 s destroy event (`G/item.cpp:549-584`, `G/char_battle.cpp:883-889`).
    /// Answers the ground VID for `SetOwnership`. A point with no sectree drops the item
    /// unseen, its draws kept (OQ10: legacy's `AddToGround` fails the same way but keeps
    /// the failed item for 300 s where nothing can observe it).
    pub(super) fn lay_drop_hit(
        &mut self,
        channel: u8,
        map: i32,
        at: (i32, i32, i32),
        item: Item,
    ) -> Option<u32> {
        use world::character::GroundItem;
        let key = (channel, map);
        let index = self.maps.get(&key)?;
        index.tree_at(at.0, at.1)?;
        let vid = self.next_ground_vid;
        self.next_ground_vid = self.next_ground_vid.wrapping_add(1).max(1);
        let ground = GroundItem {
            vid,
            item,
            x: at.0,
            y: at.1,
            last_owner: 0,
        };
        let (order, stored) = crate::item_move::StoreOrder::laid();
        // A monster drop has no row to store: the order is told at once.
        order.stored();
        self.ground.insert(
            vid,
            super::Lying {
                ground,
                channel,
                map,
                expires: self.last_pulse.saturating_add(super::MONSTER_DROP_LIFETIME),
                owner: None,
                stored,
            },
        );
        let radius = self.view_radius;
        let mut effects = Vec::new();
        if let Some(index) = self.maps.get_mut(&key) {
            index.add_to_ground(EntityKey::Ground(vid), at, radius, &mut effects);
        }
        self.deliver(&effects, None);
        Some(vid)
    }

    /// The `CQuestManager` event flag `flag`, or 0 with no quest host
    /// (`GetEventFlag`, `G/questmanager.cpp:1374-1377`).
    fn event_rate(&self, flag: &[u8]) -> i32 {
        self.quests
            .as_ref()
            .map_or(0, |quests| quests.manager().event_flag(flag))
    }

    /// `GetDropPerKillPct` (`G/item_manager.cpp:845-866`): 0 while the event flag is unset;
    /// else `40000 * delta / flag`, with the flag clamped to `default` below `minimum` (not
    /// on a test server — this build is never one).
    fn drop_per_kill_pct(&self, minimum: i32, default: i32, delta: i32, flag: &[u8]) -> i32 {
        let mut value = self.event_rate(flag);
        if value != 0 && (value < minimum || value < 0) {
            value = default;
        }
        if value == 0 {
            return 0;
        }
        40_000 * delta / value
    }

    /// `GetThreeSkillLevelAdjust` (`G/item_manager.cpp:1336-1350`).
    fn three_skill_adjust(level: u8) -> i32 {
        THREE_SKILL_BANDS
            .iter()
            .find(|&&(below, _)| level < below)
            .map_or(1, |&(_, adjust)| adjust)
    }

    /// `CHARACTER::CountSpecifyItem` ([`world::character::count_specify_item`]): how many of
    /// `vnum` the player under `vid` holds in its unlocked cells and its custom banks.
    fn count_specify_item(&self, vid: Vid, vnum: u32) -> i32 {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return 0;
        };
        let usable = common::item_slots::usable_inventory_cells(character.inven_point());
        world::character::count_specify_item(character.items(), usable, vnum)
    }

    /// `GetDropPct` (`G/item_manager.cpp:870-912`): the delta percent and the fixed
    /// [`DROP_RAND_RANGE`]. The delta is the boss table from rank BOSS up unless the victim
    /// is a stone, plus 1000 on 1-in-50000 else 500 on 1-in-10000, times the item rate
    /// (100: no premium, no China check). The premium/unique doubling and the `bonus` dead
    /// code never run (hooks).
    fn drop_pct(&mut self, stone: bool, rank: u8, mob_level: u8, killer_level: u8) -> (i32, i32) {
        use common::enums::EMobRank;
        let boss = !stone && rank >= EMobRank::Boss as u8;
        let table = if boss {
            levels::PERCENT_BY_DELTA_LEV_BOSS
        } else {
            levels::PERCENT_BY_DELTA_LEV
        };
        let index = i32::from(mob_level) + 15 - i32::from(killer_level);
        let mut delta = table[usize::try_from(index.clamp(0, 30)).unwrap_or(0)];
        if 1 == number(&mut self.dice, 1, 50_000) {
            delta += 1000;
        } else if 1 == number(&mut self.dice, 1, 10_000) {
            delta += 500;
        }
        (delta, DROP_RAND_RANGE)
    }

    /// One `CreateItem(vnum, count, 0, true, rare_pct)` the draw chain asks for: the made
    /// item, or nothing when creation refuses (no proto, an unported arm, no ids). A
    /// refusal warns naming the vnum, as the 229p seam did; no proto skips silently, as
    /// legacy's `if (item)` does. No hosted kill reaches a hit (every table is empty or
    /// unhosted, every event flag 0).
    fn make_drop_hit(&mut self, vnum: u32, count: i32, rare_pct: i32) -> Option<Item> {
        use world::character::make_drop_item;
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        let now_secs = i64::try_from(now_secs).unwrap_or(i64::MAX);
        let count_limit = self.item_count_limit;
        let attrs = Arc::clone(&self.item_attrs);
        let masks = Arc::clone(&self.item_masks);
        let protos = Arc::clone(&self.protos);
        let Some(ids) = self.item_ids.as_mut() else {
            tracing::warn!(
                vnum,
                count,
                "a kill dropped an item; no item ids are installed"
            );
            return None;
        };
        let made = DropMade {
            attrs: &attrs,
            masks: masks.pairs(),
            now_secs,
            count_limit,
        };
        match make_drop_item(
            &mut self.dice,
            &mut self.gauss,
            ids,
            &protos,
            &made,
            vnum,
            DropAsk { count, rare_pct },
        ) {
            Ok(item) => Some(item),
            Err(DropRefused::SoldOut) => None,
            Err(refused) => {
                tracing::warn!(vnum, count, ?refused, "a kill dropped an unported item");
                None
            }
        }
    }

    /// `CreateDropItem` (`G/item_manager.cpp:1100-1333`): every branch and draw in legacy's
    /// order, collecting each hit's made item. The draws run even when their tables are
    /// empty; a refused creation is skipped, as legacy's `if (item)` does.
    fn create_drop_item(&mut self, vid: u32, attacker: Vid) -> Vec<Item> {
        use gamedata::mob_proto::CHAR_TYPE_STONE;
        let Some(npc) = self.npc_by_vid(vid) else {
            return Vec::new();
        };
        let (race, rank, mob_level, drop_vnum) = (
            u32::from(npc.race),
            npc.fight.rank,
            npc.fight.level,
            npc.fight.drop_item_vnum,
        );
        let stone = npc.char_type == CHAR_TYPE_STONE;
        let Ok(character) = self.characters.find_by_vid(attacker) else {
            return Vec::new();
        };
        let Some(points) = character.points() else {
            return Vec::new();
        };
        let killer_level = points.level();
        let (delta, range) = self.drop_pct(stone, rank, mob_level, killer_level);
        let mut hits: Vec<Item> = Vec::new();
        // Common (`:1111-1148`): the rank's rows whose level band holds the killer.
        // `POLYMORPH_ITEM` is 0 on every proto (C3): the polymorph arm is dead and its
        // check is a hook returning 0.
        if usize::from(rank) < gamedata::drop::COMMON_RANKS {
            let rows = self.drops.common.ranks[usize::from(rank)].clone();
            for row in rows {
                if i32::from(killer_level) < row.lv_start || i32::from(killer_level) > row.lv_end {
                    continue;
                }
                let percent = i32::try_from(row.pct).unwrap_or(i32::MAX) * delta / 100;
                if percent >= number(&mut self.dice, 1, range) {
                    hits.extend(self.make_drop_hit(row.vnum, row.count, -1));
                }
            }
        }
        // Drop group (`:1150-1182`): every row rolls.
        if let Some(group) = self.drops.groups.groups.get(&race).cloned() {
            for row in group.items {
                let percent = i32::try_from(row.pct).unwrap_or(i32::MAX) * delta / 100;
                if percent >= number(&mut self.dice, 1, range) {
                    hits.extend(self.make_drop_hit(row.vnum, row.count, -1));
                }
            }
        }
        // Kill group (`:1184-1208`): one gate roll, then `GetOne`'s cumulative draw.
        if let Some(group) = self.drops.mob.kill.get(&race).cloned() {
            if !group.items.is_empty() && group.kill_drop != 0 {
                let percent = 40_000 * delta / group.kill_drop;
                if percent >= number(&mut self.dice, 1, range) {
                    let mut total = 0;
                    for item in &group.items {
                        total += item.part_pct;
                    }
                    let pick = number(&mut self.dice, 1, total);
                    let mut at = 0;
                    for item in &group.items {
                        at += item.part_pct;
                        if pick <= at {
                            hits.extend(self.make_drop_hit(item.vnum, item.count, item.rare_pct));
                            break;
                        }
                    }
                }
            }
        }
        // Limit group (`:1210-1232`): per-row rolls against the literal 1000000.
        if let Some(group) = self.drops.mob.limit.get(&race).cloned() {
            if group.level_limit <= i32::from(killer_level) {
                for row in group.items {
                    if i64::from(number(&mut self.dice, 1, 1_000_000)) <= i64::from(row.pct) {
                        hits.extend(self.make_drop_hit(row.vnum, row.count, -1));
                    }
                }
            }
        }
        // Thief gloves (`:1234-1260`): no premium and unique group 10002 does not exist,
        // so the gate is always false (hook) and no draw runs.
        // Etc (`:1262-1277`): the proto's `DROP_ITEM` vnum against the etc map.
        if drop_vnum != 0 {
            if let Some(prob) = self.drops.etc.probs.get(&drop_vnum).copied() {
                let percent = i32::try_from(prob).unwrap_or(i32::MAX) * delta / 100;
                if percent >= number(&mut self.dice, 1, range) {
                    hits.extend(self.make_drop_hit(drop_vnum, 1, -1));
                }
            }
        }
        // Stone drop (`:1279-1293`): `ENABLE_STONE_DROP_ON_METIN` is undefined (C15).
        // Horse book (`:1295-1302`): no riding, and the `&&` short-circuits before the draw.
        // Lotto (`:1304-1319`): the draw always runs; the DB insert waits for a lotto table
        // (hook) and the flag is 0, so it never passes.
        let _ = self.drop_per_kill_pct(100, 1000, delta, b"lotto_drop")
            >= number(&mut self.dice, 1, range);
        hits.extend(self.create_quest_drop_item(vid, attacker, delta, range));
        // The battle-pass hook and the per-item money log run over no items (D5).
        hits
    }

    /// One `xmas_sock`/`moon_drop`/`hexegonal_drop` arm (`G/item_manager.cpp:1614-1692`):
    /// true when the flag is set and the per-kill percent meets the draw.
    fn quest_event_sock_hit(
        &mut self,
        flag: &[u8],
        _vnum: u32,
        rank: u8,
        delta: i32,
        range: i32,
    ) -> bool {
        let Some(divisor) = EVENT_SOCK_DIVISORS.get(usize::from(rank)).copied() else {
            return false;
        };
        if divisor == 0 || self.event_rate(flag) == 0 {
            return false;
        }
        40_000 * delta / divisor >= number(&mut self.dice, 1, range)
    }

    /// The `new_xmas_event` ×2 draws (`G/item_manager.cpp:1768-1778`): how many of the two
    /// carry-gated per-kill draws pass. A hit is `AutoGiveItem`, not a ground item (D5).
    fn count_xmas_sock_hits(&mut self, attacker: Vid, delta: i32, range: i32) -> u32 {
        let mut count = 0;
        // Each carry vnum's check short-circuits before its own draw.
        for carry in [53_002, 53_007] {
            if self.count_specify_item(attacker, carry) > 0
                && self.drop_per_kill_pct(50, 100, delta, b"new_xmas_event")
                    >= number(&mut self.dice, 1, range)
            {
                count += 1;
            }
        }
        count
    }

    /// `mars_drop` (`G/item_manager.cpp:1887-1901`): killer level, the raw flag, the rank
    /// multiplier, then the per-kill against the draw TIMES the multiplier.
    fn quest_mars_hit(&mut self, rank: u8, killer_level: u8, delta: i32, range: i32) -> bool {
        if killer_level < 15 || self.event_rate(b"mars_drop") == 0 {
            return false;
        }
        let multiplier = MARS_MULTIPLIERS
            .get(usize::from(rank))
            .copied()
            .unwrap_or(0);
        multiplier != 0
            && self.drop_per_kill_pct(1000, 1500, delta, b"mars_drop")
                >= number(&mut self.dice, 1, range) * multiplier
    }

    /// `CreateQuestDropItem` (`G/item_manager.cpp:1595-1910`): every gated and unconditional
    /// draw in legacy's order. Flags are all 0 on a hosted kill, so the unconditional draws
    /// run and never pass; a hit is refused until D5.
    fn create_quest_drop_item(
        &mut self,
        vid: u32,
        attacker: Vid,
        delta: i32,
        range: i32,
    ) -> Vec<Item> {
        let Some(npc) = self.npc_by_vid(vid) else {
            return Vec::new();
        };
        let (rank, mob_level) = (npc.fight.rank, npc.fight.level);
        let Ok(character) = self.characters.find_by_vid(attacker) else {
            return Vec::new();
        };
        let Some(points) = character.points() else {
            return Vec::new();
        };
        let killer_level = points.level();
        let mut hits: Vec<Item> = Vec::new();
        // The drop-event helpers (`:1393-1431`, `:1542-1559`): both `alive` flags are false
        // (set only by the unported `dropevent` GM command), so both return with no draw.
        // `xmas_sock`, `moon_drop`, `hexegonal_drop`: flag first, draw inside. The rank
        // table has no BOSS/KING row: both divisors are 0 and the arms are dead there.
        for (flag, vnum) in [
            (b"xmas_sock".as_slice(), 50_010),
            (b"moon_drop".as_slice(), 50_011),
            (b"hexegonal_drop".as_slice(), 50_037),
        ] {
            if self.quest_event_sock_hit(flag, vnum, rank, delta, range) {
                hits.extend(self.make_drop_hit(vnum, 1, -1));
            }
        }
        // `hc_drop` (`:1695-1710`): level gates, then the flag, then `number(1, 100)`.
        let hc = self.event_rate(b"hc_drop");
        if killer_level >= 15 && killer_level.abs_diff(mob_level) <= 5 && hc > 0 {
            let pct = hc;
            if number(&mut self.dice, 1, 100) <= pct {
                hits.extend(self.make_drop_hit(30178, 1, -1));
            }
        }
        // The twelve unconditional draws (`:1713-1884`): the `number()` always runs.
        if self.drop_per_kill_pct(100, 2000, delta, b"2007_drop")
            >= number(&mut self.dice, 1, range)
        {
            hits.extend(self.make_drop_hit(50_043, 1, -1));
        }
        if self.drop_per_kill_pct(100, 1000, delta, b"newyear_fire")
            >= number(&mut self.dice, 1, range)
        {
            hits.extend(self.make_drop_hit(50_107, 1, -1));
        }
        if self.drop_per_kill_pct(100, 500, delta, b"newyear_moon")
            >= number(&mut self.dice, 1, range)
        {
            const WONSO: [u32; 6] = [50016, 50017, 50018, 50019, 50019, 50019];
            let vnum = WONSO[usize::try_from(number(&mut self.dice, 0, 5)).unwrap_or(0)];
            hits.extend(self.make_drop_hit(vnum, 1, -1));
        }
        if self.drop_per_kill_pct(1, 2000, delta, b"valentine_drop")
            >= number(&mut self.dice, 1, range)
        {
            const VALENTINE: [u32; 2] = [50024, 50025];
            let vnum = VALENTINE[usize::try_from(number(&mut self.dice, 0, 1)).unwrap_or(0)];
            hits.extend(self.make_drop_hit(vnum, 1, -1));
        }
        if self.drop_per_kill_pct(100, 2000, delta, b"icecream_drop")
            >= number(&mut self.dice, 1, range)
        {
            hits.extend(self.make_drop_hit(50_123, 1, -1));
        }
        // `new_xmas_event` ×2 (`:1768-1778`): the carry check short-circuits before the draw,
        // and a hit is `AutoGiveItem`, not a ground item (D5).
        if self.count_xmas_sock_hits(attacker, delta, range) != 0 {
            tracing::warn!(
                vnum = 50_010,
                "a kill granted a sock; AutoGiveItem waits for D5"
            );
        }
        if self.drop_per_kill_pct(100, 2000, delta, b"halloween_drop")
            >= number(&mut self.dice, 1, range)
        {
            hits.extend(self.make_drop_hit(30321, 1, -1));
        }
        if self.drop_per_kill_pct(100, 2000, delta, b"ramadan_drop")
            >= number(&mut self.dice, 1, range)
        {
            hits.extend(self.make_drop_hit(30315, 1, -1));
        }
        if self.drop_per_kill_pct(100, 2000, delta, b"easter_drop")
            >= number(&mut self.dice, 1, range)
        {
            let vnum = 50_160 + u32::try_from(number(&mut self.dice, 0, 19)).unwrap_or(0);
            hits.extend(self.make_drop_hit(vnum, 1, -1));
        }
        if self.drop_per_kill_pct(100, 2000, delta, b"football_drop")
            >= number(&mut self.dice, 1, range)
        {
            hits.extend(self.make_drop_hit(50_096, 1, -1));
        }
        if self.drop_per_kill_pct(100, 2000, delta, b"whiteday_drop")
            >= number(&mut self.dice, 1, range)
        {
            let vnum = WHITEDAY_ITEMS[usize::try_from(number(&mut self.dice, 0, 1)).unwrap_or(0)];
            hits.extend(self.make_drop_hit(vnum, 1, -1));
        }
        hits.extend(self.create_quest_drop_tail(rank, mob_level, killer_level, delta, range));
        hits
    }

    /// `CreateQuestDropItem`'s tail (`G/item_manager.cpp:1833-1910`): `kids_day_drop`,
    /// `medal_part_drop`, `three_skill_item`, `dragon_boat_festival_drop` and `mars_drop`,
    /// in legacy's order.
    fn create_quest_drop_tail(
        &mut self,
        rank: u8,
        mob_level: u8,
        killer_level: u8,
        delta: i32,
        range: i32,
    ) -> Vec<Item> {
        use common::enums::EMobRank;
        let mut hits: Vec<Item> = Vec::new();
        // `kids_day_drop[_high]` (`:1833-1851`): ONE draw, the flag by killer level.
        let kids = if killer_level >= 50 {
            b"kids_day_drop_high".as_slice()
        } else {
            b"kids_day_drop".as_slice()
        };
        if self.drop_per_kill_pct(100, 1000, delta, kids) >= number(&mut self.dice, 1, range) {
            hits.extend(self.make_drop_hit(50_034, 1, -1));
        }
        // `medal_part_drop` (`:1854-1862`): the mob gate short-circuits before the draw.
        if mob_level >= 30
            && self.drop_per_kill_pct(50, 100, delta, b"medal_part_drop")
                >= number(&mut self.dice, 1, range)
        {
            let vnum = MEDAL_PARTS[usize::try_from(number(&mut self.dice, 0, 4)).unwrap_or(0)];
            hits.extend(self.make_drop_hit(vnum, 1, -1));
        }
        // `three_skill_item` (`:1866-1874`): the level/rank gates short-circuit first.
        if mob_level >= 40
            && rank >= EMobRank::Boss as u8
            && self.drop_per_kill_pct(1, 1000, delta, b"three_skill_item")
                / Self::three_skill_adjust(mob_level)
                >= number(&mut self.dice, 1, range)
        {
            hits.extend(self.make_drop_hit(50513, 1, -1));
        }
        if self.drop_per_kill_pct(100, 1000, delta, b"dragon_boat_festival_drop")
            >= number(&mut self.dice, 1, range)
        {
            hits.extend(self.make_drop_hit(50_085, 1, -1));
        }
        if self.quest_mars_hit(rank, killer_level, delta, range) {
            hits.extend(self.make_drop_hit(70_035, 1, -1));
        }
        hits
    }
}

/// Whether `map` is a new-world map (`IsNewWorldMap`, `G/char.cpp:11653-11657`): one
/// of the seven `SungMaWillMap` entries.
fn is_new_world_map(map: i32) -> bool {
    SUNGMA_WILL_MAPS.iter().any(|&(index, _)| index == map)
}
