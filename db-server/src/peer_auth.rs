//! Transport-identity classification for DB peers.
//!
//! # This is not authentication
//!
//! The legacy DB protocol has no peer-authentication exchange. The legacy
//! server's only related control is an optional source-address filter on the
//! listener, plus a raw `bAuthServer` byte in the setup request. Neither is a
//! credential. A source address can be spoofed by anything on the path, and
//! the legacy `AGENTS.md` compatibility rules state plainly that a source
//! filter is not role authentication.
//!
//! This module therefore names what it actually does. It maps a connection's
//! [`SocketAddr`] to a [`DbPeerRole`] by comparing against an operator-supplied
//! allowlist, and it exists so that a transport adapter has one explicit,
//! testable place where a role is assigned. It must not be described as
//! authentication, and it is not a substitute for the credential-based adapter
//! a production deployment needs.
//!
//! # Fail-closed default
//!
//! Both allowlists default to empty, which classifies every peer as
//! [`DbPeerRole::Unclassified`] and therefore leaves the peer's
//! [`crate::peer_policy::DbPeerPolicy`] unauthenticated. Every operation is then
//! denied, including
//! the read-only boot request, and the connection is closed without a reply.
//! A `db.toml` that says nothing about trusted peers serves nobody.
//!
//! # What this does not grant
//!
//! Assigning [`DbPeerRole::Game`] or [`DbPeerRole::Auth`] here does not
//! authenticate the peer, does not prove the peer is the process it claims to
//! be, and does not bind any account or player. It only removes the
//! `AuthenticationRequired` denial. The operation matrix in
//! [`crate::peer_policy`] still refuses every operation except read-only boot
//! and base-only auth setup, and the auth slot is still bound only by an
//! explicit effect.

use std::error::Error;
use std::fmt;
use std::net::SocketAddr;

use crate::peer_policy::DbPeerRole;

/// Why a configured peer address was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerAuthConfigError {
    /// A configured entry is not a valid socket address.
    InvalidAddress {
        /// The rejected text from the configuration file.
        value: String,
        /// The parser's message.
        message: String,
    },
    /// The same address appears in both allowlists.
    ConflictingRole {
        /// The address that appears twice.
        address: String,
    },
}

impl fmt::Display for PeerAuthConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAddress { value, message } => write!(
                formatter,
                "trusted peer address {value:?} is not a valid socket address: {message}"
            ),
            Self::ConflictingRole { address } => write!(
                formatter,
                "trusted peer address {address} is listed as both a game and an auth peer"
            ),
        }
    }
}

impl Error for PeerAuthConfigError {}

/// Why a connection was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerAuthOutcome {
    /// The address matched an allowlist entry.
    Classified {
        /// The role granted to this connection.
        role: DbPeerRole,
    },
    /// No allowlist entry matched. The peer stays unauthenticated.
    Denied,
}

impl PeerAuthOutcome {
    /// Return the granted role, or `None` when the peer was refused.
    #[must_use]
    pub const fn role(self) -> Option<DbPeerRole> {
        match self {
            Self::Classified { role } => Some(role),
            Self::Denied => None,
        }
    }

    /// Return whether the peer was refused.
    #[must_use]
    pub const fn is_denied(self) -> bool {
        matches!(self, Self::Denied)
    }
}

/// An address allowlist that classifies a connection's role.
///
/// The comparison is by the address's IP only. A configured port is accepted
/// and ignored, because the legacy peer is a long-lived connection whose source
/// port is chosen by the operating system, not by the peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerAuthenticator {
    game: Vec<std::net::IpAddr>,
    auth: Vec<std::net::IpAddr>,
}

impl PeerAuthenticator {
    /// Build an authenticator from raw configuration strings.
    ///
    /// # Errors
    ///
    /// Returns [`PeerAuthConfigError::InvalidAddress`] for an unparseable entry
    /// and [`PeerAuthConfigError::ConflictingRole`] when one address appears in
    /// both lists. A conflicting entry is an error rather than a silent
    /// precedence rule, so an operator cannot enable the auth role by accident.
    pub fn from_config(game: &[String], auth: &[String]) -> Result<Self, PeerAuthConfigError> {
        let game = parse_all(game)?;
        let auth = parse_all(auth)?;
        if let Some(address) = game.iter().find(|ip| auth.contains(ip)) {
            return Err(PeerAuthConfigError::ConflictingRole {
                address: address.to_string(),
            });
        }
        Ok(Self { game, auth })
    }

    /// Build an authenticator that refuses every peer.
    ///
    /// This is the default when no addresses are configured.
    #[must_use]
    pub fn deny_all() -> Self {
        Self {
            game: Vec::new(),
            auth: Vec::new(),
        }
    }

    /// Return whether any address is allowed at all.
    ///
    /// A caller uses this to decide whether to log a startup warning: a server
    /// that refuses everyone is safe but useless, and an operator should be told
    /// which one it is.
    #[must_use]
    pub fn allows_any(&self) -> bool {
        !self.game.is_empty() || !self.auth.is_empty()
    }

