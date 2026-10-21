//! A player's hit on a player.
//!
//! `battle_melee_attack` (`G/battle.cpp:154-230`) with a player victim, `battle_hit`
//! (`G/battle.cpp:760-818`), the victim's own `Damage` (`G/char_battle.cpp:1939-2267`) and
//! the claim `SetSyncOwner` makes (`G/char.cpp:5469-5555`).
//!
//! # What legacy does
//!
//! Two players have no reach: `battle_melee_attack` sets `MaxRange` false for a player
//! against a player (`G/battle.cpp:178-184`), so the gate alone refuses the swing. The
//! attacker turns to the victim and keeps it as its battle victim, and `battle_hit` runs:
//!
//! 1. `CalcMeleeDamage` with the wielded weapon's kind resist against the victim
//!    (`G/battle.cpp:489-666`, `:775-800`); a damage of 0 ends the swing before anything
//!    else runs;
//! 2. `NormalAttackAffect` (`G/battle.cpp:734-758`): the poison, the stun and the slow
//!    land on the victim;
//! 3. the victim's `Damage` (`G/char_battle.cpp:1939-2267`):
//!    - the block: the victim's `POINT_BLOCK` holds the hit, the damage packet shows 0
//!      with `DAMAGE_BLOCK`, and nothing else runs;
//!    - the reflect: the victim's `POINT_REFLECT_MELEE` share of the damage before the
//!      bonuses runs back at the attacker as a `DAMAGE_TYPE_SPECIAL` hit;
//!    - the critical and the penetrate: the attacker's shares less the victim's resists,
//!      the first doubling the damage and the second adding the victim's defence;
//!    - the steals, the recoveries and the mana burn;
//!    - the type switch's bonuses, the mall, the empire tenth and the user damage rate;
//!    - the damage packet, to the victim and to the attacker, then the hit-point change.
//!
//! The claim follows the swing: the victim's `SetSyncOwner` takes the attacker as its
//! owner, and `OnMove(true)` stamps the attack and ends the attacker's revive-invisible
//! affect.
//!
//! # Divergences
//!
//! - **The PK policy** (`G/pvp.cpp:378-529`): the Rewrite has no PK mode, no consent duel
//!   and no alignment, so two players of the same empire never fight
//!   ([`pk_eligibility`]). Two of another empire do.
//! - **The marriage bonus** the critical and the penetrate share for a non-player victim
//!   never applies: the victim is a player.
//! - **The death's PvP steps** wait for a later slice: the consent duel's end
//!   (`CPVPManager::Dead`, `G/pvp.cpp:335-376`), the quest kill and the war score
//!   (`G/char_battle.cpp:1280-1311`), the item drop penalty (`:1343`) and the empire point
//!   (`:1390`). The guild war score stands ([`super::guild`]).
//!
//! Everything here runs on the game thread only (ADR-0002).

use common::vid::Vid;
use protocol::gc_fields::GcCreateFly;
use protocol::gc_nested::GcDamageInfo;
use protocol::gc_vid::GcSpecialEffect;
use world::character::{
    normal_attack_affect_on_player, strike_pc_by_pc, AttackAffect, PvAction, PvAttacker, PvEnd,
    PvKind, PvStrike, PvVictim,
};

use super::death::{Attacker, PcDamageHead};
use super::fight::DAMAGE_BLOCK;
use super::view::EntityKey;
use super::GameState;
use crate::sync_position::{SyncPositionVictim, SyncPositionVictimKind};

