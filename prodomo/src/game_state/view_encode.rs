//! The records a view change writes: `EncodeInsertPacket` and `EncodeRemovePacket` of a player
//! or an NPC (`G/char.cpp:1060-1275`) or of a ground item (`G/item.cpp:163-218`), sent to the
//! client of the entity whose view changed, and `PacketAround`, which sends one record to an
//! entity's view (`G/entity.cpp:88-105`).
//!
//! Each effect is encoded when it is delivered, so an insert reads the live motion. An entity
//! with no client, which is every NPC and every ground item, is sent nothing
//! (`G/char.cpp:1063-1066`, `G/item.cpp:167-168`).
//!
//! Everything here runs on the game thread only (ADR-0002).

use common::cfloat::f32_to_i32;
use common::vid::Vid;
use protocol::gc_actors::GcCharacterMove;
use protocol::gc_item_window::GcItemGroundAdd;
use protocol::gc_position::GcWalkMode;
use protocol::gc_vid::{GcHeaderAndDword, HEADER_GC_CHARACTER_DEL, HEADER_GC_ITEM_GROUND_DEL};
use world::character::{Character, Points};

use super::motion::{is_walking, Body};
use super::view::{Effect, EntityKey, Spot};
use super::GameState;
use crate::game_loop_messages::RelayScope;
use crate::loading_phase::{
    character_add, character_additional, npc_add, npc_additional, InsertAt,
};
use crate::movement::FUNC_MOVE;

/// `WALKMODE_RUN` (`G/packet.h:2377-2381`). `m_bNowWalking` is never set (V3), so every walk-mode
/// record the view sends says run.
pub(super) const WALKMODE_RUN: u8 = 0;

/// The records `to` receives for the reply buffer `reply`, which holds the records of the one
/// entrant being shown, when there is one.
type Reply<'a> = Option<(u32, &'a mut Vec<Vec<u8>>)>;

/// `of->EncodeInsertPacket(to)` for a player `of`: the records for `to`'s client, and the
/// walk-mode record of `to` that `of`'s own client gets when `to` walks.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Encoded {
    pub(super) to_records: Vec<Vec<u8>>,
    pub(super) reverse: Option<Vec<u8>>,
}

/// `[6f, vid, mode]`, a `GC_WALK_MODE`.
pub(super) fn walk_mode(vid: u32, mode: u8) -> Vec<u8> {
    GcWalkMode::new(vid, mode).encode()
}

/// `[02, vid]`, `EncodeRemovePacket` (`G/char.cpp:1256-1275`).
pub(super) fn remove_record(vid: u32) -> Vec<u8> {
    framed(|out| GcHeaderAndDword::new(HEADER_GC_CHARACTER_DEL, vid).encode_into(out))
}

/// `[1a, x, y, z, vid, vnum]`, a ground item's `EncodeInsertPacket` (`G/item.cpp:163-202`),
/// at the item's spot. The ownership record after it waits for `sys.item.ownership`.
pub(super) fn ground_add(vid: u32, vnum: u32, spot: Spot) -> Vec<u8> {
    GcItemGroundAdd {
        x: spot.x,
        y: spot.y,
        z: spot.z,
        vid,
        vnum,
    }
    .encode()
}

/// `[1b, vid]`, a ground item's `EncodeRemovePacket` (`G/item.cpp:204-218`).
pub(super) fn ground_del(vid: u32) -> Vec<u8> {
    framed(|out| GcHeaderAndDword::new(HEADER_GC_ITEM_GROUND_DEL, vid).encode_into(out))
}

/// `of->EncodeRemovePacket(to)` for a receiving player.
fn remove_of(of: EntityKey) -> Vec<u8> {
    match of {
        EntityKey::Character(vid) | EntityKey::Npc(vid) => remove_record(vid),
        EntityKey::Ground(vid) => ground_del(vid),
    }
}

/// The bytes one `encode_into` writes.
fn framed(encode: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut record = Vec::new();
    encode(&mut record);
    record
}

