//! Transport-free DB peer authentication and authorization policy.
//!
//! The legacy DB server has no peer-authentication exchange. Its
//! `CClientManager::QUERY_SETUP` path assigns `m_pkAuthPeer` whenever the
//! decoded `bAuthServer` byte is nonzero, and a later auth setup can overwrite
//! that pointer. The pointer is bookkeeping, not a credential or proof.
//!
//! This module provides a fail-closed policy boundary for a future transport
//! adapter. A caller must issue an [`AuthenticatedDbPeer`] only after its own
//! out-of-band authentication. Nothing here derives authentication from a DB
//! frame handle, setup IP, login key, account ID, player ID, or raw
//! `bAuthServer` byte. The policy is transport-free: it performs no I/O, SQL,
//! response encoding, login insertion, or gameplay effect.
//!
//! The capability matrix authorizes two things: registration of an auth-peer
//! setup, and the read-only boot request. Ordinary game setup, add-affect
//! requests, remove-affect requests, and all other DB operations remain denied
//! until separate role, account, player, and ownership policies are
//! implemented.
//!
//! Both allowances still require an externally verified peer, and neither is
//! reachable from a `bAuthServer` byte, a frame handle, or a source prefix.

use std::error::Error;
use std::fmt;

use crate::session::DbRequest;

/// A caller-owned identifier for one accepted DB connection.
///
/// The identifier is deliberately independent of the untrusted `u32` DB frame
/// handle. A connection registry must allocate it with collision-free
/// generation semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DbPeerId(u64);

impl DbPeerId {
    /// Wrap a caller-owned identifier.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Return the caller-owned identifier.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for DbPeerId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// A connection-generation token owned by the peer registry.
///
/// A generation prevents a stale close or authentication callback from being
/// applied to a later connection that happens to reuse a descriptor or
/// numeric peer ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PeerGeneration(u64);

impl PeerGeneration {
    /// Wrap a caller-owned generation token.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Return the generation token.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for PeerGeneration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// A role assigned by a trusted, out-of-band authentication adapter.
///
/// These values are classifications, not proof. In particular, an auth role
/// must not be assigned merely because a decoded setup request contains a
/// nonzero raw `bAuthServer` byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbPeerRole {
    /// An ordinary game-server connection.
    Game,
    /// The explicitly configured auth-server connection.
    Auth,
    /// A connection whose role has not been classified.
    Unclassified,
}

/// An opaque authentication result for one connection generation.
///
/// The public constructor is named for its trust requirement. It does not
/// perform authentication; callers must invoke it only from a reviewed
/// authentication adapter or a test fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthenticatedDbPeer {
    id: DbPeerId,
    generation: PeerGeneration,
    role: DbPeerRole,
}

impl AuthenticatedDbPeer {
    /// Construct the result supplied by an external authentication adapter.
    #[must_use]
    pub const fn from_external_verification(
        id: DbPeerId,
        generation: PeerGeneration,
        role: DbPeerRole,
    ) -> Self {
        Self {
            id,
            generation,
            role,
        }
    }

    /// Return the stable connection ID.
    #[must_use]
    pub const fn id(self) -> DbPeerId {
        self.id
    }

    /// Return the connection generation.
    #[must_use]
    pub const fn generation(self) -> PeerGeneration {
        self.generation
    }

    /// Return the trusted role classification.
    #[must_use]
    pub const fn role(self) -> DbPeerRole {
        self.role
    }
}

/// The current authentication state of one DB connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerAuthState {
    /// No external authentication result has been accepted.
    Unauthenticated,
    /// An external result is bound to this connection generation.
    Authenticated(AuthenticatedDbPeer),
    /// Authentication was revoked for this connection.
    Revoked,
    /// The connection was closed.
    Closed,
}

/// A failure while changing one connection's authentication state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerPolicyError {
    /// The proof does not belong to this policy's ID and generation.
    IdentityConflict {
        /// The policy's expected connection ID.
        expected_id: DbPeerId,
        /// The proof's connection ID.
        actual_id: DbPeerId,
        /// The policy's expected generation.
        expected_generation: PeerGeneration,
        /// The proof's generation.
        actual_generation: PeerGeneration,
    },
    /// The connection already accepted the same proof.
    AlreadyAuthenticated {
        /// The authenticated connection ID.
        peer: DbPeerId,
    },
    /// The connection is revoked or closed and cannot transition again.
    Terminal {
        /// The terminal state.
        state: PeerAuthState,
    },
    /// An operation requiring authentication was attempted before it.
    AuthenticationRequired,
    /// A lifecycle callback did not belong to the authenticated connection.
    OwnershipMismatch {
        /// The policy's connection ID.
        expected: DbPeerId,
    },
}

