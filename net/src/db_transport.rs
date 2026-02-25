//! Tokio transport boundary for the legacy DB peer protocol.
//!
//! This module owns I/O and fragmentation only. All nine-byte header encoding
//! and incremental frame parsing stays in [`protocol::db_wire`].

use std::error::Error;
use std::fmt;
use std::io;

use protocol::db_wire::{DbFrame, DbFrameDecoder, DbFrameError};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Maximum bytes requested from the underlying reader at one time.
const READ_CHUNK_SIZE: usize = 4096;

/// An error at the DB peer transport boundary.
#[derive(Debug)]
pub enum DbTransportError {
    /// The underlying stream failed.
    Io(io::Error),
    /// The protocol codec rejected a frame.
    Frame(DbFrameError),
    /// The peer closed with an incomplete frame still buffered.
    UnexpectedEof {
        /// Bytes retained when the peer closed.
        buffered_bytes: usize,
    },
}

impl fmt::Display for DbTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "DB transport I/O error: {error}"),
            Self::Frame(error) => write!(formatter, "DB transport frame error: {error}"),
            Self::UnexpectedEof { buffered_bytes } => write!(
                formatter,
                "DB peer closed with an incomplete frame ({buffered_bytes} buffered bytes)"
            ),
        }
    }
}

impl Error for DbTransportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Frame(error) => Some(error),
            Self::UnexpectedEof { .. } => None,
        }
    }
}

impl From<io::Error> for DbTransportError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<DbFrameError> for DbTransportError {
    fn from(error: DbFrameError) -> Self {
        Self::Frame(error)
    }
}

/// A framed DB peer stream backed by a Tokio reader and writer.
///
/// Reads may contain any number of partial frames and complete frames. Reads
/// return one [`DbFrame`] at a time and leave following frames buffered. A
/// clean EOF returns `Ok(None)`; EOF in the middle of a frame is an error.
#[derive(Debug)]
pub struct DbFrameTransport<S> {
    stream: S,
    decoder: DbFrameDecoder,
}

impl<S> DbFrameTransport<S> {
    /// Wrap a stream with the default incoming payload limit.
    pub fn new(stream: S) -> Self {
        Self::with_max_payload_size(stream, protocol::db_wire::DEFAULT_MAX_DB_PAYLOAD_SIZE)
    }

    /// Wrap a stream with an explicit incoming payload limit.
    ///
    /// The limit is checked after the nine-byte header arrives and before the
    /// transport collects the declared payload.
    pub fn with_max_payload_size(stream: S, max_payload_size: usize) -> Self {
        Self {
            stream,
            decoder: DbFrameDecoder::with_max_payload_size(max_payload_size),
        }
    }

    /// Return the maximum accepted incoming payload size.
    pub fn max_payload_size(&self) -> usize {
        self.decoder.max_payload_size()
    }

    /// Return the number of bytes currently retained by the incremental decoder.
    pub fn buffered_len(&self) -> usize {
        self.decoder.buffered_len()
    }

    /// Borrow the underlying stream.
    pub const fn get_ref(&self) -> &S {
        &self.stream
    }

    /// Mutably borrow the underlying stream.
    pub const fn get_mut(&mut self) -> &mut S {
        &mut self.stream
    }

    /// Remove the transport wrapper and return the underlying stream.
    pub fn into_inner(self) -> S {
        self.stream
    }
}

impl<S> DbFrameTransport<S>
where
    S: AsyncRead + Unpin,
{
    /// Read and decode the next legacy DB peer frame.
    ///
    /// This method accepts fragmented reads and coalesced frames. `Ok(None)`
    /// means the peer closed cleanly between frames.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying stream fails, the codec rejects the
    /// declared payload size, or EOF arrives during a frame.
    pub async fn read_frame(&mut self) -> Result<Option<DbFrame>, DbTransportError> {
        let mut chunk = [0_u8; READ_CHUNK_SIZE];

        loop {
            if let Some(frame) = self.decoder.try_decode()? {
                return Ok(Some(frame));
            }

            let bytes_read = self.stream.read(&mut chunk).await?;
            if bytes_read == 0 {
                let buffered_bytes = self.decoder.buffered_len();
                if buffered_bytes == 0 {
                    return Ok(None);
                }
                return Err(DbTransportError::UnexpectedEof { buffered_bytes });
            }

            self.decoder.feed(&chunk[..bytes_read])?;
        }
    }
}