/// `EncodeInsertPacket` of the player `vid` (`G/char.cpp:1060-1254`). `recipient` is the
/// receiving player's VID and whether it walks; `now` is `get_dword_time()`.
///
/// A body whose destination is not its spot reports the destination once the move's time is
/// up, and is sent with its move while time is left. `iDur` is the `DWORD` arithmetic read as
/// an `int` (`:1099-1101`); legacy then sends the move for any non-zero `iDur`, so a negative
/// one becomes a duration near 2^32 (D2). The Rewrite sends it only for a positive one.
pub(super) fn encode_pc_insert(
    vid: u32,
    body: &Body,
    spot: Spot,
    points: &Points,
    recipient: Option<(u32, bool)>,
    now: u32,
) -> Encoded {
    let motion = body.motion;
    let mut at = InsertAt {
        angle: body.rotation,
        x: spot.x,
        y: spot.y,
        z: spot.z,
    };
    let mut i_dur = 0_i32;
    if motion.dest != (spot.x, spot.y) {
        let left = motion
            .start_ms
            .wrapping_add(motion.duration_ms)
            .wrapping_sub(now);
        i_dur = i32::from_ne_bytes(left.to_ne_bytes());
        if i_dur <= 0 {
            at.x = motion.dest.0;
            at.y = motion.dest.1;
        }
    }
    let mut to_records = vec![
        framed(|out| character_add(&body.card, points, vid, at).encode_into(out)),
        framed(|out| character_additional(&body.card, points, vid).encode_into(out)),
    ];
    if i_dur > 0 {
        // `(BYTE) (GetRotation() / 5)`: the float truncated to an int, then its low byte.
        let rotation = f32_to_i32(body.rotation / 5.0).to_le_bytes()[0];
        let duration = u32::try_from(i_dur).unwrap_or(0);
        to_records.push(
            GcCharacterMove::new(
                FUNC_MOVE,
                0,
                rotation,
                vid,
                motion.dest.0,
                motion.dest.1,
                now,
                duration,
            )
            .encode(),
        );
        to_records.push(walk_mode(vid, WALKMODE_RUN));
    }
    let reverse = recipient
        .filter(|&(_, walking)| walking)
        .map(|(other, _)| walk_mode(other, WALKMODE_RUN));
    Encoded {
        to_records,
        reverse,
    }
}

impl GameState {
    /// The points of the player online under `vid`, once loaded.
    fn points_of(&self, vid: u32) -> Option<&Points> {
        self.characters
            .find_by_vid(Vid::new(vid))
            .ok()
            .and_then(Character::points)
    }

    /// Whether the player under `vid` walks (V3).
    fn walks(&self, vid: u32) -> bool {
        self.characters
            .find_by_vid(Vid::new(vid))
            .is_ok_and(is_walking)
    }

    /// Writes `record` to the player `to`: into `reply` when `to` is the entrant it holds the
    /// records of, and to `to`'s client otherwise.
    fn send_to(&self, to: u32, record: Vec<u8>, reply: &mut Reply<'_>) {
        match reply {
            Some((entrant, buffer)) if *entrant == to => buffer.push(record),
            _ => {
                // A player whose client is gone is sent nothing, as a character with no desc.
                let _sent = self.write_to_client(Vid::new(to), record);
            }
        }
    }

    /// Delivers view effects in order, encoding each at its delivery. `reply` holds the
    /// entrant's own records when an entrant is being shown.
    pub(super) fn deliver(&self, effects: &[Effect], reply: Reply<'_>) {
        let mut reply = reply;
        for effect in effects {
            match *effect {
                Effect::Insert {
                    of,
                    to: EntityKey::Character(to),
                } => self.deliver_insert(of, to, &mut reply),
                Effect::Remove {
                    of,
                    to: EntityKey::Character(to),
                } => self.send_to(to, remove_of(of), &mut reply),
                // An NPC and a ground item have no desc: `EncodeInsertPacket` and
                // `EncodeRemovePacket` return.
                Effect::Insert {
                    to: EntityKey::Npc(_) | EntityKey::Ground(_),
                    ..
                }
                | Effect::Remove {
                    to: EntityKey::Npc(_) | EntityKey::Ground(_),
                    ..
                } => {}
            }
        }
    }

