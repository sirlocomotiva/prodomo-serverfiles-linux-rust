//! The guild land's buildings (`do_build`, `do_clear_land`,
//! `G/cmd_gm.cpp:3435-3731`, `:4556-4571`).
//!
//! `do_build`'s six arms stand, knock down, and wall the land the caller stands
//! on (`CLand`, `G/building.cpp`): `c` creates one object past the proto, group,
//! dependency, price, gold and material gates and spends the price and the
//! materials; `d` deletes one object by VID; `w` and `e` stand and knock down the
//! four single walls; `W` and `E` stand and erase the wall blocks. `do_clear_land`
//! knocks every object down, drops the owner, and stands the claim stone. The
//! DB-peer packets (`HEADER_GD_CREATE_OBJECT` and friends) are retired per
//! ADR-0001: the create, delete and clear halves ride `GuildQuestStore` and land
//! through the land drain both paths share.
//!
//! A GM-player caller needs the land's owning guild and its master's PID, or the
//! line is silent (`sys_err` only); a caller off any land is silent the same
//! way. The price, gold and material gates — and the spend — run only `if
//! (test_server || GMLevel == GM_PLAYER)` (`:3530`, `:3589`), and the
//! out-of-bounds notice only `if (test_server)` (`:3584-3587`).
//!
//! # Divergences
//!
//! - **The occupancy check reads file cells.** Legacy's `checkAnother` sweeps
//!   `ATTR_OBJECT` cells (`ForAttrRegion`) plus every non-monster character in
//!   the footprint (`FIsIn`, `:545-560`), and `Show` stamps the footprint
//!   `ATTR_OBJECT` (`:177-184`). Nothing stamps cells yet (see
//!   `world::cells`), so the check reads the file's cells plus the standing
//!   bodies and NPCs, and never stamps: two builds can overlap where legacy's
//!   second would refuse. The stamp waits on the cell-write path.
//! - **The footprint math follows `RequestCreateObject`, not `ForAttrRegion`.**
//!   The bounds check rotates with `zRot * 2π / 360` (`:530`), while the attr
//!   sweep rotates with `lRotate / 180 * 3.14` (`G/sectree_manager.cpp:1273`).
//!   The port rotates both with π, since the sweep here walks cells, not the
//!   legacy cell loops.
//! - **The clear's stone is an event spawn.** `CLand::ClearLand` spawns mob
//!   20040 at the land center (`:1031-1036`); the port stands it through the
//!   event-spawn path, which refuses a point no sectree holds where legacy's
//!   `SpawnMob` plants it regardless.
//! - **The unknown-proto literal is mojibake in the frozen source.** The
//!   `LC_TEXT` bytes (`:3497`) are U+FFFD runs in the git blob itself; the
//!   port sends them unchanged rather than guessing the Korean.

use common::vid::Vid;

use super::GameState;
use crate::chat_line::Arg;
use crate::command::{one_argument, GM_PLAYER};
use crate::game_state::gm_lines::{int_word, GmRows};
use crate::game_state::guild::{BuildingObject, GuildQuestStore};
use crate::item_move::Mover;
use protocol::gc_chat::CHAT_TYPE_INFO;
use world::character::PointRecord;

/// `BUILDING_MAX_PRICE` (`G/cmd_gm.cpp:3501`).
const BUILDING_MAX_PRICE: u32 = 100_000_000;
/// `FLAG_VNUM` (`G/building.cpp:24`).
const FLAG_VNUM: u32 = 14200;
/// `WALL_DOOR_VNUM..WALL_RIGHT_VNUM` (`G/building.cpp:25-28`).
const WALL_VNUMS: [u32; 4] = [14201, 14202, 14203, 14204];
/// The `build W` set ids (`G/cmd_gm.cpp:3693`), parsed as `int`.
const WALL_SET_IDS: [i32; 3] = [14105, 14115, 14125];
/// The clear's claim stone (`CLand::ClearLand`, `G/building.cpp:1031`).
const CLEAR_STONE_VNUM: u32 = 20040;
/// The supply buildings (`G/building.cpp:19-21`).
const MEMBER_BONUS_VNUMS: [u32; 3] = [14061, 14062, 14063];
/// The member-count bonus each supply building grants (`:206-214`).
const MEMBER_BONUS: [i32; 3] = [6, 12, 18];

/// The unknown-proto literal (`G/cmd_gm.cpp:3497`), byte-exact from the git blob.
const UNKNOWN_PROTO: &[u8] = b"\xef\xbf\xbd\xef\xbf\xbd\xef\xbf\xbd\xef\xbf\xbd\xef\xbf\xbd\xef\xbf\xbd\xef\xbf\xbd\xef\xbf\xbd \xef\xbf\xbd\xca\xb4\xef\xbf\xbd \xef\xbf\xbd\xc7\xb9\xef\xbf\xbd\xef\xbf\xbd\xd4\xb4\xcf\xb4\xef\xbf\xbd.";

impl GameState {
    /// `do_build` (`G/cmd_gm.cpp:3435-3731`): the land gates, then the six arms
    /// on the first word's lowercased byte (`c`, `d`, `w`, `e`, `W`, `E`).
    /// Anything else tells `Invalid command %s`.
    pub(super) fn run_build_line(&mut self, vid: Vid, rest: &[u8], mover: Mover) -> Option<GmRows> {
        let (first, line) = one_argument(rest);
        let (channel, map, x, y) = self.build_spot(vid)?;
        let land_id = self.find_build_land(channel, map, x, y)?;
        if first.is_empty() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Invalid syntax: no command",
                &[],
            );
            return None;
        }
        if self.build_level(vid) == GM_PLAYER && !self.build_master_of(vid, land_id) {
            return None;
        }
        let place = (channel, map);
        let taken = self.build_halves.len();
        let rows = self.run_build_arm(vid, mover, place, land_id, &first, line);
        let halves = self.build_halves.split_off(taken);
        if halves.is_empty() {
            return rows;
        }
        if let Some(mut rows) = rows {
            rows.buildings = halves;
            return Some(rows);
        }
        let pid = self.characters.find_by_vid(vid).ok()?.player_id();
        let character = self.characters.find_by_vid(vid).ok()?;
        Some(GmRows {
            pid,
            changes: Vec::new(),
            cleared_skills: Vec::new(),
            gold: 0,
            monarch: None,
            points: character.points().cloned(),
            quickslots: None,
            buildings: halves,
            account: None,
        })
    }

    /// The six `do_build` arms on the first word's lowercased byte.
    fn run_build_arm(
        &mut self,
        vid: Vid,
        mover: Mover,
        place: (u8, i32),
        land_id: u32,
        first: &[u8],
        line: &[u8],
    ) -> Option<GmRows> {
        match first[0].to_ascii_lowercase() {
            b'c' => self.build_create(vid, mover, place, land_id, line),
            b'd' => {
                self.build_delete(vid, mover, place, land_id, line);
                None
            }
            b'w' => {
                if first[0] == b'w' {
                    self.build_walls(vid, mover, place, land_id, line);
                } else {
                    self.build_wall_blocks(vid, place, land_id, line);
                }
                None
            }
            b'e' => {
                if first[0] == b'e' {
                    self.build_unwall(vid, place, land_id);
                } else {
                    self.build_unwall_blocks(vid, place, land_id, line);
                }
                None
            }
            _ => {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"Invalid command %s",
                    &[Arg::Text(first)],
                );
                None
            }
        }
    }

    /// `do_clear_land` (`G/cmd_gm.cpp:4556-4571`): off any land the line is
    /// silent; else the notice tells and the land clears.
    pub(super) fn run_clear_land_line(&mut self, vid: Vid, mover: Mover) -> Option<GmRows> {
        let (channel, map, x, y) = self.build_spot(vid)?;
        let land_id = self.find_build_land(channel, map, x, y)?;
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"Guild Land(%d) Cleared",
            &[Arg::Int(i64::from(land_id))],
        );
        let taken = self.build_halves.len();
        self.clear_build_land((channel, map), land_id);
        let halves = self.build_halves.split_off(taken);
        let pid = self.characters.find_by_vid(vid).ok()?.player_id();
        let character = self.characters.find_by_vid(vid).ok()?;
        Some(GmRows {
            pid,
            changes: Vec::new(),
            cleared_skills: Vec::new(),
            gold: 0,
            monarch: None,
            points: character.points().cloned(),
            quickslots: None,
            buildings: halves,
            account: None,
        })
    }

    /// The caller's (Channel, map, x, y), or `None` for a caller standing nowhere.
    /// The point is the body's destination, which is where it stands.
    fn build_spot(&self, vid: Vid) -> Option<(u8, i32, i32, i32)> {
        let body = self.bodies.get(&vid)?;
        Some((
            body.channel,
            body.map,
            body.motion.dest.0,
            body.motion.dest.1,
        ))
    }

    /// `CManager::FindLand(channel, map, x, y)` (`G/building.cpp:765-796`): the
    /// first land in id order whose map matches and whose rect holds the
    /// region-local point.
    fn find_build_land(&self, channel: u8, map: i32, x: i32, y: i32) -> Option<u32> {
        let region = self.regens.region_of(channel, map)?;
        let (x, y) = (x - region.sx, y - region.sy);
        self.guild_lands.ordered().find_map(|land| {
            if land.map_index != map || x < land.x || y < land.y {
                return None;
            }
            if x > land.x.saturating_add(land.width) || y > land.y.saturating_add(land.height) {
                return None;
            }
            Some(land.id)
        })
    }

    /// The caller's GM level, silenced to implementor in test-server mode
    /// (`CHARACTER::GetGMLevel`, `G/char.cpp:6355-6356`).
    fn build_level(&self, vid: Vid) -> u8 {
        if self.test_server {
            return crate::command::GM_IMPLEMENTOR;
        }
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points().map(world::character::Points::gm_level))
            .unwrap_or(GM_PLAYER)
    }

    /// Whether the caller masters the guild owning `land` (`:3464-3476`).
    fn build_master_of(&self, vid: Vid, land: u32) -> bool {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return false;
        };
        let pid = character.player_id();
        let Some(guild) = self.guilds.land_owner(land) else {
            return false;
        };
        self.guild_of(pid) == Some(guild)
            && crate::game_state::guild::Guilds::master_of(&self.guilds, guild) == Some(pid)
    }
}

