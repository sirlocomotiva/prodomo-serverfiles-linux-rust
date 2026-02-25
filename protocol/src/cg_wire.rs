//! Safe framing for the legacy client-to-game stream.
//!
//! The legacy client stream is *not* the private Rust two-byte length-prefixed
//! format in `net::buffer`.  `CInputProcessor::Process` reads one header byte,
//! asks `CPacketInfoCG` for the packet's `sizeof`, and then consumes exactly
//! that many bytes (or waits for more input).  A zero header is a one-byte
//! keepalive special case handled before the packet-info lookup.
//!
//! This module deliberately implements the fixed-size part of that wire
//! contract.  Several legacy headers are extended by `CInputMain::Analyze`
//! after their packet-info prefix has arrived.  Those headers are reported as
//! [`ClientFrameSize::Variable`] and are rejected by the decoder/encoder
//! rather than being mistaken for a short fixed frame.
//!
//! The wire shape is:
//!
//! ```text
//! +---------------+-------------------------------+
//! | header (u8)   | fixed payload (base_size - 1) |
//! +---------------+-------------------------------+
//! ```
//!
//! No length prefix is added or consumed.

use std::error::Error;
use std::fmt;
use std::io;

use crate::cg_inventory::{
    self, HEADER_CG_AURA, HEADER_CG_CHAT, HEADER_CG_FISH_EVENT_SEND, HEADER_CG_GUILD,
    HEADER_CG_GUILD_SYMBOL_UPLOAD, HEADER_CG_MESSENGER, HEADER_CG_MYSHOP, HEADER_CG_PRIVATE_SHOP,
    HEADER_CG_SHOP, HEADER_CG_SWITCHBOT, HEADER_CG_SYNC_POSITION, HEADER_CG_WHISPER,
};

/// Number of bytes occupied by the one-byte legacy client header.
pub const CLIENT_FRAME_HEADER_SIZE: usize = 1;

/// Wire size of the legacy zero-byte keepalive frame.
pub const CG_KEEP_ALIVE_FRAME_SIZE: usize = 1;

/// Default safety bound for one complete legacy client frame.
///
/// The legacy inventory's largest fixed packet is much smaller than this
/// value.  The bound also leaves room for a future variable-frame resolver
/// while preventing one untrusted header from forcing an unbounded
/// allocation.
pub const DEFAULT_MAX_CLIENT_FRAME_SIZE: usize = 16 * 1024 * 1024;

/// Compatibility alias for [`DEFAULT_MAX_CLIENT_FRAME_SIZE`].
pub const DEFAULT_MAX_CG_FRAME_SIZE: usize = DEFAULT_MAX_CLIENT_FRAME_SIZE;

/// Headers for which legacy `Analyze` can append bytes after the
/// `CPacketInfoCG` prefix.
///
/// This list is conservative.  Some entries are feature-gated in the C++
/// build, and some subheaders (for example private-shop close/panel actions)
/// have no extension.  The inventory still registers their top-level headers,
/// so treating the header as variable is the safe default until a feature- and
/// subheader-aware resolver is added.
pub const LEGACY_CG_VARIABLE_HEADERS: &[u8] = &[
    HEADER_CG_CHAT.value(),
    HEADER_CG_WHISPER.value(),
    HEADER_CG_SYNC_POSITION.value(),
    HEADER_CG_SHOP.value(),
    HEADER_CG_MESSENGER.value(),
    HEADER_CG_GUILD.value(),
    HEADER_CG_GUILD_SYMBOL_UPLOAD.value(),
    HEADER_CG_MYSHOP.value(),
    HEADER_CG_AURA.value(),
    HEADER_CG_SWITCHBOT.value(),
    HEADER_CG_FISH_EVENT_SEND.value(),
    HEADER_CG_PRIVATE_SHOP.value(),
];

/// A size result from the legacy client packet-info inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientFrameSize {
    /// The complete wire size, including the one-byte header.
    Fixed(usize),
    /// The minimum prefix size known to `CPacketInfoCG`.
    ///
    /// `Analyze` must inspect the prefix and determine the additional bytes.
    /// This module does not guess that extension.
    Variable(usize),
}

