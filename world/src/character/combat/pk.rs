use super::super::{CharacterKind, Posture};

/// Player-killing modes used by the eligibility policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PkMode {
    /// Same-empire attacks require an existing consensual fight.
    Peace,
    /// Allows alignment-based revenge attacks.
    Revenge,
    /// Allows attacks outside the attacker's guild.
    Guild,
    /// Allows same-empire attacks.
    Free,
    /// Disallows attacks while protected by empire-map policy.
    Protect,
}

/// Immutable combatant facts consumed by PK eligibility.
/// The legacy policy has several independent state flags; keeping them
/// explicit mirrors the source policy and avoids an incomplete enum conversion.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PkActor {
    /// Stable identity used to reject self-targeting.
    pub id: u32,
    /// Runtime character category.
    pub kind: CharacterKind,
    /// Current liveness posture.
    pub posture: Posture,
    /// Whether observer mode is active.
    pub observer: bool,
    /// Whether the actor occupies a no-PK location.
    pub ban_pk: bool,
    /// Character empire.
    pub empire: u8,
    /// Empire owning the current map, or zero for neutral maps.
    pub map_empire: u8,
    /// Active PK mode.
    pub mode: PkMode,
    /// Current alignment value.
    pub alignment: i32,
    /// Whether other players may attack this actor as a killer.
    pub killer_mode: bool,
    /// Party identity when grouped.
    pub party_id: Option<u32>,
    /// Guild identity when affiliated.
    pub guild_id: Option<u32>,
    /// Whether an agreed duel or equivalent fight is active.
    pub consensual_fight: bool,
}

/// Explicit reasons an attack is ineligible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PkDenial {
    /// No target was supplied.
    InvalidTarget,
    /// Attacker and target are the same actor.
    SelfTarget,
    /// The attacker is dead.
    DeadAttacker,
    /// The target is dead.
    DeadTarget,
    /// Either position forbids PK combat.
    BanPk,
    /// Either player is observing rather than participating.
    Observer,
    /// Empire-map protection applies.
    Protected,
    /// Both players belong to the same party.
    SameParty,
    /// The selected PK mode does not permit the attack.
    PkMode,
}

/// Eligibility and attacker-side outcome of PK evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PkEligibility {
    /// The attack is permitted.
    Allowed {
        /// Whether the attacker enters killer mode.
        enter_killer_mode: bool,
    },
    /// The attack is rejected for a typed reason.
    Denied(PkDenial),
}

/// Evaluates attackability and same-empire PK mode policy.
#[must_use]
pub fn pk_eligibility(
    attacker: PkActor,
    target: Option<PkActor>,
    protect_normal_player: bool,
) -> PkEligibility {
    let Some(target) = target else {
        return PkEligibility::Denied(PkDenial::InvalidTarget);
    };
    if attacker.id == target.id {
        return PkEligibility::Denied(PkDenial::SelfTarget);
    }
    if matches!(target.posture, Posture::Dead) {
        return PkEligibility::Denied(PkDenial::DeadTarget);
    }
    if matches!(attacker.posture, Posture::Dead) {
        return PkEligibility::Denied(PkDenial::DeadAttacker);
    }
    if attacker.ban_pk || target.ban_pk {
        return PkEligibility::Denied(PkDenial::BanPk);
    }
    if !matches!(attacker.kind, CharacterKind::Player)
        || !matches!(target.kind, CharacterKind::Player)
    {
        return allowed(false);
    }
    if attacker.observer || target.observer {
        return PkEligibility::Denied(PkDenial::Observer);
    }
    if protected_on_map(attacker) || protected_on_map(target) {
        return PkEligibility::Denied(PkDenial::Protected);
    }
    if attacker.empire != target.empire {
        return if matches!(attacker.mode, PkMode::Protect) || matches!(target.mode, PkMode::Protect)
        {
            PkEligibility::Denied(PkDenial::Protected)
        } else {
            allowed(false)
        };
    }
    if attacker.party_id.is_some() && attacker.party_id == target.party_id {
        return PkEligibility::Denied(PkDenial::SameParty);
    }
    if target.killer_mode {
        return allowed(false);
    }
    if protect_normal_player
        && attacker.alignment < 0
        && target.alignment >= 0
        && matches!(target.mode, PkMode::Peace)
    {
        return PkEligibility::Denied(PkDenial::PkMode);
    }

    let mode_result = match attacker.mode {
        PkMode::Revenge if attacker.guild_id != target.guild_id => {
            if attacker.alignment < 0 && target.alignment >= 0 {
                Some(allowed(true))
            } else if attacker.alignment >= 0 && target.alignment < 0 {
                Some(allowed(false))
            } else {
                None
            }
        }
        PkMode::Guild if attacker.guild_id.is_none() || attacker.guild_id != target.guild_id => {
            Some(allowed(target.alignment >= 0 || attacker.alignment < 0))
        }
        PkMode::Free => Some(allowed(target.alignment >= 0 || attacker.alignment < 0)),
        PkMode::Peace | PkMode::Revenge | PkMode::Guild | PkMode::Protect => None,
    };
    match mode_result {
        Some(result) => result,
        None if attacker.consensual_fight || target.consensual_fight => allowed(false),
        None => PkEligibility::Denied(PkDenial::PkMode),
    }
}

const fn protected_on_map(actor: PkActor) -> bool {
    matches!(actor.mode, PkMode::Protect) && actor.empire == actor.map_empire
}

const fn allowed(enter_killer_mode: bool) -> PkEligibility {
    PkEligibility::Allowed { enter_killer_mode }
}