impl GameState {
    /// The `build c` arm (`G/cmd_gm.cpp:3479-3617`): six words (`vnum x y x_rot
    /// y_rot z_rot`) past the gates stand the object and, past the spend gate,
    /// spend the price and the materials. Fewer than six tells `Invalid syntax`.
    fn build_create(
        &mut self,
        vid: Vid,
        mover: Mover,
        place: (u8, i32),
        land_id: u32,
        line: &[u8],
    ) -> Option<GmRows> {
        let Some(words) = six_words(line) else {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Invalid syntax", &[]);
            return None;
        };
        let vnum = dword(&words[0]);
        let proto = self.build_proto_gated(vid, mover, place, land_id, vnum)?;
        let spend = self.test_server || self.build_level(vid) == GM_PLAYER;
        if spend {
            if proto.price > BUILDING_MAX_PRICE {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"The construction job failed beyond building cost information.",
                    &[],
                );
                return None;
            }
            let held = self
                .characters
                .find_by_vid(vid)
                .map_or(0, world::character::Character::gold);
            if u64::from(proto.price) > held {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"Construction costs are low.",
                    &[],
                );
                return None;
            }
            for material in proto.materials {
                if material.item_vnum == 0 {
                    break;
                }
                if !self.build_holds_material(vid, material.item_vnum, material.count) {
                    self.tell(
                        vid,
                        mover,
                        CHAT_TYPE_INFO,
                        b"Cannot build due to lack of materials.",
                        &[],
                    );
                    return None;
                }
            }
        }
        let (x_rot, y_rot, z_rot) = (
            float_word(&words[3]),
            float_word(&words[4]),
            float_word(&words[5]),
        );
        let (x, y) = (long_word(&words[1]), long_word(&words[2]));
        let stood = self.request_create_object(BuildOrder {
            place,
            land_id,
            vnum,
            x,
            y,
            rot: (x_rot, y_rot, z_rot),
            check: true,
        });
        if stood.is_none() {
            if self.test_server {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"This is a location where you cannot build buildings.",
                    &[],
                );
            }
            return None;
        }
        if !spend {
            return None;
        }
        self.build_spend(vid, mover, proto.price, &proto.materials)
    }

    /// The `build c` proto gates (`:3495-3528`): the proto answers, past the
    /// unknown-proto, group-conflict and missing-dependency refusals.
    fn build_proto_gated(
        &self,
        vid: Vid,
        mover: Mover,
        place: (u8, i32),
        land_id: u32,
        vnum: u32,
    ) -> Option<gamedata::records::ObjectProtoRecord> {
        let Some(proto) = self.object_protos.get(vnum).copied() else {
            self.tell(vid, mover, CHAT_TYPE_INFO, UNKNOWN_PROTO, &[]);
            return None;
        };
        if proto.group_vnum != 0 && self.land_has_group(place, land_id, proto.group_vnum) {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"There are types of buildings that cannot be built together.",
                &[],
            );
            return None;
        }
        if proto.dependent_group_vnum != 0
            && !self.land_has_group(place, land_id, proto.dependent_group_vnum)
        {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"The building required for construction is not built.",
                &[],
            );
            return None;
        }
        Some(proto)
    }

    /// Whether the caller holds at least `count` of `vnum` (`CountSpecifyItem`).
    fn build_holds_material(&self, vid: Vid, vnum: u32, count: u32) -> bool {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return false;
        };
        let usable = common::item_slots::usable_inventory_cells(character.inven_point());
        let held = world::character::count_specify_item(character.items(), usable, vnum);
        u64::from(count) <= u64::try_from(held.max(0)).unwrap_or(0)
    }

    /// The `build c` spend (`:3589-3614`): the gold leaves through
    /// `PointChange(POINT_GOLD, -price)` and the materials leave, in proto
    /// order, stopping at the first zero vnum.
    fn build_spend(
        &mut self,
        vid: Vid,
        mover: Mover,
        price: u32,
        materials: &[gamedata::records::ObjectMaterial],
    ) -> Option<GmRows> {
        let pid = self.characters.find_by_vid(vid).ok()?.player_id();
        let held = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::gold);
        let after = held.saturating_sub(u64::from(price));
        if let Ok(character) = self.characters.find_by_vid_mut(vid) {
            character.set_gold(after);
        }
        self.send_point_records(
            vid,
            &[PointRecord {
                kind: u8::try_from(common::point_slot::POINT_GOLD).unwrap_or(u8::MAX),
                amount: -i64::from(price),
                value: i64::try_from(after).unwrap_or(i64::MAX),
                broadcast: false,
            }],
        );
        let usable = self
            .characters
            .find_by_vid(vid)
            .ok()
            .map_or(0, |character| {
                common::item_slots::usable_inventory_cells(character.inven_point())
            });
        let mut changes = Vec::new();
        for material in materials {
            if material.item_vnum == 0 {
                break;
            }
            let Some(character) = self.characters.find_by_vid_mut(vid).ok() else {
                break;
            };
            let (items, _, slots) = character.items_points_and_quickslots_mut();
            let done = world::character::remove_specify_item(
                items,
                slots,
                &self.protos,
                usable,
                (material.item_vnum, material.count),
            );
            let packed = crate::item_move::MovedItems::new(
                pid,
                vid.raw(),
                done,
                mover,
                &self.protos,
                &self.locale,
            );
            changes.extend(packed.changes);
            for record in packed.records {
                let _sent = self.write_to_client(vid, record);
            }
        }
        let character = self.characters.find_by_vid(vid).ok()?;
        Some(GmRows {
            pid,
            changes,
            cleared_skills: Vec::new(),
            gold: -i64::from(price),
            monarch: None,
            points: character.points().cloned(),
            quickslots: None,
            buildings: Vec::new(),
            account: None,
        })
    }
}