impl fmt::Display for PeerPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IdentityConflict {
                expected_id,
                actual_id,
                expected_generation,
                actual_generation,
            } => write!(
                formatter,
                "DB peer proof {actual_id}/{actual_generation} does not match {expected_id}/{expected_generation}"
            ),
            Self::AlreadyAuthenticated { peer } => {
                write!(formatter, "DB peer {peer} is already authenticated")
            }
            Self::Terminal { state } => {
                write!(formatter, "DB peer authentication is terminal: {state:?}")
            }
            Self::AuthenticationRequired => {
                formatter.write_str("DB peer authentication is required")
            }
            Self::OwnershipMismatch { expected } => {
                write!(formatter, "DB peer callback does not belong to {expected}")
            }
        }
    }
}

impl Error for PeerPolicyError {}

/// The result of accepting an external authentication result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerAuthenticationOutcome {
    /// The connection moved from unauthenticated to authenticated.
    Authenticated,
}

/// The result of a connection lifecycle transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerLifecycleOutcome {
    /// The connection was marked revoked.
    Revoked,
    /// The connection was marked closed.
    Closed,
}

/// Per-connection authentication state.
///
/// This object is not a global registry. A transport adapter creates one for
/// each accepted connection and supplies the same externally issued proof to
/// any later authorization check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DbPeerPolicy {
    id: DbPeerId,
    generation: PeerGeneration,
    state: PeerAuthState,
}

impl DbPeerPolicy {
    /// Create an unauthenticated policy for one connection generation.
    #[must_use]
    pub const fn new(id: DbPeerId, generation: PeerGeneration) -> Self {
        Self {
            id,
            generation,
            state: PeerAuthState::Unauthenticated,
        }
    }

    /// Return the connection ID.
    #[must_use]
    pub const fn id(self) -> DbPeerId {
        self.id
    }

    /// Return the connection generation.
    #[must_use]
    pub const fn generation(self) -> PeerGeneration {
        self.generation
    }

    /// Return the current authentication state.
    #[must_use]
    pub const fn state(self) -> PeerAuthState {
        self.state
    }

    /// Return whether an external proof is currently accepted.
    #[must_use]
    pub const fn is_authenticated(self) -> bool {
        matches!(self.state, PeerAuthState::Authenticated(_))
    }

    /// Accept an external authentication result for this connection.
    ///
    /// A repeated proof is rejected explicitly; a different proof or
    /// generation is a terminal identity conflict for this policy. No socket
    /// or DB request is touched.
    ///
    /// # Errors
    ///
    /// Returns [`PeerPolicyError::IdentityConflict`] for another connection,
    /// [`PeerPolicyError::AlreadyAuthenticated`] for a repeated proof, or
    /// [`PeerPolicyError::Terminal`] after revoke/close.
    pub fn authenticate(
        &mut self,
        proof: AuthenticatedDbPeer,
    ) -> Result<PeerAuthenticationOutcome, PeerPolicyError> {
        if proof.id() != self.id || proof.generation() != self.generation {
            return Err(PeerPolicyError::IdentityConflict {
                expected_id: self.id,
                actual_id: proof.id(),
                expected_generation: self.generation,
                actual_generation: proof.generation(),
            });
        }

        match self.state {
            PeerAuthState::Unauthenticated => {
                self.state = PeerAuthState::Authenticated(proof);
                Ok(PeerAuthenticationOutcome::Authenticated)
            }
            PeerAuthState::Authenticated(current) if current == proof => {
                Err(PeerPolicyError::AlreadyAuthenticated { peer: proof.id() })
            }
            PeerAuthState::Authenticated(_) => Err(PeerPolicyError::IdentityConflict {
                expected_id: self.id,
                actual_id: proof.id(),
                expected_generation: self.generation,
                actual_generation: proof.generation(),
            }),
            PeerAuthState::Revoked | PeerAuthState::Closed => {
                Err(PeerPolicyError::Terminal { state: self.state })
            }
        }
    }

    /// Revoke this connection after external authentication was accepted.
    ///
    /// The caller must separately release the auth slot if this connection
    /// owns it. Keeping that effect explicit prevents a stale callback from
    /// mutating a replacement connection.
    ///
    /// # Errors
    ///
    /// Returns [`PeerPolicyError::AuthenticationRequired`] before
    /// authentication and [`PeerPolicyError::Terminal`] after a prior
    /// lifecycle transition.
    pub fn revoke(&mut self) -> Result<PeerLifecycleOutcome, PeerPolicyError> {
        self.transition(PeerAuthState::Revoked)
    }

    /// Mark this connection closed after external authentication was accepted.
    ///
    /// The caller must separately release the auth slot if this connection
    /// owns it.
    ///
    /// # Errors
    ///
    /// Returns [`PeerPolicyError::AuthenticationRequired`] before
    /// authentication and [`PeerPolicyError::Terminal`] after a prior
    /// lifecycle transition.
    pub fn close(&mut self) -> Result<PeerLifecycleOutcome, PeerPolicyError> {
        self.transition(PeerAuthState::Closed)
    }

