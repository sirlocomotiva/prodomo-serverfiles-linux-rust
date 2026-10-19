//! What a non-player fights with: the points `SetProto` gives it from its proto.
//!
//! `CHARACTER::SetProto` (`G/char.cpp:2466-2500`) sets the level and the four stats from the
//! proto, then `ComputePoints`, whose non-player branch (`:3009-3017`) takes the speeds from the
//! proto and whose `ComputeBattlePoints` (`:2849-2859`) makes the attack grade twice the level
//! plus twice the strength and the defence grade the level plus the vitality plus the proto's
//! `wDef`; the magic grades copy them. `ApplyMobAttribute` (`G/char_resist.cpp:235-248`) then
//! adds each enchant and resist that is not 0 through `ApplyPoint`, by the apply types of
//! `aiMobEnchantApplyIdx` and `aiMobResistsApplyIdx` (`G/constants.cpp:643-666`).
//!
//! Nothing a non-player does changes these points in the Rewrite yet: an affect on a non-player
//! waits for the affect system, and the Santa's grade bonuses (`G/char.cpp:2517-2528`) for the
//! Christmas event.

use std::collections::BTreeMap;

use common::point_slot as point;
use gamedata::records::{MobTableRecord, MOB_ENCHANTS_MAX_NUM, MOB_RESISTS_MAX_NUM};

use crate::character::ApplyArm;

/// `aiMobEnchantApplyIdx`: the apply type each of a proto's `cEnchants` adds to.
const ENCHANT_APPLIES: [u8; MOB_ENCHANTS_MAX_NUM] = [
    57, // APPLY_CURSE_PCT
    14, // APPLY_SLOW_PCT
    12, // APPLY_POISON_PCT
    13, // APPLY_STUN_PCT
    15, // APPLY_CRITICAL_PCT
    16, // APPLY_PENETRATE_PCT
];

/// `aiMobResistsApplyIdx`: the apply type each of a proto's `cResists` adds to.
const RESIST_APPLIES: [u8; MOB_RESISTS_MAX_NUM] = [
    29, // APPLY_RESIST_SWORD
    30, // APPLY_RESIST_TWOHAND
    31, // APPLY_RESIST_DAGGER
    32, // APPLY_RESIST_BELL
    33, // APPLY_RESIST_FAN
    34, // APPLY_RESIST_BOW
    35, // APPLY_RESIST_FIRE
    36, // APPLY_RESIST_ELEC
    37, // APPLY_RESIST_MAGIC
    38, // APPLY_RESIST_WIND
    41, // APPLY_POISON_REDUCE
];

/// A non-player's fighting points and the proto values its fight reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MobFight {
    /// `GetLevel`: the proto's `bLevel`.
    pub level: u8,
    /// `GetMobRank`: the proto's `bRank`.
    pub rank: u8,
    /// `GetMobBattleType`: the proto's `bBattleType`.
    pub battle_type: u8,
    /// `GetMobAttackRange`: the proto's `wAttackRange`.
    pub attack_range: u16,
    /// `wAggressiveSight`: the proto's aggressive sight range.
    pub aggressive_sight: u16,
    /// `fDamMultiply` as its IEEE-754 bits, because an `f32` would break `Eq`.
    pub dam_multiply_bits: u32,
    /// `bBerserkPoint`: below this share of its hit points a berserker turns berserk.
    pub berserk_point: u8,
    /// `bDeathBlowPoint`: the death-blow chance.
    pub death_blow_point: u8,
    /// `GetMobDamageMin` and `GetMobDamageMax`: the proto's `dwDamageRange`.
    pub damage_range: [u32; 2],
    /// The proto's `bRegenCycle`: the seconds between two recoveries.
    pub regen_cycle: u8,
    /// The proto's `bRegenPercent`: the share of the maximum one recovery gives back.
    pub regen_percent: u8,
    /// `GetMonsterDrainSPPoint`: the proto's `dwDrainSP`.
    pub drain_sp: u32,
    /// The proto's `bStoneSkinPoint`: below this share of its hit points a stone-skinned
    /// monster takes half the damage.
    pub stone_skin_point: u8,
    /// draws one pile from (`G/char_battle.cpp:558-749`).
    pub gold_range: [u32; 2],
    /// The proto's `DROP_ITEM` vnum, which `CreateDropItem`'s etc branch rolls against
    /// `etc_drop_item.txt` (`G/item_manager.cpp:1262-1277`).
    pub drop_item_vnum: u32,
    /// `m_pointsInstant.points`: every slot that is not 0.
    points: BTreeMap<usize, i32>,
}