/// Six words off the line, or `None` past a missing one.
fn six_words(line: &[u8]) -> Option<[Vec<u8>; 6]> {
    let (first, rest) = one_argument(line);
    let (second, rest) = one_argument(rest);
    let (third, rest) = one_argument(rest);
    let (fourth, rest) = one_argument(rest);
    let (fifth, rest) = one_argument(rest);
    let (sixth, _) = one_argument(rest);
    if first.is_empty()
        || second.is_empty()
        || third.is_empty()
        || fourth.is_empty()
        || fifth.is_empty()
        || sixth.is_empty()
    {
        return None;
    }
    Some([first, second, third, fourth, fifth, sixth])
}

/// One `RequestCreateObject` order: where to stand what, and whether to check
/// the footprint. Bundled so the create stays under the argument lint.
#[derive(Debug, Clone, Copy)]
struct BuildOrder {
    /// Where the object stands.
    place: (u8, i32),
    /// The land id.
    land_id: u32,
    /// The prototype vnum.
    vnum: u32,
    /// The land-local x and y.
    x: i32,
    /// The land-local y.
    y: i32,
    /// The rotations.
    rot: (f32, f32, f32),
    /// Whether to run the occupancy check.
    check: bool,
}

/// One `RequestCreateWallBlocks` side run: the corner, the walls and the
/// optional door, stepping `(x, y)` on. Bundled so the run stays under the
/// argument lint.
struct WallSide {
    /// Where the blocks stand.
    place: (u8, i32),
    /// The land id.
    land_id: u32,
    /// The corner, wall and door vnums.
    corner: u32,
    /// The wall vnum.
    wall: u32,
    /// The door vnum.
    door: u32,
    /// The clamped wall size.
    size: i32,
    /// The stepping point.
    x: i32,
    /// The stepping point's y.
    y: i32,
    /// The side's rotation in degrees.
    rot: i32,
}

impl WallSide {
    /// Run side `index` along y or x, with or without its door (`:1194-1242`).
    fn run(&mut self, state: &mut GameState, index: usize, along_y: bool, door_open: bool) {
        let delta: i32 = if index == 1 || index == 2 { 1 } else { -1 };
        // The rotation is one of 270/180/90/0, exact in `f32`.
        #[allow(clippy::cast_precision_loss)]
        let face = self.rot as f32;
        let _ = state.request_create_object(BuildOrder {
            place: self.place,
            land_id: self.land_id,
            vnum: self.corner,
            x: self.x,
            y: self.y,
            rot: (0.0, 0.0, face),
            check: false,
        });
        self.step(along_y, 700 * delta);
        if door_open {
            self.draw(state, self.size);
            self.step(along_y, 700 * delta);
            let _ = state.request_create_object(BuildOrder {
                place: self.place,
                land_id: self.land_id,
                vnum: self.door,
                x: self.x,
                y: self.y,
                rot: (0.0, 0.0, face),
                check: false,
            });
            self.step(along_y, 1300 * delta);
            self.draw(state, self.size);
        } else {
            self.draw(state, self.size * 2 + 4);
        }
        self.step(along_y, 100 * delta);
        self.rot -= 90;
    }

    /// Step the point along y or x.
    fn step(&mut self, along_y: bool, by: i32) {
        if along_y {
            self.y += by;
        } else {
            self.x += by;
        }
    }

    /// Draw `length` walls from the point (`DrawWall`).
    fn draw(&mut self, state: &mut GameState, length: i32) {
        state.draw_wall(self, length);
    }
}

/// Whether `(x, y)` stands in the footprint rect, edges included (`FIsIn`).
fn in_rect(footprint: (i32, i32, i32, i32), (x, y): (i32, i32)) -> bool {
    footprint.0 <= x && x <= footprint.2 && footprint.1 <= y && y <= footprint.3
}

/// `str_to_number(DWORD)` (`common/utils.h:52`): the leading run as `strtoul`
/// base 10, saturated as `strtoul` saturates an `unsigned long` — which on the
/// 32-bit legacy target is the full `u32` range, past `i32::MAX`.
fn dword(word: &[u8]) -> u32 {
    let Some(start) = word.iter().position(|byte| !byte.is_ascii_whitespace()) else {
        return 0;
    };
    let rest = &word[start..];
    let (digits, negative) = match rest.first() {
        Some(b'-') => (&rest[1..], true),
        Some(b'+') => (&rest[1..], false),
        _ => (rest, false),
    };
    let mut value: u64 = 0;
    for byte in digits.iter().take_while(|byte| byte.is_ascii_digit()) {
        value = (value * 10 + u64::from(byte - b'0')).min(u64::from(u32::MAX) + 1);
    }
    let value = value.clamp(0, u64::from(u32::MAX));
    if negative {
        0u32.wrapping_sub(u32::try_from(value).unwrap_or(u32::MAX))
    } else {
        u32::try_from(value).unwrap_or(u32::MAX)
    }
}

/// `str_to_number(long)`: the leading run as `strtol`, else 0.
fn long_word(word: &[u8]) -> i32 {
    int_word(word).unwrap_or(0)
}

/// `atof`: the leading float, else 0.0.
fn float_word(word: &[u8]) -> f32 {
    let text = word.iter().take_while(|byte| !byte.is_ascii_whitespace());
    let text: Vec<u8> = text.copied().collect();
    std::str::from_utf8(&text)
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(0.0)
}

impl GameState {
    /// `CLand::FindObjectByGroup` (`G/building.cpp:401-412`): whether any of
    /// the land's standing objects carries `group`.
    fn land_has_group(&self, place: (u8, i32), land_id: u32, group: u32) -> bool {
        self.building_objects.get(&place).is_some_and(|objects| {
            objects.iter().any(|object| {
                object.land_id == land_id
                    && self
                        .object_protos
                        .get(object.vnum)
                        .is_some_and(|proto| proto.group_vnum == group)
            })
        })
    }

    /// `CLand::RequestCreateObject` (`G/building.cpp:500-576`): the bounds and
    /// occupancy gates, then the stand. Answers the stood object, or `None`
    /// past a miss: no proto, no region, the rotated footprint outside the
    /// land, or another object or body in the way (only when `order.check`).
    fn request_create_object(&mut self, order: BuildOrder) -> Option<BuildingObject> {
        let (place, land_id, vnum, x, y, rot, check) = (
            order.place,
            order.land_id,
            order.vnum,
            order.x,
            order.y,
            order.rot,
            order.check,
        );
        let proto = self.object_protos.get(vnum).copied()?;
        let region = self.regens.region_of(place.0, place.1)?;
        let land = self.guild_lands.get(land_id).copied()?;
        let (x, y) = (x + region.sx, y + region.sy);
        let (sx, sy) = (region.sx + land.x, region.sy + land.y);
        let (ex, ey) = (
            sx.saturating_add(land.width),
            sy.saturating_add(land.height),
        );
        let rad = rot.2 * 2.0 * std::f32::consts::PI / 360.0;
        let (sin, cos) = rad.sin_cos();
        // The casts are `RequestCreateObject`'s own (`:531-541`): the region
        // bounds read as `float`, the rotated sums as `(int)` truncation.
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        let corner = |dx: i32, dy: i32| {
            let (fx, fy) = (dx as f32, dy as f32);
            let (tx, ty) = (fx * cos + fy * sin, fx * -sin + fy * cos);
            (x.saturating_add(tx as i32), y.saturating_add(ty as i32))
        };
        let rotated = (
            corner(proto.regions[0], proto.regions[1]),
            corner(proto.regions[2], proto.regions[3]),
        );
        if rotated.0 .0 < sx || rotated.1 .0 > ex || rotated.0 .1 < sy || rotated.1 .1 > ey {
            return None;
        }
        let footprint = (
            x.saturating_add(proto.regions[0]),
            y.saturating_add(proto.regions[1]),
            x.saturating_add(proto.regions[2]),
            y.saturating_add(proto.regions[3]),
        );
        if check && self.build_blocked(place, footprint) {
            return None;
        }
        self.stand_build_object(BuildOrder {
            place,
            land_id,
            vnum,
            x,
            y,
            rot,
            check,
        })
    }