impl<S> DbFrameTransport<S>
where
    S: AsyncWrite + Unpin,
{
    /// Encode and write one complete legacy DB peer frame.
    ///
    /// Tokio's `write_all` boundary handles short writes from the underlying
    /// stream. The frame bytes come from [`DbFrame::encode`], so this module
    /// does not maintain a second header codec.
    ///
    /// # Errors
    ///
    /// Returns an error if the payload cannot be encoded or the underlying
    /// stream fails during a partial or full write.
    pub async fn write_frame(&mut self, frame: &DbFrame) -> Result<(), DbTransportError> {
        let encoded = frame.encode()?;
        self.stream.write_all(&encoded).await?;
        Ok(())
    }

    /// Flush pending bytes on the underlying stream.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying stream cannot be flushed.
    pub async fn flush(&mut self) -> Result<(), DbTransportError> {
        self.stream.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{duplex, AsyncReadExt};

    #[tokio::test]
    async fn reads_byte_fragmented_frames() {
        let expected = DbFrame::new(0x11, 0x1234_5678, vec![1, 2, 3]);
        let wire = expected.encode().unwrap();

        // A one-byte duplex capacity forces the writer and transport reader to
        // make progress across many short operations.
        let (mut writer, reader) = duplex(1);
        let feed = tokio::spawn(async move {
            for byte in wire {
                writer.write_all(&[byte]).await.unwrap();
            }
        });

        let mut transport = DbFrameTransport::new(reader);
        assert_eq!(transport.read_frame().await.unwrap(), Some(expected));
        assert_eq!(transport.read_frame().await.unwrap(), None);
        feed.await.unwrap();
    }

    #[tokio::test]
    async fn leaves_coalesced_frames_buffered() {
        let first = DbFrame::new(0x11, 1, vec![1, 2, 3]);
        let second = DbFrame::new(0x22, 9, Vec::new());
        let mut wire = first.encode().unwrap();
        wire.extend_from_slice(&second.encode().unwrap());

        // Finish the peer write before reading so both frames are already in
        // the transport's incremental decoder buffer.
        let (mut writer, reader) = duplex(wire.len());
        let feed = tokio::spawn(async move {
            writer.write_all(&wire).await.unwrap();
        });
        feed.await.unwrap();

        let mut transport = DbFrameTransport::new(reader);
        assert_eq!(transport.read_frame().await.unwrap(), Some(first));
        assert_eq!(transport.read_frame().await.unwrap(), Some(second));
        assert_eq!(transport.read_frame().await.unwrap(), None);
    }

    #[tokio::test]
    async fn writes_every_byte_when_the_stream_accepts_short_writes() {
        let frame = DbFrame::new(0x2a, 0x1234_5678, vec![0xaa, 0xbb, 0xcc]);
        let expected = frame.encode().unwrap();

        // The one-byte capacity makes AsyncWriteExt::write_all perform multiple
        // partial writes before the full frame reaches the peer.
        let (mut peer, stream) = duplex(1);
        let send = tokio::spawn(async move {
            let mut transport = DbFrameTransport::new(stream);
            transport.write_frame(&frame).await.unwrap();
            transport.flush().await.unwrap();
        });

        let mut actual = Vec::new();
        peer.read_to_end(&mut actual).await.unwrap();
        send.await.unwrap();
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn reports_a_truncated_frame_at_peer_eof() {
        let truncated = [0x01, 0, 0, 0, 0, 4, 0, 0, 0, 0xaa, 0xbb];
        let (mut writer, reader) = duplex(1);
        let feed = tokio::spawn(async move {
            writer.write_all(&truncated).await.unwrap();
        });

        let mut transport = DbFrameTransport::with_max_payload_size(reader, 8);
        assert!(matches!(
            transport.read_frame().await,
            Err(DbTransportError::UnexpectedEof { buffered_bytes: 11 })
        ));
        feed.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_an_oversized_declared_payload_before_collecting_it() {
        let header = [0x01, 0, 0, 0, 0, 5, 0, 0, 0];
        let (mut writer, reader) = duplex(1);
        let feed = tokio::spawn(async move {
            writer.write_all(&header).await.unwrap();
        });

        let mut transport = DbFrameTransport::with_max_payload_size(reader, 4);
        assert!(matches!(
            transport.read_frame().await,
            Err(DbTransportError::Frame(DbFrameError::PayloadTooLarge {
                length: 5,
                maximum: 4,
            }))
        ));
        feed.await.unwrap();
    }

    #[tokio::test]
    async fn reports_clean_eof_between_frames() {
        let (writer, reader) = duplex(1);
        drop(writer);

        let mut transport = DbFrameTransport::new(reader);
        assert_eq!(transport.read_frame().await.unwrap(), None);
    }
}
