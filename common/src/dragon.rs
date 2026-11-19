//! The Blue Dragon's stone table: four rows of `BlueDragonSetting.DragonStone`
//! (`locale/europe/BlueDragon.lua`, loaded by `settings.lua:174`), which
//! `BlueDragon_Damage` (`G/BlueDragon.cpp:139-196`) reads per hit through
//! `BlueDragon_GetIndexFactor` (`G/BlueDragon_Binder.cpp:166-213`).
//!
//! The quest host snapshots the rows once at load; the combat paths read the
//! snapshot, never Lua. A missing table, row or number reads 0, as the binder's
//! miss arms do, so the default table is the identity: no vnum-0 non-player
//! exists, no arm matches effect 0, and no unmounted attacker matches enemy 0.

/// `2493`, the vnum `BlueDragon_Damage` scales for (`G/BlueDragon.cpp:144-167`)
/// and the `char_state.cpp:1109` battle-state arm keys on.
pub const BLUE_DRAGON_VNUM: u32 = 2493;

/// `DEF_BONUS` (`G/BlueDragon_Binder.h:4`): the stone whose live count on the map
/// scales a hit ON the dragon down.
pub const DRAGON_STONE_DEF: u32 = 1;

/// `ATK_BONUS` (`G/BlueDragon_Binder.h:5`): the stone whose live count on the map
/// scales a hit BY the dragon up.
pub const DRAGON_STONE_ATK: u32 = 2;

/// `REGEN_PECT_BONUS` (`G/BlueDragon_Binder.h:6`): the stone whose live count on
/// the map adds to the dragon's HP recovery percent.
pub const DRAGON_STONE_REGEN_PCT: u32 = 3;

/// `REGEN_TIME_BONUS` (`G/BlueDragon_Binder.h:7`): the stone whose live count on
/// the map shortens the dragon's HP recovery cycle in seconds.
pub const DRAGON_STONE_REGEN_TIME: u32 = 4;

/// `SE_PERCENT_DAMAGE1..3` (`C/length.h:867-869`): the overlap effects `FSkillBreath`
/// sends around a struck player (`G/BlueDragon_Skill.h:105-116`).
pub const SE_PERCENT_DAMAGE1: u8 = 16;
/// `SE_PERCENT_DAMAGE2` (`C/length.h:868`).
pub const SE_PERCENT_DAMAGE2: u8 = 17;
/// `SE_PERCENT_DAMAGE3` (`C/length.h:869`).
pub const SE_PERCENT_DAMAGE3: u8 = 18;

/// The stone rows `BlueDragon_Damage` reads: `DragonStone[1]` to `DragonStone[4]`,
/// in order. The damage arms scan them in order and take the first match.
pub const DRAGON_STONE_ROWS: usize = 4;

/// One `DragonStone` row: the stone's vnum, its effect, its percent value, the
/// mount vnum it fears and that mount's damage multiple.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DragonStone {
    /// `vnum`: the stone mob's vnum (8031-8034 on the owner's data).
    pub vnum: u32,
    /// `effect_type`: 1 scales hits on the dragon, 2 hits by it, 3-4 its regen.
    pub effect: u32,
    /// `val`: the percent per live stone (25, 25, 20, 10).
    pub val: u32,
    /// `enemy`: the mount vnum the stone fears (20110-20113).
    pub enemy: u32,
    /// `enemy_val`: the multiple a feared mount hits the stone for (3).
    pub enemy_val: u32,
}

/// One `[min, max] -> pct` row of `BlueDragonSetting.{hp_period,hp_damage,hp_regen}`
/// (`BlueDragon_GetRangeFactor`, `G/BlueDragon_Binder.cpp:67-163`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RangeFactor {
    /// Inclusive lower bound (`min <= val`).
    pub min: i32,
    /// Inclusive upper bound (`val <= max`).
    pub max: i32,
    /// Percentage returned when `min <= val <= max`.
    pub pct: u32,
}

/// A `{ min, max }` pair read by `BlueDragon_GetSkillFactor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MinMax {
    /// Inclusive minimum.
    pub min: i32,
    /// Inclusive maximum.
    pub max: i32,
}

/// `BlueDragonSetting.Skill0` (`FSkillBreath`, `G/BlueDragon_Skill.h:4-134`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BreathSkill {
    /// Cooldown range in seconds (`Skill0.period`).
    pub period: MinMax,
    /// Maximum `DISTANCE_APPROX` to strike (`Skill0.damage_area`).
    pub damage_area: i32,
    /// Base damage range (`Skill0.default_damage`).
    pub default_damage: MinMax,
    /// Per-job percent ranges (`[musa, assa, sura, muda]`, `Skill0.damage`).
    pub job_damage: [MinMax; 4],
    /// Per-sex percent ranges (`[male, female]`, `Skill0.gender`).
    pub gender_damage: [MinMax; 2],
}

/// `BlueDragonSetting.Skill1` (`FSkillWeakBreath`, `G/BlueDragon_Skill.h:136-176`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WeakBreathSkill {
    /// Cooldown range in seconds (`Skill1.period`).
    pub period: MinMax,
    /// Maximum `DISTANCE_APPROX` to strike (`Skill1.damage_area`).
    pub damage_area: i32,
    /// Base damage range (`Skill1.default_damage`).
    pub default_damage: MinMax,
}

/// `BlueDragonSetting.Skill2` (`FSkillEarthQuake`, `G/BlueDragon_Skill.h:178-342`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EarthQuakeSkill {
    /// Cooldown range in seconds (`Skill2.period`).
    pub period: MinMax,
    /// Maximum `DISTANCE_APPROX` to strike (`Skill2.damage_area`).
    pub damage_area: i32,
    /// Base damage range (`Skill2.default_damage`).
    pub default_damage: MinMax,
    /// Base stun duration range in seconds (`Skill2.stun_time.default`).
    pub stun_default: MinMax,
    /// Per-job bonus stun seconds (`[musa, assa, sura, muda]`, `Skill2.stun_time`).
    pub job_stun: [MinMax; 4],
    /// Per-sex bonus stun seconds (`[male, female]`, `Skill2.gender`).
    pub gender_stun: [MinMax; 2],
}

/// Full snapshot of `BlueDragonSetting` (`locale/europe/BlueDragon.lua`), read once at
/// quest-host load so combat and recovery never re-enter Lua.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BlueDragonSetting {
    /// `DragonStone[1..4]`.
    pub stones: [DragonStone; DRAGON_STONE_ROWS],
    /// `hp_period`: extra cooldown percent by dragon HP%.
    pub hp_period: Vec<RangeFactor>,
    /// `hp_damage`: extra damage percent by dragon HP%.
    pub hp_damage: Vec<RangeFactor>,
    /// `hp_regen`: extra HP recovery percent by dragon HP%.
    pub hp_regen: Vec<RangeFactor>,
    /// `Skill0` (Breath).
    pub skill0: BreathSkill,
    /// `Skill1` (Weak Breath).
    pub skill1: WeakBreathSkill,
    /// `Skill2` (`EarthQuake`).
    pub skill2: EarthQuakeSkill,
}

impl BlueDragonSetting {
    /// `BlueDragon_GetRangeFactor(key, val)` (`G/BlueDragon_Binder.cpp:137-162`): returns
    /// the first row whose `min <= val && val <= max`, or 0 when none matches.
    #[must_use]
    pub fn range_factor(rows: &[RangeFactor], val: i32) -> u32 {
        for row in rows {
            if row.min <= val && val <= row.max {
                return row.pct;
            }
        }
        0
    }
}
