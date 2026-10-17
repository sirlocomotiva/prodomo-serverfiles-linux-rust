//! A monster's `Attack` on a player (`G/char_battle.cpp:220-328`): the reach, the swing,
//! the claim through [`GameState::claim_pc`], the player's `BeginFight`, and the stamps.
//!
//! Everything here runs on the game thread only (ADR-0002).

use std::time::Duration;

use common::vid::Vid;
use protocol::gc_nested::GcDamageInfo;
use protocol::gc_vid::GcSpecialEffect;
use world::character::{mob_reach, strike_pc, MobStriker, PcHitEnd, PcStrike, PcVictim};

use super::death::Attacker;
use super::view::EntityKey;
use super::GameState;
use crate::sync_position::{
    distance_approx, judge_sync_ownership, SyncOwnershipOutcome, SyncOwnershipState,
    SyncPositionVictim,
};

impl GameState {
    /// `CHARACTER::Attack` of the monster under `mob` on the player `victim`: whether it hit.
    pub(super) fn mob_attack(&mut self, mob: u32, victim: Vid) -> bool {
        if !self.npc_can_attack_pc(mob, victim) {
            return false;
        }
        // The chat is drawn before the reach test (`:310-316`).
        self.monster_chat_attack(mob);
        let Some(target) = self.victim_state(victim) else {
            return false;
        };
        let Some(npc) = self.npc_by_vid(mob) else {
            return false;
        };
        let reach = mob_reach(npc);
        let distance = distance_approx(
            target.at.0.saturating_sub(npc.x),
            target.at.1.saturating_sub(npc.y),
        );
        if distance > reach {
            return false;
        }
        let berserk = self.minds.get(mob).is_some_and(|mind| mind.berserk);
        let affects = self.affect_flags_of(victim);
        let down = self.is_down(victim.raw());
        // Borrow fields, not methods, as `strike` does: the dice, the monster and the points
        // are disjoint.
        let Some(&(channel, map, at)) = self.npc_of.get(&mob) else {
            return false;
        };
        let (channel, map, at) = (channel, map, at);
        let characters = &self.characters;
        let Ok(character) = characters.find_by_vid(victim) else {
            return false;
        };
        let Some(points) = character.points() else {
            return false;
        };
        let dice = &mut self.dice;
        let Some(list) = self.npcs.get(&(channel, map)) else {
            return false;
        };
        let Some(npc) = list.npcs.get(at) else {
            return false;
        };
        let strike = strike_pc(
            dice,
            MobStriker { npc, berserk },
            PcVictim {
                points,
                affect_flags: affects,
                down,
            },
        );
        self.show_mob_strike(mob, victim, &strike);
        // The claim, `SetSyncOwner(mob)` on the player (`:299-302`). The NOMOVE arm reads the
        // victim's AI flags, and a player has none, so it never refuses.
        let attackable = self.npc_can_attack_pc(mob, victim);
        let actor_at = self.npc_spot(mob).unwrap_or((0, 0));
        let spot = SyncPositionVictim {
            vid: victim,
            kind: crate::sync_position::SyncPositionVictimKind::Player,
            x: target.at.0,
            y: target.at.1,
        };
        let now_span = self.clock.elapsed();
        let _claimed = self.claim_pc(Vid::new(mob), &spot, actor_at, attackable, now_span);
        // The player's `BeginFight` (`:303-304`).
        if self.can_pc_begin_fight(victim) {
            if let Some(body) = self.bodies.get_mut(&victim) {
                body.fighting = true;
            }
            self.leave_move(victim.raw());
        }
        // The stamps: the monster's `OnMove(true)` and the victim's `OnMove()` (`:317-318`).
        let now = self.clock.now();
        if let Some(combat) = self.npc_combat.get_mut(&mob) {
            combat.last_attack_ms = Some(now);
        }
        if let Some(body) = self.bodies.get_mut(&victim) {
            body.last_move = now;
        }
        true
    }