impl ClientFrameSize {
    /// Return the fixed total size, or `None` for a variable frame.
    #[must_use]
    pub const fn fixed(self) -> Option<usize> {
        match self {
            Self::Fixed(size) => Some(size),
            Self::Variable(_) => None,
        }
    }

    /// Return the fixed total size or the known variable prefix size.
    #[must_use]
    pub const fn minimum(self) -> usize {
        match self {
            Self::Fixed(size) | Self::Variable(size) => size,
        }
    }

    /// Return whether this is an exact fixed-size result.
    #[must_use]
    pub const fn is_fixed(self) -> bool {
        matches!(self, Self::Fixed(_))
    }

    /// Return whether this header needs legacy `Analyze` extension logic.
    #[must_use]
    pub const fn is_variable(self) -> bool {
        matches!(self, Self::Variable(_))
    }
}

/// Compatibility alias for [`ClientFrameSize`].
pub type LegacyClientFrameSize = ClientFrameSize;

/// An error raised while resolving, decoding, or encoding a legacy client
/// frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientFrameError {
    /// The header is not registered by the default `CPacketInfoCG` map.
    UnknownHeader {
        /// The one-byte wire header.
        header: u8,
    },
    /// The inventory knows only a prefix and the variable `Analyze` step is
    /// outside this fixed-frame codec.
    VariableLengthUnsupported {
        /// The one-byte wire header.
        header: u8,
        /// The packet-info prefix size, including the header.
        base_size: usize,
    },
    /// A variable packet declared a size smaller than its packet-info prefix.
    InvalidVariableSize {
        /// The one-byte wire header.
        header: u8,
        /// Declared complete frame size.
        declared_size: usize,
        /// Packet-info prefix size, including the header.
        base_size: usize,
    },
    /// A complete frame would exceed the decoder/encoder safety bound.
    FrameTooLarge {
        /// Required complete frame size.
        size: usize,
        /// Configured maximum complete frame size.
        maximum: usize,
    },
    /// EOF arrived while a known fixed frame was only partly buffered.
    Truncated {
        /// Header of the incomplete frame.
        header: u8,
        /// Required complete frame size.
        expected: usize,
        /// Bytes currently available for this frame.
        available: usize,
    },
    /// An encoder was given a payload whose complete size differs from the
    /// packet-info size.
    LengthMismatch {
        /// The one-byte wire header.
        header: u8,
        /// Required complete frame size.
        expected: usize,
        /// Supplied complete frame size.
        actual: usize,
    },
    /// The packet inventory contained a zero size, which cannot describe a
    /// frame containing its own header.
    InvalidInventorySize {
        /// The one-byte wire header.
        header: u8,
        /// Invalid packet-info size.
        size: usize,
    },
    /// A checked length addition overflowed `usize`.
    SizeOverflow,
    /// The backing byte vector could not reserve space for input or output.
    AllocationFailed,
}

impl ClientFrameError {
    /// Return whether this error identifies an unknown legacy header.
    #[must_use]
    pub const fn is_unknown_header(&self) -> bool {
        matches!(self, Self::UnknownHeader { .. })
    }

    /// Return whether this error identifies a variable-length header that
    /// this fixed-size codec intentionally does not decode.
    #[must_use]
    pub const fn is_variable_length(&self) -> bool {
        matches!(self, Self::VariableLengthUnsupported { .. })
    }

    /// Return whether this error identifies a variable packet with an invalid
    /// declared size.
    #[must_use]
    pub const fn is_invalid_variable_size(&self) -> bool {
        matches!(self, Self::InvalidVariableSize { .. })
    }

    /// Return whether this error represents a truncated stream at EOF.
    #[must_use]
    pub const fn is_truncated(&self) -> bool {
        matches!(self, Self::Truncated { .. })
    }
}