    /// `of->EncodeInsertPacket(to)` for a receiving player `to`.
    fn deliver_insert(&self, of: EntityKey, to: u32, reply: &mut Reply<'_>) {
        match of {
            EntityKey::Character(of) => {
                let (Some(body), Some(spot), Some(points)) = (
                    self.bodies.get(&Vid::new(of)),
                    self.spot_of(of),
                    self.points_of(of),
                ) else {
                    // A body is placed only once its points are loaded.
                    return;
                };
                let encoded = encode_pc_insert(
                    of,
                    body,
                    spot,
                    points,
                    Some((to, self.walks(to))),
                    self.clock.now(),
                );
                for record in encoded.to_records {
                    self.send_to(to, record, reply);
                }
                if let Some(record) = encoded.reverse {
                    self.send_to(of, record, reply);
                }
            }
            EntityKey::Npc(npc) => {
                let Some(npc) = self.npc_by_vid(npc) else {
                    return;
                };
                self.send_to(to, framed(|out| npc_add(npc).encode_into(out)), reply);
                if let Some(additional) = npc_additional(npc) {
                    self.send_to(to, framed(|out| additional.encode_into(out)), reply);
                }
            }
            EntityKey::Ground(vid) => {
                let Some(lying) = self.ground.get(&vid) else {
                    return;
                };
                let Some(spot) = self
                    .maps
                    .get(&(lying.channel, lying.map))
                    .and_then(|index| index.spot(of))
                else {
                    return;
                };
                self.send_to(to, ground_add(vid, lying.ground.item.vnum, spot), reply);
            }
        }
    }

    /// The NPC standing under `vid`.
    pub(super) fn npc_by_vid(&self, vid: u32) -> Option<&world::npc::Npc> {
        let &(channel, map, at) = self.npc_of.get(&vid)?;
        self.npcs.get(&(channel, map))?.npcs.get(at)
    }

    /// The index of the map `key` stands on.
    fn map_key_of(&self, key: EntityKey) -> Option<(u8, i32)> {
        match key {
            EntityKey::Character(vid) => self.place_of(vid),
            EntityKey::Npc(vid) => self
                .npc_of
                .get(&vid)
                .map(|&(channel, map, _)| (channel, map)),
            EntityKey::Ground(vid) => self
                .ground
                .get(&vid)
                .map(|lying| (lying.channel, lying.map)),
        }
    }

    /// Sends each record about the player under `vid` to its scope, in order. A player with no
    /// body has no map and no view, so nothing is sent.
    pub(super) fn relay(&self, vid: Vid, records: &[(RelayScope, Vec<u8>)]) {
        let Some(place) = self.place_of(vid.raw()) else {
            return;
        };
        let me = EntityKey::Character(vid.raw());
        for (scope, record) in records {
            match scope {
                RelayScope::Map => {
                    // Legacy walks a pointer-ordered set; the Rewrite goes in VID order (V1).
                    let mut on_map: Vec<Vid> = self
                        .bodies
                        .iter()
                        .filter(|(_, body)| (body.channel, body.map) == place)
                        .map(|(&other, _)| other)
                        .collect();
                    on_map.sort_unstable_by_key(|other| other.raw());
                    for other in on_map {
                        let _sent = self.write_to_client(other, record.clone());
                    }
                }
                RelayScope::ViewAndSelf => self.packet_around(me, record, None),
                RelayScope::ViewExceptSelf => self.packet_around(me, record, Some(me)),
            }
        }
    }