impl MobFight {
    /// The points `SetProto` gives a character of `proto`.
    #[must_use]
    pub fn of(proto: &MobTableRecord) -> Self {
        let level = i32::from(proto.level);
        let strength = i32::from(proto.str);
        let vitality = i32::from(proto.con);
        let attack = level * 2 + strength * 2;
        let defence = level + vitality + i32::from(proto.def);
        let mut fight = Self {
            level: proto.level,
            rank: proto.rank,
            battle_type: proto.battle_type,
            attack_range: proto.attack_range,
            aggressive_sight: proto.aggressive_sight,
            dam_multiply_bits: proto.dam_multiply.to_bits(),
            berserk_point: proto.berserk_point,
            death_blow_point: proto.death_blow_point,
            damage_range: proto.damage_range,
            regen_cycle: proto.regen_cycle,
            regen_percent: proto.regen_percent,
            drain_sp: proto.drain_sp,
            stone_skin_point: proto.stone_skin_point,
            gold_range: [proto.gold_min, proto.gold_max],
            drop_item_vnum: proto.drop_item_vnum,
            points: BTreeMap::new(),
        };
        for (slot, value) in [
            (point::POINT_ST, strength),
            (point::POINT_HT, vitality),
            (point::POINT_DX, i32::from(proto.dex)),
            (point::POINT_IQ, i32::from(proto.int_)),
            (point::POINT_ATT_SPEED, i32::from(proto.attack_speed)),
            (point::POINT_MOV_SPEED, i32::from(proto.moving_speed)),
            (point::POINT_CASTING_SPEED, i32::from(proto.attack_speed)),
            (point::POINT_ATT_GRADE, attack),
            (point::POINT_DEF_GRADE, defence),
            (point::POINT_MAGIC_ATT_GRADE, attack),
            (point::POINT_MAGIC_DEF_GRADE, defence),
        ] {
            fight.add(slot, value);
        }
        let attributes = ENCHANT_APPLIES
            .iter()
            .zip(proto.enchants)
            .chain(RESIST_APPLIES.iter().zip(proto.resists));
        for (&apply, value) in attributes {
            if let (ApplyArm::Point(slot), true) = (ApplyArm::of(apply), value != 0) {
                fight.add(slot, i32::from(value));
            }
        }
        fight
    }

    /// `GetPoint`: the slot's value, 0 for one nothing set.
    #[must_use]
    pub fn point(&self, slot: usize) -> i32 {
        self.points.get(&slot).copied().unwrap_or(0)
    }

    /// `GetMobDamageMultiply`: the proto's `fDamMultiply`.
    #[must_use]
    pub fn dam_multiply(&self) -> f32 {
        f32::from_bits(self.dam_multiply_bits)
    }

    /// Adds `value` to the slot, removing it at 0 (`SetProto`'s accumulation).
    pub fn add(&mut self, slot: usize, value: i32) {
        let sum = self.point(slot).saturating_add(value);
        if sum == 0 {
            self.points.remove(&slot);
        } else {
            self.points.insert(slot, sum);
        }
    }
}

#[cfg(test)]
mod tests {
    use gamedata::item_proto_value::APPLY_TYPE;

    use super::*;
    use crate::character::legacy_source::{legacy, names, span, strip_comments};

    /// A proto with every value this module reads set apart from the others.
    fn proto() -> MobTableRecord {
        MobTableRecord {
            level: 7,
            rank: 2,
            battle_type: 1,
            attack_range: 175,
            damage_range: [20, 24],
            regen_cycle: 5,
            regen_percent: 3,
            drain_sp: 9,
            stone_skin_point: 40,
            aggressive_sight: 2000,
            dam_multiply: 3.0,
            berserk_point: 20,
            death_blow_point: 30,
            str: 11,
            dex: 13,
            con: 17,
            int_: 19,
            def: 23,
            attack_speed: 110,
            moving_speed: 120,
            enchants: [1, 2, 3, 4, 5, 6],
            resists: [-7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 0],
            ..MobTableRecord::default()
        }
    }

    /// `MobFight` keeps the sight, multiplier, berserk and death-blow points (U17).
    #[test]
    fn mob_fight_keeps_the_sight_multiplier_berserk_and_death_blow_points() {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto");
        let protos = gamedata::mob_proto::MobProtos::load(&root).expect("the owner's protos");
        for (vnum, sight, multiply, berserk, death_blow) in [
            (115_u32, 2000_u16, 1.0_f32, 0_u8, 0_u8),
            (406_u32, 2000_u16, 1.0_f32, 0_u8, 0_u8),
            (6311_u32, 2000_u16, 3.0_f32, 20_u8, 30_u8),
        ] {
            let fight = MobFight::of(protos.get(vnum).expect("the row is in the file"));
            assert_eq!(fight.aggressive_sight, sight, "{vnum}");
            assert_eq!(fight.dam_multiply().to_bits(), multiply.to_bits(), "{vnum}");
            assert_eq!(fight.berserk_point, berserk, "{vnum}");
            assert_eq!(fight.death_blow_point, death_blow, "{vnum}");
        }
    }