impl fmt::Display for ClientFrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownHeader { header } => {
                write!(f, "unknown legacy client header 0x{header:02x}")
            }
            Self::VariableLengthUnsupported { header, base_size } => write!(
                f,
                "legacy client header 0x{header:02x} is variable (prefix {base_size} bytes)"
            ),
            Self::InvalidVariableSize {
                header,
                declared_size,
                base_size,
            } => write!(
                f,
                "legacy client variable header 0x{header:02x} declares {declared_size} bytes, below its {base_size}-byte prefix"
            ),
            Self::FrameTooLarge { size, maximum } => {
                write!(f, "legacy client frame size {size} exceeds limit {maximum}")
            }
            Self::Truncated {
                header,
                expected,
                available,
            } => write!(
                f,
                "truncated legacy client frame 0x{header:02x}: need {expected} bytes, have {available}"
            ),
            Self::LengthMismatch {
                header,
                expected,
                actual,
            } => write!(
                f,
                "legacy client frame 0x{header:02x} has {actual} bytes; expected {expected}"
            ),
            Self::InvalidInventorySize { header, size } => write!(
                f,
                "legacy client header 0x{header:02x} has invalid inventory size {size}"
            ),
            Self::SizeOverflow => f.write_str("legacy client frame size overflow"),
            Self::AllocationFailed => f.write_str("legacy client frame allocation failed"),
        }
    }
}

impl Error for ClientFrameError {}

impl From<ClientFrameError> for io::Error {
    fn from(error: ClientFrameError) -> Self {
        let kind = match &error {
            ClientFrameError::Truncated { .. } => io::ErrorKind::UnexpectedEof,
            ClientFrameError::SizeOverflow | ClientFrameError::AllocationFailed => {
                io::ErrorKind::Other
            }
            ClientFrameError::UnknownHeader { .. }
            | ClientFrameError::VariableLengthUnsupported { .. }
            | ClientFrameError::InvalidVariableSize { .. }
            | ClientFrameError::FrameTooLarge { .. }
            | ClientFrameError::LengthMismatch { .. }
            | ClientFrameError::InvalidInventorySize { .. } => io::ErrorKind::InvalidData,
        };
        Self::new(kind, error)
    }
}

/// Compatibility alias for [`ClientFrameError`].
pub type CgFrameError = ClientFrameError;

/// Compatibility alias for [`ClientFrameError`].
pub type LegacyClientFrameError = ClientFrameError;

/// Resolve a raw header to an exact fixed size or a variable-prefix result.
///
/// Header zero resolves to the legacy one-byte keepalive special case.  The
/// `0xf1` alias resolves as `HEADER_CG_CLIENT_VERSION2`, matching the first
/// registration in the legacy map; the shadowed Gaya registration is not
/// selected.
///
/// # Errors
///
/// Returns [`ClientFrameError::UnknownHeader`] for headers absent from the
/// default map, or [`ClientFrameError::InvalidInventorySize`] if an inventory
/// entry has a zero size.
pub fn resolve_client_frame_size(header: u8) -> Result<ClientFrameSize, ClientFrameError> {
    // `CInputProcessor::Process` handles zero before consulting the packet-info
    // map. Keep that special case explicit instead of depending on inventory
    // registration order.
    if header == 0 {
        return Ok(ClientFrameSize::Fixed(CG_KEEP_ALIVE_FRAME_SIZE));
    }

    let entry = cg_inventory::resolve_cg_packet(header)
        .ok_or(ClientFrameError::UnknownHeader { header })?;
    if entry.base_size == 0 {
        return Err(ClientFrameError::InvalidInventorySize {
            header,
            size: entry.base_size,
        });
    }

    if is_variable_client_header(header) {
        Ok(ClientFrameSize::Variable(entry.base_size))
    } else {
        Ok(ClientFrameSize::Fixed(entry.base_size))
    }
}

/// Compatibility alias for [`resolve_client_frame_size`].
///
/// # Errors
///
/// See [`resolve_client_frame_size`].
pub fn resolve_cg_frame_size(header: u8) -> Result<ClientFrameSize, ClientFrameError> {
    resolve_client_frame_size(header)
}

/// Compatibility alias for [`resolve_client_frame_size`].
///
/// # Errors
///
/// See [`resolve_client_frame_size`].
pub fn resolve_legacy_client_frame_size(header: u8) -> Result<ClientFrameSize, ClientFrameError> {
    resolve_client_frame_size(header)
}

/// Return whether a header belongs to the conservative variable-length set.
#[must_use]
pub fn is_variable_client_header(header: u8) -> bool {
    LEGACY_CG_VARIABLE_HEADERS.contains(&header)
}

/// A complete legacy client frame with the payload excluding its header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientFrame {
    /// One-byte client-to-game protocol header.
    pub header: u8,
    /// Bytes following the header.
    pub payload: Vec<u8>,
}

