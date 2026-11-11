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