    /// `CEntity::PacketAround` (`G/entity.cpp:88-105`): `record` to every player `me` sees, in
    /// key order, then to `me` itself, skipping `except`. An entity in no sectree sends
    /// nothing, not even its own copy (`:95`).
    pub(super) fn packet_around(&self, me: EntityKey, record: &[u8], except: Option<EntityKey>) {
        let Some(viewers) = self
            .map_key_of(me)
            .and_then(|place| self.maps.get(&place))
            .and_then(|index| index.viewers(me))
        else {
            return;
        };
        let mut none = None;
        for viewer in viewers {
            if Some(viewer) == except {
                continue;
            }
            if let EntityKey::Character(vid) = viewer {
                self.send_to(vid, record.to_vec(), &mut none);
            }
        }
        if Some(me) != except {
            if let EntityKey::Character(vid) = me {
                self.send_to(vid, record.to_vec(), &mut none);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use gamedata::server_attr::SectreeGrid;
    use tokio::sync::mpsc::UnboundedReceiver;
    use world::npc::{MapNpcs, Npc};

    use super::super::fixtures::{a_world, enter, enter_on, points, TestClock, PLACE};
    use super::super::motion::Motion;
    use super::*;
    use crate::loading_phase::PcCard;

    /// The card every body here shows.
    fn a_card() -> PcCard {
        PcCard {
            name: "Walker".to_owned(),
            job: 2,
            empire: 3,
            level: 30,
            conqueror_level: 0,
            language: 0,
            pk_mode: crate::loading_phase::PK_MODE_PEACE,
        }
    }

    /// A body on [`PLACE`] standing at (3200, 3200), turned `rotation` degrees.
    fn a_body(rotation: f32) -> Body {
        let (channel, map) = PLACE;
        let mut body = Body::new(channel, map, (3200, 3200), a_card());
        body.rotation = rotation;
        body
    }

    /// Where a body stands, as the index reports it, with no sectree.
    const fn at(x: i32, y: i32, z: i32) -> Spot {
        Spot {
            tree: None,
            x,
            y,
            z,
        }
    }

    /// The pair `character_add` then `character_additional` writes for [`a_card`] under `vid`.
    fn pair(vid: u32, angle: f32, (x, y, z): (i32, i32, i32), stamina: i32) -> Vec<Vec<u8>> {
        let points = points(stamina);
        let at = InsertAt { angle, x, y, z };
        vec![
            framed(|out| character_add(&a_card(), &points, vid, at).encode_into(out)),
            framed(|out| character_additional(&a_card(), &points, vid).encode_into(out)),
        ]
    }

    fn drained(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// A standing body is sent as its add, at its spot and rotation, then its summary.
    #[test]
    fn an_idle_pc_insert_is_add_then_info() {
        let body = a_body(45.0);
        let encoded = encode_pc_insert(7, &body, at(3200, 3200, 17), &points(820), None, 9_000);
        assert_eq!(
            encoded,
            Encoded {
                to_records: pair(7, 45.0, (3200, 3200, 17), 820),
                reverse: None,
            }
        );
    }

    /// A body with time left on its move is sent at its spot, then its move toward the
    /// destination with the time left, then a run walk mode (`G/char.cpp:1097-1108`,
    /// `:1211-1223`).
    #[test]
    fn a_mid_move_pc_insert_adds_move_and_its_run_walk_mode() {
        let mut body = a_body(93.0);
        body.motion = Motion {
            start: (3200, 3200),
            dest: (4800, 3200),
            start_ms: 1_000,
            duration_ms: 5_333,
            moving: true,
        };
        let encoded = encode_pc_insert(7, &body, at(4000, 3200, 0), &points(820), None, 3_000);
        let mut expected = pair(7, 93.0, (4000, 3200, 0), 820);
        // 93 / 5 is 18.6, truncated to 18; the time left is 1000 + 5333 - 3000.
        expected.push(GcCharacterMove::new(FUNC_MOVE, 0, 18, 7, 4800, 3200, 3_000, 3_333).encode());
        expected.push(walk_mode(7, WALKMODE_RUN));
        assert_eq!(encoded.to_records, expected);
        assert_eq!(walk_mode(7, WALKMODE_RUN), vec![0x6f, 7, 0, 0, 0, 0]);
    }

    /// `iDur` of 0 or below puts the destination in the add and sends no move (D2): on the
    /// last millisecond, one past it, and when the `DWORD` arithmetic wraps far negative.
    #[test]
    fn a_non_positive_i_dur_puts_the_destination_in_the_add_and_sends_no_move() {
        let mut body = a_body(90.0);
        body.motion = Motion {
            start: (3200, 3200),
            dest: (4800, 3300),
            start_ms: 1_000,
            duration_ms: 5_333,
            moving: true,
        };
        for now in [6_333, 6_334, 1_000 + 5_333 + 0x8000_0000] {
            let encoded = encode_pc_insert(7, &body, at(4000, 3200, 5), &points(820), None, now);
            assert_eq!(
                encoded.to_records,
                pair(7, 90.0, (4800, 3300, 5), 820),
                "now {now}"
            );
        }
        // `0x8000_0000` past the end reads as `INT_MIN`; one millisecond less is positive.
        let encoded = encode_pc_insert(
            7,
            &body,
            at(4000, 3200, 5),
            &points(820),
            None,
            1_000 + 5_333 + 0x8000_0001,
        );
        assert_eq!(
            encoded.to_records.len(),
            4,
            "a wrapped iDur above 0 sends the move"
        );
        // A body at its destination is sent where it stands, whatever the clock says.
        body.motion.dest = (4000, 3200);
        let encoded = encode_pc_insert(7, &body, at(4000, 3200, 5), &points(820), None, 0);
        assert_eq!(encoded.to_records, pair(7, 90.0, (4000, 3200, 5), 820));
    }

    /// A recipient that walks has its own walk mode sent to the inserted player's client.
    #[test]
    fn a_walking_recipient_gets_its_own_walk_mode_on_the_other_client() {
        let body = a_body(0.0);
        let spot = at(3200, 3200, 0);
        let walking = encode_pc_insert(7, &body, spot, &points(820), Some((8, true)), 0);
        assert_eq!(walking.reverse, Some(walk_mode(8, WALKMODE_RUN)));
        let running = encode_pc_insert(7, &body, spot, &points(820), Some((8, false)), 0);
        assert_eq!(running.reverse, None);
    }

    /// An insert or a removal for an NPC's view writes nothing, and the same effect for a
    /// player's view writes its records.
    #[test]
    fn an_npc_recipient_gets_nothing() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut seven = enter(&mut world, 7, (3200, 3200), 820);
        let mut eight = enter(&mut world, 8, (3300, 3200), 820);
        let _ = (drained(&mut seven), drained(&mut eight));
        let npc = EntityKey::Npc(0x8000_0001);
        world.deliver(
            &[
                Effect::Insert {
                    of: EntityKey::Character(7),
                    to: npc,
                },
                Effect::Remove {
                    of: EntityKey::Character(7),
                    to: npc,
                },
            ],
            None,
        );
        assert!(drained(&mut seven).is_empty());
        assert!(drained(&mut eight).is_empty());
        world.deliver(
            &[Effect::Remove {
                of: EntityKey::Character(7),
                to: EntityKey::Character(8),
            }],
            None,
        );
        assert_eq!(drained(&mut eight), vec![remove_record(7)]);
    }

    /// A warp NPC is sent as its add alone; an NPC of the NPC type adds its summary.
    #[test]
    fn a_warp_npc_insert_is_add_alone() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut seven = enter(&mut world, 7, (3200, 3200), 820);
        let npc = |vid: u32, char_type: u8| Npc {
            vid,
            vnum: 10_001,
            race: 10_001,
            char_type,
            on_click: 0,
            x: 3300,
            y: 3200,
            z: 0,
            rotation: 0,
            empire: 0,
            moving_speed: 0,
            attack_speed: 0,
            name: b"a3 4002 8995".to_vec(),
        };
        let warp = npc(0x8000_0001, gamedata::mob_proto::CHAR_TYPE_WARP);
        let smith = npc(0x8000_0002, gamedata::mob_proto::CHAR_TYPE_NPC);
        let key = PLACE;
        let _ = world.npcs.insert(
            key,
            std::sync::Arc::new(MapNpcs {
                npcs: vec![warp.clone(), smith.clone()],
                positions: Vec::new(),
            }),
        );
        let _ = world.npc_of.insert(warp.vid, (key.0, key.1, 0));
        let _ = world.npc_of.insert(smith.vid, (key.0, key.1, 1));
        world.deliver(
            &[
                Effect::Insert {
                    of: EntityKey::Npc(warp.vid),
                    to: EntityKey::Character(7),
                },
                Effect::Insert {
                    of: EntityKey::Npc(smith.vid),
                    to: EntityKey::Character(7),
                },
            ],
            None,
        );
        assert_eq!(
            drained(&mut seven),
            vec![
                framed(|out| npc_add(&warp).encode_into(out)),
                framed(|out| npc_add(&smith).encode_into(out)),
                framed(|out| npc_additional(&smith).unwrap().encode_into(out)),
            ]
        );
    }

    /// A removal is `[02, vid]`, whatever the removed entity is.
    #[test]
    fn a_character_remove_is_del() {
        assert_eq!(remove_record(0x0102_0304), vec![0x02, 4, 3, 2, 1]);
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut seven = enter(&mut world, 7, (3200, 3200), 820);
        world.deliver(
            &[Effect::Remove {
                of: EntityKey::Npc(0x8000_0001),
                to: EntityKey::Character(7),
            }],
            None,
        );
        assert_eq!(drained(&mut seven), vec![remove_record(0x8000_0001)]);
    }

    /// A ground item's add carries its spot's x, y and z, its ground VID and its vnum
    /// (`EncodeInsertPacket`, `G/item.cpp:163-202`); its removal is `[1b, vid]`.
    #[test]
    fn a_ground_item_is_added_at_its_spot_and_removed_by_its_vid() {
        let spot = Spot {
            tree: None,
            x: 3200,
            y: 3300,
            z: 55,
        };
        let expected = GcItemGroundAdd {
            x: 3200,
            y: 3300,
            z: 55,
            vid: 4,
            vnum: 27_001,
        };
        assert_eq!(ground_add(4, 27_001, spot), expected.encode());
        assert_eq!(ground_del(0x0102_0304), vec![0x1b, 4, 3, 2, 1]);
        assert_eq!(remove_of(EntityKey::Ground(5)), ground_del(5));
        assert_eq!(remove_of(EntityKey::Npc(5)), remove_record(5));
    }

    /// The entrant's own records go to the reply buffer, in effect order, and every other
    /// player's to its client; the entrant's client gets nothing directly.
    #[test]
    fn the_entrants_records_go_to_the_reply_and_others_to_their_outboxes() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut seven = enter(&mut world, 7, (3200, 3200), 820);
        let mut eight = enter(&mut world, 8, (3300, 3200), 0);
        let _ = (drained(&mut seven), drained(&mut eight));
        let mut reply = Vec::new();
        world.deliver(
            &[
                Effect::Insert {
                    of: EntityKey::Character(8),
                    to: EntityKey::Character(7),
                },
                Effect::Insert {
                    of: EntityKey::Character(7),
                    to: EntityKey::Character(8),
                },
            ],
            Some((7, &mut reply)),
        );
        let card = |vid: u32| world.bodies[&Vid::new(vid)].card.clone();
        let mut shown_eight = Vec::new();
        let at_eight = InsertAt {
            angle: 0.0,
            x: 3300,
            y: 3200,
            z: 0,
        };
        character_add(&card(8), &points(0), 8, at_eight).encode_into(&mut shown_eight);
        assert_eq!(
            reply.len(),
            3,
            "8's pair, then 8's walk mode for 7's insert to 8"
        );
        assert_eq!(reply[0], shown_eight);
        assert_eq!(reply[2], walk_mode(8, WALKMODE_RUN));
        assert!(drained(&mut seven).is_empty());
        assert_eq!(drained(&mut eight).len(), 2, "7's pair");
    }

    /// `PacketAround` sends one copy to each player in view, none to an NPC, none to a player
    /// out of view, and one to the sender.
    #[test]
    fn packet_around_sends_one_copy_to_each_viewer_and_to_itself() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut seven = enter(&mut world, 7, (3200, 3200), 820);
        let mut eight = enter(&mut world, 8, (3300, 3200), 820);
        let mut far = enter(&mut world, 9, (60_000, 60_000), 820);
        let _ = (drained(&mut seven), drained(&mut eight), drained(&mut far));
        let record = vec![0xab, 0xcd];
        world.packet_around(EntityKey::Character(8), &record, None);
        assert_eq!(drained(&mut seven), vec![record.clone()]);
        assert_eq!(drained(&mut eight), vec![record]);
        assert!(drained(&mut far).is_empty());
    }