impl ClientFrame {
    /// Construct a frame without validating its size.
    ///
    /// Validation happens in [`ClientFrame::encode`] or through a configured
    /// [`ClientFrameEncoder`].  This makes it possible to construct a frame
    /// for a variable packet as data, while still refusing to encode it with
    /// the fixed-size encoder.
    #[must_use]
    pub fn new<P: AsRef<[u8]>>(header: u8, payload: P) -> Self {
        Self {
            header,
            payload: payload.as_ref().to_vec(),
        }
    }

    /// Construct a frame while reserving payload storage fallibly.
    ///
    /// This is the allocation-aware counterpart to [`ClientFrame::new`].
    /// It is useful at an I/O boundary where an input fragment is already
    /// available and allocation failure should be reported instead of
    /// hidden in the convenience constructor.
    ///
    /// # Errors
    ///
    /// Returns [`ClientFrameError::AllocationFailed`] if the payload cannot be
    /// copied into a new vector.
    pub fn try_new<P: AsRef<[u8]>>(header: u8, payload: P) -> Result<Self, ClientFrameError> {
        let bytes = payload.as_ref();
        let mut copied = Vec::new();
        copied
            .try_reserve(bytes.len())
            .map_err(|_| ClientFrameError::AllocationFailed)?;
        copied.extend_from_slice(bytes);
        Ok(Self {
            header,
            payload: copied,
        })
    }

    /// Return the complete wire size, including the one-byte header.
    ///
    /// # Errors
    ///
    /// Returns [`ClientFrameError::SizeOverflow`] if the platform cannot
    /// represent `1 + payload.len()`.
    pub fn encoded_len(&self) -> Result<usize, ClientFrameError> {
        CLIENT_FRAME_HEADER_SIZE
            .checked_add(self.payload.len())
            .ok_or(ClientFrameError::SizeOverflow)
    }

    /// Encode this frame as raw legacy client bytes.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown header, a variable header, a payload
    /// length that differs from the inventory size, or an unrepresentable
    /// size.
    pub fn encode(&self) -> Result<Vec<u8>, ClientFrameError> {
        self.encode_with_max_frame_size(usize::MAX)
    }

    /// Alias for [`ClientFrame::encode`].
    ///
    /// # Errors
    ///
    /// See [`ClientFrame::encode`].
    pub fn try_encode(&self) -> Result<Vec<u8>, ClientFrameError> {
        self.encode()
    }

    /// Encode this frame while enforcing a maximum complete frame size.
    ///
    /// # Errors
    ///
    /// In addition to [`ClientFrame::encode`] errors, returns
    /// [`ClientFrameError::FrameTooLarge`] when the complete frame exceeds
    /// `maximum`.
    pub fn encode_with_max_frame_size(&self, maximum: usize) -> Result<Vec<u8>, ClientFrameError> {
        Self::encode_parts_with_max(self.header, &self.payload, maximum)
    }

    /// Validate and encode a borrowed payload without copying it first.
    ///
    /// Header resolution and all length checks happen before output storage is
    /// reserved. This keeps `ClientFrameEncoder::encode_parts` from allocating
    /// a large rejected payload.
    fn encode_parts_with_max<P: AsRef<[u8]>>(
        header: u8,
        payload: P,
        maximum: usize,
    ) -> Result<Vec<u8>, ClientFrameError> {
        let expected = match resolve_client_frame_size(header)? {
            ClientFrameSize::Fixed(size) => size,
            ClientFrameSize::Variable(base_size) => {
                return Err(ClientFrameError::VariableLengthUnsupported { header, base_size });
            }
        };
        let payload = payload.as_ref();
        let actual = CLIENT_FRAME_HEADER_SIZE
            .checked_add(payload.len())
            .ok_or(ClientFrameError::SizeOverflow)?;
        if actual != expected {
            return Err(ClientFrameError::LengthMismatch {
                header,
                expected,
                actual,
            });
        }
        if actual > maximum {
            return Err(ClientFrameError::FrameTooLarge {
                size: actual,
                maximum,
            });
        }

        let mut encoded = Vec::new();
        encoded
            .try_reserve(actual)
            .map_err(|_| ClientFrameError::AllocationFailed)?;
        encoded.push(header);
        encoded.extend_from_slice(payload);
        Ok(encoded)
    }
}