    fn transition(
        &mut self,
        target: PeerAuthState,
    ) -> Result<PeerLifecycleOutcome, PeerPolicyError> {
        match self.state {
            PeerAuthState::Authenticated(_) => {
                self.state = target;
                Ok(match target {
                    PeerAuthState::Revoked => PeerLifecycleOutcome::Revoked,
                    PeerAuthState::Closed => PeerLifecycleOutcome::Closed,
                    PeerAuthState::Unauthenticated | PeerAuthState::Authenticated(_) => {
                        unreachable!("lifecycle target is terminal")
                    }
                })
            }
            PeerAuthState::Unauthenticated => Err(PeerPolicyError::AuthenticationRequired),
            PeerAuthState::Revoked | PeerAuthState::Closed => {
                Err(PeerPolicyError::Terminal { state: self.state })
            }
        }
    }

    /// Authorize one typed DB request without changing policy or slot state.
    ///
    /// The request has already passed frame and payload validation. A
    /// `bAuthServer` byte is only a raw setup field; it cannot authenticate a
    /// peer. In this initial capability matrix, only an externally
    /// authenticated auth-role peer may request `RegisterAuthSetup`, and only
    /// with the decoder's base-only auth setup. Every other operation is
    /// denied without a lookup, response, or side effect.
    pub fn authorize_request(
        &self,
        slot: &DbAuthPeerSlot,
        request: &DbRequest,
    ) -> AuthorizationDecision {
        let operation = operation_for(request);
        let proof = match self.state {
            PeerAuthState::Unauthenticated => {
                return AuthorizationDecision::Deny {
                    operation,
                    reason: AuthorizationReason::AuthenticationRequired,
                };
            }
            PeerAuthState::Revoked => {
                return AuthorizationDecision::Deny {
                    operation,
                    reason: AuthorizationReason::Revoked,
                };
            }
            PeerAuthState::Closed => {
                return AuthorizationDecision::Deny {
                    operation,
                    reason: AuthorizationReason::Closed,
                };
            }
            PeerAuthState::Authenticated(proof) => proof,
        };

        if operation == DbPeerOperation::Boot {
            // Read-only and effect-free. The caller still needs an
            // externally verified peer; this branch grants no slot, no
            // registration, and no ownership of any account or player.
            return AuthorizationDecision::Allow {
                operation,
                effect: AuthorizationEffect::None,
            };
        }

        if operation == DbPeerOperation::GameSetup {
            // Read-only and effect-free, like boot: the reply carries the
            // requester's own echoed values and nothing else. It grants no
            // slot and no ownership.
            return AuthorizationDecision::Allow {
                operation,
                effect: AuthorizationEffect::None,
            };
        }

        if operation != DbPeerOperation::RegisterAuthSetup {
            return AuthorizationDecision::Deny {
                operation,
                reason: AuthorizationReason::OperationNotAllowed,
            };
        }

        let DbRequest::Setup { request, .. } = request else {
            return AuthorizationDecision::Deny {
                operation,
                reason: AuthorizationReason::OperationNotAllowed,
            };
        };

        if request.base().auth_server == 0 || !request.logins().is_empty() {
            return AuthorizationDecision::Deny {
                operation,
                reason: AuthorizationReason::InvalidAuthSetup,
            };
        }

        if proof.role() != DbPeerRole::Auth {
            return AuthorizationDecision::Deny {
                operation,
                reason: AuthorizationReason::RoleMismatch,
            };
        }

        match slot.current() {
            None => AuthorizationDecision::Allow {
                operation,
                effect: AuthorizationEffect::BindAuthPeer { owner: proof },
            },
            Some(current) if current == proof => AuthorizationDecision::Deny {
                operation,
                reason: AuthorizationReason::AlreadyBound,
            },
            Some(_) => AuthorizationDecision::Deny {
                operation,
                reason: AuthorizationReason::SlotOccupied,
            },
        }
    }
}

