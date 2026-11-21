//! Transport-free account/player routing beside the lifecycle adapter.
//!
//! This module connects the already-framed account client records and the
//! caller-routed DB completions to the transport-free `AccountPlayerSession`
//! and `ClientLifecycle` reducers. It owns neither a socket, a descriptor,
//! TEA, a DB peer, nor a character/world. The methods take `&self` and return a
//! candidate router state. A caller must apply the returned effects in order
//! and commit the candidate only after every effect has been accepted.
//!
//! The legacy `DESC::SetPhase` path writes `GC_PHASE` before installing the
//! next input processor. `AccountRouterEffect::Lifecycle` therefore remains
//! an effect, not an applied phase switch. The account bind intent precedes
//! that lifecycle effect exactly as the legacy `BindAccountTable`/`SetPhase`
//! sequence requires. A production adapter must separately perform the
//! blocked-IP, shutdown, and user-limit admission checks, apply key/DB/close
//! intents, route the full correlation generation, and perform all legacy
//! character, map, P2P, and world teardown.

use std::error::Error;
use std::fmt;

use protocol::cg_account::{CgAccountError, CgEnterGame, CgLogin, CgLoginByKey, CgPlayerSelect};
use protocol::cg_inventory::{
    HEADER_CG_CHARACTER_SELECT, HEADER_CG_ENTERGAME, HEADER_CG_LOGIN, HEADER_CG_LOGIN2,
};
use protocol::cg_wire::ClientFrame;

use crate::account_player::{
    AccountCorrelationId, AccountKeyInstallation, AccountLoginCompletion, AccountLoginInput,
    AccountPlayerCompletion, AccountPlayerEffect, AccountPlayerError, AccountPlayerPhase,
    AccountPlayerReduction, AccountPlayerSession,
};
use crate::account_records::{LEGACY_IP_BYTES, LEGACY_LOGIN_BYTES};
use crate::client_session::ClientPhase;
use crate::lifecycle::{ClientLifecycle, LifecycleEffect, LifecycleError};

/// Caller-owned values needed to turn a login-by-key frame into the legacy
/// normalized DB request.
///
/// `CgLoginByKey::login` is deliberately opaque at the protocol boundary. The
/// legacy handler applies locale-sensitive trimming/lowercasing before it
/// retains the key material. This router does not guess that policy. The caller
/// must supply the normalized fixed-width login and host/IP fields, including a
/// NUL followed by an all-zero tail; the account reducer validates those
/// canonical tails before it emits any effect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoginContext {
    /// Normalized `TPacketGDLoginByKey.szLogin` bytes.
    pub normalized_login: [u8; LEGACY_LOGIN_BYTES],
    /// Canonical descriptor host bytes copied to `TPacketGDLoginByKey.szIP`.
    pub normalized_ip: [u8; LEGACY_IP_BYTES],
    /// Explicit source-build key-installation policy.
    pub key_installation: AccountKeyInstallation,
}

impl LoginContext {
    /// Construct caller-supplied normalized login context.
    #[must_use]
    pub const fn new(
        normalized_login: [u8; LEGACY_LOGIN_BYTES],
        normalized_ip: [u8; LEGACY_IP_BYTES],
        key_installation: AccountKeyInstallation,
    ) -> Self {
        Self {
            normalized_login,
            normalized_ip,
            key_installation,
        }
    }
}

/// The account seam operation associated with a phase or routing error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountRouterOperation {
    /// Start the initial key-login request.
    Login,
    /// Start a player-load request from a character-selection frame.
    PlayerSelect,
    /// Apply a player-load completion.
    PlayerCompletion,
    /// Apply a login completion.
    LoginCompletion,
    /// Process the header-only enter-game request.
    EnterGame,
}

/// An account-router effect in the same order as the source-observed effects.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum AccountRouterEffect {
    /// An account/player reducer intent such as a bind, DB request, logout, or
    /// close. It is not an installed key or a transmitted packet.
    Account(AccountPlayerEffect),
    /// A descriptor lifecycle effect. In particular, `SetPhase` still has to
    /// be applied by the caller's descriptor boundary with its old output
    /// boundary and new input boundary.
    Lifecycle(LifecycleEffect),
}