/// Compatibility alias for [`ClientFrame`].
pub type LegacyClientFrame = ClientFrame;

/// Short client-to-game alias for [`ClientFrame`].
pub type CgFrame = ClientFrame;

/// A configurable safe encoder for fixed legacy client frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientFrameEncoder {
    max_frame_size: usize,
}

impl ClientFrameEncoder {
    /// Create an encoder with [`DEFAULT_MAX_CLIENT_FRAME_SIZE`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_frame_size: DEFAULT_MAX_CLIENT_FRAME_SIZE,
        }
    }

    /// Create an encoder with an explicit maximum complete frame size.
    #[must_use]
    pub const fn with_max_frame_size(max_frame_size: usize) -> Self {
        Self { max_frame_size }
    }

    /// Return the configured maximum complete frame size.
    #[must_use]
    pub const fn max_frame_size(&self) -> usize {
        self.max_frame_size
    }

    /// Encode a frame as raw legacy client bytes.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown, variable, incorrectly sized, oversized,
    /// or unrepresentable frames.
    pub fn encode(&self, frame: &ClientFrame) -> Result<Vec<u8>, ClientFrameError> {
        frame.encode_with_max_frame_size(self.max_frame_size)
    }

    /// Construct and encode a frame in one call.
    ///
    /// # Errors
    ///
    /// See [`ClientFrameEncoder::encode`].
    pub fn encode_parts<P: AsRef<[u8]>>(
        &self,
        header: u8,
        payload: P,
    ) -> Result<Vec<u8>, ClientFrameError> {
        ClientFrame::encode_parts_with_max(header, payload, self.max_frame_size)
    }
}

impl Default for ClientFrameEncoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Compatibility alias for [`ClientFrameEncoder`].
pub type LegacyClientFrameEncoder = ClientFrameEncoder;

/// Short client-to-game alias for [`ClientFrameEncoder`].
pub type CgFrameEncoder = ClientFrameEncoder;

/// Incremental decoder for fixed-size legacy client frames.
///
/// `feed` accepts arbitrary TCP fragments. `try_decode` returns `None` until
/// one complete frame is buffered, and leaves coalesced following bytes for
/// the next call.  Protocol errors leave the input untouched so the caller
/// can inspect or explicitly clear it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientFrameDecoder {
    buffer: Vec<u8>,
    max_frame_size: usize,
}

