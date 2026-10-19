//! A group's party: the leader its followers name, and the three messages legacy sends
//! through it (`CParty::SendMessage`, `G/party.cpp:863-956`; `CHARACTER::GetProtege`,
//! `G/char_battle.cpp:3436-3445`).
//!
//! # What legacy does
//!
//! `SpawnGroup` stands its leader first and every member after, each member naming the
//! leader's VID; the leader holds the regen and the party (`G/char_manager.cpp:640-667`).
//! The Rewrite keeps the link as `Npc::leader`, so the party is derived: the leader of
//! `mob` is its own VID when another NPC names it, else the VID it names. A lone
//! character has no party.
//!
//! - `PM_ATTACKED_BY` (`:906-930`): `Stun` (`G/char_battle.cpp:431-433`) and Battle's
//!   range arm (`G/char_state.cpp:1106-1107`) tell each member but the sender, and each
//!   member that `CanBeginFight` begins on the sender's held victim.
//! - `PM_RETURN` (`:876-904`): `Return` (`G/char.cpp:6876-6878`) tells each member but the
//!   sender, and each drops its victim and walks to a point within 500 of the home it
//!   was sent, facing it first.
//! - `PM_AGGRO_INCREASE` (`:932-954`): `UpdateAggrPointEx` (`G/char_battle.cpp:3820-3831`)
//!   tells each member but the sender, and each feeds `UpdateAggrPoint(victim, SPECIAL,
//!   share)`, a half of the hit's aggro for the leader and a third for a member. The
//!   victim must still be online.
//!
//! Everything here runs on the game thread only (ADR-0002).

use common::vid::Vid;
use world::character::number;

use super::GameState;

/// A group's party, derived from `Npc::leader`: the master and the members on its map.
impl GameState {
    /// The party `mob` belongs to: the leader's VID and every member's, in map order,
    /// the sender included. Empty when `mob` is lone — no member names it and it names
    /// none — in which case legacy's `m_pkParty` is `NULL` and no message is sent.
    pub(super) fn party_of(&self, mob: u32) -> Vec<u32> {
        let Some(&(channel, map, _)) = self.npc_of.get(&mob) else {
            return Vec::new();
        };
        let leader = self.party_leader(mob);
        if leader.is_none() {
            return Vec::new();
        }
        let leader = leader.expect("a party was found");
        let Some(list) = self.npcs.get(&(channel, map)) else {
            return Vec::new();
        };
        list.npcs
            .iter()
            .filter(|npc| npc.vid == leader || npc.leader == Some(leader))
            .map(|npc| npc.vid)
            .collect()
    }

    /// The leader of `mob`'s party: its own VID when a member names it, else the VID it
    /// names. `None` when `mob` is lone.
    pub(super) fn party_leader(&self, mob: u32) -> Option<u32> {
        let &(channel, map, _) = self.npc_of.get(&mob)?;
        let list = self.npcs.get(&(channel, map))?;
        if list.npcs.iter().any(|npc| npc.leader == Some(mob)) {
            return Some(mob);
        }
        list.npcs
            .iter()
            .find(|npc| npc.vid == mob)
            .and_then(|npc| npc.leader)
    }

    /// `PM_ATTACKED_BY` of `mob`: each member but the sender that `CanBeginFight` begins
    /// on the sender's held victim. Nothing when the sender holds none — legacy returns
    /// past a `NULL` victim — or belongs to no party.
    pub(super) fn party_attacked_by(&mut self, mob: u32) {
        let Some(victim) = self.victim_of(mob) else {
            return;
        };
        let party = self.party_of(mob);
        for member in party {
            if member != mob && self.can_begin_fight(member) {
                self.begin_fight(member, victim);
            }
        }
    }

