//! Transport-free account and player state reduction for one client descriptor.
//!
//! This module models only the source-verified account/player seam. It owns
//! one descriptor's pending login and player requests, their generation
//! correlations, and the account/player bindings that survive a successful
//! database response. It emits typed requests and binding/phase effects for a
//! caller-owned transport to apply.
//!
//! It does not execute SQL, own a DB peer, synthesize client packets, open a
//! socket, install a key, perform TEA, create a world character, run a quest,
//! or mutate a player cache. Key installation is represented as an explicit
//! source-profile effect; the adapter decides whether the legacy
//! `SetSecurityKey` call is enabled by its build. The account response is
//! considered only after a caller has supplied the source-shaped account
//! record. The local phase is
//! deliberately paired with, but not merged into, `ClientLifecycle`; the
//! caller must apply each `TransitionPhase` effect to that adjacent adapter
//! after the preceding bind/effect. Its local `Login` phase is not a check
//! of the adjacent lifecycle phase: the caller must verify
//! `ClientLifecycle::phase() == ClientPhase::Login` before `on_login`.
//! Bind and phase effects are ordered intents; the caller must apply them
//! transactionally so a rejected lifecycle transition cannot leave the
//! adjacent descriptor state ahead of this reducer. In practice, clone the
//! reducer before the event, apply the adjacent lifecycle transition to the
//! returned candidate state, and commit both states only after every effect is
//! accepted. If that transition is rejected, discard the candidate and close
//! the descriptor; this reducer has no rollback operation for an already-mutated
//! `&mut self` call. The location/world work performed by the legacy
//! `CInputDB::PlayerLoad` path remains outside this boundary.

use std::error::Error;
use std::fmt;

use protocol::db_records::{
    DbRecordError, LoginAccountRecord, LoginAlreadyRecord, LoginByKeyRequest, PlayerLoadRequest,
    PlayerResultRecord, LEGACY_ACCOUNT_STATUS_BYTES, LEGACY_IP_BYTES, LEGACY_LOGIN_BYTES,
    LEGACY_PLAYER_PER_ACCOUNT,
};
use protocol::db_wire::DbFrame;

use crate::lifecycle::PostHandshakePhase;

/// A session-seeded correlation for one account/player DB request.
///
/// The legacy DB envelope carries only the descriptor `u32` handle. That
/// handle can be reused for several in-flight requests and even for a later
/// descriptor lifetime, so a transport must retain this full generation in
/// its own completion table and pass it back here. A completion with an old
/// generation is rejected instead of being applied to a newer request.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AccountCorrelationId(u64);

impl AccountCorrelationId {
    /// Creates a correlation identifier supplied by a transport.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the numeric generation.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Normalized key-login material used to construct the legacy DB request.
///
/// `input_login.cpp:153-218` trims and lowercases the client login before it
/// stores descriptor key material and copies the fixed-width fields into
/// `TPacketGDLoginByKey`. This type intentionally accepts already-normalized
/// bytes; it does not guess a locale, encoding, or string policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccountLoginInput {
    /// Normalized `szLogin[LOGIN_MAX_LEN + 1]` bytes. `on_login` requires a
    /// NUL and an all-zero tail for deterministic fixed-width projection.
    pub login: [u8; LEGACY_LOGIN_BYTES],
    /// `dwLoginKey`.
    pub login_key: u32,
    /// Four `adwClientKey` values.
    pub client_key: [u32; 4],
    /// Descriptor host bytes copied to `szIP`. `on_login` requires a NUL and
    /// an all-zero tail for deterministic fixed-width projection.
    pub ip: [u8; LEGACY_IP_BYTES],
}

/// The all-zero value is a canonical, empty normalized field set.
///
/// `Default` is retained for tests and explicit zero-initialization. It does
/// not validate credentials or admission: an empty login is intentionally
/// forwarded to the DB boundary, which remains responsible for the source
/// `NOID` result and any credential policy.
impl Default for AccountLoginInput {
    fn default() -> Self {
        Self {
            login: [0; LEGACY_LOGIN_BYTES],
            login_key: 0,
            client_key: [0; 4],
            ip: [0; LEGACY_IP_BYTES],
        }
    }
}

impl AccountLoginInput {
    /// Construct normalized key-login material.
    #[must_use]
    pub const fn new(
        login: [u8; LEGACY_LOGIN_BYTES],
        login_key: u32,
        client_key: [u32; 4],
        ip: [u8; LEGACY_IP_BYTES],
    ) -> Self {
        Self {
            login,
            login_key,
            client_key,
            ip,
        }
    }

    /// Return whether the fixed login field contains a NUL terminator.
    ///
    /// Lowercasing and trimming remain the adapter's responsibility because
    /// this boundary cannot infer the legacy locale or encoding. The
    /// terminator check prevents an accepted request from silently losing the
    /// source `strlcpy` boundary.
    #[must_use]
    pub const fn has_login_terminator(self) -> bool {
        let mut index = 0;
        while index < self.login.len() {
            if self.login[index] == 0 {
                return true;
            }
            index += 1;
        }
        false
    }

    /// Return whether the fixed login field has a NUL followed by an all-zero
    /// tail. The canonical-tail rule makes the fixed-width Rust request
    /// deterministic instead of retaining indeterminate legacy stack bytes.
    #[must_use]
    pub const fn has_canonical_login(self) -> bool {
        has_canonical_c_string(&self.login)
    }

    /// Return whether the fixed host/IP field contains a NUL terminator.
    ///
    /// The adapter supplies the descriptor host name. Requiring a terminator
    /// preserves the legacy `strlcpy` projection into `szIP` without trying to
    /// canonicalize an address here.
    #[must_use]
    pub const fn has_ip_terminator(self) -> bool {
        let mut index = 0;
        while index < self.ip.len() {
            if self.ip[index] == 0 {
                return true;
            }
            index += 1;
        }
        false
    }

    /// Return whether the fixed host/IP field has a NUL followed by an
    /// all-zero tail.
    #[must_use]
    pub const fn has_canonical_ip(self) -> bool {
        has_canonical_c_string(&self.ip)
    }

    /// Return the normalized key material that the descriptor boundary would
    /// retain before sending the login request.
    #[must_use]
    pub const fn key_material(self) -> AccountLoginKeyMaterial {
        AccountLoginKeyMaterial {
            login: self.login,
            login_key: self.login_key,
            client_key: self.client_key,
        }
    }

    /// Project the input into the exact 67-byte packed legacy request.
    #[must_use]
    pub const fn request(self) -> LoginByKeyRequest {
        LoginByKeyRequest {
            login: self.login,
            login_key: self.login_key,
            client_key: self.client_key,
            ip: self.ip,
        }
    }
}

/// Normalized key material retained for the descriptor-level adapter.
///
/// This is state, not an installed cryptographic key. The caller may hand it
/// to the existing descriptor boundary in the same order as the legacy
/// `SetLoginKey` and `SetSecurityKey` calls, but this module never derives or
/// installs a key itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccountLoginKeyMaterial {
    /// Normalized fixed-width login bytes.
    pub login: [u8; LEGACY_LOGIN_BYTES],
    /// Source `dwLoginKey` value.
    pub login_key: u32,
    /// Four source `adwClientKey` values.
    pub client_key: [u32; 4],
}

/// A source-resolved primary login-by-key response.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum AccountLoginOutcome {
    /// A successful `TAccountTable` response.
    Success(LoginAccountRecord),
    /// A source-resolved header-31 response with no payload.
    Missing,
    /// A source-resolved header-34 response with its exact raw login bytes.
    AlreadyLoggedIn(LoginAlreadyRecord),
}

/// A source-resolved primary player-load response.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum AccountPlayerOutcome {
    /// A successful `TPlayerTable` response.
    Success(PlayerResultRecord),
    /// A source-resolved header-36 response with no payload.
    Missing,
}

/// A typed login completion retained with its DB handle and generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountLoginCompletion {
    /// DB peer descriptor handle.
    pub handle: u32,
    /// Generation returned by the matching request effect.
    pub correlation: AccountCorrelationId,
    /// Resolved primary outcome.
    pub outcome: AccountLoginOutcome,
}

/// A typed player completion retained with its DB handle and generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountPlayerCompletion {
    /// DB peer descriptor handle.
    pub handle: u32,
    /// Generation returned by the matching request effect.
    pub correlation: AccountCorrelationId,
    /// Resolved primary outcome.
    pub outcome: AccountPlayerOutcome,
}

/// The state of the account/player portion of one descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountPlayerPhase {
    /// Waiting for the initial key login request.
    Login,
    /// An account is bound and character selection is allowed.
    Select,
    /// A player is bound and the legacy loading phase is delegated.
    Loading,
    /// The enter-game input has delegated the game phase.
    Game,
    /// The descriptor account/player state is closed.
    Closed,
}

/// The source operation used in a phase, error, or completion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountDbOperation {
    /// `HEADER_GD_LOGIN_BY_KEY`.
    Login,
    /// `HEADER_GD_PLAYER_LOAD`.
    PlayerLoad,
    /// The client `HEADER_CG_ENTERGAME` phase input.
    EnterGame,
}

/// The source build condition controlling client security-key installation.
///
/// The legacy handler always calls `SetLoginKey`, but calls `SetSecurityKey`
/// only when `_IMPROVED_PACKET_ENCRYPTION_` is not defined. The reducer does
/// not infer this compile-time profile from request bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountKeyInstallation {
    /// The legacy path with `_IMPROVED_PACKET_ENCRYPTION_` disabled.
    LegacySecurityKey,
    /// The improved-encryption path, where `SetSecurityKey` is skipped.
    ImprovedPacketEncryption,
}

impl AccountKeyInstallation {
    /// Return whether the adapter must install the four-word client security
    /// key after retaining the login key.
    #[must_use]
    pub const fn installs_client_security_key(self) -> bool {
        matches!(self, Self::LegacySecurityKey)
    }
}

