//! Tokio transport boundary for fixed legacy client-to-game frames.
//!
//! This module owns stream I/O and fragmentation only. Header lookup, frame
//! sizing, variable-header rejection, and EOF validation stay in
//! [`protocol::cg_wire`]. The transport does not use the generic two-byte
//! [`crate::buffer::ReadBuffer`] framing.

use std::error::Error;
use std::fmt;
use std::io;

use protocol::cg_wire::{
    ClientFrame, ClientFrameDecoder, ClientFrameEncoder, ClientFrameError,
    DEFAULT_MAX_CLIENT_FRAME_SIZE,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Maximum bytes requested from the underlying reader at one time.
const READ_CHUNK_SIZE: usize = 4096;

/// An error at the legacy client transport boundary.
#[derive(Debug)]
pub enum ClientTransportError {
    /// The underlying stream failed.
    Io(io::Error),
    /// The protocol codec rejected a frame or the stream ended mid-frame.
    Frame(ClientFrameError),
}

impl fmt::Display for ClientTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "client transport I/O error: {error}"),
            Self::Frame(error) => write!(formatter, "client transport frame error: {error}"),
        }
    }
}

impl Error for ClientTransportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Frame(error) => Some(error),
        }
    }
}

impl From<io::Error> for ClientTransportError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ClientFrameError> for ClientTransportError {
    fn from(error: ClientFrameError) -> Self {
        Self::Frame(error)
    }
}

/// A fixed legacy client frame stream backed by a Tokio reader and writer.
///
/// Reads may contain any number of partial or coalesced frames. Each call to
/// [`Self::read_frame`] returns one complete frame and leaves following frames
/// buffered. A clean EOF returns `Ok(None)`; EOF during a known frame returns
/// the protocol's typed truncation error.
#[derive(Debug)]
pub struct ClientFrameTransport<S> {
    stream: S,
    decoder: ClientFrameDecoder,
    encoder: ClientFrameEncoder,
}

impl<S> ClientFrameTransport<S> {
    /// Wrap a stream with the default incoming and outgoing frame limit.
    #[must_use]
    pub fn new(stream: S) -> Self {
        Self::with_max_frame_size(stream, DEFAULT_MAX_CLIENT_FRAME_SIZE)
    }

    /// Wrap a stream with an explicit complete-frame limit.
    ///
    /// The same limit is used for incoming decoding and outgoing encoding.
    #[must_use]
    pub fn with_max_frame_size(stream: S, max_frame_size: usize) -> Self {
        Self {
            stream,
            decoder: ClientFrameDecoder::with_max_frame_size(max_frame_size),
            encoder: ClientFrameEncoder::with_max_frame_size(max_frame_size),
        }
    }

    /// Wrap a stream using packet-oriented terminology.
    #[must_use]
    pub fn with_max_packet_size(stream: S, max_packet_size: usize) -> Self {
        Self::with_max_frame_size(stream, max_packet_size)
    }

    /// Return the maximum accepted and emitted complete frame size.
    #[must_use]
    pub const fn max_frame_size(&self) -> usize {
        self.decoder.max_frame_size()
    }

    /// Return the maximum complete frame size using packet terminology.
    #[must_use]
    pub const fn max_packet_size(&self) -> usize {
        self.max_frame_size()
    }