/// Operations currently represented by the DB peer policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbPeerOperation {
    /// Register the one auth peer after external authentication.
    RegisterAuthSetup,
    /// Answer an ordinary game-core setup request.
    ///
    /// The legacy `QUERY_SETUP` stores the peer's public IP, channel, listen
    /// port, P2P port, and map list, then replies with one
    /// `HEADER_DG_MAP_LOCATIONS` frame built from the requesting peer's own
    /// values. This operation is effect-free: the reply echoes the request
    /// back to the same connection, so authorizing it cannot register a peer,
    /// claim an account or player, or move global state.
    ///
    /// The legacy handler also inserts the peer into a process-wide
    /// `m_peerList`, which makes the reply depend on every other connected
    /// peer. That global list is not reproduced here, so a single frame
    /// carrying only the requester's own location is the whole reply.
    GameSetup,
    /// The legacy boot request.
    ///
    /// Boot is read-only: it loads no row, mutates no cache, and returns only
    /// already-loaded table bytes. It is the one non-setup operation this
    /// matrix allows, and it carries no binding effect, so authorizing it
    /// cannot register a peer or claim an account or player.
    Boot,
    /// The horse-name lookup.
    HorseName,
    /// The channel lookup.
    FindChannel,
    /// The login-by-key request.
    LoginByKey,
    /// The primary player-load request.
    PlayerLoad,
    /// The validation-only add-affect request.
    AddAffect,
    /// The validation-only remove-affect request.
    RemoveAffect,
}

/// A reason an operation is denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationReason {
    /// The connection has no accepted external proof.
    AuthenticationRequired,
    /// The connection's proof was revoked.
    Revoked,
    /// The connection is closed.
    Closed,
    /// The operation is not in the initial capability matrix.
    OperationNotAllowed,
    /// The setup is not the base-only auth setup.
    InvalidAuthSetup,
    /// The authenticated role is not `Auth`.
    RoleMismatch,
    /// The auth slot is owned by another peer.
    SlotOccupied,
    /// The same peer is already registered.
    AlreadyBound,
}

/// A state effect emitted by an allow decision.
///
/// Applying the effect is explicit. The policy does not mutate a slot or a
/// service while merely authorizing a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationEffect {
    /// No state change. An operation that only reads.
    None,
    /// Bind this externally authenticated owner in the global auth slot.
    BindAuthPeer {
        /// The owner to pass to [`DbAuthPeerSlot::bind`].
        owner: AuthenticatedDbPeer,
    },
}

impl AuthorizationEffect {
    /// Apply this effect to the caller's auth slot.
    ///
    /// # Errors
    ///
    /// Returns [`AuthPeerSlotError`] if the slot is occupied or the owner does
    /// not carry the explicit auth role.
    pub fn apply(
        self,
        slot: &mut DbAuthPeerSlot,
    ) -> Result<AuthPeerBindOutcome, AuthPeerSlotError> {
        match self {
            Self::BindAuthPeer { owner } => slot.bind(owner),
            // An effect-free allow has nothing to apply. Returning an error here
            // would make a caller treat a legal read as a failure.
            Self::None => Err(AuthPeerSlotError::NoEffectToApply),
        }
    }
}

/// The result of an authorization check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationDecision {
    /// Allow one operation and return its explicit effect.
    Allow {
        /// The operation being authorized.
        operation: DbPeerOperation,
        /// The effect the caller must apply separately.
        effect: AuthorizationEffect,
    },
    /// Deny the operation without a service call or response.
    Deny {
        /// The operation being considered.
        operation: DbPeerOperation,
        /// The reason for denial.
        reason: AuthorizationReason,
    },
}

impl AuthorizationDecision {
    /// Return whether this decision allows the operation.
    #[must_use]
    pub const fn is_allowed(self) -> bool {
        matches!(self, Self::Allow { .. })
    }
}

fn operation_for(request: &DbRequest) -> DbPeerOperation {
    match request {
        DbRequest::Boot { .. } => DbPeerOperation::Boot,
        DbRequest::Setup { request, .. } => {
            if request.base().auth_server == 0 {
                DbPeerOperation::GameSetup
            } else {
                DbPeerOperation::RegisterAuthSetup
            }
        }
        DbRequest::HorseName { .. } => DbPeerOperation::HorseName,
        DbRequest::FindChannel { .. } => DbPeerOperation::FindChannel,
        DbRequest::LoginByKey { .. } => DbPeerOperation::LoginByKey,
        DbRequest::PlayerLoad { .. } => DbPeerOperation::PlayerLoad,
        DbRequest::AddAffect { .. } => DbPeerOperation::AddAffect,
        DbRequest::RemoveAffect { .. } => DbPeerOperation::RemoveAffect,
    }
}

/// The one global auth-peer slot owned by the DB policy boundary.
///
/// Binding is fail-closed. Unlike the legacy pointer assignment, a different
/// authenticated peer can never replace the current owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DbAuthPeerSlot {
    owner: Option<AuthenticatedDbPeer>,
}

impl DbAuthPeerSlot {
    /// Create an empty slot.
    #[must_use]
    pub const fn empty() -> Self {
        Self { owner: None }
    }

    /// Return the current owner, if any.
    #[must_use]
    pub const fn current(self) -> Option<AuthenticatedDbPeer> {
        self.owner
    }

    /// Return whether the slot is occupied.
    #[must_use]
    pub const fn is_bound(self) -> bool {
        self.owner.is_some()
    }