    /// The `checkAnother` occupancy (`:545-560`): an `ATTR_OBJECT` cell in the
    /// footprint, or a non-monster body or NPC standing in it.
    fn build_blocked(&self, place: (u8, i32), footprint: (i32, i32, i32, i32)) -> bool {
        if self.attr_in_rect(place.1, footprint, gamedata::server_attr::ATTR_OBJECT) {
            return true;
        }
        self.bodies
            .values()
            .any(|body| (body.channel, body.map) == place && in_rect(footprint, body.motion.dest))
            || self.npc_in_rect(place, footprint)
    }

    /// Whether a non-monster NPC stands in the rect on `map` (`FIsIn`,
    /// `:472-494`, minus the monster arm).
    fn npc_in_rect(&self, place: (u8, i32), footprint: (i32, i32, i32, i32)) -> bool {
        self.npcs.iter().any(|(npc_place, list)| {
            *npc_place == place
                && list.npcs.iter().any(|npc| {
                    npc.char_type != gamedata::mob_proto::CHAR_TYPE_MONSTER
                        && in_rect(footprint, (npc.x, npc.y))
                })
        })
    }

    /// Stand the object (`CManager::LoadObject` with `isBoot = false`,
    /// `:814-863`): a fresh id and VID, the entity, its NPC, and the member
    /// bonus, with the create half for the drain. The life is the proto's.
    fn stand_build_object(&mut self, order: BuildOrder) -> Option<BuildingObject> {
        let (place, land_id, vnum, x, y, rot) = (
            order.place,
            order.land_id,
            order.vnum,
            order.x,
            order.y,
            order.rot,
        );
        let life = self.object_protos.get(vnum).copied()?.life;
        let vid = self.take_npc_vid()?;
        self.next_object_id = self.next_object_id.saturating_add(1).max(1);
        let id = self.next_object_id;
        let kept = db::guild_land::StoredObject {
            id,
            land_id,
            vnum,
            map: place.1,
            x,
            y,
            rot,
            life,
        };
        let proto = self.object_protos.get(vnum).copied();
        let npc_vid = proto.and_then(|proto| self.regen_building_npc(place, &kept, &proto));
        let object = BuildingObject {
            id,
            land_id,
            vnum,
            map: place.1,
            x,
            y,
            rot,
            life,
            vid,
            npc_vid,
        };
        self.building_objects
            .entry(place)
            .or_default()
            .push(object.clone());
        self.building_places.insert(vid, place);
        self.apply_member_bonus(land_id, vnum, true);
        // `CObject::Show` (`G/building.cpp:150-187`): the entity enters the
        // sectree, and the insert fans to whoever sees it.
        self.show_building_object(place, &object);
        self.build_halves.push(GuildQuestStore::Create {
            id,
            land_id,
            vnum,
            map: place.1,
            x,
            y,
            rot: (rot.0.to_bits(), rot.1.to_bits(), rot.2.to_bits()),
            life,
        });
        Some(object)
    }

    /// `CObject::Show`'s enter (`G/building.cpp:150-187`): the object entity
    /// joins the sectree, and the insert fans to its viewers. The attribute
    /// region (`:179-186`) is not ported: the Rewrite walks by server-attr
    /// cells, which no object writes.
    fn show_building_object(&mut self, place: (u8, i32), object: &BuildingObject) {
        let mut effects = Vec::new();
        if let Some(index) = self.maps.get_mut(&place) {
            let _shown = index.show(
                super::view::EntityKey::Npc(object.vid),
                (object.x, object.y, 0),
                self.view_radius,
                &mut effects,
            );
        }
        self.deliver(&effects, None);
    }

    /// The sectree's half of `CLand::DeleteObject` (`:430-447`): the object
    /// entity leaves, and the delete fans to whoever saw it.
    fn hide_building_object(&mut self, place: (u8, i32), object: &BuildingObject) {
        let mut effects = Vec::new();
        if let Some(index) = self.maps.get_mut(&place) {
            index.remove(super::view::EntityKey::Npc(object.vid), &mut effects);
        }
        self.deliver(&effects, None);
        self.building_places.remove(&object.vid);
    }

    /// `CObject::ApplySpecialEffect` and `RemoveSpecialEffect` (`:192-255`):
    /// the supply buildings set the owning guild's member-count bonus, and
    /// knocking one down clears it.
    fn apply_member_bonus(&mut self, land_id: u32, vnum: u32, stood: bool) {
        let Some(slot) = MEMBER_BONUS_VNUMS.iter().position(|bonus| *bonus == vnum) else {
            return;
        };
        let Some(guild) = self.guilds.land_owner(land_id) else {
            return;
        };
        let bonus = if stood { MEMBER_BONUS[slot] } else { 0 };
        self.guilds.set_member_bonus(guild, bonus);
    }
}

impl GameState {
    /// The `build d` arm (`:3619-3634`): one word names the object VID to knock
    /// down. No word tells `Invalid syntax`; an unknown VID is silent.
    fn build_delete(
        &mut self,
        vid: Vid,
        mover: Mover,
        place: (u8, i32),
        land_id: u32,
        line: &[u8],
    ) {
        let (word, _) = one_argument(line);
        if word.is_empty() {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Invalid syntax", &[]);
            return;
        }
        let target = dword(&word);
        let Some(object) = self
            .building_objects
            .get(&place)
            .and_then(|objects| {
                objects
                    .iter()
                    .find(|object| object.land_id == land_id && object.vid == target)
            })
            .cloned()
        else {
            return;
        };
        self.knock_build_object(place, &object);
    }

    /// `CLand::DeleteObject` (`:430-447`): the entity and its NPC go, the
    /// member bonus clears, and the delete half lands for the drain.
    fn knock_build_object(&mut self, place: (u8, i32), object: &BuildingObject) {
        if let Some(npc) = object.npc_vid {
            self.destroy_npc(npc);
        }
        if let Some(objects) = self.building_objects.get_mut(&place) {
            objects.retain(|kept| kept.id != object.id);
        }
        self.apply_member_bonus(object.land_id, object.vnum, false);
        self.hide_building_object(place, object);
        self.build_halves
            .push(GuildQuestStore::Delete { id: object.id });
    }