/// Candidate state and ordered effects returned by one router operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountRouterReduction {
    /// State after the operation. Commit it together with the router after
    /// applying all returned effects.
    pub state: AccountPlayerRouter,
    /// Account and lifecycle effects in source order.
    pub effects: Vec<AccountRouterEffect>,
}

/// A transport-free pair of lifecycle and account/player states for one
/// descriptor-shaped candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountPlayerRouter {
    lifecycle: ClientLifecycle,
    account: AccountPlayerSession,
}

impl AccountPlayerRouter {
    /// Pair a caller-created lifecycle with a caller-created account reducer.
    ///
    /// The constructor does not infer a phase or profile. A caller normally
    /// supplies the post-handshake `ClientPhase::Login` lifecycle and a session
    /// created with a fresh correlation seed. The routing methods enforce the
    /// required outer/local phase pairing before they invoke either reducer.
    #[must_use]
    pub const fn new(lifecycle: ClientLifecycle, account: AccountPlayerSession) -> Self {
        Self { lifecycle, account }
    }

    /// Borrow the adjacent lifecycle candidate.
    #[must_use]
    pub const fn lifecycle(&self) -> ClientLifecycle {
        self.lifecycle
    }

    /// Borrow the adjacent account/player candidate.
    #[must_use]
    pub const fn account(&self) -> &AccountPlayerSession {
        &self.account
    }

    /// Consume the router and return its two independent state objects.
    #[must_use]
    pub fn into_parts(self) -> (ClientLifecycle, AccountPlayerSession) {
        (self.lifecycle, self.account)
    }

    /// Route one already-framed client record.
    ///
    /// `LOGIN2` uses the frame's login key and four little-endian client-key
    /// words, while `LoginContext` supplies the already-normalized login and
    /// host/IP bytes. The frame's opaque login buffer is not normalized or
    /// compared by this transport-free seam. `PLAYER_SELECT` is delegated to
    /// the account reducer. `ENTERGAME` is recognized and validated but cannot
    /// be committed until a caller-owned world/character prerequisite exists.
    /// Password `LOGIN` and all other headers are explicit unsupported
    /// boundaries; they are not silently consumed.
    ///
    /// # Errors
    ///
    /// Returns a protocol error for a malformed recognized frame, a missing
    /// login context, a phase mismatch, an explicit unsupported header, or the
    /// world-prerequisite gate for a valid enter-game frame.
    pub fn on_client_frame(
        &self,
        frame: &ClientFrame,
        login_context: Option<LoginContext>,
    ) -> Result<AccountRouterReduction, AccountRouterError> {
        match frame.header {
            header if header == HEADER_CG_LOGIN2.value() => {
                let packet =
                    CgLoginByKey::decode_frame(frame).map_err(AccountRouterError::Protocol)?;
                let context = login_context.ok_or(AccountRouterError::LoginContextRequired)?;
                self.on_login_input(
                    AccountLoginInput::new(
                        context.normalized_login,
                        packet.login_key,
                        packet.client_key,
                        context.normalized_ip,
                    ),
                    context.key_installation,
                )
            }
            header if header == HEADER_CG_CHARACTER_SELECT.value() => {
                let packet =
                    CgPlayerSelect::decode_frame(frame).map_err(AccountRouterError::Protocol)?;
                self.on_player_select(packet.index)
            }
            header if header == HEADER_CG_ENTERGAME.value() => {
                CgEnterGame::decode_frame(frame).map_err(AccountRouterError::Protocol)?;
                self.require_phase(
                    AccountRouterOperation::EnterGame,
                    ClientPhase::Loading,
                    AccountPlayerPhase::Loading,
                )?;
                Err(AccountRouterError::WorldPrerequisiteRequired)
            }
            header if header == HEADER_CG_LOGIN.value() => {
                // Validate the fixed password record before reporting that
                // this reducer intentionally does not route password login.
                CgLogin::decode_frame(frame).map_err(AccountRouterError::Protocol)?;
                Err(AccountRouterError::PasswordLoginUnsupported { header })
            }
            header => Err(AccountRouterError::UnsupportedHeader { header }),
        }
    }