impl GameState {
    /// `battle_hit` of the player under `attacker` on the player under `victim`
    /// (`G/battle.cpp:760-818`): the affects, then the victim's own `Damage`
    /// (`G/char_battle.cpp:1939-2267`) and what it shows. Returns the gold the hit
    /// stole, as the signed change the store applies.
    pub(super) fn pvp_strike(&mut self, attacker: Vid, victim: Vid, damage: i32) -> Option<i64> {
        // `NormalAttackAffect` (`G/battle.cpp:734-758`) runs before the victim's `Damage`.
        let affect = self.pvp_affect(attacker, victim);
        self.apply_pc_affect(victim, affect, Some(attacker));
        let strike = self.pvp_damage(attacker, victim, damage, PvKind::Normal)?;
        let mut stolen = None;
        match strike.end {
            PvEnd::Blocked => {
                // `SendDamagePacket(pAttacker, 0, DAMAGE_BLOCK)` (`G/char_battle.cpp:1950`).
                self.send_pc_damage_info_to(victim, Some(attacker), DAMAGE_BLOCK, 0);
            }
            PvEnd::Landed { damage, flag } => {
                // The reflect runs before the critical, the penetrate and the steals
                // (`G/char_battle.cpp:1977-1988`). The arm runs on the victim's share,
                // so a victim without one never answers the hit.
                if let Some(reflected) = strike.reflected {
                    self.pvp_reflect(victim, attacker, reflected);
                }
                for action in strike.actions {
                    stolen = self.pvp_action(attacker, victim, action).or(stolen);
                }
                // The victim's own damage packet and hit-point change
                // (`G/char_battle.cpp:2387-2418`).
                let killer = Attacker {
                    vid: attacker,
                    pc: true,
                };
                let head = self.damage_pc_head(Some(killer), victim, damage, Some(flag), false);
                match head {
                    PcDamageHead::Open { hp, damage } => {
                        self.damage_pc_tail(Some(killer), victim, hp, damage);
                    }
                    // `SetVictim(NULL)` and `battle_end` (`G/char_battle.cpp:307-311`): a
                    // hit that killed the victim stands the attacker down.
                    PcDamageHead::Died | PcDamageHead::Dead => {
                        if let Some(body) = self.bodies.get_mut(&attacker) {
                            body.fighting = false;
                        }
                    }
                    PcDamageHead::Gone => {}
                }
            }
        }
        stolen
    }

    /// `NormalAttackAffect` of the player under `attacker` on the player under `victim`
    /// (`G/battle.cpp:734-758`): the poison, the stun and the slow, in legacy's order.
    fn pvp_affect(&mut self, attacker: Vid, victim: Vid) -> AttackAffect {
        let flags = self.affect_flags_of(victim)[0];
        let Self {
            characters, dice, ..
        } = self;
        let Some(attacker_points) = characters
            .find_by_vid(attacker)
            .ok()
            .and_then(|character| character.points())
        else {
            return AttackAffect::default();
        };
        let Some(victim_points) = characters
            .find_by_vid(victim)
            .ok()
            .and_then(|character| character.points())
        else {
            return AttackAffect::default();
        };
        normal_attack_affect_on_player(
            dice,
            attacker_points,
            &PvVictim {
                points: victim_points,
                affect_flags: flags,
            },
        )
    }

    /// The victim's `Damage` (`G/char_battle.cpp:1939-2267`) with a player victim: the
    /// block, the reflect, the critical, the penetrate, the steals, the bonuses and the
    /// damage the hit takes.
    fn pvp_damage(
        &mut self,
        attacker: Vid,
        victim: Vid,
        damage: i32,
        kind: PvKind,
    ) -> Option<PvStrike> {
        let empire = self
            .bodies
            .get(&attacker)
            .map_or(0, |body| body.card.empire);
        let map = self.place_of(attacker.raw()).map_or(0, |place| place.1);
        let user_rate = self.user_damage_rate();
        let flags = self.affect_flags_of(victim)[0];
        let Self {
            characters, dice, ..
        } = self;
        let attacker_points = characters
            .find_by_vid(attacker)
            .ok()
            .and_then(|character| character.points())?;
        let victim_points = characters
            .find_by_vid(victim)
            .ok()
            .and_then(|character| character.points())?;
        Some(strike_pc_by_pc(
            dice,
            &PvAttacker {
                points: attacker_points,
                empire,
                map,
                user_rate,
            },
            &PvVictim {
                points: victim_points,
                affect_flags: flags,
            },
            damage,
            kind,
        ))
    }

    /// The reflect arm of the victim's `Damage` (`G/char_battle.cpp:1977-1988`): the
    /// victim's `POINT_REFLECT_MELEE` share of the damage before the bonuses runs back at
    /// the attacker as a `DAMAGE_TYPE_SPECIAL` hit, which takes neither the normal arm's
    /// block, critical, penetrate and steals nor the type switch's bonuses.
    fn pvp_reflect(&mut self, victim: Vid, attacker: Vid, damage: i32) {
        let Some(strike) = self.pvp_damage(victim, attacker, damage, PvKind::Special) else {
            return;
        };
        let PvEnd::Landed { damage, flag } = strike.end else {
            return;
        };
        let killer = Attacker {
            vid: victim,
            pc: true,
        };
        let head = self.damage_pc_head(Some(killer), attacker, damage, Some(flag), false);
        if let PcDamageHead::Open { hp, damage } = head {
            self.damage_pc_tail(Some(killer), attacker, hp, damage);
        }
    }

