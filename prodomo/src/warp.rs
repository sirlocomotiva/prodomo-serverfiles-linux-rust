#![warn(missing_docs)]

//! The Warp: the pending location legacy carries in `m_posWarp`, the two halves of a warp
//! (`CHARACTER::WarpSet` and `CHARACTER::WarpEnd`), and the `GC_WARP` record.
//!
//! # What a warp is in the legacy source
//!
//! A warp is two steps. [`WarpTarget`] is the pending destination, held in `m_posWarp` (a
//! `D3DPOINT`, so it carries a `z` the server never reads) plus `m_lWarpMapIndex`.
//!
//! 1. **Decide.** [`judge_warp_set`] is `CHARACTER::WarpSet` (`G/char.cpp:6694-6791`). It
//!    resolves the requested position through `CMapLocation::Get`, checks a private-map
//!    parentage rule and a sort-inventory cooldown, then `Stop()`s the character, `Save()`s it,
//!    removes it from its sector, stores the pending target, and sends `GC_WARP`.
//! 2. **Complete.** [`judge_warp_end`] is `CHARACTER::WarpEnd` (`G/char.cpp:6795-6846`). It
//!    checks the target map against the Channel's allow set, then either goes home or calls
//!    `Show` at the pending position and clears the pending target.
//!
//! Step 2 runs when the character's own descriptor receives `CG_WARP`, whose handler is
//! `CInputMain::Warp` (`G/input_main.cpp:2271-2274`), a `void` function whose whole body is
//! `ch->WarpEnd()`. Only the descriptor that sent `GC_WARP` holds a pending target: the
//! descriptor the client opens at the new address loads the character from the row, where
//! `m_posWarp` is zero. Whether the client sends `CG_WARP` at all, and on which connection, is
//! not in the server source; the owner's play test answers it.
//!
//! # The Rewrite's departure
//!
//! The Rewrite runs `WarpSet` on the character's descriptor (`warp_set` in the binary). It
//! writes the row with the destination **before** `GC_WARP` leaves, instead of on the close,
//! so the login at the new address always loads the destination; [`departure_records`] are the
//! two records the client receives. Step 2 is therefore never pending on a descriptor that
//! analyzes records, and `CG_WARP` completes nothing (`docs/STATUS.md`, Divergences).
//!
//! # Three different "home" numbers
//!
//! Legacy reaches the empire start through **three** different routes that read **two**
//! different tables, and the Rewrite keeps them apart because they do not agree:
//!
//! | route | map | position | source |
//! |---|---|---|---|
//! | [`home_warp_location`] | `EMPIRE_START_MAP` | `EMPIRE_START` divided by 100 | `G/input_db.cpp:424-435` |
//! | [`go_home`] | resolved from the atlas | `EMPIRE_START` as stored | `G/char.cpp:8616-8619` |
//! | [`save_position`] | `m_lWarpMapIndex` | `m_posWarp` | `G/char.cpp:1551-1565` |
//!
//! Only the first has a hard-coded map number. `GoHome` passes no map at all, so
//! `WarpSet`'s third argument takes its default of `0` (`G/char.h:980`) and the map comes from
//! `CMapLocation::Get`.
//!
//! # Where the pending target is written to the store
//!
//! `CHARACTER::Save` prefers the pending target over the live position whenever either axis
//! is non-zero (`G/char.cpp:1551-1565`):
//!
//! ```ignore
//! if (m_posWarp.x != 0 || m_posWarp.y != 0)
//! {
//!     tab.x = m_posWarp.x;
//!     tab.y = m_posWarp.y;
//!     tab.z = 0;
//!     tab.lMapIndex = m_lWarpMapIndex;
//! }
//! else
//! {
//!     tab.x = GetX();
//!     tab.y = GetY();
//!     tab.z = GetZ();
//!     tab.lMapIndex = GetMapIndex();
//! }
//! ```
//!
//! That is how the loading-phase map refusal moves a character home without any `GC_WARP`
//! being sent: the refusal sets a pending target and closes the descriptor, and the
//! save-on-close (`CHARACTER::Disconnect` to `FlushDelayedSave` or `SaveReal`,
//! `G/char.cpp:1784-1788`, reached from `DESC::Destroy`, `G/desc.cpp:121-125`) writes it to the
//! row. The client learns only that the connection closed, and the next login loads the
//! character on the home map.

use crate::channel_login::{EMPIRE_START, EMPIRE_START_MAP};
use crate::loading_phase::{map_is_allowed, public_map_index, INSTANCE_MAP_BASE};
use gamedata::map_atlas::MapAtlas;
use protocol::gc_nested::GcWarp;
use protocol::gc_vid::{GcHeaderAndDword, HEADER_GC_CHARACTER_DEL};

/// A map index at or above this is a private (instance) map, `G/char.cpp:2289`.
///
/// The Rewrite's value is `>=`, the same choice [`public_map_index`] makes; see
/// [`warp_map_for_check`] for the legacy `>` it replaces.
pub const PRIVATE_MAP_INDEX_BASE: i32 = INSTANCE_MAP_BASE;

