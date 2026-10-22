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

    /// The address this end of the connection is bound to, which the server logs as its peer.
    ///
    /// # Panics
    ///
    /// Panics when the socket has no local address, which a connected socket always has.
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.stream
            .local_addr()
            .expect("a connected socket has a local address")
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
    /// Read into `buffer` until it holds `len` bytes, waiting until `deadline`.
    ///
    /// The bytes that arrive stay in `buffer` when the wait ends, so a caller that
    /// reads a record in pieces resumes it where it stopped: nothing that came off the
    /// wire is lost. The wait ends when `deadline` says so: the socket's own timeout
    /// is the time that is left, so a quiet wire does not hold the caller for a whole
    /// [`IO_TIMEOUT`] beat past the deadline.
    ///
    /// At most `len` bytes are added, so a caller that reads a record's header first
    /// and its body afterwards keeps the two apart.
    ///
    /// # Panics
    ///
    /// Panics when the connection closes or a read fails for another reason.
    #[must_use]
    pub fn read_bytes_until(
        &mut self,
        buffer: &mut Vec<u8>,
        len: usize,
        deadline: Instant,
    ) -> bool {
        let mut chunk = [0u8; 4096];
        while buffer.len() < len {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            self.stream
                .set_read_timeout(Some(left.min(IO_TIMEOUT).max(Duration::from_millis(1))))
                .expect("a read timeout should be settable");
            let want = (len - buffer.len()).min(chunk.len());
            match self.stream.read(&mut chunk[..want]) {
                Ok(0) => panic!("the connection to {} closed", self.address),
                Ok(n) => buffer.extend_from_slice(&chunk[..n]),
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    continue;
                }
                Err(error) => panic!("expected {len} bytes from {}: {error}", self.address),
            }
        }
        true
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
        self.expect_closed_within(IO_TIMEOUT)
    }

    /// [`expect_closed`](Self::expect_closed) with a wait of `timeout` rather than
    /// [`IO_TIMEOUT`]: some legacy rules land minutes later, and the connection stays
    /// open until they do.
    ///
    /// # Panics
    ///
    /// Panics when the connection is still open after `timeout`.
    #[must_use]
    pub fn expect_closed_within(&mut self, timeout: Duration) -> Vec<u8> {
        let (received, quiet) = self.drain(timeout);
        assert_eq!(
            quiet,
            Quiet::Closed,
            "{} should have closed the connection",
            self.address
        );
        received
    }
}