    /// Route a caller-normalized login input without decoding a client frame.
    ///
    /// This is the same path used after the caller has applied the legacy
    /// normalization policy to a `CgLoginByKey` record. It is transport-free
    /// and does not perform admission checks.
    ///
    /// # Errors
    ///
    /// Returns an account validation/correlation error or a lifecycle phase
    /// mismatch.
    pub fn on_login_input(
        &self,
        input: AccountLoginInput,
        key_installation: AccountKeyInstallation,
    ) -> Result<AccountRouterReduction, AccountRouterError> {
        self.require_phase(
            AccountRouterOperation::Login,
            ClientPhase::Login,
            AccountPlayerPhase::Login,
        )?;
        let mut account = self.account.clone();
        let reduction = account
            .on_login_with_key_installation(input, key_installation)
            .map_err(AccountRouterError::Account)?;
        self.finish_account_reduction(reduction)
    }

    /// Route a decoded character-selection index.
    ///
    /// # Errors
    ///
    /// Returns an account error for an invalid slot, missing player, or
    /// correlation exhaustion, or a phase mismatch.
    pub fn on_player_select(
        &self,
        index: u8,
    ) -> Result<AccountRouterReduction, AccountRouterError> {
        self.require_phase(
            AccountRouterOperation::PlayerSelect,
            ClientPhase::Select,
            AccountPlayerPhase::Select,
        )?;
        let mut account = self.account.clone();
        let reduction = account
            .on_select(index)
            .map_err(AccountRouterError::Account)?;
        self.finish_account_reduction(reduction)
    }

    /// Route a login completion from the account store.
    ///
    /// The completion carries the full [`AccountCorrelationId`] retained when
    /// the request was issued; this method never infers a generation from a
    /// `u32` handle.
    ///
    /// # Errors
    ///
    /// Returns a handle/correlation, account identity, or lifecycle phase
    /// error. A successful response is returned as a candidate with
    /// `BindAccount` before `SetPhase(Select)`.
    pub fn on_login_result(
        &self,
        completion: AccountLoginCompletion,
    ) -> Result<AccountRouterReduction, AccountRouterError> {
        self.require_phase(
            AccountRouterOperation::LoginCompletion,
            ClientPhase::Login,
            AccountPlayerPhase::Login,
        )?;
        let mut account = self.account.clone();
        let reduction = account
            .on_login_result(completion)
            .map_err(AccountRouterError::Account)?;
        self.finish_account_reduction(reduction)
    }

    /// Route a character-load completion from the account store.
    ///
    /// # Errors
    ///
    /// Returns a handle/correlation, player identity, or lifecycle phase
    /// error. A successful response is returned as a candidate with
    /// `BindPlayer` before `SetPhase(Loading)`.
    pub fn on_player_result(
        &self,
        completion: AccountPlayerCompletion,
    ) -> Result<AccountRouterReduction, AccountRouterError> {
        self.require_phase(
            AccountRouterOperation::PlayerCompletion,
            ClientPhase::Select,
            AccountPlayerPhase::Select,
        )?;
        let mut account = self.account.clone();
        let reduction = account
            .on_player_result(completion)
            .map_err(AccountRouterError::Account)?;
        self.finish_account_reduction(reduction)
    }

    /// Route a DB transport error for a pending login.
    ///
    /// The account reducer deliberately returns a distinct `Database` error
    /// and retains the pending generation and key policy. The router leaves
    /// its state unchanged; callers can use the account accessor's
    /// `retry_login_effects` helper rather than manufacturing a missing row.
    ///
    /// # Errors
    ///
    /// Returns the account correlation/DB error or a lifecycle phase mismatch.
    pub fn on_login_error(
        &self,
        handle: u32,
        correlation: AccountCorrelationId,
    ) -> Result<AccountRouterReduction, AccountRouterError> {
        self.require_phase(
            AccountRouterOperation::LoginCompletion,
            ClientPhase::Login,
            AccountPlayerPhase::Login,
        )?;
        let mut account = self.account.clone();
        let reduction = account
            .on_login_error(handle, correlation)
            .map_err(AccountRouterError::Account)?;
        self.finish_account_reduction(reduction)
    }

    /// Route a DB transport error for a pending player load.
    ///
    /// # Errors
    ///
    /// Returns the account correlation/DB error or a lifecycle phase mismatch.
    pub fn on_player_error(
        &self,
        handle: u32,
        correlation: AccountCorrelationId,
    ) -> Result<AccountRouterReduction, AccountRouterError> {
        self.require_phase(
            AccountRouterOperation::PlayerCompletion,
            ClientPhase::Select,
            AccountPlayerPhase::Select,
        )?;
        let mut account = self.account.clone();
        let reduction = account
            .on_player_error(handle, correlation)
            .map_err(AccountRouterError::Account)?;
        self.finish_account_reduction(reduction)
    }