/// The `* 100` of `CHARACTER::SetWarpLocation` (`G/char.cpp:6672-6677`).
pub const WARP_LOCATION_SCALE: i32 = 100;

/// The pending warp destination: `m_lWarpMapIndex` and `m_posWarp`.
///
/// The legacy `m_posWarp` is a `D3DPOINT` and so has a `z`; no code in the tree reads it
/// before `Save` zeroes it, so this type has no `z` and [`save_position`] documents where the
/// zero comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WarpTarget {
    /// `m_lWarpMapIndex`.
    pub map_index: i32,
    /// `m_posWarp.x`.
    pub x: i32,
    /// `m_posWarp.y`.
    pub y: i32,
}

impl WarpTarget {
    /// Whether `WarpEnd` would run for this target: the `m_posWarp.x == 0 && m_posWarp.y == 0`
    /// early return at `G/char.cpp:6800-6801`.
    ///
    /// The test is "**both** axes are zero", so a target of `(0, 50)` counts as pending. The
    /// same expression gates [`save_position`].
    #[must_use]
    pub fn is_pending(&self) -> bool {
        !(self.x == 0 && self.y == 0)
    }

    /// The `GC_WARP` this target would send, once the address and port are resolved.
    #[must_use]
    pub fn to_record(&self, addr: i32, port: u16) -> GcWarp {
        GcWarp {
            header: GcWarp::header(),
            x: self.x,
            y: self.y,
            addr,
            port,
        }
    }
}

/// Where a character actually is: `GetX`, `GetY`, `GetZ`, and `GetMapIndex`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LivePoint {
    /// `GetX()`.
    pub x: i32,
    /// `GetY()`.
    pub y: i32,
    /// `GetZ()`, the one axis [`save_position`] drops.
    pub z: i32,
    /// `GetMapIndex()`.
    pub map_index: i32,
}

/// What `CHARACTER::Save` writes for a character's position, in source order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavePosition {
    /// The `if` branch: the pending target, with `tab.z` written as a literal `0`.
    Warp(WarpTarget),
    /// The `else` branch: the live position, `z` included.
    Live(LivePoint),
}

/// `CHARACTER::Save`'s choice of position columns (`G/char.cpp:1551-1565`).
///
/// The warp branch is taken whenever **either** axis is non-zero, which is the same condition
/// as [`WarpTarget::is_pending`].
#[must_use]
pub fn save_position(warp: &WarpTarget, live: &LivePoint) -> SavePosition {
    if warp.is_pending() {
        SavePosition::Warp(*warp)
    } else {
        SavePosition::Live(*live)
    }
}

/// `CHARACTER::SetWarpLocation(lMapIndex, x, y)` (`G/char.cpp:6672-6677`).
///
/// Legacy stores `x * 100` and `y * 100`, so the coordinates this accepts are in units of
/// 100. It performs no overflow check: the multiplication is a 32-bit `long` on the legacy
/// target and wraps silently.
///
/// This is that wrapping form, using [`i32::wrapping_mul`], and it exists to pin the legacy
/// behaviour rather than to be called. A wrapped coordinate is a position on the wrong side of
/// the world, so the Rewrite refuses that case instead: [`set_warp_location_checked`] is the
/// shape callers use, and the refusal is a Divergence recorded in `docs/STATUS.md`.
#[must_use]
pub fn set_warp_location(map_index: i32, x: i32, y: i32) -> WarpTarget {
    WarpTarget {
        map_index,
        x: x.wrapping_mul(WARP_LOCATION_SCALE),
        y: y.wrapping_mul(WARP_LOCATION_SCALE),
    }
}

/// [`set_warp_location`] with the legacy overflow refused rather than wrapped.
///
/// # Errors
///
/// `None` when either scaled axis leaves `i32`.
#[must_use]
pub fn set_warp_location_checked(map_index: i32, x: i32, y: i32) -> Option<WarpTarget> {
    let x = x.checked_mul(WARP_LOCATION_SCALE)?;
    let y = y.checked_mul(WARP_LOCATION_SCALE)?;
    Some(WarpTarget { map_index, x, y })
}

/// The pending target the loading-phase map refusal sets
/// (`CInputDB::PlayerLoad`, `G/input_db.cpp:424-435`):
///
/// ```ignore
/// SetWarpLocation(EMPIRE_START_MAP(bEmpire), EMPIRE_START_X(bEmpire) / 100, EMPIRE_START_Y(bEmpire) / 100);
/// ```
///
/// The division is cancelled by the multiplication in [`set_warp_location`], so the stored
/// coordinates equal `EMPIRE_START` exactly. That is true only because every `g_start_position`
/// value is a multiple of 100, which `every_empire_start_survives_the_hundred_scale_round_trip`
/// pins, and the `i32` division truncates toward zero exactly as the legacy `long` division
/// does.
///
/// The map is the **only** hard-coded home map in the tree, and it is the one that reaches the
/// store. `GoHome` does not use it.
///
/// # Errors
///
/// `None` for empire 0 or any empire above 3, and on a scale overflow.
///
/// Legacy accepts those empires: `EMPIRE_START_MAP` indexes `g_start_map` without a range
/// check, and `EMPIRE_START_X` returns `0` outside 1..=3, so the stored target is `(0, 0)`,
/// which [`WarpTarget::is_pending`] calls not pending, and `Save` then writes the character's
/// unchanged position. The character is never moved and the next login refuses the same map
/// again. The Rewrite returns `None` and stores nothing, which is the same stored row; the
/// resulting refusal loop is recorded as a legacy Defect in `docs/STATUS.md`.
#[must_use]
pub fn home_warp_location(empire: u8) -> Option<WarpTarget> {
    let index = usize::from(empire);
    let &(x, y) = EMPIRE_START.get(index)?;
    if empire == 0 {
        return None;
    }
    let map_index = *EMPIRE_START_MAP.get(index)?;
    set_warp_location_checked(map_index, x / WARP_LOCATION_SCALE, y / WARP_LOCATION_SCALE)
}