    #[test]
    fn the_stats_and_the_grades_come_from_the_proto() {
        let fight = MobFight::of(&proto());
        assert_eq!(fight.point(point::POINT_ST), 11);
        assert_eq!(fight.point(point::POINT_DX), 13);
        assert_eq!(fight.point(point::POINT_HT), 17);
        assert_eq!(fight.point(point::POINT_IQ), 19);
        assert_eq!(fight.point(point::POINT_ATT_SPEED), 110);
        assert_eq!(fight.point(point::POINT_MOV_SPEED), 120);
        assert_eq!(fight.point(point::POINT_CASTING_SPEED), 110);
        // 2 * 7 + 2 * 11, and 7 + 17 + 23.
        assert_eq!(fight.point(point::POINT_ATT_GRADE), 36);
        assert_eq!(fight.point(point::POINT_DEF_GRADE), 47);
        assert_eq!(fight.point(point::POINT_MAGIC_ATT_GRADE), 36);
        assert_eq!(fight.point(point::POINT_MAGIC_DEF_GRADE), 47);
        assert_eq!(
            (
                fight.level,
                fight.rank,
                fight.battle_type,
                fight.attack_range
            ),
            (7, 2, 1, 175)
        );
        assert_eq!(fight.damage_range, [20, 24]);
        assert_eq!((fight.regen_cycle, fight.regen_percent), (5, 3));
        assert_eq!((fight.drain_sp, fight.stone_skin_point), (9, 40));
        assert_eq!(fight.aggressive_sight, 2000);
        assert_eq!(fight.dam_multiply().to_bits(), 3.0_f32.to_bits());
        assert_eq!((fight.berserk_point, fight.death_blow_point), (20, 30));
    }

    #[test]
    fn the_enchants_and_resists_are_points() {
        let fight = MobFight::of(&proto());
        let enchants = [
            point::POINT_CURSE_PCT,
            point::POINT_SLOW_PCT,
            point::POINT_POISON_PCT,
            point::POINT_STUN_PCT,
            point::POINT_CRITICAL_PCT,
            point::POINT_PENETRATE_PCT,
        ];
        for (value, slot) in (1..).zip(enchants) {
            assert_eq!(fight.point(slot), value, "enchant slot {slot}");
        }
        let resists = [
            (point::POINT_RESIST_SWORD, -7),
            (point::POINT_RESIST_TWOHAND, 8),
            (point::POINT_RESIST_DAGGER, 9),
            (point::POINT_RESIST_BELL, 10),
            (point::POINT_RESIST_FAN, 11),
            (point::POINT_RESIST_BOW, 12),
            (point::POINT_RESIST_FIRE, 13),
            (point::POINT_RESIST_ELEC, 14),
            (point::POINT_RESIST_MAGIC, 15),
            (point::POINT_RESIST_WIND, 16),
            (point::POINT_POISON_REDUCE, 0),
        ];
        for (slot, value) in resists {
            assert_eq!(fight.point(slot), value, "resist slot {slot}");
        }
        assert_eq!(fight.point(point::POINT_RESIST_CRITICAL), 0);
    }

    #[test]
    fn the_apply_tables_are_legacys() {
        let text = strip_comments(&legacy("game/constants.cpp"));
        let table = |name: &str| -> Vec<u8> {
            span(&text, name, "};")
                .iter()
                .skip(1)
                .flat_map(|line| names(line, "APPLY_"))
                .map(|apply| {
                    let at = APPLY_TYPE
                        .iter()
                        .position(|entry| *entry == apply)
                        .unwrap_or_else(|| panic!("{apply} is not an apply type"));
                    u8::try_from(at).expect("an apply type fits a byte")
                })
                .collect()
        };
        assert_eq!(table("aiMobEnchantApplyIdx"), ENCHANT_APPLIES);
        assert_eq!(table("aiMobResistsApplyIdx"), RESIST_APPLIES);
    }

    #[test]
    fn a_resist_that_cancels_a_point_leaves_none() {
        let mut fight = MobFight::default();
        fight.add(point::POINT_RESIST_SWORD, 5);
        fight.add(point::POINT_RESIST_SWORD, -5);
        assert_eq!(fight.points.len(), 0);
        assert_eq!(fight.point(point::POINT_RESIST_SWORD), 0);
    }
}
