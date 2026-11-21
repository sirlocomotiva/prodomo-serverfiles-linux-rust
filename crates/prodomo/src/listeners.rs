//! The client listeners: one for auth and one for each configured Channel port (ADR-0002).
//!
//! All of them feed one accept loop. [`Listeners::accept`] polls them in turn, starting after the
//! one that accepted last, so a busy port cannot starve the others.

use std::error::Error;
use std::fmt;
use std::future::poll_fn;
use std::io;
use std::net::SocketAddr;
use std::task::Poll;

use common::config::ServerConfig;
use tokio::net::{TcpListener, TcpStream};

/// What the clients of a listener connect for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ListenerRole {
    /// The auth listener: login and the login key.
    Auth,
    /// A Channel listener, by Channel number.
    Channel(u8),
}

impl fmt::Display for ListenerRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auth => f.write_str("auth"),
            Self::Channel(number) => write!(f, "channel {number}"),
        }
    }
}

/// Every address to bind, in configuration order: auth first, then each Channel's ports.
#[must_use]
pub fn listener_plan(config: &ServerConfig) -> Vec<(ListenerRole, SocketAddr)> {
    let auth = (
        ListenerRole::Auth,
        SocketAddr::new(config.bind_ip, config.auth.port),
    );
    let channels = config.channels.iter().flat_map(|channel| {
        channel.ports.iter().map(move |&port| {
            (
                ListenerRole::Channel(channel.number),
                SocketAddr::new(config.bind_ip, port),
            )
        })
    });
    std::iter::once(auth).chain(channels).collect()
}

/// A listener that could not be bound.
#[derive(Debug)]
pub struct BindError {
    /// The listener's role.
    pub role: ListenerRole,
    /// The address it asked for.
    pub addr: SocketAddr,
    /// Why the bind failed.
    pub source: io::Error,
}

impl fmt::Display for BindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to bind the {} listener to {}: {}",
            self.role, self.addr, self.source
        )
    }
}

impl Error for BindError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

/// A bound listener and its role.
#[derive(Debug)]
pub struct RoleListener {
    role: ListenerRole,
    local_addr: SocketAddr,
    listener: TcpListener,
}

impl RoleListener {
    /// The listener's role.
    #[must_use]
    pub const fn role(&self) -> ListenerRole {
        self.role
    }

    /// The bound address. When port 0 was asked for, this carries the port the operating system
    /// chose.
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
}

/// The bound listeners of the process.
#[derive(Debug)]
pub struct Listeners {
    listeners: Vec<RoleListener>,
    next: usize,
}

impl Listeners {
    /// Bind every planned address, in order.
    ///
    /// # Errors
    ///
    /// Returns the first [`BindError`]. Listeners already bound are closed when the partial set
    /// is dropped.
    pub async fn bind(plan: &[(ListenerRole, SocketAddr)]) -> Result<Self, BindError> {
        let mut listeners = Vec::with_capacity(plan.len());
        for &(role, addr) in plan {
            let bound = TcpListener::bind(addr).await.and_then(|listener| {
                let local_addr = listener.local_addr()?;
                Ok(RoleListener {
                    role,
                    local_addr,
                    listener,
                })
            });
            listeners.push(bound.map_err(|source| BindError { role, addr, source })?);
        }
        Ok(Self { listeners, next: 0 })
    }

    /// The bound listeners, in bind order.
    pub fn iter(&self) -> impl Iterator<Item = &RoleListener> {
        self.listeners.iter()
    }

