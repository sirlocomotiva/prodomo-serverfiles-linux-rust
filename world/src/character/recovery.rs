//! The player's heal: `recovery_event`'s PC branch (`G/char.cpp:3245-3284`) and
//! `DistributeSP` (`G/char_battle.cpp:141-217`).
//!
//! Both return the amount to add, or `None` when the pool is already full, so the caller can
//! skip the `PointChange` and the record it would write.

use crate::character::points::Points;
use common::point_slot as point;

/// `aiRecoveryPercents` (`G/char.cpp:3170`).
pub const RECOVERY_PERCENTS: [i32; 10] = [1, 5, 5, 5, 5, 5, 5, 5, 5, 5];

/// The PC branch's heal (`G/char.cpp:3261-3281`): `None` at full hit points.
///
/// `idle_ms` is `now - m_dwLastMoveTime`. The percent is
/// `RECOVERY_PERCENTS[min(9, idle_ms / 3000)]`.
#[must_use]
pub fn hp_recovery(points: &Points, idle_ms: u32) -> Option<i32> {
    if points.max_hp() <= points.hp() {
        return None;
    }
    let secs = (idle_ms / 3000) as usize;
    let percent = RECOVERY_PERCENTS[secs.min(9)];
    let mut amount = 15 + points.max_hp() * percent / 100;
    let regen = points.get_point(point::POINT_HP_REGEN);
    amount += amount * regen / 100;
    Some(amount)
}

/// `DistributeSP` (`G/char_battle.cpp:141-217`): `None` at full spell points.
#[must_use]
pub fn distribute_sp(
    points: &Points,
    skill_group: u8,
    attacking: bool,
    moving: bool,
) -> Option<i32> {
    if points.sp() >= points.max_sp() {
        return None;
    }
    let max = points.max_sp();
    let job = crate::character::race_to_job(points.race());
    let shaman = job == 3;
    let sura2 = job == 2 && skill_group == 2;
    let amount = if shaman || sura2 {
        if attacking {
            2 + max / 100
        } else if moving {
            3 + max * 2 / 100
        } else {
            10 + max * 3 / 100
        }
    } else if attacking {
        2 + max / 200
    } else if moving || points.hp() < points.max_hp() {
        2 + max / 100
    } else {
        9 + max / 100
    };
    let regen = points.get_point(point::POINT_SP_REGEN);
    let amount = amount + amount * regen / 100;
    Some(amount)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::points::{Points, PointsRow};

    fn points_with(race: u8, hp: i32, sp: i32) -> Points {
        Points::load(&PointsRow {
            race,
            level: 154,
            exp: 0,
            conqueror_level: 0,
            st: 17,
            ht: 18,
            dx: 19,
            iq: 20,
            sungma: [0; 4],
            hp,
            sp,
            stamina: 0,
            inven_point: 0,
            map_index: 1,
            part_base: 0,
            hair_part: 0,
            sash_part: 0,
        })
    }

    fn alpha() -> Points {
        points_with(3, 100, 100)
    }

    #[test]
    fn u12_hp_recovery_alpha() {
        let p = alpha();
        assert_eq!(hp_recovery(&p, 0), Some(29));
        assert_eq!(hp_recovery(&p, 2999), Some(29));
        assert_eq!(hp_recovery(&p, 3000), Some(86));
        assert_eq!(hp_recovery(&p, 30000), Some(86));
    }

    #[test]
    fn u13_no_heal_at_full_hp() {
        let p = points_with(3, 1420, 100); // hp == max_hp
        assert!(hp_recovery(&p, 3000).is_none());
    }

    #[test]
    fn u14_hp_regen_50() {
        let mut p = alpha();
        let _ = p.point_change(common::point_slot::POINT_HP_REGEN, 50, false, false);
        assert_eq!(hp_recovery(&p, 3000), Some(129));
    }

    #[test]
    fn u15_distribute_sp_cells() {
        // shaman (job 3), max_sp = 600, regen 0.
        let p = alpha();
        assert_eq!(distribute_sp(&p, 1, true, false), Some(8));
        assert_eq!(distribute_sp(&p, 1, false, true), Some(15));
        assert_eq!(distribute_sp(&p, 1, false, false), Some(28));
        // sura group 2 (job 2, skill_group 2): attacking 2+max/100, moving 3+max*2/100, idle 10+max*3/100
        let s2 = points_with(2, 100, 100);
        assert_eq!(distribute_sp(&s2, 2, true, false), Some(8));
        assert_eq!(distribute_sp(&s2, 2, false, true), Some(15));
        assert_eq!(distribute_sp(&s2, 2, false, false), Some(28));
        // sura group 1 (job 2, skill_group 1): not shaman/sura2 -> warrior-like
        let s1 = points_with(2, 100, 100);
        assert_eq!(distribute_sp(&s1, 1, true, false), Some(2 + 600 / 200));
        // warrior (job 0): attacking 2+max/200, moving 2+max/100
        let w = points_with(0, 100, 100);
        assert_eq!(distribute_sp(&w, 0, true, false), Some(2 + 600 / 200));
        assert_eq!(distribute_sp(&w, 0, false, true), Some(2 + 600 / 100));
        // idle with hp < max -> 2+max/100; hp == max -> 9+max/100
        assert_eq!(distribute_sp(&w, 0, false, false), Some(2 + 600 / 100));
        let wfull = points_with(0, 1420, 100);
        assert_eq!(distribute_sp(&wfull, 0, false, false), Some(9 + 600 / 100));
    }

    #[test]
    fn u16_no_sp_at_full() {
        let p = points_with(3, 100, 600); // sp == max_sp
        assert!(distribute_sp(&p, 1, false, false).is_none());
    }
}