    /// `PM_RETURN` of `mob` past `home`: each member but the sender drops its victim and
    /// walks to `home` scattered by `number(-500, 500)` each way, facing it first. A
    /// member whose walk refuses still drops its victim.
    pub(super) fn party_return(&mut self, mob: u32, home: (i32, i32)) {
        let party = self.party_of(mob);
        for member in party {
            if member == mob {
                continue;
            }
            let point = (
                home.0.saturating_add(number(&mut self.dice, -500, 500)),
                home.1.saturating_add(number(&mut self.dice, -500, 500)),
            );
            self.set_victim(member, None);
            if let Some(spot) = self.npc_spot(member) {
                if let Some(mind) = self.minds.get_mut(member) {
                    mind.rotation = super::motion::rotation_to_xy(
                        point.0.saturating_sub(spot.0),
                        point.1.saturating_sub(spot.1),
                    );
                }
            }
            if self.npc_goto(member, point.0, point.1) {
                self.send_move_wait(member);
            }
        }
    }

    /// `PM_AGGRO_INCREASE` of `mob` on `party`: each member but the sender feeds
    /// `UpdateAggrPoint(victim, SPECIAL, share)`. Nothing when the victim is offline.
    pub(super) fn party_aggro_increase(
        &mut self,
        mob: u32,
        party: &[u32],
        victim: Vid,
        share: i32,
    ) {
        if !self.bodies.contains_key(&victim) {
            return;
        }
        for member in party {
            if *member != mob {
                self.update_aggr_point(*member, victim, super::fight::AggroDamage::Special, share);
            }
        }
    }

    /// `Follow` of the NPC under `leader` to `min_distance`: the give-up never fires for
    /// a leader — the protege follow calls it after the victim is dropped, so the chase
    /// is over — and the target neither leads nor strafes, so the walk is straight at
    /// it. True when it walks.
    pub(super) fn follow_npc(&mut self, mob: u32, leader: u32, min_distance: f32) -> bool {
        let Some(at) = self.npc_spot(mob) else {
            return false;
        };
        let Some(spot) = self.npc_spot(leader) else {
            return false;
        };
        if let Some(mind) = self.minds.get_mut(mob) {
            mind.rotation = super::motion::rotation_to_xy(
                spot.0.saturating_sub(at.0),
                spot.1.saturating_sub(at.1),
            );
        }
        let dist = f64::from(common::cfloat::i32_to_f32(
            crate::sync_position::distance_approx(
                spot.0.saturating_sub(at.0),
                spot.1.saturating_sub(at.1),
            ),
        ));
        if dist <= f64::from(min_distance) {
            return false;
        }
        let (fx, fy) = super::mob_ai::delta_by_degree(
            self.minds.get(mob).map_or(0.0, |mind| mind.rotation),
            super::motion::distance_sqrt(spot.0.saturating_sub(at.0), spot.1.saturating_sub(at.1))
                - min_distance,
        );
        let step = (
            at.0.saturating_add(common::cfloat::f32_to_i32(fx)),
            at.1.saturating_add(common::cfloat::f32_to_i32(fy)),
        );
        if !self.npc_goto(mob, step.0, step.1) {
            return false;
        }
        self.send_move_wait(mob);
        true
    }
}

#[cfg(test)]
mod tests {
    use common::vid::Vid;
    use gamedata::mob_proto::{BATTLE_TYPE_MELEE, CHAR_TYPE_MONSTER};
    use tokio::sync::mpsc::UnboundedReceiver;
    use world::npc::FIRST_NPC_VID;

    use super::super::fight::AggroDamage;
    use super::super::fixtures::{a_fighter, enter, hit, open_cells, stand_field, TestClock};
    use super::super::mob_ai::{Fsm, Victim};
    use super::*;
    use crate::game_loop::PulseProcessor;

    /// The party's master.
    const MASTER: u32 = FIRST_NPC_VID;
    /// Its follower, 100 east.
    const FOLLOWER: u32 = FIRST_NPC_VID + 1;
    /// A lone monster far away, no party.
    const LONE: u32 = FIRST_NPC_VID + 2;