    /// Accept the next connection on any listener.
    ///
    /// Cancel-safe: dropping the future before it completes loses no connection. With no
    /// listeners it never completes.
    pub async fn accept(&mut self) -> (ListenerRole, io::Result<(TcpStream, SocketAddr)>) {
        let count = self.listeners.len();
        poll_fn(|cx| {
            for offset in 0..count {
                let index = (self.next + offset) % count;
                let entry = &self.listeners[index];
                if let Poll::Ready(result) = entry.listener.poll_accept(cx) {
                    self.next = (index + 1) % count;
                    return Poll::Ready((entry.role, result));
                }
            }
            Poll::Pending
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::config::parse_server_config;
    use std::net::Ipv4Addr;

    const TWO_CHANNELS: &str = r#"
bind_ip = "127.0.0.1"
[store]
url = "postgres://prodomo@127.0.0.1/prodomo"
[auth]
port = 30001
[[channel]]
number = 1
ports = [30003, 30005]
maps = [1]
[[channel]]
number = 99
ports = [30019]
maps = [72]
"#;

    #[test]
    fn the_plan_lists_auth_then_every_channel_port_in_order() {
        let config = parse_server_config(TWO_CHANNELS, "test").unwrap();
        let host = Ipv4Addr::LOCALHOST;
        assert_eq!(
            listener_plan(&config),
            vec![
                (ListenerRole::Auth, SocketAddr::from((host, 30001))),
                (ListenerRole::Channel(1), SocketAddr::from((host, 30003))),
                (ListenerRole::Channel(1), SocketAddr::from((host, 30005))),
                (ListenerRole::Channel(99), SocketAddr::from((host, 30019))),
            ]
        );
    }

    #[test]
    fn roles_print_as_the_logs_name_them() {
        assert_eq!(ListenerRole::Auth.to_string(), "auth");
        assert_eq!(ListenerRole::Channel(99).to_string(), "channel 99");
    }

    fn loopback(role: ListenerRole) -> (ListenerRole, SocketAddr) {
        (role, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
    }

    #[tokio::test]
    async fn port_zero_binds_a_real_port_and_reports_it() {
        let listeners = Listeners::bind(&[loopback(ListenerRole::Auth)])
            .await
            .unwrap();
        let bound = listeners.iter().next().unwrap();
        assert_eq!(bound.role(), ListenerRole::Auth);
        assert_ne!(bound.local_addr().port(), 0);
    }

    #[tokio::test]
    async fn a_taken_port_names_the_listener_that_failed() {
        let first = Listeners::bind(&[loopback(ListenerRole::Auth)])
            .await
            .unwrap();
        let taken = first.iter().next().unwrap().local_addr();
        let error = Listeners::bind(&[(ListenerRole::Channel(2), taken)])
            .await
            .expect_err("the port is already bound");
        assert_eq!(error.role, ListenerRole::Channel(2));
        assert_eq!(error.addr, taken);
        assert_eq!(error.source.kind(), io::ErrorKind::AddrInUse);
    }

    #[tokio::test]
    async fn accept_reports_which_listener_took_the_connection() {
        let mut listeners = Listeners::bind(&[
            loopback(ListenerRole::Auth),
            loopback(ListenerRole::Channel(1)),
            loopback(ListenerRole::Channel(99)),
        ])
        .await
        .unwrap();
        let addrs: Vec<SocketAddr> = listeners.iter().map(RoleListener::local_addr).collect();

        let _client = TcpStream::connect(addrs[2]).await.unwrap();
        let (role, accepted) = listeners.accept().await;
        assert_eq!(role, ListenerRole::Channel(99));
        accepted.unwrap();

        let _client = TcpStream::connect(addrs[0]).await.unwrap();
        let (role, accepted) = listeners.accept().await;
        assert_eq!(role, ListenerRole::Auth);
        accepted.unwrap();
    }

    #[tokio::test]
    async fn a_waiting_connection_on_every_listener_is_taken_in_turn() {
        let mut listeners = Listeners::bind(&[
            loopback(ListenerRole::Auth),
            loopback(ListenerRole::Channel(1)),
        ])
        .await
        .unwrap();
        let addrs: Vec<SocketAddr> = listeners.iter().map(RoleListener::local_addr).collect();
        let mut clients = Vec::new();
        for _ in 0..2 {
            for &addr in &addrs {
                clients.push(TcpStream::connect(addr).await.unwrap());
            }
        }
        // Let every connection reach the accept queues before accepting.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut roles = Vec::new();
        for _ in 0..4 {
            let (role, accepted) = listeners.accept().await;
            accepted.unwrap();
            roles.push(role);
        }
        assert_eq!(
            roles,
            vec![
                ListenerRole::Auth,
                ListenerRole::Channel(1),
                ListenerRole::Auth,
                ListenerRole::Channel(1),
            ],
            "a listener with a waiting connection never takes two turns in a row"
        );
    }
}