/// A typed effect emitted by the account/player reducer.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum AccountPlayerEffect {
    /// Retain normalized login/key material for the descriptor boundary. The
    /// adapter must always apply `SetLoginKey`; it must apply
    /// `SetSecurityKey` only when `key_installation` is
    /// `LegacySecurityKey`. This effect does not install a key in the reducer.
    BindLoginKeyMaterial {
        /// Exact normalized material to retain.
        material: AccountLoginKeyMaterial,
        /// Explicit source build policy for client security-key installation.
        key_installation: AccountKeyInstallation,
    },
    /// Send the exact 67-byte login-by-key request through the caller's DB
    /// port. The transport must preserve the descriptor handle and generation.
    SendLoginByKey {
        /// Descriptor handle to put in the legacy DB envelope.
        handle: u32,
        /// Generation to retain with the asynchronous request.
        correlation: AccountCorrelationId,
        /// Exact packed request payload.
        request: LoginByKeyRequest,
    },
    /// Send the exact nine-byte player-load request through the caller's DB
    /// port. The transport must preserve both identifiers.
    SendPlayerLoad {
        /// Descriptor handle to put in the legacy DB envelope.
        handle: u32,
        /// Generation to retain with the asynchronous request.
        correlation: AccountCorrelationId,
        /// Exact packed request payload.
        request: PlayerLoadRequest,
    },
    /// Bind a validated account record after a positive login response.
    BindAccount {
        /// The complete source-shaped account record.
        account: LoginAccountRecord,
    },
    /// Bind a validated player record after a positive player response.
    BindPlayer {
        /// The account correlation selected by the request.
        account_id: u32,
        /// The source player ID selected by the request.
        player_id: u32,
        /// The account slot selected by the request.
        account_index: u8,
        /// The complete source-shaped player record.
        player: PlayerResultRecord,
    },
    /// Ask the caller's DB adapter to send the legacy logout request before a
    /// non-OK account failure. The reducer does not encode or transmit it.
    /// This effect carries only the source login field; the legacy
    /// `TLogoutPacket` password bytes are intentionally not fabricated. A
    /// descriptor teardown that requires complete legacy password bytes needs
    /// a separate full-account logout boundary.
    RequestLogout {
        /// Descriptor handle used by the DB request.
        handle: u32,
        /// `strlcpy`-bounded login bytes with a guaranteed trailing NUL for
        /// the legacy logout record.
        login: [u8; LEGACY_LOGIN_BYTES],
    },
    /// Apply a source-defined failure response without synthesizing its wire
    /// packet. The status bytes are retained for the caller's adapter.
    LoginFailure {
        /// `strlcpy`-bounded status bytes with a guaranteed trailing NUL.
        status: [u8; LEGACY_ACCOUNT_STATUS_BYTES],
    },
    /// The DB reported an already-active account. The caller must apply its
    /// descriptor/P2P disconnect action before the following failure effect.
    AlreadyLoggedIn {
        /// Exact raw `TPacketDGLoginAlready` login bytes. The adapter must
        /// make a NUL-aware bounded C-string copy before lookup, P2P, or client
        /// packet projection; the raw array is not a Rust string.
        record: LoginAlreadyRecord,
    },
    /// Apply a phase change to the adjacent transport-free lifecycle adapter.
    /// Client login-success packets and profile-dependent packet ordering stay
    /// in the caller-owned adapter.
    TransitionPhase {
        /// Target post-handshake phase.
        phase: PostHandshakePhase,
    },
    /// Clear local descriptor state and request a transport close. The caller
    /// still owns legacy character, map, DB-logout, and P2P teardown.
    Close,
}

/// A pending key-login request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingLogin {
    /// Generation assigned by this reducer.
    pub correlation: AccountCorrelationId,
    /// Exact request that must be sent.
    pub request: LoginByKeyRequest,
    /// Explicit source policy retained for an exact retry.
    pub key_installation: AccountKeyInstallation,
}

/// A pending player-load request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingPlayer {
    /// Generation assigned by this reducer.
    pub correlation: AccountCorrelationId,
    /// Exact request that must be sent.
    pub request: PlayerLoadRequest,
}

/// State and effects returned by one account/player event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountPlayerReduction {
    /// Candidate state after the event. Apply adjacent lifecycle effects
    /// before committing this value to the live reducer.
    pub state: AccountPlayerSession,
    /// Effects in source-observed order.
    pub effects: Vec<AccountPlayerEffect>,
}

/// A transport-free account/player reducer for one descriptor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountPlayerSession {
    descriptor_handle: u32,
    phase: AccountPlayerPhase,
    next_correlation: u32,
    correlation_seed: u32,
    pending_login: Option<PendingLogin>,
    login_key_material: Option<AccountLoginKeyMaterial>,
    account: Option<LoginAccountRecord>,
    pending_player: Option<PendingPlayer>,
    player: Option<PlayerResultRecord>,
}

impl AccountPlayerSession {
    /// Create a session for one legacy descriptor handle.
    ///
    /// This is a one-shot/test convenience constructor with correlation seed
    /// zero. It is **not safe for production descriptor-handle reuse**: a
    /// production transport must use [`Self::new_with_correlation_seed`] with a
    /// globally fresh seed (with collision-free allocation or tombstones), or
    /// retain a tombstone that prevents an old completion from being routed here.
    #[must_use]
    pub const fn new(descriptor_handle: u32) -> Self {
        Self::new_with_correlation_seed(descriptor_handle, 0)
    }

    /// Create a session with a caller-supplied correlation seed.
    ///
    /// The seed occupies the high 32 bits of the opaque generation and the
    /// per-session sequence occupies the low 32 bits. Reusing a descriptor
    /// handle with a different seed prevents an old completion from matching
    /// the new session's first request. The wire still carries only the legacy
    /// `u32` handle; the adapter must retain and route the full correlation.
    /// This API does not enforce seed uniqueness: production callers must use
    /// a collision-free allocator or retain a tombstone/routing registry that
    /// prevents seed reuse.
    #[must_use]
    pub const fn new_with_correlation_seed(descriptor_handle: u32, seed: u32) -> Self {
        Self {
            descriptor_handle,
            phase: AccountPlayerPhase::Login,
            next_correlation: 1,
            correlation_seed: seed,
            pending_login: None,
            login_key_material: None,
            account: None,
            pending_player: None,
            player: None,
        }
    }

    /// Return the caller-supplied correlation seed for diagnostics and routing.
    #[must_use]
    pub const fn correlation_seed(&self) -> u32 {
        self.correlation_seed
    }

    /// Return the descriptor handle used in DB effects.
    #[must_use]
    pub const fn descriptor_handle(&self) -> u32 {
        self.descriptor_handle
    }

    /// Return the current account/player phase.
    #[must_use]
    pub const fn phase(&self) -> AccountPlayerPhase {
        self.phase
    }

    /// Borrow the retained normalized login key material, if any.
    #[must_use]
    pub const fn login_key_material(&self) -> Option<&AccountLoginKeyMaterial> {
        self.login_key_material.as_ref()
    }

    /// Borrow the currently bound account, if any.
    #[must_use]
    pub const fn account(&self) -> Option<&LoginAccountRecord> {
        self.account.as_ref()
    }

    /// Borrow the currently bound player, if any.
    #[must_use]
    pub const fn player(&self) -> Option<&PlayerResultRecord> {
        self.player.as_ref()
    }

    /// Borrow the pending login, if any.
    #[must_use]
    pub const fn pending_login(&self) -> Option<&PendingLogin> {
        self.pending_login.as_ref()
    }

    /// Borrow the pending player request, if any.
    #[must_use]
    pub const fn pending_player(&self) -> Option<&PendingPlayer> {
        self.pending_player.as_ref()
    }

    /// Start a normalized key-login request using the legacy security-key
    /// policy.
    ///
    /// This convenience method selects [`AccountKeyInstallation::LegacySecurityKey`]
    /// for callers that are reproducing the ordinary legacy build. It does not
    /// inspect the payload to choose a profile. Production code that can use
    /// improved packet encryption should call
    /// [`Self::on_login_with_key_installation`] with an explicit policy.
    ///
    /// The caller must first verify that the adjacent
    /// `ClientLifecycle` is actually in `ClientPhase::Login`; this reducer's
    /// local `AccountPlayerPhase::Login` does not distinguish it from Auth or
    /// Handshake. The caller must then run the legacy blocked-IP, shutdown, and
    /// user-limit admission checks before calling either login method. A rejected
    /// admission must not call this method, because it would retain key
    /// material and emit a DB request. The input's normalized login and
    /// host/IP fields must each contain the NUL terminator and canonical zero
    /// tail required by the legacy `strlcpy` copy. The returned effects retain
    /// normalized key material before the exact `TPacketGDLoginByKey` record
    /// and newly allocated generation. A second login request is rejected while
    /// one is pending; accepting it would make same-handle completions
    /// ambiguous.
    ///
    /// # Errors
    ///
    /// Returns an error when the session is closed, the phase is not
    /// `Login`, another login is pending, an account is already bound, the
    /// normalized login or host/IP field is unterminated/non-canonical, or
    /// the local correlation space is exhausted.
    pub fn on_login(
        &mut self,
        input: AccountLoginInput,
    ) -> Result<AccountPlayerReduction, AccountPlayerError> {
        self.on_login_with_key_installation(input, AccountKeyInstallation::LegacySecurityKey)
    }

    /// Start a normalized key-login request with an explicit source key
    /// installation policy.
    ///
    /// The policy is carried in [`AccountPlayerEffect::BindLoginKeyMaterial`].
    /// The caller must apply `SetLoginKey` for every request, and must apply
    /// `SetSecurityKey` only for [`AccountKeyInstallation::LegacySecurityKey`].
    /// No compile-time encryption profile is inferred from the request bytes.
    /// The adjacent lifecycle must actually be in `ClientPhase::Login`, and the
    /// caller must run blocked-IP, shutdown, and user-limit admission checks
    /// before invoking this method. The same candidate-state staging rule as
    /// [`Self::on_login`] applies to later phase effects.
    ///
    /// # Errors
    ///
    /// Returns the same state, validation, and correlation errors as
    /// [`Self::on_login`].
    pub fn on_login_with_key_installation(
        &mut self,
        input: AccountLoginInput,
        key_installation: AccountKeyInstallation,
    ) -> Result<AccountPlayerReduction, AccountPlayerError> {
        self.require_open(AccountDbOperation::Login)?;
        self.require_phase(AccountDbOperation::Login, AccountPlayerPhase::Login)?;
        if self.pending_login.is_some() {
            return Err(AccountPlayerError::LoginAlreadyPending);
        }
        if self.account.is_some() {
            return Err(AccountPlayerError::AccountAlreadyBound);
        }
        if !input.has_login_terminator() {
            return Err(AccountPlayerError::LoginNotTerminated);
        }
        if !input.has_ip_terminator() {
            return Err(AccountPlayerError::IpNotTerminated);
        }
        if !input.has_canonical_login() {
            return Err(AccountPlayerError::LoginNotCanonical);
        }
        if !input.has_canonical_ip() {
            return Err(AccountPlayerError::IpNotCanonical);
        }

        let correlation = self.allocate_correlation()?;
        let request = input.request();
        let material = input.key_material();
        self.pending_login = Some(PendingLogin {
            correlation,
            request,
            key_installation,
        });
        self.login_key_material = Some(material);
        Ok(self.reduction(vec![
            AccountPlayerEffect::BindLoginKeyMaterial {
                material,
                key_installation,
            },
            AccountPlayerEffect::SendLoginByKey {
                handle: self.descriptor_handle,
                correlation,
                request,
            },
        ]))
    }