    /// An entity in no sectree sends nothing, not even its own copy (`G/entity.cpp:95`).
    #[test]
    fn packet_around_without_a_sectree_sends_nothing() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut seven = enter(&mut world, 7, (3200, 3200), 820);
        let mut lost = enter(&mut world, 8, (70_000, 3200), 820);
        let _ = (drained(&mut seven), drained(&mut lost));
        assert!(world.spot_of(8).is_some_and(|spot| spot.tree.is_none()));
        world.packet_around(EntityKey::Character(8), &[1], None);
        world.packet_around(EntityKey::Npc(0x8000_0001), &[2], None);
        assert!(drained(&mut lost).is_empty());
        assert!(drained(&mut seven).is_empty());
    }

    /// Each recipient reads the records in the order they were sent to it, whether it was the
    /// sender or a viewer. Which client is served first within one call is not observable, as
    /// each reads its own stream.
    #[test]
    fn packet_around_keeps_each_recipients_order() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut seven = enter(&mut world, 7, (3200, 3200), 820);
        let mut eight = enter(&mut world, 8, (3300, 3200), 820);
        let _ = (drained(&mut seven), drained(&mut eight));
        world.packet_around(EntityKey::Character(8), &[1], None);
        world.packet_around(EntityKey::Character(7), &[2], None);
        world.packet_around(EntityKey::Character(8), &[3], None);
        let sent = vec![vec![1], vec![2], vec![3]];
        assert_eq!(drained(&mut seven), sent);
        assert_eq!(drained(&mut eight), sent);
    }

    /// `except` skips that one entity, a viewer or the sender itself.
    #[test]
    fn packet_around_with_except_skips_only_that_entity() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut seven = enter(&mut world, 7, (3200, 3200), 820);
        let mut eight = enter(&mut world, 8, (3300, 3200), 820);
        let mut nine = enter(&mut world, 9, (3400, 3200), 820);
        let _ = (drained(&mut seven), drained(&mut eight), drained(&mut nine));
        let me = EntityKey::Character(8);
        world.packet_around(me, &[1], Some(EntityKey::Character(9)));
        assert_eq!(drained(&mut seven), vec![vec![1]]);
        assert_eq!(drained(&mut eight), vec![vec![1]]);
        assert!(drained(&mut nine).is_empty());
        world.packet_around(me, &[2], Some(me));
        assert_eq!(drained(&mut seven), vec![vec![2]]);
        assert!(drained(&mut eight).is_empty());
        assert_eq!(drained(&mut nine), vec![vec![2]]);
    }

    /// A map-wide record reaches the players on the sender's map of its own Channel only, out
    /// of view too: each Channel is its own core in legacy, though one world serves them all.
    #[test]
    fn a_map_relay_stays_on_the_senders_channel() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let other = (PLACE.0 + 1, PLACE.1);
        let grid = SectreeGrid {
            x: 0,
            y: 0,
            columns: 10,
            rows: 10,
        };
        world.host_map(other.0, other.1, grid);
        let mut seven = enter(&mut world, 7, (3200, 3200), 820);
        let mut far = enter(&mut world, 9, (60_000, 60_000), 820);
        let mut eight = enter_on(&mut world, other, 8, (3200, 3200), 820);
        let _ = (drained(&mut seven), drained(&mut far), drained(&mut eight));
        world.relay(Vid::new(7), &[(RelayScope::Map, vec![1])]);
        assert_eq!(drained(&mut seven), vec![vec![1]]);
        assert_eq!(drained(&mut far), vec![vec![1]]);
        assert!(drained(&mut eight).is_empty(), "8 is on another Channel");
    }
}