    /// `do_clear_land`'s wipe (`CLand::ClearLand`, `:1021-1037`): every object
    /// on the land goes in id order, the owner drops, and the claim stone
    /// stands at the land center.
    fn clear_build_land(&mut self, place: (u8, i32), land_id: u32) {
        let mut objects = self
            .building_objects
            .get(&place)
            .map(|objects| {
                objects
                    .iter()
                    .filter(|object| object.land_id == land_id)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        objects.sort_by_key(|object| object.id);
        for object in &objects {
            if let Some(npc) = object.npc_vid {
                self.destroy_npc(npc);
            }
            self.hide_building_object(place, object);
            self.apply_member_bonus(land_id, object.vnum, false);
        }
        if let Some(objects) = self.building_objects.get_mut(&place) {
            objects.retain(|object| object.land_id != land_id);
        }
        self.guilds.drop_land_owner(land_id);
        self.build_halves.push(GuildQuestStore::Clear { land_id });
        // `CManager::UpdateLand`'s fan (`G/building.cpp:740-752`): in legacy the
        // DB answers the `RequestUpdate` the clear's `SetOwner(0)` sent
        // (`G/building.cpp:1031`) and the game fans the cleared land to every
        // client on its map. The Rewrite has no DB peer, so the clear fans at
        // once, once the owner is dropped.
        self.fan_land_update(land_id);
        if let (Some(land), Some(region)) = (
            self.guild_lands.get(land_id).copied(),
            self.regens.region_of(place.0, place.1),
        ) {
            let at = (
                region.sx + land.x + land.width / 2,
                region.sy + land.y + land.height / 2,
            );
            let _stone = self.spawn_event_mob(place, CLEAR_STONE_VNUM, at, 0, false);
        }
    }
}

impl GameState {
    /// The `build w` arm (`:3639-3669`): a GM's direction word (`s`, `n`, `e`,
    /// `w`) stands the four single walls facing it. A player is silent; an
    /// unknown direction tells `guild.wall.build unknown_direction[%s]`.
    fn build_walls(&mut self, vid: Vid, mover: Mover, place: (u8, i32), land_id: u32, line: &[u8]) {
        if self.build_level(vid) == GM_PLAYER {
            return;
        }
        let (word, _) = one_argument(line);
        let facing = match word.first().copied() {
            Some(b's') => 0,
            Some(b'n') => 180,
            Some(b'e') => 90,
            Some(b'w') => 270,
            _ => {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"guild.wall.build unknown_direction[%s]",
                    &[Arg::Text(&word)],
                );
                return;
            }
        };
        self.request_create_wall(place, land_id, facing);
        if self.test_server {
            self.request_create_flags(place, land_id);
        }
    }

    /// `CLand::RequestCreateWall` (`G/building.cpp:1080-1139`): the four single
    /// walls around the land center, facing `facing` (0/180/90/270). The
    /// occupancy check is off (`WALL_ANOTHER_CHECKING_ENABLE = false`).
    fn request_create_wall(&mut self, place: (u8, i32), land_id: u32, facing: i32) {
        let Some(land) = self.guild_lands.get(land_id).copied() else {
            return;
        };
        let (wall_x, wall_y) = (land.x + land.width / 2, land.y + land.height / 2);
        let (half_w, half_h) = (1000, 1362);
        // The faces are legacy's `==` arms (`:1090-1129`): 0 and 180 stand the
        // north-south pair, 90 and 270 the east-west pair, and only 0 and 180
        // face their own degree; 90 and 270 both face 90.
        let spots = match facing {
            0 => (0.0, wall_x, wall_y + half_h, wall_x, wall_y - half_h, false),
            180 => (
                180.0,
                wall_x,
                wall_y - half_h,
                wall_x,
                wall_y + half_h,
                false,
            ),
            90 => (90.0, wall_x + half_h, wall_y, wall_x - half_h, wall_y, true),
            _ => (90.0, wall_x - half_h, wall_y, wall_x + half_h, wall_y, true),
        };
        let (face, door_x, door_y, back_x, back_y, sideways) = spots;
        let (left, right) = if sideways {
            ((wall_x, wall_y - half_w), (wall_x, wall_y + half_w))
        } else {
            ((wall_x - half_w, wall_y), (wall_x + half_w, wall_y))
        };
        #[allow(clippy::cast_precision_loss)]
        for (vnum, x, y) in [
            (WALL_VNUMS[0], door_x, door_y),
            (WALL_VNUMS[1], back_x, back_y),
            (WALL_VNUMS[2], left.0, left.1),
            (WALL_VNUMS[3], right.0, right.1),
        ] {
            // The rotations ride the door's point (`door_x`, `door_y`) as the
            // x and y rotations; the z rotation faces the wall.
            let _ = self.request_create_object(BuildOrder {
                place,
                land_id,
                vnum,
                x,
                y,
                rot: (door_x as f32, door_y as f32, face),
                check: false,
            });
        }
    }

    /// The four test-server flags (`:1131-1137`): the land corners at 0, 90,
    /// 180 and 270 degrees.
    fn request_create_flags(&mut self, place: (u8, i32), land_id: u32) {
        let Some(land) = self.guild_lands.get(land_id).copied() else {
            return;
        };
        for (x, y, rot) in [
            (land.x + 50, land.y + 50, 0.0),
            (land.x + land.width - 50, land.y + 50, 90.0),
            (land.x + land.width - 50, land.y + land.height - 50, 180.0),
            (land.x + 50, land.y + land.height - 50, 270.0),
        ] {
            let _ = self.request_create_object(BuildOrder {
                place,
                land_id,
                vnum: FLAG_VNUM,
                x,
                y,
                rot: (0.0, 0.0, rot),
                check: false,
            });
        }
    }

    /// The `build e` arm (`:3671-3676`): a GM knocks the four single walls
    /// down (plus the flags in test-server mode). A player is silent.
    fn build_unwall(&mut self, vid: Vid, place: (u8, i32), land_id: u32) {
        if self.build_level(vid) == GM_PLAYER {
            return;
        }
        let targets: Vec<BuildingObject> = self
            .building_objects
            .get(&place)
            .map(|objects| {
                objects
                    .iter()
                    .filter(|object| {
                        object.land_id == land_id
                            && (WALL_VNUMS.contains(&object.vnum)
                                || (self.test_server && object.vnum == FLAG_VNUM))
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        for object in &targets {
            self.knock_build_object(place, object);
        }
    }
}

impl GameState {
    /// The `build W` arm (`G/cmd_gm.cpp:3678-3711`): a GM's six words (set id,
    /// wall size, four door flags) stand the wall blocks. A wrong set id only
    /// logs, and a player is silent. The set id parses as `str_to_number(int)`,
    /// the size as `str_to_number(char)` — the low byte, wrapping — and the
    /// doors as `str_to_number(bool)`: nonzero is open.
    fn build_wall_blocks(&mut self, vid: Vid, place: (u8, i32), land_id: u32, line: &[u8]) {
        if self.build_level(vid) == GM_PLAYER {
            return;
        }
        let (set_word, rest) = one_argument(line);
        let (size_word, rest) = one_argument(rest);
        let (east_word, rest) = one_argument(rest);
        let (west_word, rest) = one_argument(rest);
        let (south_word, rest) = one_argument(rest);
        let (north_word, _) = one_argument(rest);
        let set = int_word(&set_word).unwrap_or(0);
        if !WALL_SET_IDS.contains(&set) {
            return;
        }
        // The truncation is legacy's `(char)strtol` (`common/utils.h:16`).
        #[allow(clippy::cast_possible_truncation)]
        let size = i32::from(int_word(&size_word).unwrap_or(0) as i8);
        let doors = [
            int_word(&west_word).unwrap_or(0) != 0,
            int_word(&north_word).unwrap_or(0) != 0,
            int_word(&east_word).unwrap_or(0) != 0,
            int_word(&south_word).unwrap_or(0) != 0,
        ];
        self.request_create_wall_blocks(
            place,
            land_id,
            u32::try_from(set).unwrap_or(0),
            size,
            doors,
        );
    }

    /// `CLand::RequestCreateWallBlocks` (`G/building.cpp:1172-1244`): the four
    /// corner-to-side runs, each a corner, `wallSize` walls, an optional door,
    /// and `wallSize` walls — or `wallSize * 2 + 4` walls with no door.
    /// `wallSize` clamps to 0..3. The occupancy check is off.
    fn request_create_wall_blocks(
        &mut self,
        place: (u8, i32),
        land_id: u32,
        set: u32,
        size: i32,
        doors: [bool; 4],
    ) {
        let Some(land) = self.guild_lands.get(land_id).copied() else {
            return;
        };
        let size = size.clamp(0, 3);
        let (corner, wall, door) = (set - 4, set - 3, set - 1);
        let (x, y) = (
            land.x + land.width / 2 - (1300 + size * 500),
            land.y + land.height / 2 + (1300 + size * 500),
        );
        let mut run = WallSide {
            place,
            land_id,
            corner,
            wall,
            door,
            size,
            x,
            y,
            rot: 270,
        };
        for (index, along_y) in [true, false, true, false].iter().enumerate() {
            run.run(self, index, *along_y, doors[index]);
        }
    }

    /// `CLand::DrawWall` (`G/building.cpp:1040-1077`): `length` walls stepping
    /// 500 a wall in the direction the side's rotation names, stepping its point on.
    fn draw_wall(&mut self, side: &mut WallSide, length: i32) {
        let face = ((side.rot % 360) / 90) * 90;
        let (dx, dy) = match face {
            90 => (0, 500),
            180 => (500, 0),
            270 => (0, -500),
            _ => (-500, 0),
        };
        // The face is one of 0/90/180/270, exact in `f32`.
        #[allow(clippy::cast_precision_loss)]
        let face_f = face as f32;
        for _ in 0..length.max(0) {
            let _ = self.request_create_object(BuildOrder {
                place: side.place,
                land_id: side.land_id,
                vnum: side.wall,
                x: side.x,
                y: side.y,
                rot: (0.0, 0.0, face_f),
                check: false,
            });
            side.x += dx;
            side.y += dy;
        }
    }

    /// The `build E` arm (`G/cmd_gm.cpp:3713-3723`): a GM's set id erases its
    /// wall blocks (corner, wall and door vnums). A player is silent.
    fn build_unwall_blocks(&mut self, vid: Vid, place: (u8, i32), land_id: u32, line: &[u8]) {
        if self.build_level(vid) == GM_PLAYER {
            return;
        }
        let (word, _) = one_argument(line);
        let set = dword(&word);
        let targets: Vec<BuildingObject> = self
            .building_objects
            .get(&place)
            .map(|objects| {
                objects
                    .iter()
                    .filter(|object| {
                        object.land_id == land_id
                            && [
                                set.wrapping_sub(4),
                                set.wrapping_sub(3),
                                set.wrapping_sub(1),
                            ]
                            .contains(&object.vnum)
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        for object in &targets {
            self.knock_build_object(place, object);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::game_state::fixtures::{a_world, enter_on, points_at, region, TestClock};
    use crate::game_state::guild::GuildQuestStore;
    use protocol::gc_guild::GUILD_SKILL_COUNT;
    use world::npc::{NpcVids, Respawner, FIRST_NPC_VID};

    /// The land's place: channel 1, map 1.
    const PLACE: (u8, i32) = (1, 1);

    /// A world with the land, the protos and the region: land 1 on map 1 at
    /// (100, 100) 3000 square, owned by guild 7, whose master is PID 1.
    /// One object proto with the 200-square footprint, priced and gated.
    fn proto(
        vnum: u32,
        price: u32,
        group: u32,
        dependent: u32,
    ) -> gamedata::records::ObjectProtoRecord {
        gamedata::records::ObjectProtoRecord {
            vnum,
            price,
            regions: [-100, -100, 100, 100],
            group_vnum: group,
            dependent_group_vnum: dependent,
            ..Default::default()
        }
    }

    /// The fixture protos: the gated 14043/14044/14045, the single walls, and
    /// set 14105's corner, wall and door.
    fn protos() -> gamedata::object_proto::ObjectProtos {
        gamedata::object_proto::ObjectProtos::from_records([
            proto(14043, 1000, 0, 0),
            proto(14044, 1000, 9, 0),
            proto(14045, 1000, 0, 9),
            proto(14201, 0, 0, 0),
            proto(14202, 0, 0, 0),
            proto(14203, 0, 0, 0),
            proto(14204, 0, 0, 0),
            proto(14101, 0, 0, 0),
            proto(14102, 0, 0, 0),
            proto(14104, 0, 0, 0),
        ])
    }

    /// Land 1 on map 1 at (100, 100), `width` square.
    fn land(width: i32) -> gamedata::land::Lands {
        gamedata::land::Lands::from_records([gamedata::records::LandRecord {
            id: 1,
            map_index: 1,
            x: 100,
            y: 100,
            width,
            height: width,
            guild_id: 0,
            guild_level_limit: 0,
            price: 1,
        }])
    }

    /// Guild 7, mastered by PID 1.
    fn guild() -> db::guilds::StoredGuilds {
        db::guilds::StoredGuilds {
            guilds: vec![db::guilds::StoredGuild {
                id: 7,
                name: "G".to_string(),
                master_pid: 1,
                sp: 0,
                level: 5,
                exp: 0,
                gold: 0,
                ladder_point: 0,
                win: 0,
                draw: 0,
                loss: 0,
                skill_point: 0,
                skills: vec![0; GUILD_SKILL_COUNT],
            }],
            grades: Vec::new(),
            members: vec![db::guilds::StoredMember {
                guild_id: 7,
                pid: 1,
                grade: 1,
                is_general: false,
                job: 0,
                level: 10,
                offer: 0,
                name: "P1".to_string(),
            }],
        }
    }

    fn built(clock: &TestClock) -> GameState {
        let mut world = a_world(clock)
            .with_lands(Arc::new(land(3000)))
            .with_object_protos(Arc::new(protos()));
        let mut region = region();
        region.index = 1;
        world.schedule_map(PLACE, &region);
        world.host_map(
            1,
            1,
            gamedata::server_attr::SectreeGrid {
                x: 0,
                y: 0,
                columns: 10,
                rows: 10,
            },
        );
        world.install_land_owners(&vec![(1, 7)]);
        world.install_respawner(
            Respawner::new(
                Arc::new(gamedata::mob_proto::MobProtos::from_rows(Vec::new())),
                Arc::new(gamedata::mob_group::MobGroups::default()),
                Arc::new(
                    gamedata::mob_locale_names::MobLocaleNames::parse(b"VNUM\tNAME\n").unwrap(),
                ),
                NpcVids::starting_at(FIRST_NPC_VID + 100),
            ),
            Arc::new(world::cells::HostedCells::default()),
        );
        world.install_guilds(&guild(), &[], &[]);
        world
    }

    /// A line mover (the caller is alive, at peace, unaligned).
    const fn mover() -> Mover {
        Mover {
            recently_fought: false,
            empire: 1,
            language: 1,
            pk_mode: crate::loading_phase::PK_MODE_PEACE,
            affect_flags: [0; 2],
            dead: false,
            killer_mode: false,
            alignment: 0,
            view_guild: (0, 0),
            premium: (0, 0),
        }
    }

    /// The master entered on the land at (2000, 2000), with 10k gold. The vid
    /// is the pid, so the guild's master row makes the caller the master; the
    /// caller's own body would otherwise block the footprint it builds on.
    fn master(
        world: &mut GameState,
    ) -> (Vid, Mover, tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) {
        let inbox = enter_on(world, PLACE, 1, (2000, 2000), 10_000);
        let mut points = points_at(10, 10_000, 0);
        if world.test_server {
            points.set_gm_level(crate::command::GM_IMPLEMENTOR);
        }
        world
            .characters
            .find_by_vid_mut(Vid::new(1))
            .expect("entered")
            .set_points(Some(points));
        world
            .characters
            .find_by_vid_mut(Vid::new(1))
            .expect("entered")
            .set_gold(10_000);
        (Vid::new(1), mover(), inbox)
    }

    /// The text of a `GC_CHAT` line.
    fn text_of(line: &[u8]) -> Vec<u8> {
        let size = usize::from(u16::from_le_bytes([line[1], line[2]]));
        assert_eq!(size, line.len());
        assert_eq!(line[0], 4, "HEADER_GC_CHAT");
        line[10..].to_vec()
    }

    #[test]
    fn the_create_stands_and_spends() {
        let clock = TestClock::default();
        let mut world = built(&clock);
        // A true player-master: out of test-server mode the GM level reads 0, so the
        // master gate and the spend gate both run on the player arms.
        world.test_server = false;
        let (vid, mover, mut inbox) = master(&mut world);
        assert_eq!(world.build_level(vid), GM_PLAYER);
        while inbox.try_recv().is_ok() {}
        let rows = world
            .run_build_line(vid, b"c 14043 200 200 0 0 0", mover)
            .expect("the spend answers");
        assert_eq!(rows.gold, -1000);
        assert_eq!(rows.buildings.len(), 1);
        assert!(matches!(
            rows.buildings[0],
            GuildQuestStore::Create { vnum: 14043, .. }
        ));
        let objects = world
            .building_objects
            .get(&(1, 1))
            .expect("the object stands");
        assert_eq!(objects.len(), 1);
        assert_eq!((objects[0].x, objects[0].y), (200, 200));
        let gold = world.characters.find_by_vid(vid).expect("entered").gold();
        assert_eq!(gold, 9_000);
        while inbox.try_recv().is_ok() {}
    }
    /// A caller who is neither the master nor a GM is silent, and nothing
    /// stands: the stranger's line answers nothing and tells nothing.
    #[test]
    fn a_stranger_builds_nothing_and_hears_nothing() {
        let clock = TestClock::default();
        let mut world = built(&clock);
        let (_vid, mover, mut inbox) = master(&mut world);
        while inbox.try_recv().is_ok() {}
        let inbox2 = enter_on(&mut world, PLACE, 2, (2000, 2000), 10_000);
        drop(inbox2);
        while inbox.try_recv().is_ok() {}
        let rows = world.run_build_line(Vid::new(2), b"c 14043 200 200 0 0 0", mover);
        assert!(rows.is_none(), "a stranger is silent");
        assert!(!world.building_objects.contains_key(&PLACE));
        assert!(inbox.try_recv().is_err(), "no tell to anyone");
    }

    /// The refusals speak: an unknown proto, a group conflict, a missing
    /// dependency, and fewer than six words each tell their own line.
    #[test]
    fn the_refusals_speak_their_own_lines() {
        let clock = TestClock::default();
        let mut world = built(&clock);
        let (vid, mover, mut inbox) = master(&mut world);
        while inbox.try_recv().is_ok() {}
        assert!(world
            .run_build_line(vid, b"c 99999 200 200 0 0 0", mover)
            .is_none());
        let unknown = text_of(&inbox.try_recv().expect("the unknown-proto tell"));
        assert_eq!(unknown, UNKNOWN_PROTO);
        assert!(world.run_build_line(vid, b"c 14043", mover).is_none());
        assert_eq!(
            text_of(&inbox.try_recv().expect("the syntax tell")),
            b"Invalid syntax"
        );
        // The group piece stands, so the 14044 gate conflicts with it.
        let group = world
            .run_build_line(vid, b"c 14044 500 500 0 0 0", mover)
            .expect("the group piece stands");
        assert_eq!(group.buildings.len(), 1);
        while inbox.try_recv().is_ok() {}
        assert!(world
            .run_build_line(vid, b"c 14044 700 700 0 0 0", mover)
            .is_none());
        let conflict = text_of(&inbox.try_recv().expect("the conflict tell"));
        assert!(
            conflict.windows(13).any(|w| w == b"cannot be bui"),
            "{conflict:?}"
        );
        // The dependency is missing: 14045 refuses until a group-9 piece stands.
        // (14044 carries group 9, so after its knock-down 14045 refuses again.)
        let objects = world
            .building_objects
            .get(&PLACE)
            .expect("the piece stands");
        let target = objects[0].vid;
        world.run_build_line(vid, format!("d {target}").as_bytes(), mover);
        while inbox.try_recv().is_ok() {}
        assert!(world
            .run_build_line(vid, b"c 14045 500 500 0 0 0", mover)
            .is_none());
        let missing = text_of(&inbox.try_recv().expect("the missing tell"));
        assert!(
            missing.windows(15).any(|w| w == b"required for co"),
            "{missing:?}"
        );
    }

    /// The delete knocks the object down and halves the delete: the object
    /// leaves the land and the drain answers the delete half.
    #[test]
    fn the_delete_knocks_and_halves() {
        let clock = TestClock::default();
        let mut world = built(&clock);
        let (vid, mover, mut inbox) = master(&mut world);
        while inbox.try_recv().is_ok() {}
        world
            .run_build_line(vid, b"c 14043 200 200 0 0 0", mover)
            .expect("the piece stands");
        let objects = world
            .building_objects
            .get(&PLACE)
            .expect("the piece stands");
        let (id, target) = (objects[0].id, objects[0].vid);
        while inbox.try_recv().is_ok() {}
        let rows = world
            .run_build_line(vid, format!("d {target}").as_bytes(), mover)
            .expect("the delete halves");
        assert_eq!(rows.buildings, vec![GuildQuestStore::Delete { id }]);
        assert!(world
            .building_objects
            .get(&PLACE)
            .expect("the land lists")
            .is_empty());
        let gone = inbox.try_recv().expect("the knock's DEL");
        assert_eq!(gone[0], 2, "HEADER_GC_CHARACTER_DEL");
        assert_eq!(
            u32::from_le_bytes([gone[1], gone[2], gone[3], gone[4]]),
            target,
            "the entity goes"
        );
        // An unknown VID and a bare word are quiet past the gates.
        assert!(world.run_build_line(vid, b"d 1", mover).is_none());
        assert!(world.run_build_line(vid, b"d", mover).is_none());
        assert_eq!(
            text_of(&inbox.try_recv().expect("the syntax tell")),
            b"Invalid syntax"
        );
    }

    /// The walls stand four around the center facing the word (`s` faces 0):
    /// two on the north-south run and two on the east-west run.
    #[test]
    fn the_walls_stand_four_around_the_center() {
        let clock = TestClock::default();
        let mut world = built(&clock);
        let (vid, mover, mut inbox) = master(&mut world);
        while inbox.try_recv().is_ok() {}
        world.run_build_line(vid, b"w s", mover);
        let objects = world.building_objects.get(&PLACE).expect("the walls stand");
        assert_eq!(objects.len(), 4);
        for object in objects {
            assert!(WALL_VNUMS.contains(&object.vnum), "{}", object.vnum);
        }
        // The center is (100 + 3000/2, 100 + 3000/2) = (1600, 1600).
        let mut at: Vec<(i32, i32)> = objects.iter().map(|object| (object.x, object.y)).collect();
        at.sort_unstable();
        assert_eq!(
            at,
            vec![(600, 1600), (1600, 238), (1600, 2962), (2600, 1600)]
        );
        let mut adds = 0;
        while let Ok(add) = inbox.try_recv() {
            assert_eq!(add[0], 1, "HEADER_GC_CHARACTER_ADD");
            adds += 1;
        }
        assert_eq!(adds, 4, "the four walls show");
        // An unknown direction tells its line.
        world.run_build_line(vid, b"w q", mover);
        let unknown = text_of(&inbox.try_recv().expect("the direction tell"));
        assert!(
            unknown.windows(17).any(|w| w == b"unknown_direction"),
            "{unknown:?}"
        );
    }

    /// The stood object's ADD fans through its sectree, and the knock's DEL
    /// after it: the master in view sees both, keyed by the object entity's
    /// own VID (no NPC stands for a wall).
    #[test]
    fn the_stand_shows_and_the_knock_hides() {
        let clock = TestClock::default();
        let mut world = built(&clock);
        let (vid, mover, mut inbox) = master(&mut world);
        while inbox.try_recv().is_ok() {}
        world
            .run_build_line(vid, b"c 14043 200 200 0 0 0", mover)
            .expect("the piece stands");
        let object = world.building_objects[&PLACE]
            .iter()
            .find(|object| object.vnum == 14_043)
            .expect("the piece stands")
            .clone();
        assert_eq!(
            world.building_places.get(&object.vid),
            Some(&PLACE),
            "the entity keys its place"
        );
        let stood = inbox.try_recv().expect("the stand's ADD");
        assert_eq!(stood[0], 1, "HEADER_GC_CHARACTER_ADD");
        assert_eq!(stood[21], 5, "CHAR_TYPE_BUILDING");
        assert_eq!(
            u16::from_le_bytes([stood[22], stood[23]]),
            14_043,
            "as 14043"
        );
        assert_eq!(
            u32::from_le_bytes([stood[1], stood[2], stood[3], stood[4]]),
            object.vid,
            "the entity's own VID"
        );
        // The create's spend records follow the ADD; the knock's DEL is next.
        while inbox.try_recv().is_ok() {}
        world
            .run_build_line(vid, format!("d {}", object.vid).as_bytes(), mover)
            .expect("the knock halves");
        let gone = inbox.try_recv().expect("the knock's DEL");
        assert_eq!(gone[0], 2, "HEADER_GC_CHARACTER_DEL");
        assert_eq!(
            u32::from_le_bytes([gone[1], gone[2], gone[3], gone[4]]),
            object.vid,
            "the entity goes"
        );
        assert!(
            !world.building_places.contains_key(&object.vid),
            "the place unkeys"
        );
        assert!(inbox.try_recv().is_err(), "the ADD and the DEL");
    }

    /// `clear_land` wipes the land: the object goes, the owner drops, the
    /// clear halves, and the claim stone stands at the land center.
    #[test]
    fn the_clear_wipes_the_land_and_stones_the_center() {
        let clock = TestClock::default();
        let mut world = built(&clock);
        let (vid, mover, mut inbox) = master(&mut world);
        while inbox.try_recv().is_ok() {}
        world
            .run_build_line(vid, b"c 14043 200 200 0 0 0", mover)
            .expect("the piece stands");
        while inbox.try_recv().is_ok() {}
        let rows = world
            .run_clear_land_line(vid, mover)
            .expect("the clear halves");
        assert_eq!(rows.buildings, vec![GuildQuestStore::Clear { land_id: 1 }]);
        assert!(world
            .building_objects
            .get(&PLACE)
            .expect("the land lists")
            .is_empty());
        assert_eq!(world.guilds.land_owner(1), None);
        let clear = text_of(&inbox.try_recv().expect("the cleared tell"));
        assert!(clear.windows(7).any(|w| w == b"Cleared"), "{clear:?}");
        let gone = inbox.try_recv().expect("the wiped object's DEL");
        assert_eq!(gone[0], 2, "HEADER_GC_CHARACTER_DEL");
        // The clear fans the land's update behind the notice: the one-element
        // list with no owner (the guildless names no NAME).
        let fan = inbox.try_recv().expect("the cleared land's fan");
        assert_eq!(fan[0], 130, "HEADER_GC_LAND_LIST");
        assert_eq!(fan.len(), 3 + 24);
        assert_eq!(&fan[3..7], &1u32.to_le_bytes(), "land 1");
        assert_eq!(&fan[23..27], &0u32.to_le_bytes(), "no owner");
        // No claim stone stands: the fixture's mob table is empty, so the
        // center spawn finds no proto.
        assert!(inbox.try_recv().is_err(), "the tell, the DEL and the fan");
    }
    /// The wall-block runs span the center ±2000 at size 1, past a 3000-square
    /// land's edges, so the block tests widen land 1 to 10000 square: every
    /// piece lands inside and the counts pin the runs, not the clipping.
    fn widened(clock: &TestClock) -> GameState {
        built(clock).with_lands(Arc::new(land(10000)))
    }

    /// `build W` stands the four sides' blocks and `build E` erases them: set
    /// 14105 at size 1 with no doors is four corners and 24 walls, and the set
    /// id erases exactly those. A wrong set id stands nothing.
    #[test]
    fn the_wall_blocks_stand_and_erase_by_set() {
        let clock = TestClock::default();
        let mut world = widened(&clock);
        let (vid, mover, mut inbox) = master(&mut world);
        while inbox.try_recv().is_ok() {}
        world.run_build_line(vid, b"W 1 1 0 0 0 0", mover);
        assert!(!world.building_objects.contains_key(&PLACE));
        world.run_build_line(vid, b"W 14105 1 0 0 0 0", mover);
        let objects = world
            .building_objects
            .get(&PLACE)
            .expect("the blocks stand");
        assert_eq!(objects.len(), 28);
        assert_eq!(
            objects.iter().filter(|object| object.vnum == 14101).count(),
            4
        );
        assert_eq!(
            objects.iter().filter(|object| object.vnum == 14102).count(),
            24
        );
        assert!(objects.iter().all(|object| object.land_id == 1));
        let mut adds = 0;
        while let Ok(add) = inbox.try_recv() {
            assert_eq!(add[0], 1, "HEADER_GC_CHARACTER_ADD");
            adds += 1;
        }
        assert_eq!(adds, 28, "the blocks show");
        let rows = world
            .run_build_line(vid, b"E 14105", mover)
            .expect("the erase halves");
        assert_eq!(rows.buildings.len(), 28);
        assert!(world
            .building_objects
            .get(&PLACE)
            .expect("the land lists")
            .is_empty());
    }

    /// The wall size parses as `char`: 1000 wraps to -24 and clamps to 0, so
    /// each side is a corner and four walls.
    #[test]
    fn the_wall_size_wraps_as_char() {
        let clock = TestClock::default();
        let mut world = widened(&clock);
        let (vid, mover, mut inbox) = master(&mut world);
        while inbox.try_recv().is_ok() {}
        world.run_build_line(vid, b"W 14105 1000 0 0 0 0", mover);
        let objects = world
            .building_objects
            .get(&PLACE)
            .expect("the blocks stand");
        assert_eq!(objects.len(), 20);
    }
    /// Open doors split the sides: size 1 with all four doors is four corners,
    /// eight walls and four doors.
    #[test]
    fn the_open_doors_split_the_sides() {
        let clock = TestClock::default();
        let mut world = widened(&clock);
        let (vid, mover, mut inbox) = master(&mut world);
        while inbox.try_recv().is_ok() {}
        world.run_build_line(vid, b"W 14105 1 1 1 1 1", mover);
        let objects = world
            .building_objects
            .get(&PLACE)
            .expect("the blocks stand");
        assert_eq!(objects.len(), 16);
        assert_eq!(
            objects.iter().filter(|object| object.vnum == 14101).count(),
            4
        );
        assert_eq!(
            objects.iter().filter(|object| object.vnum == 14102).count(),
            8
        );
        assert_eq!(
            objects.iter().filter(|object| object.vnum == 14104).count(),
            4
        );
    }
    /// The door words name the sides west, north, east, south: only the west
    /// word open puts the one door on side 0's run (`x0`).
    #[test]
    fn the_west_word_opens_side_zero() {
        let clock = TestClock::default();
        let mut world = widened(&clock);
        let (vid, mover, mut inbox) = master(&mut world);
        while inbox.try_recv().is_ok() {}
        world.run_build_line(vid, b"W 14105 1 0 1 0 0", mover);
        let objects = world
            .building_objects
            .get(&PLACE)
            .expect("the blocks stand");
        let doors: Vec<_> = objects
            .iter()
            .filter(|object| object.vnum == 14104)
            .collect();
        assert_eq!(doors.len(), 1);
        // `x0` on the widened land: 100 + 5000 - (1300 + 500).
        assert_eq!((doors[0].x, doors[0].y), (3300, 6900 - 700 - 500 - 700));
    }
}
