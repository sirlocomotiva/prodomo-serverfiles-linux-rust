//! Legacy DB peer framing.
//!
//! The game and database processes use a peer header that is separate from
//! both the client packet stream and the private Rust length-prefixed stream:
//!
//! ```text
//! +---------------+---------------+---------------+-------------------+
//! | header (u8)   | handle (u32)  | length (u32)  | payload (length)  |
//! +---------------+---------------+---------------+-------------------+
//! ```
//!
//! Multi-byte values are little endian, matching the legacy x86 target. The
//! `length` field counts payload bytes only. A zero-length payload is valid and
//! is used by the legacy return/acknowledgement packets.

use std::error::Error;
use std::fmt;

/// Number of bytes in a legacy DB peer header.
pub const DB_PEER_HEADER_SIZE: usize = 9;

/// Default upper bound for a buffered DB payload.
///
/// The legacy input buffer grows dynamically and does not define a protocol
/// limit. A Rust peer must impose a limit before trusting a remote length, so
/// this conservative default prevents a single header from forcing an
/// unbounded allocation. Applications with larger boot streams can select a
/// different limit with [`DbFrameDecoder::with_max_payload_size`].
pub const DEFAULT_MAX_DB_PAYLOAD_SIZE: usize = 16 * 1024 * 1024;

/// An error raised while encoding or decoding a legacy DB peer frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbFrameError {
    /// The payload is too large for the configured decoder limit.
    PayloadTooLarge {
        /// Length declared by the frame header.
        length: u32,
        /// Maximum accepted payload length.
        maximum: usize,
    },
    /// A payload cannot be represented by the legacy `u32` length field.
    PayloadLengthOverflow {
        /// Length that cannot be represented.
        length: usize,
    },
    /// The input buffer or frame total cannot be represented by `usize`.
    SizeOverflow,
    /// The payload could not be allocated with the available memory.
    AllocationFailed {
        /// Number of payload bytes that could not be reserved.
        length: usize,
    },
}

impl fmt::Display for DbFrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PayloadTooLarge { length, maximum } => {
                write!(f, "DB payload length {length} exceeds limit {maximum}")
            }
            Self::PayloadLengthOverflow { length } => {
                write!(f, "DB payload length {length} does not fit in u32")
            }
            Self::SizeOverflow => f.write_str("DB frame size does not fit in usize"),
            Self::AllocationFailed { length } => {
                write!(f, "could not allocate a DB payload of {length} bytes")
            }
        }
    }
}

impl Error for DbFrameError {}

/// A decoded legacy DB peer frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbFrame {
    /// One-byte GD/DG protocol header.
    pub header: u8,
    /// Correlation handle used by the legacy peer implementation.
    pub handle: u32,
    /// Payload bytes following the nine-byte peer header.
    pub payload: Vec<u8>,
}

impl DbFrame {
    /// Create a frame from its header fields and payload.
    pub fn new(header: u8, handle: u32, payload: Vec<u8>) -> Self {
        Self {
            header,
            handle,
            payload,
        }
    }

    /// Encode this frame in legacy little-endian DB peer format.
    ///
    /// # Errors
    ///
    /// Returns an error when the payload cannot fit the legacy `u32` length
    /// field or the platform size type.
    pub fn encode(&self) -> Result<Vec<u8>, DbFrameError> {
        let payload_length =
            u32::try_from(self.payload.len()).map_err(|_| DbFrameError::PayloadLengthOverflow {
                length: self.payload.len(),
            })?;
        let total_length = DB_PEER_HEADER_SIZE
            .checked_add(self.payload.len())
            .ok_or(DbFrameError::SizeOverflow)?;

        let mut encoded = Vec::with_capacity(total_length);
        encoded.push(self.header);
        encoded.extend_from_slice(&self.handle.to_le_bytes());
        encoded.extend_from_slice(&payload_length.to_le_bytes());
        encoded.extend_from_slice(&self.payload);
        Ok(encoded)
    }
}

/// Incremental decoder for legacy DB peer frames.
///
/// `feed` accepts arbitrary TCP fragments. `try_decode` returns `None` when a
/// complete frame is not yet buffered. Once a frame is returned, any following
/// bytes remain buffered for the next call.
#[derive(Debug, Clone)]
pub struct DbFrameDecoder {
    buffer: Vec<u8>,
    max_payload_size: usize,
}

impl DbFrameDecoder {
    /// Create a decoder with [`DEFAULT_MAX_DB_PAYLOAD_SIZE`].
    pub fn new() -> Self {
        Self::with_max_payload_size(DEFAULT_MAX_DB_PAYLOAD_SIZE)
    }

    /// Create a decoder with an explicit maximum payload size.
    pub fn with_max_payload_size(max_payload_size: usize) -> Self {
        Self {
            buffer: Vec::new(),
            max_payload_size,
        }
    }

    /// Return the configured maximum payload size.
    pub fn max_payload_size(&self) -> usize {
        self.max_payload_size
    }

    /// Return the number of bytes currently buffered.
    pub fn buffered_len(&self) -> usize {
        self.buffer.len()
    }