    const SPOT: (i32, i32) = (3200, 3200);
    const EAST: (i32, i32) = (3300, 3200);
    const FAR: (i32, i32) = (9000, 9000);
    const NEAR: (i32, i32) = (3100, 3200);
    const T0: u32 = 1_000;
    const P0: u64 = 1;

    fn a_party(no_wander: bool) -> (TestClock, GameState, UnboundedReceiver<Vec<u8>>) {
        a_party_with_hp(no_wander, 10_000)
    }

    fn a_party_with_hp(
        no_wander: bool,
        hp: i32,
    ) -> (TestClock, GameState, UnboundedReceiver<Vec<u8>>) {
        let clock = TestClock::default();
        clock.set(T0);
        let master = a_fighter(MASTER, 406, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, hp);
        let mut follower = a_fighter(
            FOLLOWER,
            406,
            BATTLE_TYPE_MELEE,
            (175, 2000),
            0,
            EAST,
            10_000,
        );
        follower.leader = Some(MASTER);
        let lone = a_fighter(LONE, 406, BATTLE_TYPE_MELEE, (175, 2000), 0, FAR, 10_000);
        let mut world = stand_field(
            &clock,
            vec![master, follower, lone],
            open_cells(),
            no_wander,
        );
        let mut inbox = enter(&mut world, 1, NEAR, 12_345);
        world.start_pc_recovery(Vid::new(1));
        world.process_pulse(P0);
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        (clock, world, inbox)
    }

    /// The party is the master and its followers in map order; a lone monster has none.
    #[test]
    fn the_party_is_the_master_and_its_followers_in_map_order() {
        let (_clock, world, _inbox) = a_party(true);
        assert_eq!(world.party_of(MASTER), vec![MASTER, FOLLOWER]);
        assert_eq!(world.party_of(FOLLOWER), vec![MASTER, FOLLOWER]);
        assert!(world.party_of(LONE).is_empty());
        assert_eq!(world.party_leader(MASTER), Some(MASTER));
        assert_eq!(world.party_leader(FOLLOWER), Some(MASTER));
        assert_eq!(world.party_leader(LONE), None);
    }

    /// A hit master tells its follower to begin, and no lone monster (`G/char_state.cpp`'s
    /// Battle range arm): the master begins on the hit, swings within three Pulses, and the
    /// swing's `PM_ATTACKED_BY` begins the follower on the same victim.
    #[test]
    fn a_hit_master_tells_its_follower_to_begin_and_no_lone_monster() {
        let (_clock, mut world, _inbox) = a_party(true);
        hit(&mut world, 1, MASTER);
        for pulse in P0 + 1..=P0 + 3 {
            world.process_pulse(pulse);
        }
        assert_eq!(
            world.minds.get(FOLLOWER).and_then(|mind| mind.victim),
            Some(Victim::Here(Vid::new(1))),
            "the follower begins on the master's victim"
        );
        assert!(
            world.minds.get(FOLLOWER).is_some_and(|mind| mind.fighting),
            "the follower fights"
        );
        assert_eq!(world.minds.get(LONE).and_then(|mind| mind.victim), None);
        assert_eq!(world.victim_of(FOLLOWER), Some(Vid::new(1)));
    }

    /// A master already fighting tells its follower through the stun: the first hit sets
    /// the victim, the lethal second hit stuns, and the stun's `PM_ATTACKED_BY` runs past
    /// the held victim, so the follower fights while the master lies stunned.
    #[test]
    fn a_stun_tells_the_follower_while_the_master_lies_stunned() {
        let (_clock, mut world, _inbox) = a_party_with_hp(true, 10_000);
        hit(&mut world, 1, MASTER);
        assert_eq!(world.victim_of(MASTER), Some(Vid::new(1)));
        world.npc_mut(MASTER).expect("the master stands").hp = 1;
        hit(&mut world, 1, MASTER);
        assert!(
            world.npc_by_vid(MASTER).is_some_and(|npc| npc.stunned),
            "a lethal hit stuns the master"
        );
        assert_eq!(
            world.minds.get(FOLLOWER).and_then(|mind| mind.victim),
            Some(Victim::Here(Vid::new(1))),
            "the stun's message begins the follower"
        );
        assert_eq!(world.minds.get(LONE).and_then(|mind| mind.victim), None);
    }