    /// `battle_is_attackable` of the monster under `mob` on the player `victim`
    /// (`G/battle.cpp:99-152`): neither dead, neither on BANPK, and the monster neither
    /// stunned nor dead. The rest passes for a monster on a player (D-OBS).
    fn npc_can_attack_pc(&self, mob: u32, victim: Vid) -> bool {
        let Some(npc) = self.npc_by_vid(mob) else {
            return false;
        };
        if npc.stunned || npc.dead {
            return false;
        }
        if self.is_dead(victim.raw()) {
            return false;
        }
        let Some((_, map)) = self.place_of(victim.raw()) else {
            return false;
        };
        let Some(spot) = self.spot_of(victim.raw()) else {
            return false;
        };
        if self.is_banpk(map, spot.x, spot.y) {
            return false;
        }
        let Some(&(_, mob_map, _)) = self.npc_of.get(&mob) else {
            return false;
        };
        if mob_map != map {
            return false;
        }
        !self.is_banpk(map, npc.x, npc.y)
    }

    /// What the monster's swing showed: each effect around the player, then the hit.
    fn show_mob_strike(&mut self, mob: u32, victim: Vid, strike: &PcStrike) {
        let around = EntityKey::Character(victim.raw());
        for effect in &strike.effects {
            let mut record = Vec::new();
            GcSpecialEffect {
                effect_type: *effect,
                vid: victim.raw(),
            }
            .encode_into(&mut record);
            self.packet_around(around, &record, None);
        }
        let attacker = Attacker {
            vid: Vid::new(mob),
            pc: false,
        };
        match strike.end {
            PcHitEnd::Blocked => self.send_pc_damage_info(victim, super::fight::DAMAGE_BLOCK, 0),
            PcHitEnd::Down => {
                let _hit = self.damage_pc(Some(attacker), victim, 0, None);
            }
            PcHitEnd::Landed { damage, flag } => {
                let _hit = self.damage_pc(Some(attacker), victim, damage, Some(flag));
            }
        }
    }

    /// `SetSyncOwner(actor)` on the player `victim`, past the NOMOVE arm (a player has no AI
    /// flags): `!attackable` runs the `DAMAGE_BLOCK` and refuses; else `judge_sync_ownership`
    /// decides. The client sync passes `!(dead victim || down actor)`, the monster
    /// `npc_can_attack_pc`.
    pub(super) fn claim_pc(
        &mut self,
        actor: Vid,
        victim: &SyncPositionVictim,
        actor_at: (i32, i32),
        attackable: bool,
        now: Duration,
    ) -> bool {
        if !attackable {
            self.block_pc_claim(victim.vid, actor);
            return false;
        }
        let state = self
            .bodies
            .get(&victim.vid)
            .map(|body| body.sync)
            .unwrap_or_default();
        match judge_sync_ownership(actor, victim, state, actor_at.0, actor_at.1, now) {
            SyncOwnershipOutcome::Refused => false,
            // The owner past the limit keeps the claim as it was and sends nothing.
            SyncOwnershipOutcome::Kept => true,
            SyncOwnershipOutcome::Accepted {
                owner_changed,
                record,
            } => {
                if let Some(body) = self.bodies.get_mut(&victim.vid) {
                    if owner_changed {
                        body.last_sync = None;
                    }
                    body.sync = SyncOwnershipState {
                        owner: Some(actor),
                        claimed_at: Some(now),
                    };
                }
                let key = EntityKey::Character(victim.vid.raw());
                self.packet_around(key, &record, None);
                true
            }
        }
    }

    /// `CanBeginFight` of the player `victim` (`G/char_battle.cpp:108-115`): standing (not
    /// `fighting`), neither dead nor stunned, and `CanMove` (no stun affect and no own shop,
    /// neither of which a player can have yet).
    fn can_pc_begin_fight(&self, victim: Vid) -> bool {
        self.bodies.get(&victim).is_some_and(|body| !body.fighting) && !self.is_down(victim.raw())
    }