    /// Rebuild the ordered login effects for a transport retry without
    /// allocating a new request or changing the pending generation.
    ///
    /// Call this after [`Self::on_login_error`] instead of reconstructing a
    /// request from the wire record. The returned bind effect includes the
    /// retained key-installation policy, so an adapter cannot accidentally
    /// replay only the DB packet and lose the `SetSecurityKey` decision.
    ///
    /// # Errors
    ///
    /// Returns `LoginNotPending` when no login request is retained. A closed
    /// session returns an empty vector because the late retry is ignored.
    pub fn retry_login_effects(&self) -> Result<Vec<AccountPlayerEffect>, AccountPlayerError> {
        if self.phase == AccountPlayerPhase::Closed {
            return Ok(Vec::new());
        }
        let pending = self
            .pending_login
            .as_ref()
            .ok_or(AccountPlayerError::LoginNotPending)?;
        let material = self
            .login_key_material
            .ok_or(AccountPlayerError::LoginNotPending)?;
        Ok(vec![
            AccountPlayerEffect::BindLoginKeyMaterial {
                material,
                key_installation: pending.key_installation,
            },
            AccountPlayerEffect::SendLoginByKey {
                handle: self.descriptor_handle,
                correlation: pending.correlation,
                request: pending.request,
            },
        ])
    }

    /// Rebuild the ordered player-load effect for a transport retry without
    /// allocating a new request or changing the pending generation.
    ///
    /// # Errors
    ///
    /// Returns `PlayerNotPending` when no player request is retained. A closed
    /// session returns an empty vector because the late retry is ignored.
    pub fn retry_player_effects(&self) -> Result<Vec<AccountPlayerEffect>, AccountPlayerError> {
        if self.phase == AccountPlayerPhase::Closed {
            return Ok(Vec::new());
        }
        let pending = self
            .pending_player
            .as_ref()
            .ok_or(AccountPlayerError::PlayerNotPending)?;
        Ok(vec![AccountPlayerEffect::SendPlayerLoad {
            handle: self.descriptor_handle,
            correlation: pending.correlation,
            request: pending.request,
        }])
    }

    /// Select an account slot and start the exact player-load request.
    ///
    /// The bounds check intentionally occurs before indexing. This preserves
    /// the active four-slot contract without reproducing the legacy unsafe
    /// out-of-bounds read. A zero player ID or `bChangeName` is rejected
    /// without producing a request. The legacy zero-ID branch also closes the
    /// descriptor; this pure reducer returns `PlayerIdMissing` without
    /// fabricating that transport effect, so the adapter must close on it.
    ///
    /// # Errors
    ///
    /// Returns an error for a closed or wrong-phase session, a missing
    /// account, an out-of-range index, an empty/rename-required slot, an
    /// already-pending request, or an exhausted correlation space.
    pub fn on_select(&mut self, index: u8) -> Result<AccountPlayerReduction, AccountPlayerError> {
        self.require_open(AccountDbOperation::PlayerLoad)?;
        self.require_phase(AccountDbOperation::PlayerLoad, AccountPlayerPhase::Select)?;
        if self.pending_player.is_some() {
            return Err(AccountPlayerError::PlayerAlreadyPending);
        }
        if self.player.is_some() {
            return Err(AccountPlayerError::PlayerAlreadyBound);
        }
        let account = self
            .account
            .as_ref()
            .ok_or(AccountPlayerError::AccountNotBound)?;
        let index_usize = usize::from(index);
        if index_usize >= LEGACY_PLAYER_PER_ACCOUNT {
            return Err(AccountPlayerError::InvalidPlayerIndex { index });
        }
        let selected = account.players[index_usize];
        if selected.id == 0 {
            return Err(AccountPlayerError::PlayerIdMissing { index });
        }
        if selected.change_name != 0 {
            return Err(AccountPlayerError::NameChangeRequired { index });
        }

        let request = PlayerLoadRequest {
            account_id: account.id,
            player_id: selected.id,
            account_index: index,
        };
        let correlation = self.allocate_correlation()?;
        self.pending_player = Some(PendingPlayer {
            correlation,
            request,
        });
        Ok(self.reduction(vec![AccountPlayerEffect::SendPlayerLoad {
            handle: self.descriptor_handle,
            correlation,
            request,
        }]))
    }

    /// Apply an already-resolved login completion.
    ///
    /// A missing/already response clears the pending login generation and
    /// leaves the phase at `Login`; retained key material remains until local
    /// close because this seam does not perform descriptor teardown. A non-OK
    /// account response emits the source-ordered logout request and
    /// login-failure effects. A success first binds the account and then emits
    /// the delegated `Select` transition. Status is checked first, matching the
    /// legacy cleanup order; a zero account ID or login mismatch is rejected
    /// without mutation only for an `OK` response.
    ///
    /// The returned `state` is a candidate state. If it contains
    /// `TransitionPhase`, clone the reducer before invoking this method, apply
    /// the matching `ClientLifecycle::transition_to` to the returned candidate,
    /// and commit the account/player and lifecycle states only if the
    /// transition succeeds. There is no rollback method on this reducer; a
    /// rejected adjacent lifecycle transition is terminal for the adapter and
    /// must close the descriptor.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing or mismatched pending correlation,
    /// malformed source data, or an invalid account ID/login. A closed session
    /// returns an empty reduction.
    pub fn on_login_result(
        &mut self,
        completion: AccountLoginCompletion,
    ) -> Result<AccountPlayerReduction, AccountPlayerError> {
        if self.phase == AccountPlayerPhase::Closed {
            return Ok(self.reduction(Vec::new()));
        }
        let pending = self
            .pending_login
            .as_ref()
            .ok_or(AccountPlayerError::LoginNotPending)?;
        self.require_completion(
            AccountDbOperation::Login,
            completion.handle,
            completion.correlation,
            pending.correlation,
        )?;
        let request_login = pending.request.login;
        let outcome = completion.outcome;

        match outcome {
            AccountLoginOutcome::Missing => {
                self.pending_login = None;
                Ok(self.reduction(vec![login_failure(b"NOID")]))
            }
            AccountLoginOutcome::AlreadyLoggedIn(record) => {
                self.pending_login = None;
                Ok(self.reduction(vec![
                    AccountPlayerEffect::AlreadyLoggedIn { record },
                    login_failure(b"ALREADY"),
                ]))
            }
            AccountLoginOutcome::Success(account) => {
                if !status_is_ok(&account.status) {
                    self.pending_login = None;
                    return Ok(self.reduction(vec![
                        AccountPlayerEffect::RequestLogout {
                            handle: self.descriptor_handle,
                            login: bounded_c_string(&account.login),
                        },
                        login_failure(&account.status),
                    ]));
                }
                if account.id == 0 {
                    return Err(AccountPlayerError::ZeroAccountId);
                }
                if !legacy_c_string_eq(&account.login, &request_login) {
                    return Err(AccountPlayerError::AccountLoginMismatch);
                }
                self.pending_login = None;
                self.account = Some(account);
                self.phase = AccountPlayerPhase::Select;
                Ok(self.reduction(vec![
                    AccountPlayerEffect::BindAccount { account },
                    AccountPlayerEffect::TransitionPhase {
                        phase: PostHandshakePhase::Select,
                    },
                ]))
            }
        }
    }

    /// Apply an already-resolved player completion.
    ///
    /// The legacy failure arm is a no-op after the request has completed, so
    /// a positive missing result clears the correlation but emits no phase or
    /// player effect. A successful record must have a nonzero ID equal to the
    /// requested player ID. The player bind precedes the delegated `Loading`
    /// transition.
    ///
    /// The returned `state` is a candidate state. Clone the reducer before
    /// invoking this method, apply the matching
    /// `ClientLifecycle::transition_to` to the returned candidate before
    /// committing both states, and treat a rejected lifecycle transition as
    /// terminal for the adapter. This reducer does not roll back an
    /// already-mutated call.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing or mismatched pending correlation or a
    /// zero or mismatched player ID. A closed session returns an empty
    /// reduction.
    pub fn on_player_result(
        &mut self,
        completion: AccountPlayerCompletion,
    ) -> Result<AccountPlayerReduction, AccountPlayerError> {
        if self.phase == AccountPlayerPhase::Closed {
            return Ok(self.reduction(Vec::new()));
        }
        let pending = self
            .pending_player
            .as_ref()
            .ok_or(AccountPlayerError::PlayerNotPending)?;
        self.require_completion(
            AccountDbOperation::PlayerLoad,
            completion.handle,
            completion.correlation,
            pending.correlation,
        )?;
        let request = pending.request;
        let outcome = completion.outcome;

        match outcome {
            AccountPlayerOutcome::Missing => {
                self.pending_player = None;
                Ok(self.reduction(Vec::new()))
            }
            AccountPlayerOutcome::Success(player) => {
                if player.id == 0 {
                    return Err(AccountPlayerError::ZeroPlayerId);
                }
                if player.id != request.player_id {
                    return Err(AccountPlayerError::PlayerIdMismatch {
                        expected: request.player_id,
                        actual: player.id,
                    });
                }
                self.pending_player = None;
                self.player = Some(player);
                self.phase = AccountPlayerPhase::Loading;
                Ok(self.reduction(vec![
                    AccountPlayerEffect::BindPlayer {
                        account_id: request.account_id,
                        player_id: request.player_id,
                        account_index: request.account_index,
                        player,
                    },
                    AccountPlayerEffect::TransitionPhase {
                        phase: PostHandshakePhase::Loading,
                    },
                ]))
            }
        }
    }