    /// A hit feeds the follower's aggro through the party share: the master's 100 feeds
    /// its own 200 (the insert's 100 plus the feed's 100), and the follower's row holds
    /// `(0, 100)` (the insert's 50 plus the feed's 50) — the damage in the aggro, not the
    /// total — while the lone monster's map stays empty.
    #[test]
    fn a_hit_shares_half_its_aggro_with_the_followers() {
        let (_clock, mut world, _inbox) = a_party(true);
        world.update_aggr_point(MASTER, Vid::new(1), AggroDamage::Normal, 100);
        let master = world.npc_combat[&MASTER].damage[&1];
        assert_eq!((master.total, master.aggro), (0, 200));
        let follower = world.npc_combat[&FOLLOWER].damage[&1];
        assert_eq!((follower.total, follower.aggro), (0, 100));
        assert!(world
            .npc_combat
            .get(&LONE)
            .is_none_or(|combat| combat.damage.is_empty()));
    }

    /// A member's hit shares a third with the master: the follower's 99 feeds its own 198
    /// and the master's row `(0, 66)`.
    #[test]
    fn a_members_hit_shares_a_third_with_the_master() {
        let (_clock, mut world, _inbox) = a_party(true);
        world.update_aggr_point(FOLLOWER, Vid::new(1), AggroDamage::Normal, 99);
        let follower = world.npc_combat[&FOLLOWER].damage[&1];
        assert_eq!((follower.total, follower.aggro), (0, 198));
        let master = world.npc_combat[&MASTER].damage[&1];
        assert_eq!((master.total, master.aggro), (0, 66));
        let _ = CHAR_TYPE_MONSTER;
    }

    /// A returning master scatters its follower home: the follower drops its victim and
    /// walks toward within 500 of the home, while the lone monster keeps its victim and
    /// its spot.
    #[test]
    fn a_return_scatters_the_follower_home_and_leaves_the_lone_monster() {
        let (clock, mut world, _inbox) = a_party(true);
        hit(&mut world, 1, MASTER);
        // The fight moves on through `Goto`, then past its move duration: the monster
        // stands 500 east, so `Return` walks it home.
        assert!(world.npc_goto(MASTER, SPOT.0 + 500, SPOT.1));
        let home = world.npc_combat[&MASTER].last_attacked.at;
        let duration = world
            .minds
            .get(MASTER)
            .expect("the master walks")
            .motion
            .duration_ms;
        clock.set(T0 + duration);
        for pulse in P0 + 5..=P0 + 8 {
            world.process_pulse(pulse);
        }
        assert_eq!(
            world.npc_spot(MASTER),
            Some((SPOT.0 + 500, SPOT.1)),
            "the walk ends 500 east"
        );
        // Both begin after the walk: the walk's Battle chases would drop a victim 4000
        // or more away.
        world.begin_fight(FOLLOWER, Vid::new(1));
        world.begin_fight(LONE, Vid::new(1));
        assert!(world.npc_return(MASTER));
        assert_eq!(world.victim_of(MASTER), None);
        assert_eq!(
            world.victim_of(FOLLOWER),
            None,
            "the scatter drops the victim"
        );
        assert_eq!(world.victim_of(LONE), Some(Vid::new(1)));
        let mind = world.minds.get(FOLLOWER).expect("the follower walks");
        assert_eq!(mind.asked, Some(Fsm::Move), "the follower walks");
        let dest = mind.motion.dest;
        assert!(
            dest.0.saturating_sub(home.0).abs() <= 500
                && dest.1.saturating_sub(home.1).abs() <= 500,
            "within 500 of home: {dest:?} past {home:?}"
        );
        let lone = world.npc_by_vid(LONE).expect("the lone stands");
        assert_eq!((lone.x, lone.y), FAR, "the lone monster does not walk");
    }
}