/// The empire start in `EMPIRE_START` units, which is what `GoHome` passes
/// (`G/char.cpp:8616-8619`).
///
/// `GoHome` calls `WarpSet(EMPIRE_START_X(e), EMPIRE_START_Y(e))` with **no map**, so
/// `WarpSet`'s third argument is its default `0` and the map is resolved from the position.
/// This is not the same route as [`home_warp_location`], and the two can disagree.
#[must_use]
pub fn go_home(empire: u8) -> Option<(i32, i32)> {
    EMPIRE_START.get(usize::from(empire)).copied()
}

/// `SECTREE_MANAGER::GetRecallPositionByEmpire` (`G/sectree_manager.cpp:493-532`): where a
/// character of `empire` is sent back to on a map, or `None` when the map index is not in the
/// atlas.
///
/// A private map's index (10000 and up) is folded onto its base map. The position is the base
/// map's `Town.txt` spawn for the empire when the file lists one per empire and the empire is 1
/// to 3, and its single spawn otherwise.
///
/// Under `__VERSION_162__`, which the owner's build defines (`common/prodomodefines.h:39`),
/// legacy first answers a restart position a quest registered with `add_restart_city_pos`. No
/// owner quest calls it (control: `pc.warp` is found in the same quest tree), and it is not
/// ported, so that table is always empty and the atlas answers.
#[must_use]
pub fn recall_position(atlas: &MapAtlas, map_index: i32, empire: u8) -> Option<(i32, i32)> {
    let region = atlas.region(public_map_index(map_index))?;
    Some(match (region.empire_spawns, empire) {
        (Some(spawns), 1..=3) => spawns[usize::from(empire - 1)],
        _ => region.spawn,
    })
}

/// The map index `WarpEnd` tests against the allow set (`G/char.cpp:6804-6806`).
///
/// ```ignore
/// int index = m_lWarpMapIndex;
/// if (index > 10000)
///     index /= 10000;
/// ```
///
/// **The Rewrite uses `>=`, where legacy writes `>`.** [`public_map_index`] already makes that
/// choice for the loading phase, and `WarpEnd` folds the index for the same purpose and feeds
/// the same `map_allow_find`, so this module reuses it rather than adding a second copy that
/// could drift apart from the first.
///
/// Legacy's `>` is an off-by-one: a pending private map index of exactly `10000` is not
/// folded, so it is tested against the allow set as the public map `10000` instead of as
/// private map `1000` of the `1` parent, and a Channel that hosts `1` refuses the warp. Five
/// sibling tests in the same tree fold with `>=` (`G/input_db.cpp:418`, `G/char.cpp:1968`,
/// `G/char.cpp:2289`, `G/char.cpp:6727`, `G/char.cpp:8902`), and `WarpSet` gates the same
/// family on `lPrivateMapIndex >= 10000` (`G/char.cpp:6727`) 78 lines above the `>`
/// it contradicts. It is recorded as a Defect in `docs/STATUS.md` and not reproduced:
/// reproducing it would refuse a warp to a valid parent map.
///
/// # Panics
///
/// Never at runtime: a plain `i32` comparison and division.
#[must_use]
pub fn warp_map_for_check(map_index: i32) -> i32 {
    public_map_index(map_index)
}

/// What `CHARACTER::WarpEnd` decided (`G/char.cpp:6795-6846`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarpEndOutcome {
    /// `m_posWarp` is unset, so `WarpEnd` returns at once. Nothing is sent and no state
    /// changes.
    NotPending,
    /// The target map is not allowed. `ENABLE_GOHOME_IF_MAP_NOT_ALLOWED` is defined directly
    /// above `WarpEnd` (`G/char.cpp:6794`), so legacy calls `GoHome()` and returns; the
    /// close-phase branch in the `#else` is not compiled. The coordinates are
    /// [`go_home`]'s, and the map is left to `WarpSet` to resolve.
    GoHome {
        /// `EMPIRE_START_X(e)`, `EMPIRE_START_Y(e)`.
        x: i32,
        /// `EMPIRE_START_Y(e)`.
        y: i32,
    },
    /// The target is allowed: `Show(m_lWarpMapIndex, m_posWarp.x, m_posWarp.y, 0)`, `Stop()`,
    /// then the pending target is cleared.
    Show {
        /// The pending target, which the caller must clear after showing it.
        target: WarpTarget,
    },
}