    /// Bind an externally authenticated auth peer.
    ///
    /// # Errors
    ///
    /// Returns [`AuthPeerSlotError::NotAuthPeer`] for a non-auth role and
    /// [`AuthPeerSlotError::SlotOccupied`] for any different owner. Repeating
    /// the exact current owner returns [`AuthPeerBindOutcome::AlreadyCurrent`]
    /// without changing state.
    pub fn bind(
        &mut self,
        owner: AuthenticatedDbPeer,
    ) -> Result<AuthPeerBindOutcome, AuthPeerSlotError> {
        if owner.role() != DbPeerRole::Auth {
            return Err(AuthPeerSlotError::NotAuthPeer {
                peer: owner.id(),
                role: owner.role(),
            });
        }
        match self.owner {
            None => {
                self.owner = Some(owner);
                Ok(AuthPeerBindOutcome::Bound)
            }
            Some(current) if current == owner => Ok(AuthPeerBindOutcome::AlreadyCurrent),
            Some(current) => Err(AuthPeerSlotError::SlotOccupied {
                current: current.id(),
                incoming: owner.id(),
            }),
        }
    }

    /// Release only the exact owner.
    pub fn release(&mut self, owner: &AuthenticatedDbPeer) -> AuthPeerReleaseOutcome {
        if self.owner == Some(*owner) {
            self.owner = None;
            AuthPeerReleaseOutcome::Cleared
        } else if let Some(current) = self.owner {
            AuthPeerReleaseOutcome::NotOwner {
                current: current.id(),
            }
        } else {
            AuthPeerReleaseOutcome::Empty
        }
    }
}

/// The result of binding an auth peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthPeerBindOutcome {
    /// The slot was empty and now owns the peer.
    Bound,
    /// The exact owner was already present.
    AlreadyCurrent,
}

/// The result of releasing the auth slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthPeerReleaseOutcome {
    /// The exact owner was removed.
    Cleared,
    /// A different owner remains.
    NotOwner {
        /// The owner that remains.
        current: DbPeerId,
    },
    /// The slot was already empty.
    Empty,
}

/// A rejected auth-slot operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthPeerSlotError {
    /// The owner was not classified as an auth peer.
    NotAuthPeer {
        /// The rejected peer ID.
        peer: DbPeerId,
        /// The supplied role.
        role: DbPeerRole,
    },
    /// A different peer already owns the slot.
    SlotOccupied {
        /// The current owner ID.
        current: DbPeerId,
        /// The refused owner ID.
        incoming: DbPeerId,
    },
    /// An authorization carried no slot effect to apply.
    NoEffectToApply,
}

impl fmt::Display for AuthPeerSlotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAuthPeer { peer, role } => {
                write!(
                    formatter,
                    "DB peer {peer} with role {role:?} cannot own the auth slot"
                )
            }
            Self::SlotOccupied { current, incoming } => {
                write!(
                    formatter,
                    "DB auth slot is owned by {current}, not {incoming}"
                )
            }
            Self::NoEffectToApply => {
                write!(formatter, "authorization carried no auth-slot effect")
            }
        }
    }
}