    /// Add a TCP fragment to the decoder.
    ///
    /// # Errors
    ///
    /// Returns [`DbFrameError::SizeOverflow`] if the buffered length cannot be
    /// represented by the platform size type.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), DbFrameError> {
        self.buffer
            .len()
            .checked_add(bytes.len())
            .ok_or(DbFrameError::SizeOverflow)?;
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }

    /// Try to remove and return one complete frame.
    ///
    /// # Errors
    ///
    /// Returns [`DbFrameError::PayloadTooLarge`] for a remote length above the
    /// configured limit, or [`DbFrameError::SizeOverflow`] if a frame total
    /// cannot be represented by the platform size type.
    pub fn try_decode(&mut self) -> Result<Option<DbFrame>, DbFrameError> {
        if self.buffer.len() < DB_PEER_HEADER_SIZE {
            return Ok(None);
        }

        let header = self.buffer[0];
        let handle = u32::from_le_bytes([
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
            self.buffer[4],
        ]);
        let payload_length = u32::from_le_bytes([
            self.buffer[5],
            self.buffer[6],
            self.buffer[7],
            self.buffer[8],
        ]);

        if payload_length as usize > self.max_payload_size {
            return Err(DbFrameError::PayloadTooLarge {
                length: payload_length,
                maximum: self.max_payload_size,
            });
        }

        let frame_length = DB_PEER_HEADER_SIZE
            .checked_add(payload_length as usize)
            .ok_or(DbFrameError::SizeOverflow)?;
        if self.buffer.len() < frame_length {
            return Ok(None);
        }

        let payload_length = payload_length as usize;
        let mut payload = Vec::new();
        payload
            .try_reserve_exact(payload_length)
            .map_err(|_| DbFrameError::AllocationFailed {
                length: payload_length,
            })?;
        payload.extend_from_slice(&self.buffer[DB_PEER_HEADER_SIZE..frame_length]);
        self.buffer.drain(..frame_length);
        Ok(Some(DbFrame {
            header,
            handle,
            payload,
        }))
    }

    /// Remove all buffered bytes after a protocol or connection error.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }
}

impl Default for DbFrameDecoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_the_legacy_nine_byte_header_and_payload() {
        let frame = DbFrame::new(0x2a, 0x1234_5678, vec![0xaa, 0xbb, 0xcc]);
        assert_eq!(
            frame.encode().unwrap(),
            vec![0x2a, 0x78, 0x56, 0x34, 0x12, 0x03, 0x00, 0x00, 0x00, 0xaa, 0xbb, 0xcc,]
        );
    }

    #[test]
    fn decodes_fragmented_and_coalesced_frames() {
        let first = DbFrame::new(1, 7, vec![1, 2, 3]).encode().unwrap();
        let second = DbFrame::new(2, 8, vec![]).encode().unwrap();
        let mut input = first.clone();
        input.extend_from_slice(&second);

        let mut decoder = DbFrameDecoder::new();
        decoder.feed(&input[..4]).unwrap();
        assert_eq!(decoder.try_decode().unwrap(), None);
        decoder.feed(&input[4..]).unwrap();
        assert_eq!(
            decoder.try_decode().unwrap(),
            Some(DbFrame::new(1, 7, vec![1, 2, 3]))
        );
        assert_eq!(
            decoder.try_decode().unwrap(),
            Some(DbFrame::new(2, 8, vec![]))
        );
        assert_eq!(decoder.try_decode().unwrap(), None);
        assert_eq!(decoder.buffered_len(), 0);
    }

    #[test]
    fn preserves_empty_payloads() {
        let frame = DbFrame::new(0xff, u32::MAX, Vec::new());
        let encoded = frame.encode().unwrap();
        assert_eq!(encoded.len(), DB_PEER_HEADER_SIZE);

        let mut decoder = DbFrameDecoder::new();
        decoder.feed(&encoded).unwrap();
        assert_eq!(decoder.try_decode().unwrap(), Some(frame));
    }

    #[test]
    fn rejects_a_declared_length_before_collecting_the_payload() {
        let mut decoder = DbFrameDecoder::with_max_payload_size(4);
        decoder.feed(&[1, 0, 0, 0, 0, 5, 0, 0, 0]).unwrap();
        assert_eq!(
            decoder.try_decode(),
            Err(DbFrameError::PayloadTooLarge {
                length: 5,
                maximum: 4,
            })
        );
        assert_eq!(decoder.buffered_len(), DB_PEER_HEADER_SIZE);
        decoder.clear();
        assert_eq!(decoder.buffered_len(), 0);
    }

    #[test]
    fn returns_none_when_a_complete_header_has_only_a_partial_payload() {
        let mut decoder = DbFrameDecoder::new();
        decoder.feed(&[1, 0, 0, 0, 0, 4, 0, 0, 0, 0xaa]).unwrap();
        assert_eq!(decoder.try_decode().unwrap(), None);
        decoder.feed(&[0xbb, 0xcc, 0xdd]).unwrap();
        assert_eq!(
            decoder.try_decode().unwrap(),
            Some(DbFrame::new(1, 0, vec![0xaa, 0xbb, 0xcc, 0xdd]))
        );
    }

    #[test]
    fn accepts_the_configured_boundary() {
        let frame = DbFrame::new(1, 2, vec![9; 8]);
        let mut decoder = DbFrameDecoder::with_max_payload_size(8);
        decoder.feed(&frame.encode().unwrap()).unwrap();
        assert_eq!(decoder.try_decode().unwrap(), Some(frame));
    }
}