    /// Apply a decoded login DB frame without dispatching it.
    ///
    /// The caller supplies the generation retained when the request was sent.
    /// Malformed and unsupported frames are errors; they are never converted
    /// into a positive missing result.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsupported header, malformed exact-length
    /// payload, or a mismatched pending correlation. A closed session is
    /// ignored with an empty reduction.
    pub fn on_login_frame(
        &mut self,
        frame: &DbFrame,
        correlation: AccountCorrelationId,
    ) -> Result<AccountPlayerReduction, AccountPlayerError> {
        if self.phase == AccountPlayerPhase::Closed {
            return Ok(self.reduction(Vec::new()));
        }
        self.require_pending_frame(AccountDbOperation::Login, frame.handle, correlation)?;
        let outcome = decode_login_frame(frame)?;
        self.on_login_result(AccountLoginCompletion {
            handle: frame.handle,
            correlation,
            outcome,
        })
    }

    /// Apply a decoded player DB frame without dispatching it.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsupported header, malformed exact-length
    /// payload, or a mismatched pending correlation. A closed session is
    /// ignored with an empty reduction.
    pub fn on_player_frame(
        &mut self,
        frame: &DbFrame,
        correlation: AccountCorrelationId,
    ) -> Result<AccountPlayerReduction, AccountPlayerError> {
        if self.phase == AccountPlayerPhase::Closed {
            return Ok(self.reduction(Vec::new()));
        }
        self.require_pending_frame(AccountDbOperation::PlayerLoad, frame.handle, correlation)?;
        let outcome = decode_player_frame(frame)?;
        self.on_player_result(AccountPlayerCompletion {
            handle: frame.handle,
            correlation,
            outcome,
        })
    }

    /// Record a transport/DB error for a login request without treating it as
    /// a source-resolved missing response.
    ///
    /// The pending generation and key-installation policy are deliberately
    /// retained. A retry can use [`Self::retry_login_effects`] to rebuild the
    /// original bind-and-send effect pair with this same generation; calling
    /// `on_login` again is rejected and allocates no new request. The caller can
    /// instead close the descriptor. This reducer does not fabricate a legacy
    /// `LOGIN_NOT_EXIST` frame.
    ///
    /// # Errors
    ///
    /// Returns a distinct `Database` error after validating the pending
    /// handle and generation, or an error when no matching request exists. A
    /// session already in `Closed` returns an empty reduction because the
    /// event is a late completion after terminal teardown.
    pub fn on_login_error(
        &mut self,
        handle: u32,
        correlation: AccountCorrelationId,
    ) -> Result<AccountPlayerReduction, AccountPlayerError> {
        if self.phase == AccountPlayerPhase::Closed {
            return Ok(self.reduction(Vec::new()));
        }
        let pending = self
            .pending_login
            .as_ref()
            .ok_or(AccountPlayerError::LoginNotPending)?;
        self.require_completion(
            AccountDbOperation::Login,
            handle,
            correlation,
            pending.correlation,
        )?;
        Err(AccountPlayerError::Database {
            operation: AccountDbOperation::Login,
            handle,
            correlation,
        })
    }

    /// Record a transport/DB error for a player request without treating it
    /// as a source-resolved missing response. A retry must resend the
    /// already-returned `SendPlayerLoad` effect with the same generation;
    /// [`Self::retry_player_effects`] can rebuild that effect without changing
    /// state. Calling `on_select` again is rejected and allocates no new request.
    ///
    /// # Errors
    ///
    /// Returns a distinct `Database` error after validating the pending
    /// handle and generation, or an error when no matching request exists. A
    /// session already in `Closed` returns an empty reduction because the
    /// event is a late completion after terminal teardown.
    pub fn on_player_error(
        &mut self,
        handle: u32,
        correlation: AccountCorrelationId,
    ) -> Result<AccountPlayerReduction, AccountPlayerError> {
        if self.phase == AccountPlayerPhase::Closed {
            return Ok(self.reduction(Vec::new()));
        }
        let pending = self
            .pending_player
            .as_ref()
            .ok_or(AccountPlayerError::PlayerNotPending)?;
        self.require_completion(
            AccountDbOperation::PlayerLoad,
            handle,
            correlation,
            pending.correlation,
        )?;
        Err(AccountPlayerError::Database {
            operation: AccountDbOperation::PlayerLoad,
            handle,
            correlation,
        })
    }

    /// Apply the source `HEADER_CG_ENTERGAME` phase transition.
    ///
    /// The legacy handler performs world and gameplay work before calling
    /// `SetPhase(PHASE_GAME)`. This reducer only requires a bound player in
    /// `Loading` and delegates the phase effect. A missing bound character is
    /// returned as `PlayerNotBound`; the adapter must apply the legacy
    /// terminal close because this boundary does not own descriptor teardown.
    /// The returned `state` is a candidate state. Clone the reducer before
    /// invoking this method, apply `ClientLifecycle::transition_to(Game)` to
    /// the returned candidate, and commit both states only if that adjacent
    /// transition succeeds. A rejected transition is terminal for the adapter;
    /// this reducer has no rollback API.
    ///
    /// # Errors
    ///
    /// Returns an error when the session is closed, not in `Loading`, or has
    /// no bound player.
    pub fn on_enter_game(&mut self) -> Result<AccountPlayerReduction, AccountPlayerError> {
        self.require_open(AccountDbOperation::EnterGame)?;
        self.require_phase(AccountDbOperation::EnterGame, AccountPlayerPhase::Loading)?;
        if self.player.is_none() {
            return Err(AccountPlayerError::PlayerNotBound);
        }
        self.phase = AccountPlayerPhase::Game;
        Ok(self.reduction(vec![AccountPlayerEffect::TransitionPhase {
            phase: PostHandshakePhase::Game,
        }]))
    }

    /// Close the descriptor and clear all pending and bound state.
    ///
    /// `Close` is a local reducer effect only. A transport adapter remains
    /// responsible for the legacy descriptor teardown: character disconnect,
    /// account/login map removal, DB logout, and P2P cleanup.
    pub fn close(&mut self) -> AccountPlayerReduction {
        if self.phase == AccountPlayerPhase::Closed {
            return self.reduction(Vec::new());
        }
        self.phase = AccountPlayerPhase::Closed;
        self.pending_login = None;
        self.login_key_material = None;
        self.account = None;
        self.pending_player = None;
        self.player = None;
        self.reduction(vec![AccountPlayerEffect::Close])
    }

    fn allocate_correlation(&mut self) -> Result<AccountCorrelationId, AccountPlayerError> {
        let sequence = self.next_correlation;
        self.next_correlation = sequence
            .checked_add(1)
            .ok_or(AccountPlayerError::CorrelationExhausted)?;
        let value = (u64::from(self.correlation_seed) << 32) | u64::from(sequence);
        Ok(AccountCorrelationId::new(value))
    }

    fn require_open(&self, operation: AccountDbOperation) -> Result<(), AccountPlayerError> {
        if self.phase == AccountPlayerPhase::Closed {
            Err(AccountPlayerError::Closed { operation })
        } else {
            Ok(())
        }
    }

    fn require_phase(
        &self,
        operation: AccountDbOperation,
        expected: AccountPlayerPhase,
    ) -> Result<(), AccountPlayerError> {
        if self.phase == expected {
            Ok(())
        } else {
            Err(AccountPlayerError::InvalidPhase {
                operation,
                phase: self.phase,
                expected,
            })
        }
    }

    fn require_pending_frame(
        &self,
        operation: AccountDbOperation,
        handle: u32,
        correlation: AccountCorrelationId,
    ) -> Result<(), AccountPlayerError> {
        let expected = match operation {
            AccountDbOperation::Login => {
                self.pending_login
                    .as_ref()
                    .ok_or(AccountPlayerError::LoginNotPending)?
                    .correlation
            }
            AccountDbOperation::PlayerLoad => {
                self.pending_player
                    .as_ref()
                    .ok_or(AccountPlayerError::PlayerNotPending)?
                    .correlation
            }
            AccountDbOperation::EnterGame => {
                return Err(AccountPlayerError::InvalidPhase {
                    operation,
                    phase: self.phase,
                    expected: AccountPlayerPhase::Loading,
                });
            }
        };
        self.require_completion(operation, handle, correlation, expected)
    }

    fn require_completion(
        &self,
        operation: AccountDbOperation,
        handle: u32,
        correlation: AccountCorrelationId,
        expected: AccountCorrelationId,
    ) -> Result<(), AccountPlayerError> {
        if handle != self.descriptor_handle {
            return Err(AccountPlayerError::HandleMismatch {
                operation,
                expected: self.descriptor_handle,
                actual: handle,
            });
        }
        if correlation != expected {
            return Err(AccountPlayerError::CorrelationMismatch {
                operation,
                expected,
                actual: correlation,
            });
        }
        Ok(())
    }

    fn reduction(&self, effects: Vec<AccountPlayerEffect>) -> AccountPlayerReduction {
        AccountPlayerReduction {
            state: self.clone(),
            effects,
        }
    }
}

/// Create a one-shot/test session with descriptor handle zero and seed zero.
///
/// This has the same handle-reuse limitation as [`Self::new`].
impl Default for AccountPlayerSession {
    fn default() -> Self {
        Self::new(0)
    }
}

/// Decode a primary login DB frame without applying it.
fn decode_login_frame(frame: &DbFrame) -> Result<AccountLoginOutcome, AccountPlayerError> {
    match frame.header {
        protocol::db_records::HEADER_DG_LOGIN_SUCCESS => LoginAccountRecord::decode(&frame.payload)
            .map(AccountLoginOutcome::Success)
            .map_err(|source| AccountPlayerError::MalformedLoginSuccess { source }),
        protocol::db_records::HEADER_DG_LOGIN_NOT_EXIST if frame.payload.is_empty() => {
            Ok(AccountLoginOutcome::Missing)
        }
        protocol::db_records::HEADER_DG_LOGIN_NOT_EXIST => {
            Err(AccountPlayerError::MalformedLoginMissing {
                actual_len: frame.payload.len(),
            })
        }
        protocol::db_records::HEADER_DG_LOGIN_ALREADY => LoginAlreadyRecord::decode(&frame.payload)
            .map(AccountLoginOutcome::AlreadyLoggedIn)
            .map_err(|source| AccountPlayerError::MalformedLoginAlready { source }),
        header => Err(AccountPlayerError::UnsupportedResponseHeader {
            operation: AccountDbOperation::Login,
            header,
        }),
    }
}