    fn require_phase(
        &self,
        operation: AccountRouterOperation,
        expected_lifecycle: ClientPhase,
        expected_account: AccountPlayerPhase,
    ) -> Result<(), AccountRouterError> {
        let actual_lifecycle = self.lifecycle.phase();
        let actual_account = self.account.phase();
        if actual_lifecycle == expected_lifecycle && actual_account == expected_account {
            Ok(())
        } else {
            Err(AccountRouterError::PhaseMismatch {
                operation,
                lifecycle: actual_lifecycle,
                account: actual_account,
            })
        }
    }

    fn finish_account_reduction(
        &self,
        reduction: AccountPlayerReduction,
    ) -> Result<AccountRouterReduction, AccountRouterError> {
        let mut lifecycle = self.lifecycle;
        let mut effects = Vec::with_capacity(reduction.effects.len());
        for effect in reduction.effects {
            match effect {
                AccountPlayerEffect::TransitionPhase { phase } => {
                    let next = lifecycle
                        .transition_to(phase)
                        .map_err(AccountRouterError::Lifecycle)?;
                    effects.extend(next.effects.into_iter().map(AccountRouterEffect::Lifecycle));
                    lifecycle = next.state;
                }
                other => effects.push(AccountRouterEffect::Account(other)),
            }
        }
        Ok(AccountRouterReduction {
            state: Self {
                lifecycle,
                account: reduction.state,
            },
            effects,
        })
    }
}

/// A rejection at the transport-free account/lifecycle router boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountRouterError {
    /// A recognized fixed account frame had an invalid shape or header.
    Protocol(CgAccountError),
    /// A valid password-login frame is outside this reducer slice.
    PasswordLoginUnsupported {
        /// Exact password-login header.
        header: u8,
    },
    /// A valid login-by-key frame needs caller-supplied normalized context.
    LoginContextRequired,
    /// A valid enter-game frame reached the world/character prerequisite gate.
    WorldPrerequisiteRequired,
    /// The frame header is not handled by this narrow account router.
    UnsupportedHeader {
        /// Unhandled one-byte client header.
        header: u8,
    },
    /// The outer lifecycle and local account state are not the required pair.
    PhaseMismatch {
        /// Operation that was rejected.
        operation: AccountRouterOperation,
        /// Current outer lifecycle phase.
        lifecycle: ClientPhase,
        /// Current local account/player phase.
        account: AccountPlayerPhase,
    },
    /// The account/player reducer rejected the request or completion.
    Account(AccountPlayerError),
    /// The adjacent lifecycle reducer rejected a phase transition.
    Lifecycle(LifecycleError),
}

impl fmt::Display for AccountRouterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(source) => write!(formatter, "malformed account client frame: {source}"),
            Self::PasswordLoginUnsupported { header } => {
                write!(formatter, "password-login header 0x{header:02x} is not routed")
            }
            Self::LoginContextRequired => {
                formatter.write_str("login-by-key routing requires normalized caller context")
            }
            Self::WorldPrerequisiteRequired => {
                formatter.write_str("enter-game requires a world and character prerequisite")
            }
            Self::UnsupportedHeader { header } => {
                write!(formatter, "account router does not handle client header 0x{header:02x}")
            }
            Self::PhaseMismatch {
                operation,
                lifecycle,
                account,
            } => write!(
                formatter,
                "{operation:?} requires aligned lifecycle/account phases; current phases are {lifecycle:?}/{account:?}"
            ),
            Self::Account(source) => write!(formatter, "account/player routing error: {source}"),
            Self::Lifecycle(source) => write!(formatter, "lifecycle routing error: {source}"),
        }
    }
}