    /// Return the number of bytes currently retained by the incremental decoder.
    #[must_use]
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

impl<S> ClientFrameTransport<S>
where
    S: AsyncRead + Unpin,
{
    /// Read and decode the next fixed legacy client frame.
    ///
    /// The method accepts arbitrary fragmented reads and coalesced frames. A
    /// clean EOF between frames returns `Ok(None)`. If EOF arrives while a
    /// fixed frame is incomplete, the protocol decoder returns its typed
    /// [`ClientFrameError::Truncated`] error.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying stream fails, or if the protocol
    /// decoder rejects a header, frame size, or truncated frame.
    pub async fn read_frame(&mut self) -> Result<Option<ClientFrame>, ClientTransportError> {
        let mut chunk = [0_u8; READ_CHUNK_SIZE];

        loop {
            if let Some(frame) = self.decoder.try_decode()? {
                return Ok(Some(frame));
            }

            let bytes_read = self.stream.read(&mut chunk).await?;
            if bytes_read == 0 {
                self.decoder.finish()?;
                return Ok(None);
            }

            self.decoder.feed(&chunk[..bytes_read])?;
        }
    }
}

impl<S> ClientFrameTransport<S>
where
    S: AsyncWrite + Unpin,
{
    /// Encode and write one complete fixed legacy client frame.
    ///
    /// The protocol encoder validates the header and exact frame size. Tokio's
    /// `write_all` boundary completes all short writes before returning.
    ///
    /// # Errors
    ///
    /// Returns an error if the frame cannot be encoded or if the underlying
    /// stream fails during a partial or complete write.
    pub async fn write_frame(&mut self, frame: &ClientFrame) -> Result<(), ClientTransportError> {
        let encoded = self.encoder.encode(frame)?;
        self.stream.write_all(&encoded).await?;
        Ok(())
    }

    /// Flush pending bytes on the underlying stream.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying stream cannot be flushed.
    pub async fn flush(&mut self) -> Result<(), ClientTransportError> {
        self.stream.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::cg_inventory::{
        HEADER_CG_CHAT, HEADER_CG_LOGIN, HEADER_CG_MOVE, HEADER_CG_TIME_SYNC,
    };
    use tokio::io::{duplex, AsyncReadExt};

    fn move_frame() -> ClientFrame {
        ClientFrame::new(HEADER_CG_MOVE.value(), vec![0x11; 15])
    }

    #[tokio::test]
    async fn reads_byte_fragmented_frames() {
        let first = move_frame();
        let second = ClientFrame::new(HEADER_CG_TIME_SYNC.value(), vec![0x22; 12]);
        let mut wire = first.encode().unwrap();
        wire.extend_from_slice(&second.encode().unwrap());

        // A one-byte duplex capacity forces many short reads.
        let (mut writer, reader) = duplex(1);
        let feed = tokio::spawn(async move {
            for byte in wire {
                writer.write_all(&[byte]).await.unwrap();
            }
        });

        let mut transport = ClientFrameTransport::new(reader);
        assert_eq!(transport.read_frame().await.unwrap(), Some(first));
        assert_eq!(transport.read_frame().await.unwrap(), Some(second));
        assert_eq!(transport.read_frame().await.unwrap(), None);
        feed.await.unwrap();
    }

    #[tokio::test]
    async fn leaves_coalesced_frames_buffered() {
        let first = move_frame();
        let second = ClientFrame::new(HEADER_CG_TIME_SYNC.value(), vec![0x33; 12]);
        let mut wire = first.encode().unwrap();
        wire.extend_from_slice(&second.encode().unwrap());

        let (mut writer, reader) = duplex(wire.len());
        let feed = tokio::spawn(async move {
            writer.write_all(&wire).await.unwrap();
        });
        feed.await.unwrap();

        let mut transport = ClientFrameTransport::new(reader);
        assert_eq!(transport.read_frame().await.unwrap(), Some(first));
        assert_eq!(transport.read_frame().await.unwrap(), Some(second));
        assert_eq!(transport.read_frame().await.unwrap(), None);
    }

    #[tokio::test]
    async fn writes_every_byte_when_the_stream_accepts_short_writes() {
        let frame = move_frame();
        let expected = frame.encode().unwrap();
        let (mut peer, stream) = duplex(1);

        let send = tokio::spawn(async move {
            let mut transport = ClientFrameTransport::new(stream);
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
        let truncated = [HEADER_CG_LOGIN.value(), 0, 1];
        let (mut writer, reader) = duplex(1);
        let feed = tokio::spawn(async move {
            writer.write_all(&truncated).await.unwrap();
        });

        let mut transport = ClientFrameTransport::new(reader);
        assert!(matches!(
            transport.read_frame().await,
            Err(ClientTransportError::Frame(ClientFrameError::Truncated {
                header,
                expected: 49,
                available: 3,
            })) if header == HEADER_CG_LOGIN.value()
        ));
        feed.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_an_oversized_frame_after_its_header() {
        let (mut writer, reader) = duplex(1);
        let feed = tokio::spawn(async move {
            writer.write_all(&[HEADER_CG_MOVE.value()]).await.unwrap();
        });

        let mut transport = ClientFrameTransport::with_max_frame_size(reader, 8);
        assert!(matches!(
            transport.read_frame().await,
            Err(ClientTransportError::Frame(
                ClientFrameError::FrameTooLarge {
                    size: 16,
                    maximum: 8,
                }
            ))
        ));
        feed.await.unwrap();
    }

    #[tokio::test]
    async fn delegates_variable_headers_to_the_protocol_error() {
        let (mut writer, reader) = duplex(4);
        let feed = tokio::spawn(async move {
            writer
                .write_all(&[HEADER_CG_CHAT.value(), 0, 0, 0])
                .await
                .unwrap();
        });

        let mut transport = ClientFrameTransport::new(reader);
        assert!(matches!(
            transport.read_frame().await,
            Err(ClientTransportError::Frame(
                ClientFrameError::VariableLengthUnsupported {
                    header,
                    base_size: 4,
                }
            )) if header == HEADER_CG_CHAT.value()
        ));
        feed.await.unwrap();
    }

    #[tokio::test]
    async fn reports_clean_eof_between_frames() {
        let (writer, reader) = duplex(1);
        drop(writer);

        let mut transport = ClientFrameTransport::new(reader);
        assert_eq!(transport.read_frame().await.unwrap(), None);
    }

    #[tokio::test]
    async fn enforces_the_same_limit_when_writing() {
        let (peer, stream) = duplex(1);
        let mut transport = ClientFrameTransport::with_max_frame_size(stream, 8);
        assert!(matches!(
            transport.write_frame(&move_frame()).await,
            Err(ClientTransportError::Frame(
                ClientFrameError::FrameTooLarge {
                    size: 16,
                    maximum: 8,
                }
            ))
        ));
        drop(peer);
    }
}