/// Decode a primary player DB frame without applying it.
fn decode_player_frame(frame: &DbFrame) -> Result<AccountPlayerOutcome, AccountPlayerError> {
    match frame.header {
        protocol::db_records::HEADER_DG_PLAYER_LOAD_SUCCESS => {
            PlayerResultRecord::decode(&frame.payload)
                .map(AccountPlayerOutcome::Success)
                .map_err(|source| AccountPlayerError::MalformedPlayerSuccess { source })
        }
        protocol::db_records::HEADER_DG_PLAYER_LOAD_FAILED if frame.payload.is_empty() => {
            Ok(AccountPlayerOutcome::Missing)
        }
        protocol::db_records::HEADER_DG_PLAYER_LOAD_FAILED => {
            Err(AccountPlayerError::MalformedPlayerMissing {
                actual_len: frame.payload.len(),
            })
        }
        header => Err(AccountPlayerError::UnsupportedResponseHeader {
            operation: AccountDbOperation::PlayerLoad,
            header,
        }),
    }
}

const fn has_canonical_c_string<const N: usize>(field: &[u8; N]) -> bool {
    let mut index = 0;
    while index < N {
        if field[index] == 0 {
            index += 1;
            while index < N {
                if field[index] != 0 {
                    return false;
                }
                index += 1;
            }
            return true;
        }
        index += 1;
    }
    false
}

fn bounded_c_string<const N: usize>(source: &[u8]) -> [u8; N] {
    let mut output = [0; N];
    if N != 0 {
        let source_len = source
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(source.len());
        let copy_len = source_len.min(N - 1);
        output[..copy_len].copy_from_slice(&source[..copy_len]);
    }
    output
}

fn login_failure(status: &[u8]) -> AccountPlayerEffect {
    AccountPlayerEffect::LoginFailure {
        status: bounded_c_string(status),
    }
}

/// Compare the bytes before the first NUL in a fixed legacy array.
///
/// Bytes after the terminator are retained by the protocol record but are not
/// part of the C-string value. An unterminated array compares its entire
/// length, so it cannot accidentally equal a shorter expected value.
fn fixed_c_string_eq(value: &[u8], expected: &[u8]) -> bool {
    let value_len = value
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(value.len());
    value_len == expected.len() && &value[..value_len] == expected
}

fn status_is_ok(status: &[u8; LEGACY_ACCOUNT_STATUS_BYTES]) -> bool {
    fixed_c_string_eq(status, b"OK")
}

fn legacy_c_string_eq(left: &[u8], right: &[u8]) -> bool {
    let left_len = left
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(left.len());
    let right_len = right
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(right.len());
    left_len == right_len && left[..left_len] == right[..right_len]
}

/// A rejection or malformed input at the account/player boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AccountPlayerError {
    /// The session is closed and does not consume a new request.
    Closed {
        /// Operation that was rejected.
        operation: AccountDbOperation,
    },
    /// A request arrived in the wrong source phase.
    InvalidPhase {
        /// Operation that was rejected.
        operation: AccountDbOperation,
        /// Current phase.
        phase: AccountPlayerPhase,
        /// Required phase.
        expected: AccountPlayerPhase,
    },
    /// A second login request was submitted while one was pending.
    LoginAlreadyPending,
    /// The normalized fixed login field had no NUL terminator.
    LoginNotTerminated,
    /// The fixed host/IP field had no NUL terminator.
    IpNotTerminated,
    /// The normalized login had bytes after its NUL that were not zero.
    LoginNotCanonical,
    /// The host/IP field had bytes after its NUL that were not zero.
    IpNotCanonical,
    /// An account is already bound.
    AccountAlreadyBound,
    /// No account is bound for a player request.
    AccountNotBound,
    /// A player request is already pending.
    PlayerAlreadyPending,
    /// A player is already bound.
    PlayerAlreadyBound,
    /// No player record is bound for the enter-game request.
    PlayerNotBound,
    /// A player slot is outside `0..PLAYER_PER_ACCOUNT`.
    InvalidPlayerIndex {
        /// Supplied one-byte slot index.
        index: u8,
    },
    /// The selected slot has no player ID.
    PlayerIdMissing {
        /// Selected slot index.
        index: u8,
    },
    /// The selected slot still requires a name change.
    NameChangeRequired {
        /// Selected slot index.
        index: u8,
    },
    /// No login request is waiting for a completion.
    LoginNotPending,
    /// No player request is waiting for a completion.
    PlayerNotPending,
    /// A completion used a different descriptor handle.
    HandleMismatch {
        /// Operation whose handle was checked.
        operation: AccountDbOperation,
        /// Descriptor handle owned by this session.
        expected: u32,
        /// Handle supplied by the completion.
        actual: u32,
    },
    /// A completion used a stale or otherwise unknown generation.
    CorrelationMismatch {
        /// Operation whose generation was checked.
        operation: AccountDbOperation,
        /// Generation currently pending.
        expected: AccountCorrelationId,
        /// Generation supplied by the completion.
        actual: AccountCorrelationId,
    },
    /// A successful account response carried ID zero.
    ZeroAccountId,
    /// A successful account response did not match the normalized request.
    AccountLoginMismatch,
    /// A successful player response carried ID zero.
    ZeroPlayerId,
    /// A successful player response did not match the requested player.
    PlayerIdMismatch {
        /// Player ID in the request.
        expected: u32,
        /// Player ID in the response.
        actual: u32,
    },
    /// A login response header is not a supported primary key-login outcome.
    UnsupportedResponseHeader {
        /// Operation whose response was decoded.
        operation: AccountDbOperation,
        /// Unsupported one-byte header.
        header: u8,
    },
    /// A login success payload was not an exact account record.
    MalformedLoginSuccess {
        /// Exact record codec failure.
        source: DbRecordError,
    },
    /// A login already payload was not exactly 31 bytes.
    MalformedLoginAlready {
        /// Exact record codec failure.
        source: DbRecordError,
    },
    /// A login missing payload was not empty.
    MalformedLoginMissing {
        /// Unexpected payload length.
        actual_len: usize,
    },
    /// A player success payload was not an exact player record.
    MalformedPlayerSuccess {
        /// Exact record codec failure.
        source: DbRecordError,
    },
    /// A player failure payload was not empty.
    MalformedPlayerMissing {
        /// Unexpected payload length.
        actual_len: usize,
    },
    /// A DB/transport error is distinct from a source-resolved missing row.
    /// In practice `operation` is `Login` or `PlayerLoad`; `EnterGame` is a
    /// phase operation and is not emitted by the DB error methods.
    Database {
        /// Failed operation.
        operation: AccountDbOperation,
        /// Descriptor handle.
        handle: u32,
        /// Pending generation.
        correlation: AccountCorrelationId,
    },
    /// The local generation space is exhausted; IDs are never wrapped.
    CorrelationExhausted,
}

impl fmt::Display for AccountPlayerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed { .. }
            | Self::InvalidPhase { .. }
            | Self::LoginAlreadyPending
            | Self::LoginNotTerminated
            | Self::IpNotTerminated
            | Self::LoginNotCanonical
            | Self::IpNotCanonical
            | Self::AccountAlreadyBound
            | Self::AccountNotBound
            | Self::PlayerAlreadyPending
            | Self::PlayerAlreadyBound
            | Self::PlayerNotBound
            | Self::InvalidPlayerIndex { .. }
            | Self::PlayerIdMissing { .. }
            | Self::NameChangeRequired { .. }
            | Self::LoginNotPending
            | Self::PlayerNotPending
            | Self::HandleMismatch { .. }
            | Self::CorrelationMismatch { .. } => fmt_state_error(self, formatter),
            _ => fmt_response_error(self, formatter),
        }
    }
}

fn fmt_state_error(error: &AccountPlayerError, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    match error {
        AccountPlayerError::Closed { operation } => {
            write!(
                formatter,
                "account/player session is closed for {operation:?}"
            )
        }
        AccountPlayerError::InvalidPhase {
            operation,
            phase,
            expected,
        } => write!(
            formatter,
            "{operation:?} requires phase {expected:?}, current phase is {phase:?}"
        ),
        AccountPlayerError::LoginAlreadyPending => {
            formatter.write_str("a login request is already pending")
        }
        AccountPlayerError::LoginNotTerminated => {
            formatter.write_str("normalized login field has no NUL terminator")
        }
        AccountPlayerError::IpNotTerminated => {
            formatter.write_str("normalized host/IP field has no NUL terminator")
        }
        AccountPlayerError::LoginNotCanonical => {
            formatter.write_str("normalized login has non-zero bytes after its NUL")
        }
        AccountPlayerError::IpNotCanonical => {
            formatter.write_str("normalized host/IP has non-zero bytes after its NUL")
        }
        AccountPlayerError::AccountAlreadyBound => {
            formatter.write_str("an account is already bound")
        }
        AccountPlayerError::AccountNotBound => formatter.write_str("no account is bound"),
        AccountPlayerError::PlayerAlreadyPending => {
            formatter.write_str("a player request is already pending")
        }
        AccountPlayerError::PlayerAlreadyBound => formatter.write_str("a player is already bound"),
        AccountPlayerError::PlayerNotBound => formatter.write_str("no player is bound"),
        AccountPlayerError::InvalidPlayerIndex { index } => {
            write!(
                formatter,
                "player index {index} is outside the account slots"
            )
        }
        AccountPlayerError::PlayerIdMissing { index } => {
            write!(formatter, "player slot {index} has no player ID")
        }
        AccountPlayerError::NameChangeRequired { index } => {
            write!(formatter, "player slot {index} requires a name change")
        }
        AccountPlayerError::LoginNotPending => formatter.write_str("no login request is pending"),
        AccountPlayerError::PlayerNotPending => formatter.write_str("no player request is pending"),
        AccountPlayerError::HandleMismatch {
            operation,
            expected,
            actual,
        } => write!(
            formatter,
            "{operation:?} completion handle {actual} does not match {expected}"
        ),
        AccountPlayerError::CorrelationMismatch {
            operation,
            expected,
            actual,
        } => write!(
            formatter,
            "{operation:?} completion generation {} does not match {}",
            actual.get(),
            expected.get()
        ),
        _ => formatter.write_str("unknown account/player error"),
    }
}