impl Error for AccountRouterError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Protocol(source) => Some(source),
            Self::Account(source) => Some(source),
            Self::Lifecycle(source) => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_player::{AccountLoginOutcome, AccountPlayerOutcome};
    use crate::account_records::{LoginAccountRecord, PlayerResultRecord};
    use crate::handshake::HandshakeServerKind;
    use protocol::cg_handshake::{CgHandshakeHeader, CgInboundHandshake};
    use protocol::simple_player::SimplePlayerRecord;

    const HANDLE: u32 = 0x1234_5678;
    const SEED: u32 = 0x1000_0001;
    const TOKEN: u32 = 0x0102_0304;

    fn c_bytes<const N: usize>(value: &str) -> [u8; N] {
        let mut bytes = [0; N];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        bytes
    }

    fn login_lifecycle() -> ClientLifecycle {
        let start = ClientLifecycle::start(TOKEN, HandshakeServerKind::Game, 0);
        start
            .state
            .on_handshake(
                CgInboundHandshake::new(CgHandshakeHeader::Handshake, TOKEN, 0, 0),
                0,
            )
            .unwrap()
            .state
    }

    fn router() -> AccountPlayerRouter {
        AccountPlayerRouter::new(
            login_lifecycle(),
            AccountPlayerSession::new_with_correlation_seed(HANDLE, SEED),
        )
    }

    fn context() -> LoginContext {
        let mut ip = [0; LEGACY_IP_BYTES];
        ip[..9].copy_from_slice(b"127.0.0.1");
        LoginContext::new(
            c_bytes("alice"),
            ip,
            AccountKeyInstallation::ImprovedPacketEncryption,
        )
    }

    fn account() -> LoginAccountRecord {
        let player = SimplePlayerRecord {
            id: 0x1112_1314,
            name: c_bytes("hero"),
            ..SimplePlayerRecord::default()
        };
        LoginAccountRecord {
            id: 0x0102_0304,
            login: c_bytes("alice"),
            status: c_bytes("OK"),
            players: [player; 4],
            ..LoginAccountRecord::default()
        }
    }

    fn player() -> PlayerResultRecord {
        PlayerResultRecord {
            id: 0x1112_1314,
            ..PlayerResultRecord::default()
        }
    }

    fn login_success(correlation: AccountCorrelationId) -> AccountLoginCompletion {
        AccountLoginCompletion {
            handle: HANDLE,
            correlation,
            outcome: AccountLoginOutcome::Success(account()),
        }
    }

    fn player_success(correlation: AccountCorrelationId) -> AccountPlayerCompletion {
        AccountPlayerCompletion {
            handle: HANDLE,
            correlation,
            outcome: AccountPlayerOutcome::Success(player()),
        }
    }

    fn login_frame() -> ClientFrame {
        CgLoginByKey::new(
            c_bytes("ALICE"),
            0x1020_3040,
            [1, 0x0102_0304, 0x0506_0708, 0x090a_0b0c],
        )
        .to_frame()
    }

    #[test]
    fn login_frame_routes_normalized_context_and_returns_candidate_without_mutating_router() {
        let original = router();
        let result = original
            .on_client_frame(&login_frame(), Some(context()))
            .unwrap();
        assert_eq!(original.account().phase(), AccountPlayerPhase::Login);
        assert!(original.account().pending_login().is_none());
        assert_eq!(result.state.account().phase(), AccountPlayerPhase::Login);
        assert!(matches!(
            result.effects.as_slice(),
            [
                AccountRouterEffect::Account(AccountPlayerEffect::BindLoginKeyMaterial { .. }),
                AccountRouterEffect::Account(AccountPlayerEffect::SendLoginByKey { .. }),
            ]
        ));
        let pending = result.state.account().pending_login().unwrap();
        assert_eq!(pending.correlation.get(), (u64::from(SEED) << 32) | 1);
        assert_eq!(pending.request.login_key, 0x1020_3040);
    }

    #[test]
    fn missing_context_and_password_login_are_explicit_boundaries() {
        let original = router();
        assert_eq!(
            original.on_client_frame(&login_frame(), None),
            Err(AccountRouterError::LoginContextRequired)
        );
        let password = CgLogin::new([0; LEGACY_LOGIN_BYTES], [0; 17]).to_frame();
        assert_eq!(
            original.on_client_frame(&password, None),
            Err(AccountRouterError::PasswordLoginUnsupported { header: 1 })
        );
        assert!(original.account().pending_login().is_none());
    }

    #[test]
    fn login_completion_binds_account_before_lifecycle_select_effect() {
        let login = router()
            .on_client_frame(&login_frame(), Some(context()))
            .unwrap();
        let pending = login.state.account().pending_login().unwrap().clone();
        let result = login
            .state
            .on_login_result(login_success(pending.correlation))
            .unwrap();
        assert_eq!(result.state.lifecycle().phase(), ClientPhase::Select);
        assert_eq!(result.state.account().phase(), AccountPlayerPhase::Select);
        assert!(matches!(
            result.effects.as_slice(),
            [
                AccountRouterEffect::Account(AccountPlayerEffect::BindAccount { .. }),
                AccountRouterEffect::Lifecycle(LifecycleEffect::SetPhase {
                    phase: ClientPhase::Select,
                    ..
                }),
            ]
        ));
    }

    #[test]
    fn selection_and_player_completion_keep_bind_before_loading_effect() {
        let login = router()
            .on_client_frame(&login_frame(), Some(context()))
            .unwrap();
        let pending_login = login.state.account().pending_login().unwrap().correlation;
        let selected = login
            .state
            .on_login_result(login_success(pending_login))
            .unwrap();
        let select = selected
            .state
            .on_client_frame(&CgPlayerSelect::new(0).to_frame(), None)
            .unwrap();
        assert!(matches!(
            select.effects.as_slice(),
            [AccountRouterEffect::Account(
                AccountPlayerEffect::SendPlayerLoad { .. }
            )]
        ));
        let pending_player = select.state.account().pending_player().unwrap().clone();
        let loaded = select
            .state
            .on_player_result(player_success(pending_player.correlation))
            .unwrap();
        assert_eq!(loaded.state.lifecycle().phase(), ClientPhase::Loading);
        assert_eq!(loaded.state.account().phase(), AccountPlayerPhase::Loading);
        assert!(matches!(
            loaded.effects.as_slice(),
            [
                AccountRouterEffect::Account(AccountPlayerEffect::BindPlayer { .. }),
                AccountRouterEffect::Lifecycle(LifecycleEffect::SetPhase {
                    phase: ClientPhase::Loading,
                    ..
                }),
            ]
        ));
    }

    #[test]
    fn enter_game_is_validated_but_not_committed_without_world_prerequisite() {
        let login = router()
            .on_client_frame(&login_frame(), Some(context()))
            .unwrap();
        let pending_login = login.state.account().pending_login().unwrap().correlation;
        let selected = login
            .state
            .on_login_result(login_success(pending_login))
            .unwrap();
        let select = selected
            .state
            .on_client_frame(&CgPlayerSelect::new(0).to_frame(), None)
            .unwrap();
        let pending_player = select.state.account().pending_player().unwrap().correlation;
        let loaded = select
            .state
            .on_player_result(player_success(pending_player))
            .unwrap();
        let before = loaded.state.clone();
        assert_eq!(
            before.on_client_frame(&CgEnterGame::new().to_frame(), None),
            Err(AccountRouterError::WorldPrerequisiteRequired)
        );
        assert_eq!(before.lifecycle().phase(), ClientPhase::Loading);
        assert_eq!(before.account().phase(), AccountPlayerPhase::Loading);
    }

    #[test]
    fn phase_mismatch_is_checked_before_account_reducer_mutation() {
        let start = ClientLifecycle::start(TOKEN, HandshakeServerKind::Game, 0);
        let wrong = AccountPlayerRouter::new(
            start.state,
            AccountPlayerSession::new_with_correlation_seed(HANDLE, SEED),
        );
        let error = wrong
            .on_client_frame(&login_frame(), Some(context()))
            .unwrap_err();
        assert!(matches!(
            error,
            AccountRouterError::PhaseMismatch {
                operation: AccountRouterOperation::Login,
                lifecycle: ClientPhase::Handshake,
                account: AccountPlayerPhase::Login,
            }
        ));
        assert!(wrong.account().pending_login().is_none());
    }

    #[test]
    fn db_error_keeps_pending_generation_for_retry() {
        let login = router()
            .on_client_frame(&login_frame(), Some(context()))
            .unwrap();
        let correlation = login.state.account().pending_login().unwrap().correlation;
        let error = login.state.on_login_error(HANDLE, correlation).unwrap_err();
        assert!(matches!(error, AccountRouterError::Account(_)));
        assert_eq!(
            login.state.account().pending_login().unwrap().correlation,
            correlation
        );
        let retry = login.state.account().retry_login_effects().unwrap();
        assert!(matches!(
            retry.as_slice(),
            [
                AccountPlayerEffect::BindLoginKeyMaterial { .. },
                AccountPlayerEffect::SendLoginByKey { .. },
            ]
        ));
    }
}