impl ClientFrameDecoder {
    /// Create a decoder with [`DEFAULT_MAX_CLIENT_FRAME_SIZE`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_max_frame_size(DEFAULT_MAX_CLIENT_FRAME_SIZE)
    }

    /// Create a decoder with an explicit maximum complete frame size.
    #[must_use]
    pub fn with_max_frame_size(max_frame_size: usize) -> Self {
        Self {
            buffer: Vec::new(),
            max_frame_size,
        }
    }

    /// Compatibility constructor using packet-oriented terminology.
    #[must_use]
    pub fn with_max_packet_size(max_packet_size: usize) -> Self {
        Self::with_max_frame_size(max_packet_size)
    }

    /// Return the configured maximum complete frame size.
    #[must_use]
    pub const fn max_frame_size(&self) -> usize {
        self.max_frame_size
    }

    /// Compatibility alias for [`ClientFrameDecoder::max_frame_size`].
    #[must_use]
    pub const fn max_size(&self) -> usize {
        self.max_frame_size
    }

    /// Return the number of bytes currently buffered.
    #[must_use]
    pub fn buffered_len(&self) -> usize {
        self.buffer.len()
    }

    /// Return whether no bytes are buffered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Peek at the next header without consuming input.
    #[must_use]
    pub fn peek_header(&self) -> Option<u8> {
        self.buffer.first().copied()
    }

    /// Borrow the currently buffered bytes without consuming them.
    #[must_use]
    pub fn buffered_bytes(&self) -> &[u8] {
        &self.buffer
    }

    /// Add an arbitrary TCP fragment to the decoder.
    ///
    /// The input length is checked before appending.  A coalesced read may
    /// exceed the per-frame maximum because it can contain several complete
    /// frames; the maximum is applied to each frame by `try_decode`.
    ///
    /// # Errors
    ///
    /// Returns [`ClientFrameError::SizeOverflow`] if the combined buffered
    /// length overflows `usize`, or [`ClientFrameError::AllocationFailed`]
    /// if the backing vector cannot reserve the fragment.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), ClientFrameError> {
        self.buffer
            .len()
            .checked_add(bytes.len())
            .ok_or(ClientFrameError::SizeOverflow)?;
        self.buffer
            .try_reserve(bytes.len())
            .map_err(|_| ClientFrameError::AllocationFailed)?;
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }

    /// Compatibility alias for [`ClientFrameDecoder::feed`].
    ///
    /// # Errors
    ///
    /// See [`ClientFrameDecoder::feed`].
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), ClientFrameError> {
        self.feed(bytes)
    }

    /// Try to consume one complete fixed-size frame.
    ///
    /// # Errors
    ///
    /// Returns a typed error for an unknown or variable header, a frame above
    /// the configured maximum, or a checked size/allocation failure.  A short
    /// known fixed frame is not an error; it returns `Ok(None)` until more TCP
    /// bytes arrive.
    pub fn try_decode(&mut self) -> Result<Option<ClientFrame>, ClientFrameError> {
        let Some(&header) = self.buffer.first() else {
            return Ok(None);
        };

        let frame_size = resolve_client_frame_size(header)?;
        let size = match frame_size {
            ClientFrameSize::Fixed(size) => size,
            ClientFrameSize::Variable(base_size) => {
                return Err(ClientFrameError::VariableLengthUnsupported { header, base_size });
            }
        };
        if size > self.max_frame_size {
            return Err(ClientFrameError::FrameTooLarge {
                size,
                maximum: self.max_frame_size,
            });
        }
        if self.buffer.len() < size {
            return Ok(None);
        }

        let payload_len = size
            .checked_sub(CLIENT_FRAME_HEADER_SIZE)
            .ok_or(ClientFrameError::InvalidInventorySize { header, size })?;
        let mut payload = Vec::new();
        payload
            .try_reserve(payload_len)
            .map_err(|_| ClientFrameError::AllocationFailed)?;
        payload.extend_from_slice(&self.buffer[CLIENT_FRAME_HEADER_SIZE..size]);
        self.buffer.drain(..size);
        Ok(Some(ClientFrame { header, payload }))
    }

    /// Compatibility alias for [`ClientFrameDecoder::try_decode`].
    ///
    /// # Errors
    ///
    /// See [`ClientFrameDecoder::try_decode`].
    pub fn decode(&mut self) -> Result<Option<ClientFrame>, ClientFrameError> {
        self.try_decode()
    }

    /// Compatibility alias for [`ClientFrameDecoder::try_decode`].
    ///
    /// # Errors
    ///
    /// See [`ClientFrameDecoder::try_decode`].
    pub fn try_next(&mut self) -> Result<Option<ClientFrame>, ClientFrameError> {
        self.try_decode()
    }

    /// Validate EOF without consuming complete buffered frames.
    ///
    /// An empty decoder is a clean EOF.  If the final buffered frame is a
    /// short known fixed frame, this returns [`ClientFrameError::Truncated`].
    /// Complete frames followed by a partial frame report the partial frame.
    /// Unknown and variable headers remain explicit protocol errors.
    ///
    /// # Errors
    ///
    /// Returns a typed error for truncated, unknown, variable, oversized, or
    /// checked-size-invalid buffered input.
    pub fn finish(&self) -> Result<(), ClientFrameError> {
        let mut offset = 0;
        while offset < self.buffer.len() {
            let header = self.buffer[offset];
            let frame_size = resolve_client_frame_size(header)?;
            let size = match frame_size {
                ClientFrameSize::Fixed(size) => size,
                ClientFrameSize::Variable(base_size) => {
                    return Err(ClientFrameError::VariableLengthUnsupported { header, base_size });
                }
            };
            if size > self.max_frame_size {
                return Err(ClientFrameError::FrameTooLarge {
                    size,
                    maximum: self.max_frame_size,
                });
            }
            let end = offset
                .checked_add(size)
                .ok_or(ClientFrameError::SizeOverflow)?;
            if end > self.buffer.len() {
                return Err(ClientFrameError::Truncated {
                    header,
                    expected: size,
                    available: self.buffer.len() - offset,
                });
            }
            offset = end;
        }
        Ok(())
    }

    /// Compatibility alias for [`ClientFrameDecoder::finish`].
    ///
    /// # Errors
    ///
    /// See [`ClientFrameDecoder::finish`].
    pub fn finish_eof(&self) -> Result<(), ClientFrameError> {
        self.finish()
    }

    /// Decode one frame, then validate EOF if no frame was available.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`ClientFrameDecoder::try_decode`] and
    /// [`ClientFrameDecoder::finish`].
    pub fn decode_eof(&mut self) -> Result<Option<ClientFrame>, ClientFrameError> {
        if let Some(frame) = self.try_decode()? {
            return Ok(Some(frame));
        }
        self.finish()?;
        Ok(None)
    }

    /// Remove all buffered bytes after a protocol or connection error.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }
}