fn fmt_response_error(
    error: &AccountPlayerError,
    formatter: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    match error {
        AccountPlayerError::ZeroAccountId => {
            formatter.write_str("successful account response has ID zero")
        }
        AccountPlayerError::AccountLoginMismatch => {
            formatter.write_str("successful account response login does not match request")
        }
        AccountPlayerError::ZeroPlayerId => {
            formatter.write_str("successful player response has ID zero")
        }
        AccountPlayerError::PlayerIdMismatch { expected, actual } => write!(
            formatter,
            "successful player response ID {actual} does not match requested {expected}"
        ),
        AccountPlayerError::UnsupportedResponseHeader { operation, header } => {
            write!(
                formatter,
                "unsupported {operation:?} response header {header}"
            )
        }
        AccountPlayerError::MalformedLoginSuccess { source } => {
            write!(
                formatter,
                "malformed login success account payload: {source}"
            )
        }
        AccountPlayerError::MalformedLoginAlready { source } => {
            write!(formatter, "malformed login already payload: {source}")
        }
        AccountPlayerError::MalformedLoginMissing { actual_len } => write!(
            formatter,
            "login missing response has {actual_len} bytes; expected 0"
        ),
        AccountPlayerError::MalformedPlayerSuccess { source } => {
            write!(formatter, "malformed player success payload: {source}")
        }
        AccountPlayerError::MalformedPlayerMissing { actual_len } => write!(
            formatter,
            "player failure response has {actual_len} bytes; expected 0"
        ),
        AccountPlayerError::Database {
            operation,
            handle,
            correlation,
        } => write!(
            formatter,
            "{operation:?} DB error for handle {handle}, generation {}",
            correlation.get()
        ),
        AccountPlayerError::CorrelationExhausted => {
            formatter.write_str("account correlation space is exhausted")
        }
        _ => formatter.write_str("unknown account/player response error"),
    }
}