    /// `SendDamagePacket` to the player under `victim` (`G/char_battle.cpp:1720-1742`): its
    /// own `GC_DAMAGE_INFO`.
    pub(super) fn send_pc_damage_info(&self, victim: Vid, flag: u8, damage: i32) {
        let mut record = Vec::new();
        GcDamageInfo {
            header: GcDamageInfo::header(),
            vid: victim.raw(),
            flag,
            damage,
        }
        .encode_into(&mut record);
        let _sent = self.write_to_client(victim, record);
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Range;
    use std::time::Duration;

    use common::point_slot as point;
    use common::vid::Vid;
    use gamedata::mob_proto::BATTLE_TYPE_MELEE;
    use gamedata::server_attr::{
        CellBlock, SectreeGrid, ServerAttr, ATTR_BANPK, CELL_SIZE, SECTREE_CELLS,
    };
    use protocol::gc_actors::GcCharacterMove;
    use protocol::gc_inventory::{
        HEADER_GC_CHARACTER_MOVE, HEADER_GC_DAMAGE_INFO, HEADER_GC_DEAD, HEADER_GC_OWNERSHIP,
        HEADER_GC_PHASE, HEADER_GC_PLAYER_POINT_CHANGE, HEADER_GC_STUN, HEADER_GC_TARGET,
    };
    use protocol::gc_nested::{GcDamageInfo, GcPointChange};
    use tokio::sync::mpsc::UnboundedReceiver;
    use world::cells::{HostedCells, MapCells};
    use world::character::{number, Pcg32};
    use world::npc::FIRST_NPC_VID;

    use super::super::fixtures::{
        a_fighter, enter, hit, open_cells, region, stand_field, TestClock, PLACE,
    };
    use super::super::mob_ai::Fsm;
    use super::*;
    use crate::game_loop::PulseProcessor;
    use crate::movement::FUNC_ATTACK;
    use crate::sync_position::{
        ownership_record, SyncPositionVictim, SyncPositionVictimKind, SYNC_CLAIM_LIFETIME,
    };

    /// The monster every test fights.
    const ME: u32 = FIRST_NPC_VID;

    /// Where it stands.
    const SPOT: (i32, i32) = (3200, 3200);

    /// 100 west of it: within every reach used here.
    const NEAR: (i32, i32) = (3100, 3200);

    /// When the tests' first Pulse runs, in milliseconds.
    const T0: u32 = 1_000;

    /// The tests' first Pulse.
    const P0: u64 = 1;

    fn drained(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// `a_fighter` 406 (MELEE, range 175, sight 2000) at [`SPOT`] with `hp`, and the player 1
    /// at `at`, after Pulse [`P0`]; the inbox is the player's, drained.
    fn a_fight(
        npc_hp: i32,
        at: (i32, i32),
        no_wander: bool,
    ) -> (TestClock, GameState, UnboundedReceiver<Vec<u8>>) {
        let clock = TestClock::default();
        clock.set(T0);
        let npc = a_fighter(ME, 406, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, npc_hp);
        let mut world = stand_field(&clock, vec![npc], open_cells(), true);
        let _ = no_wander;
        let mut inbox = enter(&mut world, 1, at, 12_345);
        world.start_pc_recovery(Vid::new(1));
        world.process_pulse(P0);
        drained(&mut inbox);
        (clock, world, inbox)
    }

    /// The first byte of each record, with a point change's word read as its low byte.
    fn headers(records: &[Vec<u8>]) -> Vec<u8> {
        records
            .iter()
            .map(|record| record.first().copied().unwrap_or(0))
            .collect()
    }

    /// `GC_DAMAGE_INFO{victim, flag, damage}`.
    fn damage_info(vid: u32, flag: u8, damage: i32) -> Vec<u8> {
        let mut record = Vec::new();
        GcDamageInfo {
            header: GcDamageInfo::header(),
            vid,
            flag,
            damage,
        }
        .encode_into(&mut record);
        record
    }

    /// The cells of [`PLACE`], `ATTR_BANPK` in the columns of `band`.
    fn banpk_cells(band: Range<i32>) -> std::sync::Arc<HostedCells> {
        let side = SECTREE_CELLS;
        let cells: Vec<u32> = (0..side * side)
            .map(|at| {
                let x = i32::try_from(at % side).unwrap() * CELL_SIZE;
                if band.contains(&x) {
                    ATTR_BANPK
                } else {
                    0
                }
            })
            .collect();
        let attr = ServerAttr::from_blocks(1, 1, vec![CellBlock::from_cells(&cells)]).unwrap();
        let grid = SectreeGrid::of(&region()).unwrap();
        let mut hosted = HostedCells::default();
        hosted.insert(PLACE.1, MapCells::new(grid, std::sync::Arc::new(attr)));
        std::sync::Arc::new(hosted)
    }

    /// A seed whose first `number(0, 60)` is not 0, so no swing chats.
    fn quiet_seed() -> u64 {
        for seed in 0..100u64 {
            let mut dice = Pcg32::new(seed, 0);
            if number(&mut dice, 0, 60) != 0 {
                return seed;
            }
        }
        unreachable!("one seed in a hundred chats")
    }

    /// The hit points of the player 1.
    fn hp_of(world: &mut GameState) -> i32 {
        world
            .characters
            .find_by_vid_mut(Vid::new(1))
            .unwrap()
            .items_and_points_mut()
            .1
            .unwrap()
            .hp()
    }

    /// A monster in reach swings every 50 Pulses in legacy's record order (G6).
    #[test]
    fn a_monster_in_reach_swings_every_50_pulses_in_legacys_record_order() {
        let (clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        let hp = hp_of(&mut world);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        let seed = quiet_seed();
        world.dice = Pcg32::new(seed, 0);
        world.process_pulse(P0 + 1);
        let heard = drained(&mut alpha);
        assert_eq!(
            headers(&heard),
            [
                HEADER_GC_DAMAGE_INFO.value(),
                HEADER_GC_PLAYER_POINT_CHANGE.value(),
                HEADER_GC_OWNERSHIP.value(),
                HEADER_GC_CHARACTER_MOVE.value(),
            ]
        );
        let hit = GcDamageInfo::decode(&heard[0]).expect("a damage info");
        assert_eq!((hit.vid, hit.flag), (1, 1));
        assert!(
            (1..=5).contains(&hit.damage),
            "the <3 floor: {}",
            hit.damage
        );
        let changed = GcPointChange::decode(&heard[1]).expect("a point change");
        assert_eq!(
            (changed.vid, changed.change_type, changed.value),
            (
                1,
                u8::try_from(point::POINT_HP).unwrap(),
                i64::from(hp - hit.damage)
            )
        );
        assert_eq!(heard[2], ownership_record(Vid::new(ME), Vid::new(1)));
        let moved = GcCharacterMove::decode(&heard[3]).expect("a move");
        assert_eq!((moved.b_func, moved.dw_vid), (FUNC_ATTACK, ME));
        // The draw order: the chat, the damage roll, the `<3` floor's `number(1, 5)`.
        let mut expect = Pcg32::new(seed, 0);
        let _chat = number(&mut expect, 0, 60);
        let _roll = number(&mut expect, 0, 0);
        let _floor = number(&mut expect, 1, 5);
        assert_eq!(world.dice, expect, "three draws a swing");
        let mind = world.minds.get(ME).unwrap();
        assert_eq!((mind.next_pulse, mind.duration), (52, 50));
        // The next swing, 50 Pulses later: the 4000 ms wait at attack speed 0 is over.
        clock.set(T0 + 4100);
        let mut expect = world.dice.clone();
        let _chat = number(&mut expect, 0, 60);
        let _roll = number(&mut expect, 0, 0);
        let _floor = number(&mut expect, 1, 5);
        world.process_pulse(52);
        let heard = drained(&mut alpha);
        assert_eq!(heard[0][0], HEADER_GC_DAMAGE_INFO.value());
        assert_eq!(world.dice, expect, "three draws a swing again");
    }

    /// The attack timer waits out the remainder (G7).
    #[test]
    fn the_attack_timer_waits_out_the_remainder() {
        let (clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        world
            .npc_mut(ME)
            .unwrap()
            .fight
            .add(point::POINT_ATT_SPEED, 100);
        world.dice = Pcg32::new(quiet_seed(), 0);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        world.process_pulse(P0 + 1);
        assert_eq!(drained(&mut alpha)[0][0], HEADER_GC_DAMAGE_INFO.value());
        clock.set(T0 + 1500);
        world.process_pulse(P0 + 52);
        let mind = world.minds.get(ME).unwrap();
        assert_eq!((mind.next_pulse, mind.duration), (P0 + 64, 12));
        assert!(
            drained(&mut alpha).is_empty(),
            "no swing before the wait is over"
        );
    }

    /// A monster on a BANPK cell cannot swing and waits 12 (G8).
    #[test]
    fn a_monster_on_a_banpk_cell_cannot_swing_and_waits_12() {
        let clock = TestClock::default();
        clock.set(T0);
        let npc = a_fighter(ME, 406, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000);
        let mut world = stand_field(&clock, vec![npc], banpk_cells(3150..3250), true);
        let mut alpha = enter(&mut world, 1, NEAR, 12_345);
        world.start_pc_recovery(Vid::new(1));
        world.process_pulse(P0);
        drained(&mut alpha);
        world.begin_fight(ME, Vid::new(1));
        world.process_pulse(P0 + 1);
        let mind = world.minds.get(ME).unwrap();
        assert_eq!((mind.state, mind.duration), (Fsm::Battle, 12));
        assert!(
            !drained(&mut alpha)
                .iter()
                .any(|record| record[0] == HEADER_GC_DAMAGE_INFO.value()),
            "no swing from a BANPK cell"
        );
    }

    /// The swing on a stunned player kills it with the monster as killer (G24).
    #[test]
    fn the_swing_on_a_stunned_player_kills_it_with_the_monster_as_killer() {
        let (clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        world.dice = Pcg32::new(quiet_seed(), 0);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        world.process_pulse(P0 + 1);
        drained(&mut alpha);
        // The stun, due at 77; the next swing at 52 lands first.
        world.damage_pc(None, Vid::new(1), i32::MAX, None);
        assert!(world.is_down(1));
        drained(&mut alpha);
        clock.set(T0 + 4100);
        world.process_pulse(52);
        let heard = drained(&mut alpha);
        assert_eq!(
            headers(&heard),
            [
                HEADER_GC_OWNERSHIP.value(),
                HEADER_GC_OWNERSHIP.value(),
                HEADER_GC_PHASE.value(),
                HEADER_GC_DEAD.value(),
                HEADER_GC_DAMAGE_INFO.value(),
                HEADER_GC_CHARACTER_MOVE.value(),
            ]
        );
        // `ClearSync`: the player's own release, then what it owned (the monster its
        // punch claimed).
        assert_eq!(heard[0], ownership_record(Vid::new(0), Vid::new(1)));
        assert_eq!(heard[1], ownership_record(Vid::new(0), Vid::new(ME)));
        assert_eq!(
            heard[4],
            damage_info(1, super::super::fight::DAMAGE_BLOCK, 0)
        );
        let moved = GcCharacterMove::decode(&heard[5]).expect("a move");
        assert_eq!((moved.b_func, moved.dw_vid), (FUNC_ATTACK, ME));
        // The death has a killer that is not a player: the penalty flag is set.
        assert!(world
            .pc_combat
            .get(&1)
            .is_some_and(|combat| combat.death_penalty));
        assert!(world.pc_combat.get(&1).is_some_and(|combat| combat.dead));
    }

    /// A stun event and a swing due on one Pulse: the event kills first (G25).
    #[test]
    fn a_stun_event_and_a_swing_due_on_one_pulse() {
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        // The stun, due at 76; the punch lands at 75, so the first Battle update
        // is due on the same Pulse.
        world.damage_pc(None, Vid::new(1), i32::MAX, None);
        for pulse in P0 + 1..76 {
            world.process_pulse(pulse);
        }
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        world.process_pulse(76);
        let heard = drained(&mut alpha);
        // The stun event killed without a killer; Battle then saw the victim dead.
        assert!(world.pc_combat.get(&1).is_some_and(|combat| combat.dead));
        assert!(!world
            .pc_combat
            .get(&1)
            .is_some_and(|combat| combat.death_penalty));
        let mind = world.minds.get(ME).unwrap();
        assert_eq!((mind.state, world.victim_of(ME)), (Fsm::Idle, None));
        assert!(
            !heard
                .iter()
                .any(|record| record[0] == HEADER_GC_DAMAGE_INFO.value()),
            "no swing on a dead victim"
        );
        assert!(
            !heard.iter().any(|record| {
                record[0] == HEADER_GC_CHARACTER_MOVE.value()
                    && GcCharacterMove::decode(record)
                        .is_ok_and(|moved| moved.b_func == FUNC_ATTACK)
            }),
            "no attack move on a dead victim"
        );
    }

    /// A dead or destroyed monster releases the players it owns (G26).
    #[test]
    fn a_dead_or_destroyed_monster_releases_the_players_it_owns() {
        // Death: the claim, then a punch that kills.
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        world.dice = Pcg32::new(quiet_seed(), 0);
        hit(&mut world, 1, ME);
        world.process_pulse(P0 + 1);
        drained(&mut alpha);
        assert_eq!(
            world.bodies.get(&Vid::new(1)).unwrap().sync.owner,
            Some(Vid::new(ME))
        );
        world.npc_mut(ME).unwrap().hp = 1;
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        for pulse in P0 + 2..=77 {
            world.process_pulse(pulse);
        }
        let heard = drained(&mut alpha);
        let unowned = ownership_record(Vid::new(0), Vid::new(1));
        let freed = ownership_record(Vid::new(0), Vid::new(ME));
        let own_at = heard.iter().position(|record| *record == freed);
        let owned_at = heard.iter().position(|record| *record == unowned);
        assert!(
            own_at.is_some_and(|first| owned_at.is_some_and(|second| first < second)),
            "the monster's own release comes before what it owned: {heard:?}"
        );
        // Destroy: the claim, then the world takes the monster down.
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        world.dice = Pcg32::new(quiet_seed(), 0);
        assert!(world.mob_attack(ME, Vid::new(1)));
        drained(&mut alpha);
        world.destroy_npc(ME);
        let heard = drained(&mut alpha);
        let own_at = heard.iter().position(|record| *record == freed);
        let owned_at = heard.iter().position(|record| *record == unowned);
        assert!(
            own_at.is_some_and(|first| owned_at.is_some_and(|second| first < second)),
            "destroy releases the owned after the own: {heard:?}"
        );
    }

    /// A monster's claim on a player is kept by its owner and refused to others past 250 (G28).
    #[test]
    fn a_monsters_claim_on_a_player_is_kept_by_its_owner_and_refused_to_others_beyond_250() {
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        let victim = SyncPositionVictim {
            vid: Vid::new(1),
            kind: SyncPositionVictimKind::Player,
            x: NEAR.0,
            y: NEAR.1,
        };
        let now = world.clock.elapsed();
        let me = Vid::new(ME);
        let other = Vid::new(ME + 1);
        // The first claim is accepted and shown.
        assert!(world.claim_pc(me, &victim, SPOT, true, now));
        assert_eq!(drained(&mut alpha), [ownership_record(me, Vid::new(1))]);
        assert_eq!(world.bodies.get(&Vid::new(1)).unwrap().sync.owner, Some(me));
        // The owner past 250 keeps the claim with no record and no new stamp.
        world.bodies.get_mut(&Vid::new(1)).unwrap().last_sync = Some(Duration::from_millis(5));
        assert!(world.claim_pc(me, &victim, (NEAR.0 + 300, NEAR.1), true, now));
        assert!(drained(&mut alpha).is_empty());
        assert_eq!(
            world.bodies.get(&Vid::new(1)).unwrap().last_sync,
            Some(Duration::from_millis(5))
        );
        // Another monster past 250 is refused.
        assert!(!world.claim_pc(other, &victim, (NEAR.0 + 300, NEAR.1), true, now));
        assert!(drained(&mut alpha).is_empty());
        assert_eq!(world.bodies.get(&Vid::new(1)).unwrap().sync.owner, Some(me));
        // An owner change within 250 resets `last_sync`.
        let later = now + SYNC_CLAIM_LIFETIME;
        assert!(world.claim_pc(other, &victim, NEAR, true, later));
        assert_eq!(world.bodies.get(&Vid::new(1)).unwrap().last_sync, None);
        assert_eq!(
            world.bodies.get(&Vid::new(1)).unwrap().sync.owner,
            Some(other)
        );
        // A dead player gets the `DAMAGE_BLOCK`.
        drained(&mut alpha);
        world.damage_pc(None, Vid::new(1), i32::MAX, None);
        world.damage_pc(None, Vid::new(1), i32::MAX, None);
        assert!(world.is_dead(1));
        drained(&mut alpha);
        assert!(!world.claim_pc(me, &victim, SPOT, false, later));
        assert_eq!(
            drained(&mut alpha),
            [damage_info(1, super::super::fight::DAMAGE_BLOCK, 0)]
        );
    }

    /// The targeters see the player's hit points before its stun (G29).
    #[test]
    fn the_targeters_see_the_players_hit_points_before_its_stun() {
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        let mut bravo = enter(&mut world, 2, (3300, 3200), 12_346);
        world.target(Vid::new(2), 1);
        drained(&mut alpha);
        drained(&mut bravo);
        let hp = hp_of(&mut world);
        world.damage_pc(None, Vid::new(1), hp - 1, None);
        drained(&mut alpha);
        world.dice = Pcg32::new(quiet_seed(), 0);
        assert!(world.mob_attack(ME, Vid::new(1)));
        let heard = drained(&mut bravo);
        let target_at = heard
            .iter()
            .position(|record| record[0] == HEADER_GC_TARGET.value());
        let stun_at = heard
            .iter()
            .position(|record| record[0] == HEADER_GC_STUN.value());
        assert!(
            target_at.is_some_and(|first| stun_at.is_some_and(|second| first < second)),
            "the target flush comes before the stun: {heard:?}"
        );
    }

    /// A running player stops at the first monster hit until it moves again (G35).
    #[test]
    fn a_running_player_stops_at_the_first_monster_hit_until_it_moves_again() {
        const OTHER: u32 = FIRST_NPC_VID + 1;
        let clock = TestClock::default();
        clock.set(T0);
        let npcs = vec![
            a_fighter(ME, 406, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000),
            a_fighter(OTHER, 406, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000),
        ];
        let mut world = stand_field(&clock, npcs, open_cells(), true);
        let mut alpha = enter(&mut world, 1, NEAR, 12_345);
        world.start_pc_recovery(Vid::new(1));
        world.process_pulse(P0);
        drained(&mut alpha);
        world.dice = Pcg32::new(quiet_seed(), 0);
        assert!(world.goto(1, NEAR.0 + 2000, NEAR.1));
        clock.set(T0 + 500);
        world.process_pulse(P0 + 1);
        let frozen = world.spot_of(1).unwrap();
        assert_ne!((frozen.x, frozen.y), NEAR, "the player stepped");
        // The first landed hit freezes it: fighting, out of Move and out of the movers.
        assert!(world.mob_attack(ME, Vid::new(1)));
        assert!(world.bodies.get(&Vid::new(1)).unwrap().fighting);
        assert!(!world.bodies.get(&Vid::new(1)).unwrap().motion.moving);
        assert!(!world.movers.contains(&1));
        for pulse in (P0 + 2)..=(P0 + 5) {
            world.process_pulse(pulse);
        }
        let held = world.spot_of(1).unwrap();
        assert_eq!((held.x, held.y), (frozen.x, frozen.y));
        // The chase does not lead it: it is not moving.
        assert!(!world.victim_state(Vid::new(1)).unwrap().moving);
        // Its next move runs again; a second hit changes nothing.
        world.goto(1, NEAR.0 + 2000, NEAR.1);
        assert!(world.bodies.get(&Vid::new(1)).unwrap().motion.moving);
        assert!(world.mob_attack(ME, Vid::new(1)));
        assert!(world.bodies.get(&Vid::new(1)).unwrap().fighting);
        // Its own kill stands it down, and the next monster hit freezes it again.
        world.npc_mut(ME).unwrap().hp = 1;
        hit(&mut world, 1, ME);
        assert!(world.npc_by_vid(ME).is_some_and(|npc| npc.stunned));
        hit(&mut world, 1, ME);
        assert!(world.npc_by_vid(ME).is_some_and(|npc| npc.dead));
        assert!(!world.bodies.get(&Vid::new(1)).unwrap().fighting);
        assert!(world.mob_attack(OTHER, Vid::new(1)));
        assert!(world.bodies.get(&Vid::new(1)).unwrap().fighting);
        // Its death clears the flag too.
        world.damage_pc(None, Vid::new(1), i32::MAX, None);
        world.damage_pc(None, Vid::new(1), i32::MAX, None);
        assert!(world.is_dead(1));
        assert!(!world.bodies.get(&Vid::new(1)).unwrap().fighting);
    }
}
