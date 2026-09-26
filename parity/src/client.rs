//! A blocking TCP client that sends and expects raw client-wire bytes.
//!
//! It knows nothing about records: a scenario builds each frame with the `protocol` codecs and
//! checks each answer against golden bytes, so the client cannot share a bug with the server.

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

/// How long [`Client::expect_bytes`] and [`Client::expect_closed`] wait.
pub const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// What a read that returned nothing new meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quiet {
    /// The server sent nothing and kept the connection open.
    Open,
    /// The server closed the connection.
    Closed,
}

/// One connection to the server under test.
pub struct Client {
    stream: TcpStream,
    address: SocketAddr,
}

impl Client {
    /// Connect to `address`.
    ///
    /// # Panics
    ///
    /// Panics when the server does not accept within [`IO_TIMEOUT`].
    #[must_use]
    pub fn connect(address: SocketAddr) -> Self {
        let stream = TcpStream::connect_timeout(&address, IO_TIMEOUT)
            .unwrap_or_else(|error| panic!("{address} should accept: {error}"));
        stream
            .set_nodelay(true)
            .expect("TCP_NODELAY should be settable");
        Self { stream, address }
    }

    /// Send `bytes` as they are.
    ///
    /// # Panics
    ///
    /// Panics when the write fails.
    pub fn send(&mut self, bytes: &[u8]) {
        self.stream
            .write_all(bytes)
            .unwrap_or_else(|error| panic!("writing to {} failed: {error}", self.address));
    }

    /// Read exactly `len` bytes, waiting at most [`IO_TIMEOUT`].
    ///
    /// # Panics
    ///
    /// Panics when the connection closes or the time runs out first.
    #[must_use]
    pub fn expect_bytes(&mut self, len: usize) -> Vec<u8> {
        let mut buffer = vec![0; len];
        self.stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .expect("a read timeout should be settable");
        self.stream
            .read_exact(&mut buffer)
            .unwrap_or_else(|error| panic!("expected {len} bytes from {}: {error}", self.address));
        buffer
    }

    /// Read whatever the server sends within `window`, and say whether it then closed.
    ///
    /// # Panics
    ///
    /// Panics on a read error other than a timeout or a reset.
    #[must_use]
    pub fn drain(&mut self, window: Duration) -> (Vec<u8>, Quiet) {
        let deadline = Instant::now() + window;
        let mut received = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return (received, Quiet::Open);
            }
            self.stream
                .set_read_timeout(Some(left))
                .expect("a read timeout should be settable");
            match self.stream.read(&mut buffer) {
                Ok(0) => return (received, Quiet::Closed),
                Ok(n) => received.extend_from_slice(&buffer[..n]),
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    return (received, Quiet::Open);
                }
                Err(error) if error.kind() == ErrorKind::ConnectionReset => {
                    return (received, Quiet::Closed);
                }
                Err(error) => panic!("reading from {} failed: {error}", self.address),
            }
        }
    }

    /// Wait until the server closes the connection, and return what it sent before that.
    ///
    /// # Panics
    ///
    /// Panics when the connection is still open after [`IO_TIMEOUT`].
    #[must_use]
    pub fn expect_closed(&mut self) -> Vec<u8> {
        let (received, quiet) = self.drain(IO_TIMEOUT);
        assert_eq!(
            quiet,
            Quiet::Closed,
            "{} should have closed the connection",
            self.address
        );
        received
    }
}