impl Default for ClientFrameDecoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Compatibility alias for [`ClientFrameDecoder`].
pub type LegacyClientFrameDecoder = ClientFrameDecoder;

/// Short client-to-game alias for [`ClientFrameDecoder`].
pub type CgFrameDecoder = ClientFrameDecoder;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg_inventory::{
        CG_KEEP_ALIVE, HEADER_CG_LOGIN, HEADER_CG_MOVE, HEADER_CG_TIME_SYNC,
    };

    fn fixed_frame(header: u8, size: usize) -> Vec<u8> {
        let mut bytes = vec![header];
        bytes.resize(size, 0xa5);
        bytes
    }

    #[test]
    fn resolves_keepalive_and_fixed_sizes_without_a_length_prefix() {
        assert_eq!(
            resolve_client_frame_size(CG_KEEP_ALIVE.value()),
            Ok(ClientFrameSize::Fixed(1))
        );
        assert_eq!(
            resolve_client_frame_size(HEADER_CG_MOVE.value()),
            Ok(ClientFrameSize::Fixed(16))
        );
        assert_eq!(
            resolve_client_frame_size(0xf1),
            Ok(ClientFrameSize::Fixed(67))
        );
    }

    #[test]
    fn marks_variable_headers_instead_of_using_their_prefix() {
        let expected = [
            (HEADER_CG_CHAT.value(), 4),
            (HEADER_CG_WHISPER.value(), 28),
            (HEADER_CG_SYNC_POSITION.value(), 3),
            (HEADER_CG_SHOP.value(), 2),
            (HEADER_CG_MESSENGER.value(), 2),
            (HEADER_CG_GUILD.value(), 2),
            (HEADER_CG_GUILD_SYMBOL_UPLOAD.value(), 7),
            (HEADER_CG_MYSHOP.value(), 35),
            (HEADER_CG_AURA.value(), 4),
            (HEADER_CG_SWITCHBOT.value(), 7),
            (HEADER_CG_FISH_EVENT_SEND.value(), 2),
            (HEADER_CG_PRIVATE_SHOP.value(), 2),
        ];
        assert_eq!(
            LEGACY_CG_VARIABLE_HEADERS,
            expected.map(|(header, _)| header)
        );
        for (header, base_size) in expected {
            assert!(is_variable_client_header(header));
            assert_eq!(
                resolve_client_frame_size(header),
                Ok(ClientFrameSize::Variable(base_size))
            );
        }
        assert!(!is_variable_client_header(HEADER_CG_MOVE.value()));
    }

    #[test]
    fn decodes_fragmented_and_coalesced_frames() {
        let first = fixed_frame(CG_KEEP_ALIVE.value(), 1);
        let second = fixed_frame(HEADER_CG_MOVE.value(), 16);
        let third = fixed_frame(HEADER_CG_TIME_SYNC.value(), 13);
        let mut input = first.clone();
        input.extend_from_slice(&second);
        input.extend_from_slice(&third);

        let mut decoder = ClientFrameDecoder::new();
        decoder.feed(&input[..1]).unwrap();
        assert_eq!(
            decoder.try_decode().unwrap(),
            Some(ClientFrame::new(CG_KEEP_ALIVE.value(), []))
        );
        decoder.feed(&input[1..3]).unwrap();
        assert_eq!(decoder.try_decode().unwrap(), None);
        decoder.feed(&input[3..]).unwrap();

        assert_eq!(
            decoder.try_decode().unwrap().unwrap().header,
            HEADER_CG_MOVE.value()
        );
        assert_eq!(
            decoder.try_decode().unwrap(),
            Some(ClientFrame::new(HEADER_CG_TIME_SYNC.value(), [0xa5; 12]))
        );
        assert_eq!(decoder.try_decode().unwrap(), None);
        assert!(decoder.is_empty());
    }

    #[test]
    fn rejects_unknown_headers_without_consuming_them() {
        let mut decoder = ClientFrameDecoder::new();
        decoder.feed(&[0x99, 1, 2]).unwrap();
        assert_eq!(
            decoder.try_decode(),
            Err(ClientFrameError::UnknownHeader { header: 0x99 })
        );
        assert_eq!(decoder.buffered_len(), 3);
        decoder.clear();
        assert!(decoder.is_empty());
    }

    #[test]
    fn rejects_variable_headers_in_this_fixed_slice() {
        let mut decoder = ClientFrameDecoder::new();
        decoder
            .feed(&fixed_frame(HEADER_CG_CHAT.value(), 4))
            .unwrap();
        assert_eq!(
            decoder.try_decode(),
            Err(ClientFrameError::VariableLengthUnsupported {
                header: HEADER_CG_CHAT.value(),
                base_size: 4,
            })
        );
    }

    #[test]
    fn enforces_the_configured_frame_maximum() {
        let mut decoder = ClientFrameDecoder::with_max_frame_size(8);
        decoder.feed(&[HEADER_CG_MOVE.value()]).unwrap();
        assert_eq!(
            decoder.try_decode(),
            Err(ClientFrameError::FrameTooLarge {
                size: 16,
                maximum: 8,
            })
        );

        let mut keepalive = ClientFrameDecoder::with_max_frame_size(1);
        keepalive.feed(&[0]).unwrap();
        assert_eq!(keepalive.try_decode().unwrap().unwrap().header, 0);
    }

    #[test]
    fn clean_and_truncated_eof_are_distinct() {
        let mut decoder = ClientFrameDecoder::new();
        assert_eq!(decoder.finish(), Ok(()));
        assert_eq!(decoder.decode_eof().unwrap(), None);

        decoder.feed(&[HEADER_CG_LOGIN.value(), 0, 1]).unwrap();
        assert_eq!(
            decoder.finish(),
            Err(ClientFrameError::Truncated {
                header: HEADER_CG_LOGIN.value(),
                expected: 49,
                available: 3,
            })
        );
    }

    #[test]
    fn encoder_validates_header_and_exact_size() {
        let frame = ClientFrame::new(HEADER_CG_MOVE.value(), [0x11; 15]);
        assert_eq!(frame.encode().unwrap().len(), 16);
        assert_eq!(frame.encode().unwrap()[0], HEADER_CG_MOVE.value());

        assert_eq!(
            ClientFrame::new(HEADER_CG_MOVE.value(), [0x11; 14]).encode(),
            Err(ClientFrameError::LengthMismatch {
                header: HEADER_CG_MOVE.value(),
                expected: 16,
                actual: 15,
            })
        );
        assert_eq!(
            ClientFrame::new(HEADER_CG_CHAT.value(), [0; 3]).encode(),
            Err(ClientFrameError::VariableLengthUnsupported {
                header: HEADER_CG_CHAT.value(),
                base_size: 4,
            })
        );
        assert_eq!(
            ClientFrame::new(0x99, []).encode(),
            Err(ClientFrameError::UnknownHeader { header: 0x99 })
        );
    }

    #[test]
    fn configurable_encoder_rejects_oversized_frames() {
        let encoder = ClientFrameEncoder::with_max_frame_size(8);
        assert_eq!(
            encoder.encode_parts(HEADER_CG_MOVE.value(), [0; 15]),
            Err(ClientFrameError::FrameTooLarge {
                size: 16,
                maximum: 8,
            })
        );
    }

    #[test]
    fn encoder_rejects_header_before_reading_payload() {
        struct MustNotRead;

        impl AsRef<[u8]> for MustNotRead {
            fn as_ref(&self) -> &[u8] {
                panic!("payload must not be read for an unknown header")
            }
        }

        let result = ClientFrameEncoder::new().encode_parts(0x99, MustNotRead);
        assert_eq!(
            result,
            Err(ClientFrameError::UnknownHeader { header: 0x99 })
        );
    }
}