    /// Return the number of configured game-role addresses.
    #[must_use]
    pub fn game_peer_count(&self) -> usize {
        self.game.len()
    }

    /// Return the number of configured auth-role addresses.
    #[must_use]
    pub fn auth_peer_count(&self) -> usize {
        self.auth.len()
    }

    /// Classify one connection's source address.
    #[must_use]
    pub fn classify(&self, peer: SocketAddr) -> PeerAuthOutcome {
        let ip = peer.ip();
        if self.game.contains(&ip) {
            return PeerAuthOutcome::Classified {
                role: DbPeerRole::Game,
            };
        }
        if self.auth.contains(&ip) {
            return PeerAuthOutcome::Classified {
                role: DbPeerRole::Auth,
            };
        }
        PeerAuthOutcome::Denied
    }
}

fn parse_all(entries: &[String]) -> Result<Vec<std::net::IpAddr>, PeerAuthConfigError> {
    entries
        .iter()
        .map(|value| {
            let trimmed = value.trim();
            let parsed = trimmed
                .parse::<std::net::IpAddr>()
                .or_else(|_| {
                    // Accept "host:port" and keep only the host, so an operator
                    // can paste either form into the TOML file.
                    trimmed.parse::<SocketAddr>().map(|addr| addr.ip())
                })
                .map_err(|error| PeerAuthConfigError::InvalidAddress {
                    value: value.clone(),
                    message: error.to_string(),
                })?;
            Ok(parsed)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addrs(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn the_default_authenticator_denies_every_peer() {
        let auth = PeerAuthenticator::deny_all();
        assert!(!auth.allows_any());
        for peer in [
            "127.0.0.1:5000".parse().expect("valid"),
            "10.0.0.1:1".parse().expect("valid"),
            "[::1]:9".parse().expect("valid"),
        ] {
            assert_eq!(auth.classify(peer), PeerAuthOutcome::Denied);
            assert_eq!(auth.classify(peer).role(), None);
        }
    }

    #[test]
    fn an_empty_config_file_also_denies_every_peer() {
        let auth = PeerAuthenticator::from_config(&[], &[]).expect("empty lists are valid");
        assert!(!auth.allows_any());
        assert!(auth
            .classify("127.0.0.1:5000".parse().expect("valid"))
            .is_denied());
    }

    #[test]
    fn a_listed_address_is_classified_by_role() {
        let auth = PeerAuthenticator::from_config(&addrs(&["127.0.0.1"]), &addrs(&["10.0.0.5"]))
            .expect("valid addresses");
        assert!(auth.allows_any());
        assert_eq!(auth.game_peer_count(), 1);
        assert_eq!(auth.auth_peer_count(), 1);
        assert_eq!(
            auth.classify("127.0.0.1:40000".parse().expect("valid")),
            PeerAuthOutcome::Classified {
                role: DbPeerRole::Game
            }
        );
        assert_eq!(
            auth.classify("10.0.0.5:1".parse().expect("valid")),
            PeerAuthOutcome::Classified {
                role: DbPeerRole::Auth
            }
        );
        assert!(auth
            .classify("127.0.0.2:40000".parse().expect("valid"))
            .is_denied());
    }

    #[test]
    fn the_source_port_is_ignored_but_the_host_is_not() {
        let auth = PeerAuthenticator::from_config(&addrs(&["127.0.0.1"]), &[]).expect("valid");
        assert!(!auth
            .classify("127.0.0.1:65000".parse().expect("valid"))
            .is_denied());
        assert!(auth
            .classify("127.0.0.2:1".parse().expect("valid"))
            .is_denied());
    }

    #[test]
    fn a_host_port_entry_keeps_only_the_host() {
        let auth = PeerAuthenticator::from_config(&addrs(&["127.0.0.1:9999"]), &[]).expect("valid");
        assert!(!auth
            .classify("127.0.0.1:40000".parse().expect("valid"))
            .is_denied());
    }

    #[test]
    fn an_unparseable_address_is_an_error_not_a_silent_skip() {
        let error = PeerAuthenticator::from_config(&addrs(&["not-an-address"]), &[]);
        assert!(matches!(
            error,
            Err(PeerAuthConfigError::InvalidAddress { .. })
        ));
    }

    #[test]
    fn one_address_may_not_hold_both_roles() {
        let error = PeerAuthenticator::from_config(&addrs(&["127.0.0.1"]), &addrs(&["127.0.0.1"]));
        assert_eq!(
            error,
            Err(PeerAuthConfigError::ConflictingRole {
                address: "127.0.0.1".to_string()
            })
        );
    }

    #[test]
    fn whitespace_around_a_configured_address_is_tolerated() {
        let auth = PeerAuthenticator::from_config(&addrs(&["  127.0.0.1  "]), &[]).expect("valid");
        assert!(!auth
            .classify("127.0.0.1:1".parse().expect("valid"))
            .is_denied());
    }
}