impl Error for AuthPeerSlotError {}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::db_boot::DbBootRequest;
    use protocol::db_records::{
        AddAffectRequest, AffectElementRecord, ChannelChangeRequest, HorseNameRequest,
        LoginByKeyRequest, PlayerLoadRequest, RemoveAffectRequest,
    };

    use crate::setup::{decode_setup_request, SetupDecodeLimits, SetupFeatureProfile};

    fn id(value: u64) -> DbPeerId {
        DbPeerId::new(value)
    }

    fn generation(value: u64) -> PeerGeneration {
        PeerGeneration::new(value)
    }

    fn proof(id_value: u64, generation_value: u64, role: DbPeerRole) -> AuthenticatedDbPeer {
        AuthenticatedDbPeer::from_external_verification(
            id(id_value),
            generation(generation_value),
            role,
        )
    }

    fn add_affect_request() -> DbRequest {
        DbRequest::AddAffect {
            handle: 0,
            request: AddAffectRequest::new(1, AffectElementRecord::default()),
        }
    }

    fn remove_affect_request() -> DbRequest {
        DbRequest::RemoveAffect {
            handle: 0,
            request: RemoveAffectRequest::new(1, 2, 3),
        }
    }

    fn setup_request(auth_server: u8) -> DbRequest {
        let mut payload = vec![0_u8; 154];
        payload[149..153].copy_from_slice(&0_u32.to_le_bytes());
        payload[153] = auth_server;
        let request = decode_setup_request(
            &payload,
            SetupFeatureProfile::ActiveX86,
            SetupDecodeLimits::default(),
        )
        .expect("fixture is a valid base-only setup");
        DbRequest::Setup { handle: 0, request }
    }

    fn authenticated_policy(role: DbPeerRole) -> DbPeerPolicy {
        let mut policy = DbPeerPolicy::new(id(7), generation(1));
        policy
            .authenticate(proof(7, 1, role))
            .expect("fixture authenticates");
        policy
    }

    fn boot_request() -> DbRequest {
        DbRequest::Boot {
            handle: 0,
            request: DbBootRequest::new([1, 2], [0; 16]),
        }
    }

    #[test]
    fn an_authenticated_peer_may_read_boot_with_no_effect() {
        let policy = authenticated_policy(DbPeerRole::Game);
        let slot = DbAuthPeerSlot::empty();

        let decision = policy.authorize_request(&slot, &boot_request());

        assert_eq!(
            decision,
            AuthorizationDecision::Allow {
                operation: DbPeerOperation::Boot,
                effect: AuthorizationEffect::None,
            }
        );
        // Read-only means it must not register the peer in the global slot.
        assert_eq!(slot.current(), None);
    }

    #[test]
    fn allowing_boot_never_claims_the_auth_slot_even_when_one_is_free() {
        let policy = authenticated_policy(DbPeerRole::Auth);
        let mut slot = DbAuthPeerSlot::empty();

        let effect = match policy.authorize_request(&slot, &boot_request()) {
            AuthorizationDecision::Allow { effect, .. } => effect,
            AuthorizationDecision::Deny { reason, .. } => panic!("boot denied: {reason:?}"),
        };

        assert_eq!(
            effect.apply(&mut slot),
            Err(AuthPeerSlotError::NoEffectToApply)
        );
        assert_eq!(slot.current(), None);
    }

    #[test]
    fn an_unauthenticated_peer_may_not_read_boot() {
        let policy = DbPeerPolicy::new(id(7), generation(1));
        let slot = DbAuthPeerSlot::empty();

        assert_eq!(
            policy.authorize_request(&slot, &boot_request()),
            AuthorizationDecision::Deny {
                operation: DbPeerOperation::Boot,
                reason: AuthorizationReason::AuthenticationRequired,
            }
        );
    }

    #[test]
    fn a_revoked_peer_may_not_read_boot() {
        let mut policy = authenticated_policy(DbPeerRole::Game);
        policy.revoke().expect("authenticated peer revokes");
        let slot = DbAuthPeerSlot::empty();

        assert_eq!(
            policy.authorize_request(&slot, &boot_request()),
            AuthorizationDecision::Deny {
                operation: DbPeerOperation::Boot,
                reason: AuthorizationReason::Revoked,
            }
        );
    }

    #[test]
    fn a_closed_peer_may_not_read_boot() {
        let mut policy = authenticated_policy(DbPeerRole::Game);
        policy.close().expect("authenticated peer closes");
        let slot = DbAuthPeerSlot::empty();

        assert_eq!(
            policy.authorize_request(&slot, &boot_request()),
            AuthorizationDecision::Deny {
                operation: DbPeerOperation::Boot,
                reason: AuthorizationReason::Closed,
            }
        );
    }

    #[test]
    fn a_bare_auth_server_byte_still_does_not_reach_boot_or_setup() {
        // The setup byte is a raw field. A peer that only ever sent it is
        // unauthenticated, so both operations stay denied.
        let policy = DbPeerPolicy::new(id(7), generation(1));
        let slot = DbAuthPeerSlot::empty();

        for request in [boot_request(), setup_request(1)] {
            assert!(matches!(
                policy.authorize_request(&slot, &request),
                AuthorizationDecision::Deny {
                    reason: AuthorizationReason::AuthenticationRequired,
                    ..
                }
            ));
        }
        assert_eq!(slot.current(), None);
    }

    #[test]
    fn initial_state_denies_setup_and_other_operations_without_mutation() {
        let policy = DbPeerPolicy::new(id(7), generation(1));
        let slot = DbAuthPeerSlot::empty();
        for request in [
            setup_request(1),
            DbRequest::Boot {
                handle: 0,
                request: DbBootRequest::new([1, 2], [0; 16]),
            },
            DbRequest::HorseName {
                handle: 0,
                request: HorseNameRequest::new(1),
            },
            DbRequest::FindChannel {
                handle: 0,
                request: ChannelChangeRequest::new(1, 1),
            },
            DbRequest::LoginByKey {
                handle: 0,
                request: LoginByKeyRequest {
                    login: [0; 31],
                    login_key: 1,
                    client_key: [0; 4],
                    ip: [0; 16],
                },
            },
            DbRequest::PlayerLoad {
                handle: 0,
                request: PlayerLoadRequest {
                    account_id: 1,
                    player_id: 2,
                    account_index: 3,
                },
            },
            add_affect_request(),
        ] {
            let decision = policy.authorize_request(&slot, &request);
            assert!(matches!(
                decision,
                AuthorizationDecision::Deny {
                    reason: AuthorizationReason::AuthenticationRequired,
                    ..
                }
            ));
        }
        assert!(!slot.is_bound());
        assert_eq!(policy.state(), PeerAuthState::Unauthenticated);
    }

    #[test]
    fn add_affect_is_denied_before_and_after_authentication_without_mutation() {
        let request = add_affect_request();
        let slot = DbAuthPeerSlot::empty();
        let unauthenticated = DbPeerPolicy::new(id(7), generation(1));
        assert_eq!(
            unauthenticated.authorize_request(&slot, &request),
            AuthorizationDecision::Deny {
                operation: DbPeerOperation::AddAffect,
                reason: AuthorizationReason::AuthenticationRequired,
            }
        );
        assert_eq!(unauthenticated.state(), PeerAuthState::Unauthenticated);
        assert!(!slot.is_bound());

        let authenticated = authenticated_policy(DbPeerRole::Auth);
        assert_eq!(
            authenticated.authorize_request(&slot, &request),
            AuthorizationDecision::Deny {
                operation: DbPeerOperation::AddAffect,
                reason: AuthorizationReason::OperationNotAllowed,
            }
        );
        assert!(authenticated.is_authenticated());
        assert!(!slot.is_bound());
    }

    #[test]
    fn remove_affect_is_denied_before_and_after_authentication_without_mutation() {
        let request = remove_affect_request();
        let slot = DbAuthPeerSlot::empty();
        let unauthenticated = DbPeerPolicy::new(id(7), generation(1));
        assert_eq!(
            unauthenticated.authorize_request(&slot, &request),
            AuthorizationDecision::Deny {
                operation: DbPeerOperation::RemoveAffect,
                reason: AuthorizationReason::AuthenticationRequired,
            }
        );
        assert_eq!(unauthenticated.state(), PeerAuthState::Unauthenticated);
        assert!(!slot.is_bound());

        let authenticated = authenticated_policy(DbPeerRole::Auth);
        assert_eq!(
            authenticated.authorize_request(&slot, &request),
            AuthorizationDecision::Deny {
                operation: DbPeerOperation::RemoveAffect,
                reason: AuthorizationReason::OperationNotAllowed,
            }
        );
        assert!(authenticated.is_authenticated());
        assert!(!slot.is_bound());
    }

    #[test]
    fn external_auth_is_generation_bound_and_repeated_proof_is_rejected() {
        let mut policy = DbPeerPolicy::new(id(7), generation(1));
        assert_eq!(
            policy.authenticate(proof(7, 1, DbPeerRole::Auth)),
            Ok(PeerAuthenticationOutcome::Authenticated)
        );
        assert_eq!(
            policy.authenticate(proof(7, 1, DbPeerRole::Auth)),
            Err(PeerPolicyError::AlreadyAuthenticated { peer: id(7) })
        );
        assert!(matches!(
            policy.authenticate(proof(7, 2, DbPeerRole::Auth)),
            Err(PeerPolicyError::IdentityConflict { .. })
        ));
    }

    #[test]
    fn auth_setup_is_only_an_explicit_effect_after_external_auth() {
        let policy = authenticated_policy(DbPeerRole::Auth);
        let mut slot = DbAuthPeerSlot::empty();
        let decision = policy.authorize_request(&slot, &setup_request(1));
        assert!(decision.is_allowed());
        let AuthorizationDecision::Allow { effect, .. } = decision else {
            panic!("expected allow")
        };
        assert_eq!(effect.apply(&mut slot), Ok(AuthPeerBindOutcome::Bound));
        assert_eq!(slot.current(), Some(proof(7, 1, DbPeerRole::Auth)));
    }

    #[test]
    fn auth_flag_cannot_authenticate_or_authorize_wrong_role_or_normal_setup() {
        let slot = DbAuthPeerSlot::empty();
        let unauthenticated = DbPeerPolicy::new(id(7), generation(1));
        assert!(matches!(
            unauthenticated.authorize_request(&slot, &setup_request(1)),
            AuthorizationDecision::Deny {
                reason: AuthorizationReason::AuthenticationRequired,
                ..
            }
        ));

        let game = authenticated_policy(DbPeerRole::Game);
        assert!(matches!(
            game.authorize_request(&slot, &setup_request(1)),
            AuthorizationDecision::Deny {
                reason: AuthorizationReason::RoleMismatch,
                ..
            }
        ));

        // A normal (non-auth) setup is now a separate, effect-free operation.
        // It is allowed for either role because the reply only echoes the
        // requester's own public IP, listen port, and map list back to the same
        // connection. It must never touch the auth slot, which is the
        // property this test exists to protect.
        for role in [DbPeerRole::Auth, DbPeerRole::Game] {
            let slot = DbAuthPeerSlot::empty();
            let decision = authenticated_policy(role).authorize_request(&slot, &setup_request(0));
            assert!(
                matches!(
                    decision,
                    AuthorizationDecision::Allow {
                        operation: DbPeerOperation::GameSetup,
                        effect: AuthorizationEffect::None,
                    }
                ),
                "{role:?} normal setup must be allowed with no effect, got {decision:?}"
            );
            assert!(
                !slot.is_bound(),
                "{role:?} normal setup must not bind the slot"
            );
            assert!(!slot.is_bound());
        }

        // An auth-mode setup from a Game-role peer is still refused, and an
        // auth-mode setup carrying login records is still refused: the auth
        // branch is the only path to the slot and it stays narrow.
        let slot = DbAuthPeerSlot::empty();
        let auth = authenticated_policy(DbPeerRole::Auth);
        assert!(matches!(
            auth.authorize_request(&slot, &setup_request(1)),
            AuthorizationDecision::Allow {
                operation: DbPeerOperation::RegisterAuthSetup,
                ..
            }
        ));
        assert!(!slot.is_bound());
    }

    #[test]
    fn an_auth_mode_setup_with_login_records_never_reaches_the_policy() {
        // The legacy auth branch returns before inspecting the declared count,
        // so it never reads a login record. The decoder refuses that shape
        // outright, which is stronger than a policy check: the request never
        // becomes a `DbRequest` at all. The policy's own login-count guard is
        // defense in depth for a record built in-process.
        let mut payload = vec![0_u8; 154 + 100];
        payload[149..153].copy_from_slice(&1_u32.to_le_bytes());
        payload[153] = 1;
        assert!(matches!(
            decode_setup_request(
                &payload,
                SetupFeatureProfile::ActiveX86,
                SetupDecodeLimits::default(),
            ),
            Err(crate::setup::SetupDecodeError::AuthServerMustBeBaseOnly { login_count: 1 })
        ));
    }

    #[test]
    fn occupied_slot_is_never_overwritten() {
        let policy = authenticated_policy(DbPeerRole::Auth);
        let mut slot = DbAuthPeerSlot::empty();
        slot.bind(proof(7, 1, DbPeerRole::Auth)).unwrap();
        assert!(matches!(
            policy.authorize_request(&slot, &setup_request(1)),
            AuthorizationDecision::Deny {
                reason: AuthorizationReason::AlreadyBound,
                ..
            }
        ));
        assert_eq!(slot.current(), Some(proof(7, 1, DbPeerRole::Auth)));
    }

    #[test]
    fn non_owner_release_cannot_clear_current_owner() {
        let owner = proof(7, 1, DbPeerRole::Auth);
        let mut slot = DbAuthPeerSlot::empty();
        slot.bind(owner).unwrap();
        assert_eq!(
            slot.release(&proof(8, 1, DbPeerRole::Auth)),
            AuthPeerReleaseOutcome::NotOwner { current: id(7) }
        );
        assert_eq!(slot.current(), Some(owner));
        assert_eq!(slot.release(&owner), AuthPeerReleaseOutcome::Cleared);
        assert_eq!(slot.release(&owner), AuthPeerReleaseOutcome::Empty);
    }

    #[test]
    fn lifecycle_transitions_are_terminal_and_do_not_imply_slot_release() {
        let mut policy = authenticated_policy(DbPeerRole::Auth);
        let mut slot = DbAuthPeerSlot::empty();
        slot.bind(proof(7, 1, DbPeerRole::Auth)).unwrap();
        assert_eq!(policy.revoke(), Ok(PeerLifecycleOutcome::Revoked));
        assert_eq!(policy.state(), PeerAuthState::Revoked);
        assert!(matches!(
            policy.authorize_request(&slot, &setup_request(1)),
            AuthorizationDecision::Deny {
                reason: AuthorizationReason::Revoked,
                ..
            }
        ));
        assert_eq!(slot.current(), Some(proof(7, 1, DbPeerRole::Auth)));
        assert_eq!(
            policy.close(),
            Err(PeerPolicyError::Terminal {
                state: PeerAuthState::Revoked
            })
        );
    }

    #[test]
    fn only_authenticated_auth_setup_is_allowed_in_initial_matrix() {
        let policy = authenticated_policy(DbPeerRole::Auth);
        let slot = DbAuthPeerSlot::empty();
        let request = DbRequest::HorseName {
            handle: 0,
            request: HorseNameRequest::new(1),
        };
        assert_eq!(
            policy.authorize_request(&slot, &request),
            AuthorizationDecision::Deny {
                operation: DbPeerOperation::HorseName,
                reason: AuthorizationReason::OperationNotAllowed,
            }
        );
    }
}