impl Error for AccountPlayerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::MalformedLoginSuccess { source }
            | Self::MalformedLoginAlready { source }
            | Self::MalformedPlayerSuccess { source } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::db_records::{
        SimplePlayerRecord, HEADER_DG_LOGIN_NOT_EXIST, HEADER_DG_LOGIN_SUCCESS,
        HEADER_DG_PLAYER_LOAD_FAILED, HEADER_DG_PLAYER_LOAD_SUCCESS, HEADER_GD_LOGIN_BY_KEY,
    };

    const HANDLE: u32 = 0x1234_5678;

    fn c_bytes<const N: usize>(value: &str) -> [u8; N] {
        let mut bytes = [0; N];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        bytes
    }

    fn input() -> AccountLoginInput {
        let mut ip = [0; LEGACY_IP_BYTES];
        ip[..9].copy_from_slice(b"127.0.0.1");
        AccountLoginInput::new(
            c_bytes("alice"),
            0x1020_3040,
            [1, 0x0102_0304, 0x0506_0708, 0x090a_0b0c],
            ip,
        )
    }

    #[allow(clippy::field_reassign_with_default)]
    fn account() -> LoginAccountRecord {
        let mut value = LoginAccountRecord::default();
        value.id = 0x0102_0304;
        value.login = c_bytes("alice");
        value.status = c_bytes("OK");
        value.players[0] = SimplePlayerRecord {
            id: 0x1112_1314,
            name: c_bytes("hero"),
            ..SimplePlayerRecord::default()
        };
        value
    }

    #[allow(clippy::field_reassign_with_default)]
    fn player() -> PlayerResultRecord {
        let mut value = PlayerResultRecord::default();
        value.id = 0x1112_1314;
        value
    }

    fn start_login(
        session: &mut AccountPlayerSession,
    ) -> (AccountCorrelationId, LoginByKeyRequest) {
        let reduction = session.on_login(input()).unwrap();
        assert!(matches!(
            reduction.effects.as_slice(),
            [
                AccountPlayerEffect::BindLoginKeyMaterial { .. },
                AccountPlayerEffect::SendLoginByKey { .. }
            ]
        ));
        let effect = reduction.effects.get(1).unwrap();
        let AccountPlayerEffect::SendLoginByKey {
            handle,
            correlation,
            request,
        } = effect
        else {
            panic!("expected login request effect");
        };
        assert_eq!(*handle, HANDLE);
        (*correlation, *request)
    }

    #[allow(clippy::large_types_passed_by_value)]
    fn complete_login(
        session: &mut AccountPlayerSession,
        correlation: AccountCorrelationId,
        value: LoginAccountRecord,
    ) -> AccountPlayerReduction {
        session
            .on_login_result(AccountLoginCompletion {
                handle: HANDLE,
                correlation,
                outcome: AccountLoginOutcome::Success(value),
            })
            .unwrap()
    }

    fn select_player(
        session: &mut AccountPlayerSession,
    ) -> (AccountCorrelationId, PlayerLoadRequest) {
        let reduction = session.on_select(0).unwrap();
        let effect = reduction.effects.first().unwrap();
        let AccountPlayerEffect::SendPlayerLoad {
            handle,
            correlation,
            request,
        } = effect
        else {
            panic!("expected player request effect");
        };
        assert_eq!(*handle, HANDLE);
        (*correlation, *request)
    }

    #[test]
    fn unterminated_normalized_login_is_rejected_before_key_binding_or_db_send() {
        let mut session = AccountPlayerSession::new(HANDLE);
        let mut value = input();
        value.login = [b'X'; LEGACY_LOGIN_BYTES];
        let error = session.on_login(value).unwrap_err();
        assert_eq!(error, AccountPlayerError::LoginNotTerminated);
        assert!(session.pending_login().is_none());
        assert!(session.login_key_material().is_none());
        assert_eq!(session.next_correlation, 1);
    }

    #[test]
    fn unterminated_host_ip_is_rejected_before_key_binding_or_db_send() {
        let mut session = AccountPlayerSession::new(HANDLE);
        let mut value = input();
        value.ip = [b'X'; LEGACY_IP_BYTES];
        let error = session.on_login(value).unwrap_err();
        assert_eq!(error, AccountPlayerError::IpNotTerminated);
        assert!(session.pending_login().is_none());
        assert!(session.login_key_material().is_none());
        assert_eq!(session.next_correlation, 1);
    }

    #[test]
    fn noncanonical_fixed_field_tails_are_rejected_before_effects() {
        let mut session = AccountPlayerSession::new(HANDLE);
        let mut login_tail = input();
        login_tail.login[6] = b'X';
        assert_eq!(
            session.on_login(login_tail).unwrap_err(),
            AccountPlayerError::LoginNotCanonical
        );
        assert!(session.pending_login().is_none());

        let mut ip_tail = input();
        ip_tail.ip[10] = b'X';
        assert_eq!(
            session.on_login(ip_tail).unwrap_err(),
            AccountPlayerError::IpNotCanonical
        );
        assert!(session.pending_login().is_none());
        assert!(session.login_key_material().is_none());
    }

    #[test]
    fn key_installation_policy_is_explicit_and_controls_security_key_effect() {
        let mut legacy = AccountPlayerSession::new(HANDLE);
        let legacy_reduction = legacy.on_login(input()).unwrap();
        let AccountPlayerEffect::BindLoginKeyMaterial {
            key_installation: legacy_policy,
            ..
        } = &legacy_reduction.effects[0]
        else {
            panic!("expected login key material effect");
        };
        assert_eq!(*legacy_policy, AccountKeyInstallation::LegacySecurityKey);
        assert!(legacy_policy.installs_client_security_key());

        let mut improved = AccountPlayerSession::new(HANDLE);
        let improved_reduction = improved
            .on_login_with_key_installation(
                input(),
                AccountKeyInstallation::ImprovedPacketEncryption,
            )
            .unwrap();
        let AccountPlayerEffect::BindLoginKeyMaterial {
            key_installation: improved_policy,
            ..
        } = &improved_reduction.effects[0]
        else {
            panic!("expected login key material effect");
        };
        assert_eq!(
            *improved_policy,
            AccountKeyInstallation::ImprovedPacketEncryption
        );
        assert!(!improved_policy.installs_client_security_key());
    }

    #[test]
    fn retry_helpers_preserve_effect_order_policy_and_exact_requests() {
        let login_input = input();
        let mut login = AccountPlayerSession::new(HANDLE);
        let login_reduction = login
            .on_login_with_key_installation(
                login_input,
                AccountKeyInstallation::ImprovedPacketEncryption,
            )
            .unwrap();
        let AccountPlayerEffect::SendLoginByKey {
            correlation: login_correlation,
            request: login_request,
            ..
        } = login_reduction.effects[1].clone()
        else {
            panic!("expected login request effect");
        };
        assert!(matches!(
            login.on_login_error(HANDLE, login_correlation),
            Err(AccountPlayerError::Database { .. })
        ));
        assert_eq!(
            login.retry_login_effects().unwrap(),
            vec![
                AccountPlayerEffect::BindLoginKeyMaterial {
                    material: login_input.key_material(),
                    key_installation: AccountKeyInstallation::ImprovedPacketEncryption,
                },
                AccountPlayerEffect::SendLoginByKey {
                    handle: HANDLE,
                    correlation: login_correlation,
                    request: login_request,
                },
            ]
        );

        let mut player = AccountPlayerSession::new(HANDLE);
        let (login_id, _) = start_login(&mut player);
        complete_login(&mut player, login_id, account());
        let (player_id, player_request) = select_player(&mut player);
        assert!(matches!(
            player.on_player_error(HANDLE, player_id),
            Err(AccountPlayerError::Database { .. })
        ));
        assert_eq!(
            player.retry_player_effects().unwrap(),
            vec![AccountPlayerEffect::SendPlayerLoad {
                handle: HANDLE,
                correlation: player_id,
                request: player_request,
            }]
        );
    }

    #[test]
    fn default_input_is_empty_but_is_not_rejected_as_a_credential() {
        let mut session = AccountPlayerSession::new(HANDLE);
        let reduction = session.on_login(AccountLoginInput::default()).unwrap();
        assert!(matches!(
            reduction.effects.as_slice(),
            [
                AccountPlayerEffect::BindLoginKeyMaterial { .. },
                AccountPlayerEffect::SendLoginByKey { .. }
            ]
        ));
    }

    #[test]
    fn login_request_preserves_exact_67_byte_record_and_db_envelope() {
        let mut session = AccountPlayerSession::new(HANDLE);
        let normalized = input();
        let reduction = session.on_login(normalized).unwrap();
        assert!(matches!(
            reduction.effects.as_slice(),
            [
                AccountPlayerEffect::BindLoginKeyMaterial { .. },
                AccountPlayerEffect::SendLoginByKey { .. }
            ]
        ));
        let AccountPlayerEffect::SendLoginByKey {
            handle,
            correlation,
            request,
        } = reduction.effects[1]
        else {
            panic!("expected login request effect");
        };
        assert_eq!(handle, HANDLE);
        assert_eq!(correlation.get(), 1);
        assert_eq!(
            session.login_key_material(),
            Some(&normalized.key_material())
        );
        assert_eq!(request.encode().len(), 67);
        let frame = DbFrame::new(HEADER_GD_LOGIN_BY_KEY, HANDLE, request.encode());
        let bytes = frame.encode().unwrap();
        assert_eq!(&bytes[..5], &[101, 0x78, 0x56, 0x34, 0x12]);
        assert_eq!(&bytes[5..9], &67_u32.to_le_bytes());
        assert_eq!(&bytes[9..], &request.encode()[..]);
    }

    #[test]
    fn happy_order_binds_before_select_loading_and_game_transitions() {
        let mut session = AccountPlayerSession::new(HANDLE);
        let (login_id, _) = start_login(&mut session);
        let login = complete_login(&mut session, login_id, account());
        assert_eq!(login.state.phase(), AccountPlayerPhase::Select);
        assert!(matches!(
            login.effects.as_slice(),
            [
                AccountPlayerEffect::BindAccount { .. },
                AccountPlayerEffect::TransitionPhase {
                    phase: PostHandshakePhase::Select
                }
            ]
        ));

        let (player_id, request) = select_player(&mut session);
        assert_eq!(request.encode().len(), 9);
        let player_result = session
            .on_player_result(AccountPlayerCompletion {
                handle: HANDLE,
                correlation: player_id,
                outcome: AccountPlayerOutcome::Success(player()),
            })
            .unwrap();
        assert_eq!(player_result.state.phase(), AccountPlayerPhase::Loading);
        assert!(matches!(
            player_result.effects.as_slice(),
            [
                AccountPlayerEffect::BindPlayer { .. },
                AccountPlayerEffect::TransitionPhase {
                    phase: PostHandshakePhase::Loading
                }
            ]
        ));

        let game = session.on_enter_game().unwrap();
        assert_eq!(game.state.phase(), AccountPlayerPhase::Game);
        assert_eq!(
            game.effects,
            vec![AccountPlayerEffect::TransitionPhase {
                phase: PostHandshakePhase::Game
            }]
        );
    }

    #[test]
    fn exact_player_load_request_uses_account_bound_values_and_index() {
        let mut session = AccountPlayerSession::new(HANDLE);
        let (login_id, _) = start_login(&mut session);
        complete_login(&mut session, login_id, account());
        let mut expected = account();
        expected.players[0].id = 0x1112_1314;
        let (_correlation, request) = select_player(&mut session);
        assert_eq!(
            request.encode(),
            vec![4, 3, 2, 1, 0x14, 0x13, 0x12, 0x11, 0]
        );
    }

    #[test]
    fn stale_and_wrong_handle_completions_do_not_bind_or_clear_pending() {
        let mut session = AccountPlayerSession::new(HANDLE);
        let (expected, _) = start_login(&mut session);
        let error = session
            .on_login_result(AccountLoginCompletion {
                handle: HANDLE,
                correlation: AccountCorrelationId::new(999),
                outcome: AccountLoginOutcome::Success(account()),
            })
            .unwrap_err();
        assert!(matches!(
            error,
            AccountPlayerError::CorrelationMismatch { .. }
        ));
        assert_eq!(session.pending_login().unwrap().correlation, expected);
        let error = session
            .on_login_result(AccountLoginCompletion {
                handle: HANDLE + 1,
                correlation: expected,
                outcome: AccountLoginOutcome::Success(account()),
            })
            .unwrap_err();
        assert!(matches!(error, AccountPlayerError::HandleMismatch { .. }));
        assert_eq!(session.pending_login().unwrap().correlation, expected);
    }

    #[test]
    fn fresh_correlation_seed_prevents_old_completion_after_handle_reuse() {
        let mut old = AccountPlayerSession::new_with_correlation_seed(HANDLE, 0x1000_0001);
        let (old_correlation, _) = start_login(&mut old);
        let mut fresh = AccountPlayerSession::new_with_correlation_seed(HANDLE, 0x2000_0002);
        let (new_correlation, _) = start_login(&mut fresh);
        assert_ne!(old_correlation, new_correlation);
        assert!(matches!(
            fresh.on_login_result(AccountLoginCompletion {
                handle: HANDLE,
                correlation: old_correlation,
                outcome: AccountLoginOutcome::Success(account()),
            }),
            Err(AccountPlayerError::CorrelationMismatch { .. })
        ));
        assert!(fresh.pending_login().is_some());
    }

    #[test]
    fn enter_game_phase_errors_use_the_non_db_operation_name() {
        let mut session = AccountPlayerSession::new(HANDLE);
        let error = session.on_enter_game().unwrap_err();
        assert_eq!(
            error,
            AccountPlayerError::InvalidPhase {
                operation: AccountDbOperation::EnterGame,
                phase: AccountPlayerPhase::Login,
                expected: AccountPlayerPhase::Loading,
            }
        );
        assert!(error.to_string().contains("EnterGame"));
        assert!(matches!(error, AccountPlayerError::InvalidPhase { .. }));
    }

    #[test]
    fn correlation_exhaustion_does_not_wrap_or_retain_request_state() {
        let mut exhausted = AccountPlayerSession::new(HANDLE);
        exhausted.next_correlation = u32::MAX;
        assert_eq!(
            exhausted.on_login(input()).unwrap_err(),
            AccountPlayerError::CorrelationExhausted
        );
        assert_eq!(exhausted.next_correlation, u32::MAX);
        assert!(exhausted.pending_login().is_none());
        assert!(exhausted.login_key_material().is_none());

        let mut fresh = AccountPlayerSession::new(HANDLE);
        let (correlation, _) = start_login(&mut fresh);
        assert_eq!(correlation, AccountCorrelationId::new(1));
    }

    #[test]
    fn transport_error_keeps_login_pending_and_retry_uses_same_generation() {
        let mut session = AccountPlayerSession::new(HANDLE);
        let (correlation, request) = start_login(&mut session);
        let error = session.on_login_error(HANDLE, correlation).unwrap_err();
        assert!(matches!(error, AccountPlayerError::Database { .. }));
        let pending = session.pending_login().unwrap();
        assert_eq!(pending.correlation, correlation);
        assert_eq!(pending.request, request);
        assert_eq!(
            pending.key_installation,
            AccountKeyInstallation::LegacySecurityKey
        );
        let retry = session.retry_login_effects().unwrap();
        assert!(matches!(
            retry.as_slice(),
            [
                AccountPlayerEffect::BindLoginKeyMaterial {
                    key_installation: AccountKeyInstallation::LegacySecurityKey,
                    ..
                },
                AccountPlayerEffect::SendLoginByKey {
                    correlation: retry_correlation,
                    request: retry_request,
                    ..
                }
            ] if *retry_correlation == correlation && *retry_request == request
        ));
        assert_eq!(
            session.on_login(input()).unwrap_err(),
            AccountPlayerError::LoginAlreadyPending
        );
        assert_eq!(session.pending_login().unwrap().correlation, correlation);
    }

    #[test]
    fn duplicate_login_success_cannot_replace_bound_account() {
        let mut session = AccountPlayerSession::new(HANDLE);
        let (correlation, _) = start_login(&mut session);
        complete_login(&mut session, correlation, account());
        let error = session
            .on_login_result(AccountLoginCompletion {
                handle: HANDLE,
                correlation,
                outcome: AccountLoginOutcome::Success(account()),
            })
            .unwrap_err();
        assert_eq!(error, AccountPlayerError::LoginNotPending);
        assert_eq!(session.phase(), AccountPlayerPhase::Select);
    }

    #[test]
    fn missing_and_already_login_clear_pending_without_binding() {
        let mut session = AccountPlayerSession::new(HANDLE);
        let (correlation, _) = start_login(&mut session);
        let missing = session
            .on_login_result(AccountLoginCompletion {
                handle: HANDLE,
                correlation,
                outcome: AccountLoginOutcome::Missing,
            })
            .unwrap();
        assert_eq!(missing.state.phase(), AccountPlayerPhase::Login);
        assert!(matches!(
            missing.effects.as_slice(),
            [AccountPlayerEffect::LoginFailure { .. }]
        ));
        assert!(session.account().is_none());

        let (correlation, _) = start_login(&mut session);
        let mut raw = [0xabu8; 31];
        raw[..6].copy_from_slice(b"alice\0");
        let already = session
            .on_login_result(AccountLoginCompletion {
                handle: HANDLE,
                correlation,
                outcome: AccountLoginOutcome::AlreadyLoggedIn(LoginAlreadyRecord { login: raw }),
            })
            .unwrap();
        assert_eq!(already.state.phase(), AccountPlayerPhase::Login);
        assert!(matches!(
            already.effects.as_slice(),
            [
                AccountPlayerEffect::AlreadyLoggedIn { .. },
                AccountPlayerEffect::LoginFailure { .. }
            ]
        ));
        assert!(session.account().is_none());
    }

    #[test]
    fn non_ok_account_status_is_a_failure_and_zero_id_is_rejected_without_clearing() {
        let mut session = AccountPlayerSession::new(HANDLE);
        let (correlation, _) = start_login(&mut session);
        let mut bad = account();
        bad.status = c_bytes("LOCKED");
        let reduction = session
            .on_login_result(AccountLoginCompletion {
                handle: HANDLE,
                correlation,
                outcome: AccountLoginOutcome::Success(bad),
            })
            .unwrap();
        assert_eq!(reduction.state.phase(), AccountPlayerPhase::Login);
        assert!(matches!(
            reduction.effects.as_slice(),
            [
                AccountPlayerEffect::RequestLogout { .. },
                AccountPlayerEffect::LoginFailure { .. }
            ]
        ));
        assert!(session.pending_login().is_none());
        assert!(session.account().is_none());

        let (correlation, _) = start_login(&mut session);
        let mut zero = account();
        zero.id = 0;
        let error = session
            .on_login_result(AccountLoginCompletion {
                handle: HANDLE,
                correlation,
                outcome: AccountLoginOutcome::Success(zero),
            })
            .unwrap_err();
        assert_eq!(error, AccountPlayerError::ZeroAccountId);
        assert_eq!(session.pending_login().unwrap().correlation, correlation);
    }

    #[test]
    fn account_status_uses_c_string_bytes_and_ignores_source_tail() {
        let mut session = AccountPlayerSession::new(HANDLE);
        let (correlation, _) = start_login(&mut session);
        let mut trailing = account();
        trailing.status = [b'O', b'K', 0, b'X', b'Y', 0, 0, 0, 0];
        let reduction = complete_login(&mut session, correlation, trailing);
        assert_eq!(reduction.state.phase(), AccountPlayerPhase::Select);

        let mut unterminated = AccountPlayerSession::new(HANDLE);
        let (correlation, _) = start_login(&mut unterminated);
        let mut bad = account();
        bad.status = *b"OKX\0\0\0\0\0\0";
        let rejected = unterminated
            .on_login_result(AccountLoginCompletion {
                handle: HANDLE,
                correlation,
                outcome: AccountLoginOutcome::Success(bad),
            })
            .unwrap();
        assert_eq!(rejected.state.phase(), AccountPlayerPhase::Login);
        assert!(rejected.state.pending_login().is_none());
        assert!(rejected.state.account().is_none());
    }

    #[test]
    fn non_ok_status_runs_logout_before_identity_validation() {
        let mut session = AccountPlayerSession::new(HANDLE);
        let (correlation, _) = start_login(&mut session);
        let mut value = account();
        value.id = 0;
        value.login = c_bytes("other");
        value.status = c_bytes("LOCKED");
        let reduction = session
            .on_login_result(AccountLoginCompletion {
                handle: HANDLE,
                correlation,
                outcome: AccountLoginOutcome::Success(value),
            })
            .unwrap();
        assert!(matches!(
            reduction.effects.as_slice(),
            [
                AccountPlayerEffect::RequestLogout { .. },
                AccountPlayerEffect::LoginFailure { .. }
            ]
        ));
        assert!(session.pending_login().is_none());
    }

    #[test]
    fn unterminated_account_status_is_truncated_with_a_nul_like_strlcpy() {
        let mut session = AccountPlayerSession::new(HANDLE);
        let (correlation, _) = start_login(&mut session);
        let mut value = account();
        value.status = *b"123456789";
        let reduction = session
            .on_login_result(AccountLoginCompletion {
                handle: HANDLE,
                correlation,
                outcome: AccountLoginOutcome::Success(value),
            })
            .unwrap();
        let effect = &reduction.effects[1];
        let AccountPlayerEffect::LoginFailure { status } = effect else {
            panic!("expected login failure");
        };
        assert_eq!(&status[..8], b"12345678");
        assert_eq!(status[8], 0);
    }

    #[test]
    fn raw_frame_checks_handle_before_decoding_malformed_payload() {
        let mut session = AccountPlayerSession::new(HANDLE);
        let (correlation, _) = start_login(&mut session);
        let malformed = DbFrame::new(HEADER_DG_LOGIN_SUCCESS, HANDLE + 1, vec![1]);
        assert!(matches!(
            session.on_login_frame(&malformed, correlation),
            Err(AccountPlayerError::HandleMismatch { .. })
        ));
        assert!(session.pending_login().is_some());
    }

    #[test]
    fn wrong_password_header_is_not_a_positive_missing_result() {
        let mut session = AccountPlayerSession::new(HANDLE);
        let (correlation, _) = start_login(&mut session);
        let frame = DbFrame::new(
            protocol::db_records::HEADER_DG_LOGIN_WRONG_PASSWD,
            HANDLE,
            vec![],
        );
        assert!(matches!(
            session.on_login_frame(&frame, correlation),
            Err(AccountPlayerError::UnsupportedResponseHeader { .. })
        ));
        assert!(session.pending_login().is_some());
    }

    #[test]
    fn selection_rejects_wrong_phase_index_empty_slot_and_name_change() {
        let mut no_account = AccountPlayerSession::new(HANDLE);
        assert!(matches!(
            no_account.on_select(0),
            Err(AccountPlayerError::InvalidPhase { .. })
        ));

        let mut out_of_range = AccountPlayerSession::new(HANDLE);
        let (correlation, _) = start_login(&mut out_of_range);
        complete_login(&mut out_of_range, correlation, account());
        assert_eq!(
            out_of_range.on_select(4).unwrap_err(),
            AccountPlayerError::InvalidPlayerIndex { index: 4 }
        );

        let mut empty = AccountPlayerSession::new(HANDLE);
        let (correlation, _) = start_login(&mut empty);
        let mut empty_account = account();
        empty_account.players[0].id = 0;
        complete_login(&mut empty, correlation, empty_account);
        assert_eq!(
            empty.on_select(0).unwrap_err(),
            AccountPlayerError::PlayerIdMissing { index: 0 }
        );

        let mut rename = AccountPlayerSession::new(HANDLE);
        let (correlation, _) = start_login(&mut rename);
        let mut rename_account = account();
        rename_account.players[0].change_name = 1;
        complete_login(&mut rename, correlation, rename_account);
        assert_eq!(
            rename.on_select(0).unwrap_err(),
            AccountPlayerError::NameChangeRequired { index: 0 }
        );
        assert!(rename.pending_player().is_none());
    }

    #[test]
    fn player_missing_is_a_noop_but_clears_pending_and_db_error_is_distinct() {
        let mut session = AccountPlayerSession::new(HANDLE);
        let (login_id, _) = start_login(&mut session);
        complete_login(&mut session, login_id, account());
        let (player_id, _) = select_player(&mut session);
        let error = session.on_player_error(HANDLE, player_id).unwrap_err();
        assert!(matches!(error, AccountPlayerError::Database { .. }));
        let pending = session.pending_player().unwrap();
        assert_eq!(pending.correlation, player_id);
        assert!(matches!(
            session.retry_player_effects().unwrap().as_slice(),
            [AccountPlayerEffect::SendPlayerLoad {
                correlation,
                request,
                ..
            }] if *correlation == player_id && *request == pending.request
        ));
        assert!(matches!(
            session.on_select(0),
            Err(AccountPlayerError::PlayerAlreadyPending)
        ));
        let missing = session
            .on_player_result(AccountPlayerCompletion {
                handle: HANDLE,
                correlation: player_id,
                outcome: AccountPlayerOutcome::Missing,
            })
            .unwrap();
        assert!(missing.effects.is_empty());
        assert_eq!(missing.state.phase(), AccountPlayerPhase::Select);
        assert!(session.pending_player().is_none());
    }

    #[test]
    fn player_id_mismatch_is_rejected_and_stale_success_cannot_rebind() {
        let mut session = AccountPlayerSession::new(HANDLE);
        let (login_id, _) = start_login(&mut session);
        complete_login(&mut session, login_id, account());
        let (player_id, _) = select_player(&mut session);
        let mut wrong = player();
        wrong.id = 0xdead_beef;
        assert_eq!(
            session
                .on_player_result(AccountPlayerCompletion {
                    handle: HANDLE,
                    correlation: player_id,
                    outcome: AccountPlayerOutcome::Success(wrong),
                })
                .unwrap_err(),
            AccountPlayerError::PlayerIdMismatch {
                expected: 0x1112_1314,
                actual: 0xdead_beef,
            }
        );
        assert!(session.pending_player().is_some());
        session
            .on_player_result(AccountPlayerCompletion {
                handle: HANDLE,
                correlation: player_id,
                outcome: AccountPlayerOutcome::Success(player()),
            })
            .unwrap();
        assert_eq!(
            session
                .on_player_result(AccountPlayerCompletion {
                    handle: HANDLE,
                    correlation: player_id,
                    outcome: AccountPlayerOutcome::Success(player()),
                })
                .unwrap_err(),
            AccountPlayerError::PlayerNotPending
        );
        assert_eq!(session.phase(), AccountPlayerPhase::Loading);
    }

    #[test]
    fn raw_frames_decode_exact_records_and_reject_wrong_or_malformed_payloads() {
        let mut session = AccountPlayerSession::new(HANDLE);
        let (login_id, _) = start_login(&mut session);
        let success = DbFrame::new(HEADER_DG_LOGIN_SUCCESS, HANDLE, account().encode());
        let reduction = session.on_login_frame(&success, login_id).unwrap();
        assert_eq!(reduction.state.phase(), AccountPlayerPhase::Select);

        let (player_id, _) = select_player(&mut session);
        let player_frame = DbFrame::new(HEADER_DG_PLAYER_LOAD_SUCCESS, HANDLE, player().encode());
        assert_eq!(
            session
                .on_player_frame(&player_frame, player_id)
                .unwrap()
                .state
                .phase(),
            AccountPlayerPhase::Loading
        );

        let mut new_session = AccountPlayerSession::new(HANDLE);
        let (login_id, _) = start_login(&mut new_session);
        let malformed = DbFrame::new(HEADER_DG_LOGIN_SUCCESS, HANDLE, vec![1]);
        assert!(matches!(
            new_session.on_login_frame(&malformed, login_id),
            Err(AccountPlayerError::MalformedLoginSuccess { .. })
        ));
        assert!(new_session.pending_login().is_some());
        let unsupported = DbFrame::new(HEADER_DG_LOGIN_NOT_EXIST + 100, HANDLE, vec![]);
        assert!(matches!(
            new_session.on_login_frame(&unsupported, login_id),
            Err(AccountPlayerError::UnsupportedResponseHeader { .. })
        ));
        let mut player_session = AccountPlayerSession::new(HANDLE);
        let (login_id, _) = start_login(&mut player_session);
        complete_login(&mut player_session, login_id, account());
        let (player_id, _) = select_player(&mut player_session);
        let missing_with_bytes = DbFrame::new(HEADER_DG_PLAYER_LOAD_FAILED, HANDLE, vec![7]);
        assert!(matches!(
            player_session.on_player_frame(&missing_with_bytes, player_id),
            Err(AccountPlayerError::MalformedPlayerMissing { .. })
        ));
        assert!(player_session.pending_player().is_some());
    }

    #[test]
    fn close_clears_all_state_and_late_completion_is_ignored() {
        let mut session = AccountPlayerSession::new(HANDLE);
        let (login_id, _) = start_login(&mut session);
        let close = session.close();
        assert_eq!(close.effects, vec![AccountPlayerEffect::Close]);
        assert_eq!(close.state.phase(), AccountPlayerPhase::Closed);
        assert!(close.state.account().is_none());
        assert!(close.state.login_key_material().is_none());
        assert!(close.state.pending_login().is_none());
        let late = session
            .on_login_result(AccountLoginCompletion {
                handle: HANDLE,
                correlation: login_id,
                outcome: AccountLoginOutcome::Success(account()),
            })
            .unwrap();
        assert!(late.effects.is_empty());
        assert_eq!(late.state.phase(), AccountPlayerPhase::Closed);
        assert!(session.close().effects.is_empty());
    }
}
