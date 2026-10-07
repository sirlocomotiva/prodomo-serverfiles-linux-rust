//! `CG_SYNC_POSITION` in the world: `CInputMain::SyncPosition` (`G/input_main.cpp:2010-2165`),
//! whose policy is [`crate::sync_position::process`], run against the world's bodies.
//!
//! A victim is any entity on the claimer's own Channel and map: a player, whose body moves, or
//! an NPC, whose kind the policy skips. Each accepted claim's `TPacketGCOwnership` goes to the
//! victim's view and the victim (`SetSyncOwner`'s `PacketAround`, `G/char.cpp:5555`) as it is
//! judged, and the accepted positions go to the claimer's view without the claimer
//! (`G/input_main.cpp:2161`) after every element is judged.
//!
//! Everything here runs on the game thread only (ADR-0002).

use std::time::Duration;

use common::vid::Vid;
use gamedata::mob_proto::{CHAR_TYPE_GOTO, CHAR_TYPE_MONSTER, CHAR_TYPE_NPC, CHAR_TYPE_WARP};
use protocol::cg_variable::SyncPositionPacket;

use super::view::EntityKey;
use super::GameState;
use crate::sync_position::{
    judge_sync_ownership, process, SyncOwnershipOutcome, SyncOwnershipState, SyncPositionActor,
    SyncPositionCloseReason, SyncPositionPorts, SyncPositionResult, SyncPositionVictim,
    SyncPositionVictimKind,
};

/// The ports of one `CG_SYNC_POSITION`, over the world.
struct WorldSync<'a> {
    world: &'a mut GameState,
    /// The claimer's Channel and map, which every victim must stand on.
    place: (u8, i32),
    /// The claimer's point, which `SetSyncOwner`'s `DISTANCE_APPROX` compares.
    actor: (i32, i32),
    /// The one clock reading of the frame, as legacy's one `get_dword_time()`.
    now: Duration,
}

/// The sync kind of an NPC of `char_type`.
const fn npc_kind(char_type: u8) -> SyncPositionVictimKind {
    match char_type {
        CHAR_TYPE_MONSTER => SyncPositionVictimKind::Monster,
        CHAR_TYPE_NPC => SyncPositionVictimKind::Npc,
        CHAR_TYPE_WARP => SyncPositionVictimKind::Warp,
        CHAR_TYPE_GOTO => SyncPositionVictimKind::Goto,
        _ => SyncPositionVictimKind::Other,
    }
}

impl WorldSync<'_> {
    /// The entity `vid` names on the claimer's map.
    fn key_of(&self, vid: Vid) -> Option<EntityKey> {
        let raw = vid.raw();
        if self.world.place_of(raw) == Some(self.place) {
            return Some(EntityKey::Character(raw));
        }
        self.world
            .npc_of
            .get(&raw)
            .filter(|&&(channel, map, _)| (channel, map) == self.place)
            .map(|_| EntityKey::Npc(raw))
    }
}

impl SyncPositionPorts for WorldSync<'_> {
    fn resolve(&mut self, vid: Vid) -> Option<SyncPositionVictim> {
        let key = self.key_of(vid)?;
        let index = self.world.maps.get(&self.place)?;
        let spot = index.spot(key)?;
        let kind = match key {
            EntityKey::Character(_) => SyncPositionVictimKind::Player,
            EntityKey::Npc(raw) => npc_kind(self.world.npc_by_vid(raw)?.char_type),
        };
        Some(SyncPositionVictim {
            vid,
            kind,
            x: spot.x,
            y: spot.y,
        })
    }

    fn set_sync_owner(&mut self, actor: Vid, victim: &SyncPositionVictim) -> bool {
        // An NPC has no body to own; every kind that stands today is skipped before this.
        let state = self
            .world
            .bodies
            .get(&victim.vid)
            .and_then(|body| body.sync_owner)
            .map_or(
                SyncOwnershipState {
                    owner: None,
                    claimed_at: Duration::ZERO,
                },
                |(owner, claimed_at)| SyncOwnershipState {
                    owner: Some(Vid::new(owner)),
                    claimed_at,
                },
            );
        let (x, y) = self.actor;
        match judge_sync_ownership(actor, victim, state, x, y, self.now) {
            SyncOwnershipOutcome::Refused => false,
            // The owner past the limit keeps the claim as it was and sends nothing.
            SyncOwnershipOutcome::Kept => true,
            SyncOwnershipOutcome::Accepted {
                owner_changed,
                record,
            } => {
                // A new owner resets the last-sync stamp, which lets its first sync pass the
                // 100 ms test; every accepted claim refreshes the claim's time.
                if let Some(body) = self.world.bodies.get_mut(&victim.vid) {
                    if owner_changed {
                        body.last_sync = None;
                    }
                    body.sync_owner = Some((actor.raw(), self.now));
                }
                self.broadcast_around_victim(victim.vid, &record);
                true
            }
        }
    }

    fn last_sync_time(&mut self, victim: Vid) -> Option<Duration> {
        self.world.bodies.get(&victim)?.last_sync
    }

    fn set_last_sync_time(&mut self, victim: Vid, now: Duration) {
        if let Some(body) = self.world.bodies.get_mut(&victim) {
            body.last_sync = Some(now);
        }
    }

    fn sync(&mut self, victim: Vid, x: i32, y: i32) {
        let _moved = self.world.sync_body(victim.raw(), x, y);
    }

    fn close(&mut self, _actor: Vid, _reason: SyncPositionCloseReason) {
        // The result carries the reason, and the connection closes on it.
    }

    fn broadcast_around_victim(&mut self, victim: Vid, packet: &[u8]) {
        if let Some(key) = self.key_of(victim) {
            self.world.packet_around(key, packet, None);
        }
    }

    fn broadcast(&mut self, actor: Vid, packet: &[u8]) {
        let me = EntityKey::Character(actor.raw());
        self.world.packet_around(me, packet, Some(me));
    }
}