/// `CHARACTER::WarpEnd` as a decision, with the map allow set and the empire injected.
///
/// `channel_maps` is the Channel's configured map list, which is the union of its legacy
/// `MAP_ALLOW` sets. [`map_is_allowed`] is the same `map_allow_find`
/// (`G/config.cpp:174-183`) the loading phase already uses, so both call sites share one
/// implementation.
#[must_use]
pub fn judge_warp_end(warp: &WarpTarget, empire: u8, channel_maps: &[u32]) -> WarpEndOutcome {
    if !warp.is_pending() {
        return WarpEndOutcome::NotPending;
    }
    if !map_is_allowed(warp_map_for_check(warp.map_index), channel_maps) {
        // `GoHome()` then `return`. Legacy drops the pending target here without clearing it,
        // so the `GoHome` `WarpSet` overwrites it. The Rewrite records the same target and
        // lets the caller overwrite it the same way.
        // `GoHome` uses `EMPIRE_START_X`/`_Y`, which return `0` outside empires 1..=3, so a
        // refused warp for such an empire re-enters `WarpSet` at `(0, 0)` exactly as legacy
        // does. Nothing resolves that to a map, so the `WarpSet` refusal stands.
        let (x, y) = go_home(empire).unwrap_or_default();
        return WarpEndOutcome::GoHome { x, y };
    }
    WarpEndOutcome::Show { target: *warp }
}

/// The inputs of `CHARACTER::WarpSet` that are not the map lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WarpSetRequest {
    /// `x`, the requested `m_posWarp.x`.
    pub x: i32,
    /// `y`, the requested `m_posWarp.y`.
    pub y: i32,
    /// `lPrivateMapIndex`, legacy's third argument, defaulted to `0` by `GoHome`
    /// (`G/char.h:980`).
    pub private_map_index: i32,
    /// `GetSortInventoryPulse()`, which despite its name is a `time(0)` second: the sort sets
    /// it to `get_global_time() + 15` (`G/char.cpp:10861`).
    pub sort_inventory_pulse: i64,
    /// `get_global_time()`, which is `time(0)` plus the DB's clock offset (`global_time_gap`),
    /// in seconds (`G/utils.cpp:5-10`). Legacy compares the pulse to it with `>`. The name is
    /// older than that reading and is kept so the tests that pin the comparison keep their
    /// meaning.
    pub now_ms: i64,
}

/// How `CHARACTER::WarpSet` ended, in source order (`G/char.cpp:6694-6791`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarpSetOutcome {
    /// `!IsPC()`. Only a `CHARACTER::IsNPC()` can reach `WarpSet` in this deployment, and a
    /// NPC returns here.
    RefusedNotPc,
    /// `CMapLocation::Get` found no map at the requested position. Legacy logs
    /// `cannot find map location index` and returns.
    RefusedNoMap,
    /// `lPrivateMapIndex / 10000 != lMapIndex`: a private map whose index is not a child of
    /// the resolved public map. Legacy logs `Invalid map index` and returns.
    RefusedPrivateMap {
        /// The public map `CMapLocation::Get` resolved.
        resolved_map_index: i32,
    },
    /// `__SORT_INVENTORY_ITEMS__` is on, so a sort cooldown blocks the warp. Legacy sends
    /// `You cannot warp after sorting your inventory` as a `CHAT_TYPE_INFO` line.
    RefusedSortingInventory,
    /// The warp was set: the character is stopped, saved, removed from its sector, the
    /// pending target stored, and `GC_WARP` sent.
    Accepted {
        /// The public map `CMapLocation::Get` resolved.
        resolved_map_index: i32,
        /// The map actually stored, which is the private index when one was given.
        stored_map_index: i32,
        /// Whether the supplementary data block for the new map is sent. Legacy sends it only
        /// when the resolved map differs from the current one (`G/char.cpp:6712-6725`).
        sends_sdb: bool,
    },
}

/// `CHARACTER::WarpSet` as a decision, with `CMapLocation::Get` and the allow set injected.
///
/// `resolve` is `CMapLocation::instance().Get` and returns the public map at a position.
/// `current_map_index` is the map the character is on now, and it is the `lCurMapIndex` of
/// the same-supplementary-block test, which legacy recomputes rather than reusing
/// `GetMapIndex()`.
///
/// Legacy's `ENABLE_NEWSTUFF` block replaces `p.lAddr` with the configured proxy address when
/// one is set (`G/char.cpp:6769-6772`). The Rewrite has no proxy feature, so the address is
/// always the resolved one; the difference is a Divergence recorded in `docs/STATUS.md`.
#[must_use]
pub fn judge_warp_set(
    request: &WarpSetRequest,
    current_map_index: i32,
    is_pc: bool,
    resolve: &dyn Fn(i32, i32) -> Option<i32>,
) -> WarpSetOutcome {
    if !is_pc {
        return WarpSetOutcome::RefusedNotPc;
    }
    let Some(resolved_map_index) = resolve(request.x, request.y) else {
        return WarpSetOutcome::RefusedNoMap;
    };
    let mut stored_map_index = resolved_map_index;
    if request.private_map_index >= PRIVATE_MAP_INDEX_BASE {
        if request.private_map_index / PRIVATE_MAP_INDEX_BASE != resolved_map_index {
            return WarpSetOutcome::RefusedPrivateMap { resolved_map_index };
        }
        stored_map_index = request.private_map_index;
    }
    if request.sort_inventory_pulse > request.now_ms {
        return WarpSetOutcome::RefusedSortingInventory;
    }
    WarpSetOutcome::Accepted {
        resolved_map_index,
        stored_map_index,
        sends_sdb: current_map_index != resolved_map_index,
    }
}