    /// What one of the victim's `Damage` actions does (`G/char_battle.cpp:1990-2122`).
    /// Returns the gold the action stole.
    fn pvp_action(&mut self, attacker: Vid, victim: Vid, action: PvAction) -> Option<i64> {
        match action {
            PvAction::Effect(effect_type) => {
                // `EffectPacket` (`G/char.cpp:4141-4160`): around the victim.
                let mut record = Vec::new();
                GcSpecialEffect {
                    effect_type,
                    vid: victim.raw(),
                }
                .encode_into(&mut record);
                self.packet_around(EntityKey::Character(victim.raw()), &record, None);
                None
            }
            PvAction::Fly(fly_type) => {
                // `CreateFly` (`G/char_battle.cpp:129-139`): from the victim to the
                // attacker, around the victim.
                let mut record = Vec::new();
                GcCreateFly::new(fly_type, victim.raw(), attacker.raw()).encode_into(&mut record);
                self.packet_around(EntityKey::Character(victim.raw()), &record, None);
                None
            }
            PvAction::Attacker { kind, amount } => {
                self.change_pc_point(attacker, kind, amount, false);
                None
            }
            PvAction::Victim { kind, amount } => {
                self.change_pc_point(victim, kind, amount, false);
                None
            }
            PvAction::Gold(amount) => self.steal_gold(attacker, amount),
        }
    }

    /// `SetSyncOwner` of the player under `victim` (`G/char.cpp:5469-5555`): the attacker
    /// becomes the victim's sync owner, and `OnMove(true)` (`G/char.cpp:6148-6179`) stamps
    /// the attack and ends the attacker's revive-invisible affect. The victim's own
    /// `OnMove()` stamps its move.
    pub(super) fn pvp_claim(&mut self, attacker: Vid, victim: Vid) {
        let now = self.clock.now();
        let elapsed = self.clock.elapsed();
        let (Some(spot), Some(victim_spot)) =
            (self.spot_of(attacker.raw()), self.spot_of(victim.raw()))
        else {
            return;
        };
        let sync_victim = SyncPositionVictim {
            vid: victim,
            kind: SyncPositionVictimKind::Player,
            x: victim_spot.x,
            y: victim_spot.y,
        };
        // `battle_is_attackable` (`G/battle.cpp:99-152`): a dead victim or a downed
        // attacker refuses the claim, and the refusal shows the block. A stunned victim
        // is still attackable: legacy's `Damage` kills it at once.
        let attackable = !self
            .pc_combat
            .get(&victim.raw())
            .is_some_and(|combat| combat.dead)
            && !self.is_down(attacker.raw());
        let _claimed = self.claim_pc(
            attacker,
            &sync_victim,
            (spot.x, spot.y),
            attackable,
            elapsed,
        );
        if let Some(body) = self.bodies.get_mut(&attacker) {
            body.last_move = now;
            body.last_attack = Some(now);
        }
        self.remove_revive_invisible(attacker);
        if let Some(body) = self.bodies.get_mut(&victim) {
            body.last_move = now;
        }
    }

    /// `SendDamagePacket` of the player under `victim` (`G/char_battle.cpp:1720-1742`):
    /// the record goes to the victim and, when the attacker is a player, to the attacker.
    pub(super) fn send_pc_damage_info_to(
        &self,
        victim: Vid,
        attacker: Option<Vid>,
        flag: u8,
        damage: i32,
    ) {
        let mut record = Vec::new();
        GcDamageInfo {
            header: GcDamageInfo::header(),
            vid: victim.raw(),
            flag,
            damage,
        }
        .encode_into(&mut record);
        let _victim = self.write_to_client(victim, record.clone());
        if let Some(attacker) = attacker {
            let _attacker = self.write_to_client(attacker, record);
        }
    }
}