impl GameState {
    /// Runs one `CG_SYNC_POSITION` of the player under `vid`. `None` when it has no body; the
    /// result says whether its descriptor closes.
    pub(super) fn sync_positions(
        &mut self,
        vid: Vid,
        packet: &SyncPositionPacket,
    ) -> Option<SyncPositionResult> {
        let place = self.place_of(vid.raw())?;
        let spot = self.spot_of(vid.raw())?;
        let sync_hack_count = self.bodies.get(&vid)?.sync_hack_count;
        let now = self.clock.elapsed();
        let mut actor = SyncPositionActor {
            vid,
            x: spot.x,
            y: spot.y,
            sync_hack_count,
        };
        let mut port = WorldSync {
            world: self,
            place,
            actor: (spot.x, spot.y),
            now,
        };
        let result = process(&mut port, packet, &mut actor, now);
        // A refusal counts on the claimer, and the count outlives the frame.
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.sync_hack_count = actor.sync_hack_count;
        }
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use common::vid::Vid;
    use protocol::cg_variable::{SyncPositionElement, SyncPositionPacket};
    use tokio::sync::mpsc::UnboundedReceiver;

    use super::super::fixtures::{a_world, enter, TestClock};
    use crate::sync_position::{
        SyncPositionCloseReason, HEADER_GC_SYNC_POSITION, SYNC_HACK_LIMIT_COUNT,
        SYNC_POSITION_PREFIX_SIZE,
    };

    const HEADER_GC_OWNERSHIP: u8 = 0x3e;

    fn headers(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<u8> {
        std::iter::from_fn(|| inbox.try_recv().ok())
            .map(|record| record[0])
            .collect()
    }

    fn one_element(vid: u32, x: i32, y: i32) -> SyncPositionPacket {
        SyncPositionPacket {
            declared_size: SYNC_POSITION_PREFIX_SIZE + 12,
            elements: vec![SyncPositionElement { vid, x, y }],
        }
    }

    /// The owner of a claim on a victim past `DISTANCE_APPROX` 250 keeps the claim and moves
    /// the victim, but neither refreshes the claim's time nor sends `GC_OWNERSHIP`
    /// (`G/char.cpp:5510-5518`); within the limit the same owner refreshes both.
    #[test]
    fn an_owner_past_the_claim_range_keeps_the_claim_without_a_record() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut claimer = enter(&mut world, 7, (3200, 3200), 820);
        let mut victim = enter(&mut world, 8, (3500, 3200), 820);
        let claimed_at = Duration::from_millis(500);
        world.bodies.get_mut(&Vid::new(8)).unwrap().sync_owner = Some((7, claimed_at));
        let _ = (headers(&mut claimer), headers(&mut victim));
        clock.set(1000);

        let result = world.sync_positions(Vid::new(7), &one_element(8, 3600, 3200));
        assert_eq!(result.map(|result| result.accepted_elements.len()), Some(1));
        let spot = world.spot_of(8).unwrap();
        assert_eq!((spot.x, spot.y), (3600, 3200));
        assert_eq!(world.bodies[&Vid::new(8)].sync_owner, Some((7, claimed_at)));
        assert_eq!(
            world.bodies[&Vid::new(8)].last_sync,
            Some(Duration::from_millis(1000))
        );
        assert_eq!(headers(&mut victim), [HEADER_GC_SYNC_POSITION]);
        assert_eq!(headers(&mut claimer), Vec::<u8>::new());

        // Within the range: the claim's time is refreshed and the record goes to both.
        world.bodies.get_mut(&Vid::new(8)).unwrap().last_sync = None;
        assert!(world.sync_body(8, 3400, 3200));
        clock.set(2000);
        let result = world.sync_positions(Vid::new(7), &one_element(8, 3450, 3200));
        assert_eq!(result.map(|result| result.accepted_elements.len()), Some(1));
        assert_eq!(
            world.bodies[&Vid::new(8)].sync_owner,
            Some((7, Duration::from_millis(2000)))
        );
        assert_eq!(
            headers(&mut victim),
            [HEADER_GC_OWNERSHIP, HEADER_GC_SYNC_POSITION]
        );
        assert_eq!(headers(&mut claimer), [HEADER_GC_OWNERSHIP]);
    }