/// The records the character's own client receives when `WarpSet` succeeds: the
/// `EncodeRemovePacket(this)` of leaving its sectree, which is a `GC_CHARACTER_DEL` of its own
/// VID (`G/char.cpp:6749-6755`, `:1256-1273`), then `GC_WARP` (`:6763-6785`).
///
/// The viewers' `GC_CHARACTER_DEL` from `ViewCleanup` is not here: the world sends it when the
/// character leaves the world, after the warp is decided (V11 in `docs/STATUS.md`).
#[must_use]
pub fn departure_records(vid: u32, target: &WarpTarget, addr: i32, port: u16) -> Vec<Vec<u8>> {
    let mut removed = Vec::new();
    GcHeaderAndDword::new(HEADER_GC_CHARACTER_DEL, vid).encode_into(&mut removed);
    let mut warp = Vec::new();
    target.to_record(addr, port).encode_into(&mut warp);
    vec![removed, warp]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel_login::EMPIRE_START;
    use crate::loading_phase::map_is_allowed;

    const NO_MAPS: &[u32] = &[];

    /// A pending target, so `WarpEnd` does not return at its first line.
    fn pending(map_index: i32, x: i32, y: i32) -> WarpTarget {
        WarpTarget { map_index, x, y }
    }

    fn request(x: i32, y: i32) -> WarpSetRequest {
        WarpSetRequest {
            x,
            y,
            private_map_index: 0,
            sort_inventory_pulse: 0,
            now_ms: 1_000,
        }
    }

    /// The identity resolver: every position is on the map it names.
    fn on_map(map_index: i32) -> impl Fn(i32, i32) -> Option<i32> {
        move |_, _| Some(map_index)
    }

    /// The owner's map atlas.
    fn owners_atlas() -> MapAtlas {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe/map");
        MapAtlas::load(&dir).expect("the legacy map directory parses")
    }

    #[test]
    fn the_recall_position_is_the_empires_town_spawn() {
        let atlas = owners_atlas();
        // Map 1's `Town.txt` lists a spawn per empire; each empire gets its own, and an empire
        // outside 1..=3 gets the single spawn.
        let map_1: Vec<Option<(i32, i32)>> = (0..=4)
            .map(|empire| recall_position(&atlas, 1, empire))
            .collect();
        assert_eq!(
            map_1,
            [
                Some((469_300, 964_200)),
                Some((469_300, 964_200)),
                Some((417_600, 956_100)),
                Some((417_600, 956_100)),
                Some((469_300, 964_200)),
            ]
        );
        let map_3 = recall_position(&atlas, 3, 1);
        assert_eq!(map_3, Some((353_100, 882_900)));
        // Map 72 lists one spawn, which every empire gets.
        for empire in 0..=3 {
            assert_eq!(
                recall_position(&atlas, 72, empire),
                Some((10_000, 1_207_800))
            );
        }
        // A private map reads its base map, from 10000 on.
        assert_eq!(recall_position(&atlas, 10_000, 2), map_1[2]);
        assert_eq!(recall_position(&atlas, 30_007, 1), map_3);
        assert_eq!(
            recall_position(&atlas, 729_999, 3),
            Some((10_000, 1_207_800))
        );
        // An index the atlas does not list has no recall position.
        for index in [0, 9_999, -1, 99_990_000] {
            assert_eq!(recall_position(&atlas, index, 1), None, "{index}");
        }
    }

    /// The `g_start_map` and `g_start_position` agreement, which `home_warp_location` needs
    /// to be right. `channel_login` proves the table against the real map data; here the
    /// point is only that the value this module reads is the value that table says.
    #[test]
    fn the_empire_start_map_and_position_agree_where_the_real_atlas_agrees() {
        for empire in 1..=3u8 {
            let target = home_warp_location(empire).expect("empires 1..=3 have a home");
            let empire = usize::from(empire);
            assert_eq!(
                target.map_index, EMPIRE_START_MAP[empire],
                "empire {empire} map"
            );
            assert_eq!(
                (target.x, target.y),
                EMPIRE_START[empire],
                "empire {empire} position"
            );
        }
    }

    /// `SetWarpLocation` multiplies by 100 and the only caller divides by 100, so the two
    /// cancel. That is a property of the *values*, not of the arithmetic: `g_start_position`
    /// is a hand-written table, and a value that was not a multiple of 100 would truncate on
    /// the way in and land 99 units away from the intended start. Nothing in the legacy source
    /// checks it.
    #[test]
    fn every_empire_start_survives_the_hundred_scale_round_trip() {
        for (empire, &(x, y)) in EMPIRE_START.iter().enumerate() {
            assert_eq!(
                x % WARP_LOCATION_SCALE,
                0,
                "empire {empire} x {x} is not a multiple of {WARP_LOCATION_SCALE}"
            );
            assert_eq!(
                y % WARP_LOCATION_SCALE,
                0,
                "empire {empire} y {y} is not a multiple of {WARP_LOCATION_SCALE}"
            );
            // And the round trip itself, so a change to either constant fails here.
            let scaled = set_warp_location(1, x / WARP_LOCATION_SCALE, y / WARP_LOCATION_SCALE);
            assert_eq!((scaled.x, scaled.y), (x, y), "empire {empire} round trip");
        }
    }

    /// The negative axis case, which `g_start_position` happens not to contain but the
    /// arithmetic does. C++ integer division truncates toward zero, so the only caller of
    /// `SetWarpLocation`, which divides by 100, turns `-150` into `-1` and then stores
    /// `-100`. The round trip is lossy for a negative value that is not a multiple of 100.
    #[test]
    fn the_scale_round_trip_truncates_toward_zero_like_legacy() {
        assert_eq!(
            (-150i32).div_euclid(100),
            -2,
            "Euclidean, which C++ does not do"
        );
        let scaled = set_warp_location(7, -150 / WARP_LOCATION_SCALE, 250 / WARP_LOCATION_SCALE);
        assert_eq!(scaled.x, -100, "C++ truncation gives -1, so -100, not -150");
        assert_eq!(scaled.y, 200);
    }

    #[test]
    fn a_scale_overflow_is_refused_rather_than_wrapped() {
        assert_eq!(
            set_warp_location(1, i32::MAX, 0).x,
            -100,
            "2147483647 * 100 is 100 short of 50 * 2^32, so it wraps to -100"
        );
        assert_eq!(
            set_warp_location_checked(1, i32::MAX, 0),
            None,
            "the Rewrite refuses"
        );
        // The most negative multiple of 100 an `i32` holds, and the next one down.
        assert_eq!(
            set_warp_location_checked(1, -21_474_836, 0),
            Some(WarpTarget {
                map_index: 1,
                x: -2_147_483_600,
                y: 0
            }),
            "the last multiple of 100 is allowed"
        );
        assert_eq!(
            set_warp_location_checked(1, -21_474_837, 0),
            None,
            "one below it is not"
        );
    }

    /// `CHARACTER::Save` prefers the pending target over the live position, and the test is
    /// "either axis", so `(0, 50)` still warps.
    #[test]
    fn the_save_prefers_the_pending_target_when_either_axis_is_set() {
        let live = LivePoint {
            x: 111,
            y: 222,
            z: 33,
            map_index: 1,
        };
        assert_eq!(
            save_position(&pending(1, 500, 600), &live),
            SavePosition::Warp(pending(1, 500, 600))
        );
        assert_eq!(
            save_position(&pending(1, 0, 600), &live),
            SavePosition::Warp(pending(1, 0, 600)),
            "one axis is enough"
        );
        assert_eq!(
            save_position(&pending(1, 500, 0), &live),
            SavePosition::Warp(pending(1, 500, 0)),
            "one axis is enough"
        );
        assert_eq!(
            save_position(&pending(1, 0, 0), &live),
            SavePosition::Live(live),
            "both axes zero falls through to the live position"
        );
    }

    /// `WarpEnd` returns before touching anything when the pending target is unset, and
    /// `Save` is the same test, so the two agree by construction.
    #[test]
    fn an_unset_pending_target_is_neither_saved_north_warped() {
        let empty = pending(1, 0, 0);
        assert!(!empty.is_pending());
        assert_eq!(
            save_position(
                &empty,
                &LivePoint {
                    x: 1,
                    y: 2,
                    z: 3,
                    map_index: 1
                }
            ),
            SavePosition::Live(LivePoint {
                x: 1,
                y: 2,
                z: 3,
                map_index: 1
            })
        );
        assert_eq!(judge_warp_end(&empty, 1, &[1]), WarpEndOutcome::NotPending);
    }

    /// An allowed target is shown at its own coordinates, and the caller is told to clear it.
    #[test]
    fn an_allowed_target_is_shown() {
        let target = pending(1, 469_300, 964_200);
        assert_eq!(
            judge_warp_end(&target, 2, &[1, 21]),
            WarpEndOutcome::Show { target }
        );
    }

    /// `ENABLE_GOHOME_IF_MAP_NOT_ALLOWED` is defined above `WarpEnd` in the legacy source, so
    /// the refusal goes home rather than to `PHASE_CLOSE`.
    #[test]
    fn a_target_on_a_map_the_channel_does_not_host_goes_home() {
        let target = pending(41, 969_600, 278_400);
        assert_eq!(
            judge_warp_end(&target, 3, &[1, 21]),
            WarpEndOutcome::GoHome {
                x: 969_600,
                y: 278_400
            }
        );
    }

    /// `GoHome` passes no map, so the refusal is the same coordinates again: for the empire
    /// whose start is on the map just refused, the home *is* the refused target and the warp
    /// still cannot happen. Legacy behaves identically; the Rewrite does not invent a
    /// different destination.
    #[test]
    fn a_home_that_is_itself_not_hosted_still_reports_the_same_coordinates() {
        assert_eq!(
            judge_warp_end(&pending(1, 469_300, 964_200), 1, NO_MAPS),
            WarpEndOutcome::GoHome {
                x: 469_300,
                y: 964_200
            }
        );
    }

    /// `EMPIRE_START_X` returns `0` outside empires 1..=3, so a refused warp for empire 0
    /// re-enters `WarpSet` at the origin.
    #[test]
    fn an_empire_outside_one_to_three_goes_to_the_origin() {
        assert_eq!(
            judge_warp_end(&pending(41, 1, 2), 0, NO_MAPS),
            WarpEndOutcome::GoHome { x: 0, y: 0 }
        );
        assert_eq!(
            judge_warp_end(&pending(41, 1, 2), 9, NO_MAPS),
            WarpEndOutcome::GoHome { x: 0, y: 0 }
        );
    }

    /// The pending target is stored in `WarpSet` coordinates, which are the same units, so
    /// `GoHome` and `SetWarpLocation` hand the same scale to `m_posWarp` without the `* 100`.
    #[test]
    fn go_home_passes_the_stored_scale_while_set_warp_location_scales() {
        assert_eq!(go_home(1), Some((469_300, 964_200)), "unscaled");
        assert_eq!(
            home_warp_location(1).map(|t| (t.x, t.y)),
            Some((469_300, 964_200)),
            "divided then scaled back"
        );
    }

    /// Legacy's `WarpEnd` folds with `>` and the Rewrite with `>=`, so a private map index of
    /// exactly `10000` is the whole difference. The five sibling `>=` sites are listed in the
    /// module docs; this pins the Rewrite's choice at the boundary.
    #[test]
    fn the_private_map_fold_is_inclusive_at_exactly_ten_thousand() {
        assert_eq!(
            warp_map_for_check(10_000),
            1,
            "the Rewrite folds 10000 to parent 1"
        );
        assert_eq!(warp_map_for_check(10_001), 1);
        assert_eq!(warp_map_for_check(20_500), 2);
        assert_eq!(
            warp_map_for_check(9_999),
            9_999,
            "below the base is untouched"
        );
        assert_eq!(warp_map_for_check(0), 0);
        assert_eq!(
            warp_map_for_check(-5),
            -5,
            "a negative index is never folded"
        );
    }

    /// A pending private map is judged on its parent, which is what a Channel's list holds.
    #[test]
    fn a_private_target_is_judged_on_the_parent_map() {
        let target = pending(10_001, 10, 20);
        assert_eq!(
            judge_warp_end(&target, 1, &[1]),
            WarpEndOutcome::Show { target },
            "a Channel hosting the parent accepts the private index"
        );
        assert_eq!(
            judge_warp_end(&target, 1, &[10_001]),
            WarpEndOutcome::GoHome {
                x: 469_300,
                y: 964_200
            },
            "hosting the private index is not hosting the parent"
        );
    }

    /// Legacy's `g_bAuthServer` arm has no counterpart: every game listener hosts a Channel.
    /// The Rewrite's allow test is the Channel's list, and the loading phase shares it.
    #[test]
    fn the_allow_test_is_the_channels_map_list() {
        assert!(map_is_allowed(1, &[1, 21]));
        assert!(!map_is_allowed(41, &[1, 21]));
        assert!(!map_is_allowed(1, NO_MAPS));
        assert!(
            !map_is_allowed(1_000_000, &[1]),
            "an index is compared whole, not folded or truncated"
        );
    }

    /// `WarpSet` in source order: not a PC, then no map, then private-map parentage, then the
    /// sort cooldown, and only then the warp is set.
    #[test]
    fn warp_set_refuses_in_source_order() {
        assert_eq!(
            judge_warp_set(&request(1, 2), 1, false, &on_map(1)),
            WarpSetOutcome::RefusedNotPc,
            "the PC test comes first"
        );
        assert_eq!(
            judge_warp_set(&request(1, 2), 1, true, &|_, _| None),
            WarpSetOutcome::RefusedNoMap
        );
        let wrong_parent = WarpSetRequest {
            private_map_index: 20_000,
            ..request(1, 2)
        };
        assert_eq!(
            judge_warp_set(&wrong_parent, 1, true, &on_map(1)),
            WarpSetOutcome::RefusedPrivateMap {
                resolved_map_index: 1
            }
        );
        let sorting = WarpSetRequest {
            sort_inventory_pulse: 5_000,
            ..request(1, 2)
        };
        assert_eq!(
            judge_warp_set(&sorting, 1, true, &on_map(1)),
            WarpSetOutcome::RefusedSortingInventory
        );
    }

    /// The cooldown is `GetSortInventoryPulse() > get_global_time()`, so an equal pulse is
    /// already over.
    #[test]
    fn the_sort_cooldown_is_strict() {
        let now = 1_000;
        let past = WarpSetRequest {
            sort_inventory_pulse: now,
            ..request(1, 2)
        };
        assert!(matches!(
            judge_warp_set(&past, 1, true, &on_map(1)),
            WarpSetOutcome::Accepted { .. }
        ));
        let future = WarpSetRequest {
            sort_inventory_pulse: now + 1,
            ..request(1, 2)
        };
        assert_eq!(
            judge_warp_set(&future, 1, true, &on_map(1)),
            WarpSetOutcome::RefusedSortingInventory
        );
    }

    /// A private index that *is* a child of the resolved map is stored, and the resolved
    /// public map is what the supplementary-block test compares.
    #[test]
    fn a_valid_private_target_stores_the_private_index() {
        let req = WarpSetRequest {
            private_map_index: 10_500,
            ..request(1, 2)
        };
        assert_eq!(
            judge_warp_set(&req, 7, true, &on_map(1)),
            WarpSetOutcome::Accepted {
                resolved_map_index: 1,
                stored_map_index: 10_500,
                sends_sdb: true,
            }
        );
    }

    /// Legacy sends the supplementary data block only when the resolved map differs from the
    /// current one, and it recomputes the current map rather than reusing `GetMapIndex()`.
    #[test]
    fn the_supplementary_block_is_sent_only_when_the_map_changes() {
        assert_eq!(
            judge_warp_set(&request(1, 2), 1, true, &on_map(1)),
            WarpSetOutcome::Accepted {
                resolved_map_index: 1,
                stored_map_index: 1,
                sends_sdb: false,
            }
        );
        assert_eq!(
            judge_warp_set(&request(1, 2), 21, true, &on_map(1)),
            WarpSetOutcome::Accepted {
                resolved_map_index: 1,
                stored_map_index: 1,
                sends_sdb: true,
            }
        );
    }

    /// `GC_WARP` is 15 bytes with the three `long`s in source order and the `WORD` last, and
    /// the port is a `u16`, so the port 65535 is representable while 65536 is not.
    #[test]
    fn the_warp_record_carries_the_target_and_the_resolved_endpoint() {
        let target = pending(10_500, 469_300, 964_200);
        let mut bytes = Vec::new();
        target
            .to_record(i32::from_le_bytes([127, 0, 0, 1]), 30003)
            .encode_into(&mut bytes);
        assert_eq!(
            bytes.len(),
            protocol::gc_nested::GC_WARP_WIRE_SIZE,
            "15 bytes"
        );
        assert_eq!(bytes[0], 0x41, "HEADER_GC_WARP");
        assert_eq!(&bytes[1..5], &469_300i32.to_le_bytes());
        assert_eq!(&bytes[5..9], &964_200i32.to_le_bytes());
        assert_eq!(
            &bytes[9..13],
            &i32::from_le_bytes([127, 0, 0, 1]).to_le_bytes()
        );
        assert_eq!(&bytes[13..15], &30003u16.to_le_bytes());
    }

    /// The map the loading phase refuses is moved home by the save, so the stored row after
    /// the refusal is the empire start, not the refused position. This is the whole point of
    /// the module: no `GC_WARP` is sent on that path.
    #[test]
    fn the_loading_refusal_stores_the_home_and_sends_no_warp() {
        let refused = LivePoint {
            x: 969_600,
            y: 278_400,
            z: 0,
            map_index: 41,
        };
        let home = home_warp_location(3).expect("empire 3 has a home");
        assert_eq!(home.map_index, 41, "g_start_map[3] is 41");
        assert_eq!(save_position(&home, &refused), SavePosition::Warp(home));
        // Empire 3's start IS map 41, so the very map the Channel refused is the home. The
        // refusal is therefore about a Channel that hosts neither.
        assert!(!map_is_allowed(home.map_index, &[1, 21]));
        // And with the home on a hosted map the save writes the home coordinates.
        let hosted = home_warp_location(2).expect("empire 2 has a home");
        assert_eq!(hosted.map_index, 21);
        assert!(map_is_allowed(hosted.map_index, &[1, 21]));
        assert_eq!(save_position(&hosted, &refused), SavePosition::Warp(hosted));
    }

    #[test]
    fn a_departure_deletes_the_character_for_its_own_client_and_then_names_the_destination() {
        let target = WarpTarget {
            map_index: 3,
            x: 400_200,
            y: 899_500,
        };
        let records = departure_records(
            0x0A0B_0C0D,
            &target,
            i32::from_le_bytes([127, 0, 0, 1]),
            0x7533,
        );
        assert_eq!(
            records,
            vec![
                vec![2, 0x0D, 0x0C, 0x0B, 0x0A],
                vec![
                    0x41, 0x48, 0x1B, 0x06, 0x00, 0xAC, 0xB9, 0x0D, 0x00, 127, 0, 0, 1, 0x33, 0x75
                ],
            ],
            "GC_CHARACTER_DEL of the own VID, then the 15-byte GC_WARP"
        );
    }
}