    /// The refusal count is the claimer's (`m_iSyncHackCount`, `G/char.h:2206`): it outlives
    /// the frame that raised it, so ten refusals over ten frames leave the eleventh to close
    /// the descriptor (`G/input_main.cpp:2117-2127`).
    #[test]
    fn the_sync_hack_count_outlives_its_frame() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _claimer = enter(&mut world, 7, (3200, 3200), 820);
        let _victim = enter(&mut world, 8, (3300, 3200), 820);
        clock.set(1000);
        let first = world.sync_positions(Vid::new(7), &one_element(8, 3310, 3200));
        assert_eq!(first.map(|result| result.accepted_elements.len()), Some(1));

        // Each frame in the same millisecond is too soon and counts one refusal.
        for count in 1..=SYNC_HACK_LIMIT_COUNT {
            let result = world.sync_positions(Vid::new(7), &one_element(8, 3320, 3200));
            let result = result.expect("the claimer has a body");
            assert_eq!(result.close_reason, None);
            assert!(result.accepted_elements.is_empty());
            assert_eq!(world.bodies[&Vid::new(7)].sync_hack_count, count);
        }
        let result = world.sync_positions(Vid::new(7), &one_element(8, 3320, 3200));
        assert_eq!(
            result.and_then(|result| result.close_reason),
            Some(SyncPositionCloseReason::SyncIntervalHackLimit)
        );
    }

    /// A new owner resets the victim's last-sync stamp (`G/char.cpp:5521-5534`), so its first
    /// sync passes the 100 ms test though the old owner synced the victim 50 ms before; its
    /// own next sync 50 ms on is too soon.
    #[test]
    fn a_new_owners_first_sync_is_not_too_soon() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _old = enter(&mut world, 7, (3200, 3200), 820);
        let _victim = enter(&mut world, 8, (3500, 3200), 820);
        let _new = enter(&mut world, 9, (3550, 3200), 820);
        world.bodies.get_mut(&Vid::new(8)).unwrap().sync_owner =
            Some((7, Duration::from_millis(1000)));
        // The old owner, past 250, keeps the claim and stamps the sync at 1050.
        clock.set(1050);
        let result = world.sync_positions(Vid::new(7), &one_element(8, 3510, 3200));
        assert_eq!(result.map(|result| result.accepted_elements.len()), Some(1));
        assert_eq!(
            world.bodies[&Vid::new(8)].last_sync,
            Some(Duration::from_millis(1050))
        );
        // At 1100 the claim has lapsed and 9, within 250, takes it and syncs.
        clock.set(1100);
        let result = world.sync_positions(Vid::new(9), &one_element(8, 3520, 3200));
        assert_eq!(result.map(|result| result.accepted_elements.len()), Some(1));
        let victim = &world.bodies[&Vid::new(8)];
        assert_eq!(victim.sync_owner, Some((9, Duration::from_millis(1100))));
        assert_eq!(victim.last_sync, Some(Duration::from_millis(1100)));
        assert_eq!(world.bodies[&Vid::new(9)].sync_hack_count, 0);
        // Control: the same owner 50 ms on is too soon, and the refusal is counted.
        clock.set(1150);
        let result = world.sync_positions(Vid::new(9), &one_element(8, 3530, 3200));
        assert_eq!(result.map(|result| result.accepted_elements.len()), Some(0));
        assert_eq!(world.bodies[&Vid::new(9)].sync_hack_count, 1);
    }
}
