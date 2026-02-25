//! Pure legacy TEA block codec.
//!
//! The default FreeBSD/C++ build uses the small TEA variant in
//! `server/server/libthecore/tea.cpp`: an 8-byte block, a 16-byte key, 32
//! rounds, and 32-bit wrapping arithmetic. The active target is x86, so the
//! byte representation of each `DWORD` is little endian.
//!
//! This module implements a pure one-block codec, explicitly aligned-slice
//! helpers, and an owned whole-input padding helper. It never mutates a
//! caller-owned input, negotiates a key, or wires encryption into a
//! descriptor/session.

#![warn(missing_docs)]

use std::error::Error;
use std::fmt;

/// Number of bytes in one legacy TEA block.
pub const TEA_BLOCK_SIZE: usize = 8;

/// Number of bytes in one legacy TEA key.
pub const TEA_KEY_SIZE: usize = 16;

/// Number of Feistel rounds used by the legacy implementation.
pub const TEA_ROUNDS: usize = 32;

/// The legacy TEA round constant.
pub const TEA_DELTA: u32 = 0x9e37_79b9;

/// A fixed-size legacy TEA key.
pub type TeaKey = [u8; TEA_KEY_SIZE];

/// A fixed-size legacy TEA block.
pub type TeaBlock = [u8; TEA_BLOCK_SIZE];

/// Owned ciphertext produced with the legacy whole-input padding rule.
///
/// `logical_len` is the original plaintext length. `bytes` always has a
/// length that is a multiple of eight; the caller can use `logical_len` when
/// it needs the unpadded frame boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedTeaCiphertext {
    logical_len: usize,
    bytes: Vec<u8>,
}

impl OwnedTeaCiphertext {
    /// Return the original plaintext length before legacy padding.
    #[must_use]
    pub const fn logical_len(&self) -> usize {
        self.logical_len
    }

    /// Return the rounded ciphertext/wire length.
    #[must_use]
    pub fn wire_len(&self) -> usize {
        self.bytes.len()
    }

    /// Borrow the rounded ciphertext bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consume the wrapper and return the rounded ciphertext bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// Encrypt one exact eight-byte block with the legacy TEA variant.
///
/// The input and key bytes are interpreted as little-endian 32-bit words on
/// the active x86 wire profile. All additions and shifts use wrapping 32-bit
/// arithmetic, matching the source C++ behavior.
#[must_use]
pub fn encrypt_block(plaintext: &TeaBlock, key: &TeaKey) -> TeaBlock {
    let key_words = load_key(key);
    let mut y = load_word(plaintext, 0);
    let mut z = load_word(plaintext, 4);
    let mut sum = 0_u32;

    for _ in 0..TEA_ROUNDS {
        let y_mix = ((z << 4) ^ (z >> 5)).wrapping_add(z);
        y = y.wrapping_add(y_mix ^ sum.wrapping_add(key_words[(sum & 3) as usize]));
        sum = sum.wrapping_add(TEA_DELTA);
        let z_mix = ((y << 4) ^ (y >> 5)).wrapping_add(y);
        let z_key = key_words[((sum >> 11) & 3) as usize];
        z = z.wrapping_add(z_mix ^ sum.wrapping_add(z_key));
    }

    let mut block = [0_u8; TEA_BLOCK_SIZE];
    store_word(&mut block, 0, y);
    store_word(&mut block, 4, z);
    block
}

/// Decrypt one exact eight-byte block with the legacy TEA variant.
///
/// This is the inverse of [`encrypt_block`] and uses the same explicit
/// little-endian word and wrapping-arithmetic rules.
#[must_use]
pub fn decrypt_block(ciphertext: &TeaBlock, key: &TeaKey) -> TeaBlock {
    let key_words = load_key(key);
    let mut y = load_word(ciphertext, 0);
    let mut z = load_word(ciphertext, 4);
    let mut sum = TEA_DELTA.wrapping_mul(32);

    for _ in 0..TEA_ROUNDS {
        let z_mix = ((y << 4) ^ (y >> 5)).wrapping_add(y);
        let z_key = key_words[((sum >> 11) & 3) as usize];
        z = z.wrapping_sub(z_mix ^ sum.wrapping_add(z_key));
        sum = sum.wrapping_sub(TEA_DELTA);
        let y_mix = ((z << 4) ^ (z >> 5)).wrapping_add(z);
        y = y.wrapping_sub(y_mix ^ sum.wrapping_add(key_words[(sum & 3) as usize]));
    }

    let mut block = [0_u8; TEA_BLOCK_SIZE];
    store_word(&mut block, 0, y);
    store_word(&mut block, 4, z);
    block
}

/// Encrypt a slice whose length is already a multiple of eight.
///
/// Unlike the legacy `TEA_Encrypt(const DWORD *, ...)` API, this function
/// rejects unaligned input instead of silently rounding it up or writing zero
/// padding. An empty input is valid and produces an empty output.
///
/// # Errors
///
/// Returns [`TeaError::UnalignedLength`] when `data.len()` is not a multiple
/// of eight bytes.
pub fn encrypt_aligned(data: &[u8], key: &TeaKey) -> Result<Vec<u8>, TeaError> {
    check_alignment(data.len())?;
    let mut output = Vec::with_capacity(data.len());
    for chunk in data.chunks_exact(TEA_BLOCK_SIZE) {
        let mut block = [0_u8; TEA_BLOCK_SIZE];
        block.copy_from_slice(chunk);
        output.extend_from_slice(&encrypt_block(&block, key));
    }
    Ok(output)
}

/// Decrypt a slice whose length is already a multiple of eight.
///
/// This helper does not remove or reinterpret padding. Any non-empty trailing
/// bytes are rejected because the legacy receive path must handle its own
/// incomplete ciphertext tail separately.
///
/// # Errors
///
/// Returns [`TeaError::UnalignedLength`] when `data.len()` is not a multiple
/// of eight bytes.
pub fn decrypt_aligned(data: &[u8], key: &TeaKey) -> Result<Vec<u8>, TeaError> {
    check_alignment(data.len())?;
    let mut output = Vec::with_capacity(data.len());
    for chunk in data.chunks_exact(TEA_BLOCK_SIZE) {
        let mut block = [0_u8; TEA_BLOCK_SIZE];
        block.copy_from_slice(chunk);
        output.extend_from_slice(&decrypt_block(&block, key));
    }
    Ok(output)
}

/// Encrypt an owned frame using the legacy whole-input zero-padding rule.
///
/// Unlike the legacy routine, this function never writes through a
/// `const` source pointer. It copies the input, appends explicit zero bytes up
/// to the next block boundary, and returns both the original logical length
/// and the rounded ciphertext bytes. The receive path must still retain any
/// 1..7-byte ciphertext tail and interpret decrypted zero padding as legacy
/// keepalive bytes; this helper does not claim to do either.
///
/// # Errors
///
/// Returns [`TeaError::LengthOverflow`] if the rounded length cannot be
/// represented, or [`TeaError::AllocationFailed`] if the output cannot be
/// reserved.
pub fn encrypt_padded(data: &[u8], key: &TeaKey) -> Result<OwnedTeaCiphertext, TeaError> {
    let remainder = data.len() % TEA_BLOCK_SIZE;
    let padding = if remainder == 0 {
        0
    } else {
        TEA_BLOCK_SIZE - remainder
    };
    let wire_len = data
        .len()
        .checked_add(padding)
        .ok_or(TeaError::LengthOverflow)?;
    let mut padded = Vec::new();
    padded
        .try_reserve_exact(wire_len)
        .map_err(|_| TeaError::AllocationFailed)?;
    padded.extend_from_slice(data);
    padded.resize(wire_len, 0);

    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(wire_len)
        .map_err(|_| TeaError::AllocationFailed)?;
    for chunk in padded.chunks_exact(TEA_BLOCK_SIZE) {
        let mut block = [0_u8; TEA_BLOCK_SIZE];
        block.copy_from_slice(chunk);
        bytes.extend_from_slice(&encrypt_block(&block, key));
    }
    debug_assert_eq!(bytes.len(), wire_len);
    Ok(OwnedTeaCiphertext {
        logical_len: data.len(),
        bytes,
    })
}

/// Decrypt a rounded ciphertext buffer without trimming zero padding.
///
/// The returned bytes include the explicit padding added by
/// [`encrypt_padded`]. The caller owns the logical-length decision, just as
/// the legacy receive processor owns keepalive consumption.
///
/// # Errors
///
/// Returns [`TeaError::UnalignedLength`] when the ciphertext length is not a
/// multiple of eight.
pub fn decrypt_padded(ciphertext: &[u8], key: &TeaKey) -> Result<Vec<u8>, TeaError> {
    decrypt_aligned(ciphertext, key)
}

/// Incremental legacy-TEA ciphertext decoder with an explicit pending limit.
///
/// The legacy descriptor decrypts only the largest eight-byte prefix and keeps
/// a 1..7-byte ciphertext tail for the next socket read. This state machine
/// models that boundary without owning a socket. It returns every decrypted
/// byte, including the legacy zero padding that the input processor may
/// consume as one-byte keepalives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyTeaDecoder {
    key: TeaKey,
    pending: Vec<u8>,
    max_pending_bytes: usize,
}

impl LegacyTeaDecoder {
    /// Construct a decoder with an explicit maximum retained ciphertext size.
    ///
    /// The limit must be at least one eight-byte block. The key is copied into
    /// the decoder; no key installation or phase transition is performed.
    ///
    /// # Errors
    ///
    /// Returns [`TeaStreamError::InvalidLimit`] when `max_pending_bytes` is
    /// smaller than one TEA block.
    pub fn new(key: TeaKey, max_pending_bytes: usize) -> Result<Self, TeaStreamError> {
        if max_pending_bytes < TEA_BLOCK_SIZE {
            return Err(TeaStreamError::InvalidLimit {
                maximum: max_pending_bytes,
            });
        }
        Ok(Self {
            key,
            pending: Vec::new(),
            max_pending_bytes,
        })
    }

    /// Feed arbitrary ciphertext fragments and return newly decrypted bytes.
    ///
    /// Input is not consumed when the pending limit would be exceeded. Only a
    /// complete eight-byte prefix is removed from the pending buffer; a
    /// non-aligned tail remains for the next call.
    ///
    /// # Errors
    ///
    /// Returns [`TeaStreamError::PendingLimitExceeded`] if the new pending
    /// length exceeds the configured bound, or
    /// [`TeaStreamError::AllocationFailed`] if a bounded output buffer cannot
    /// be reserved.
    pub fn feed(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, TeaStreamError> {
        let new_len = self.pending.len().checked_add(ciphertext.len()).ok_or(
            TeaStreamError::PendingLimitExceeded {
                attempted: usize::MAX,
                maximum: self.max_pending_bytes,
            },
        )?;
        if new_len > self.max_pending_bytes {
            return Err(TeaStreamError::PendingLimitExceeded {
                attempted: new_len,
                maximum: self.max_pending_bytes,
            });
        }

        let old_len = self.pending.len();
        if self.pending.try_reserve(ciphertext.len()).is_err() {
            return Err(TeaStreamError::AllocationFailed);
        }
        self.pending.extend_from_slice(ciphertext);
        let aligned_len = self.pending.len() - self.pending.len() % TEA_BLOCK_SIZE;
        if aligned_len == 0 {
            return Ok(Vec::new());
        }

        let mut aligned = Vec::new();
        if let Err(_error) = aligned.try_reserve_exact(aligned_len) {
            self.pending.truncate(old_len);
            return Err(TeaStreamError::AllocationFailed);
        }
        aligned.extend_from_slice(&self.pending[..aligned_len]);
        match decrypt_aligned(&aligned, &self.key) {
            Ok(plaintext) => {
                self.pending.drain(..aligned_len);
                Ok(plaintext)
            }
            Err(error) => {
                self.pending.truncate(old_len);
                Err(TeaStreamError::Tea(error))
            }
        }
    }

    /// Return the number of ciphertext bytes retained for a future fragment.
    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Return whether a non-aligned ciphertext tail is currently retained.
    #[must_use]
    pub fn has_tail(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Check that a stream ended on a complete block boundary.
    ///
    /// # Errors
    ///
    /// Returns [`TeaStreamError::IncompleteTail`] when a 1..7-byte ciphertext
    /// tail remains.
    pub fn finish(&self) -> Result<(), TeaStreamError> {
        if self.pending.is_empty() {
            Ok(())
        } else {
            Err(TeaStreamError::IncompleteTail {
                pending: self.pending.len(),
            })
        }
    }
}

/// A rejected legacy TEA slice length or owned-output operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeaError {
    /// The input length was not a multiple of the eight-byte block size.
    UnalignedLength {
        /// Supplied input length.
        length: usize,
    },
    /// Rounded output length arithmetic overflowed `usize`.
    LengthOverflow,
    /// The owned ciphertext buffer could not be reserved.
    AllocationFailed,
}

impl fmt::Display for TeaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnalignedLength { length } => write!(
                formatter,
                "legacy TEA input length {length} is not a multiple of {TEA_BLOCK_SIZE}"
            ),
            Self::LengthOverflow => write!(formatter, "legacy TEA rounded length overflows usize"),
            Self::AllocationFailed => write!(formatter, "legacy TEA output allocation failed"),
        }
    }
}

impl Error for TeaError {}

/// A rejected incremental legacy-TEA stream operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeaStreamError {
    /// The configured pending-ciphertext bound is smaller than one block.
    InvalidLimit {
        /// Configured maximum retained bytes.
        maximum: usize,
    },
    /// Feeding the fragment would exceed the configured pending bound.
    PendingLimitExceeded {
        /// Pending length after the proposed feed.
        attempted: usize,
        /// Configured maximum retained bytes.
        maximum: usize,
    },
    /// End-of-stream was reached with a non-aligned ciphertext tail.
    IncompleteTail {
        /// Retained ciphertext bytes.
        pending: usize,
    },
    /// A bounded intermediate buffer could not be reserved.
    AllocationFailed,
    /// The underlying aligned TEA operation failed.
    Tea(TeaError),
}

impl fmt::Display for TeaStreamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimit { maximum } => write!(
                formatter,
                "legacy TEA pending limit {maximum} is smaller than {TEA_BLOCK_SIZE}"
            ),
            Self::PendingLimitExceeded { attempted, maximum } => write!(
                formatter,
                "legacy TEA pending length {attempted} exceeds configured limit {maximum}"
            ),
            Self::IncompleteTail { pending } => write!(
                formatter,
                "legacy TEA stream ended with {pending} non-aligned ciphertext bytes"
            ),
            Self::AllocationFailed => write!(formatter, "legacy TEA stream allocation failed"),
            Self::Tea(error) => write!(formatter, "legacy TEA stream decode failed: {error}"),
        }
    }
}

impl Error for TeaStreamError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Tea(error) => Some(error),
            _ => None,
        }
    }
}

impl From<TeaError> for TeaStreamError {
    fn from(error: TeaError) -> Self {
        Self::Tea(error)
    }
}

fn check_alignment(length: usize) -> Result<(), TeaError> {
    if length % TEA_BLOCK_SIZE == 0 {
        Ok(())
    } else {
        Err(TeaError::UnalignedLength { length })
    }
}

fn load_key(key: &TeaKey) -> [u32; 4] {
    [
        load_word(key, 0),
        load_word(key, 4),
        load_word(key, 8),
        load_word(key, 12),
    ]
}

fn load_word(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

fn store_word(data: &mut [u8], offset: usize, value: u32) {
    data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_golden_block_vectors_match_explicit_little_endian_words() {
        let zero_key: TeaKey = [0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0];
        let encrypted = encrypt_block(&[0; TEA_BLOCK_SIZE], &zero_key);
        assert_eq!(encrypted, [0x59, 0x0a, 0x84, 0x1e, 0x0b, 0x2c, 0x05, 0xb3]);
        assert_eq!(decrypt_block(&encrypted, &zero_key), [0; TEA_BLOCK_SIZE]);

        let static_key: TeaKey = *b"1234abcd5678efgh";
        let encrypted = encrypt_block(&[0; TEA_BLOCK_SIZE], &static_key);
        assert_eq!(encrypted, [0xb7, 0x5a, 0xc3, 0xda, 0xdd, 0xd3, 0x55, 0xf9]);
        assert_eq!(decrypt_block(&encrypted, &static_key), [0; TEA_BLOCK_SIZE]);
    }

    #[test]
    fn all_byte_blocks_round_trip_without_changing_input() {
        let key: TeaKey = *b"0123456789abcdef";
        for first in [0_u8, 1, 0x7f, 0x80, u8::MAX] {
            for second in [0_u8, 0x55, 0xaa, u8::MAX] {
                let plaintext: TeaBlock = [first, 1, 2, 3, second, 5, 6, 7];
                let original = plaintext;
                let ciphertext = encrypt_block(&plaintext, &key);
                assert_eq!(plaintext, original);
                assert_eq!(decrypt_block(&ciphertext, &key), original);
            }
        }
    }

    #[test]
    fn owned_padding_returns_logical_and_wire_lengths_without_mutating_input() {
        let key: TeaKey = *b"0123456789abcdef";
        for logical_len in 0..=16_usize {
            let input: Vec<u8> = (0..logical_len)
                .map(|index| u8::try_from(index).unwrap_or(0).wrapping_mul(29))
                .collect();
            let original = input.clone();
            let encrypted = encrypt_padded(&input, &key).unwrap();
            let expected_wire_len = logical_len.div_ceil(TEA_BLOCK_SIZE) * TEA_BLOCK_SIZE;
            assert_eq!(encrypted.logical_len(), logical_len);
            assert_eq!(encrypted.wire_len(), expected_wire_len);
            assert_eq!(encrypted.as_bytes().len(), expected_wire_len);
            assert_eq!(input, original);

            let decrypted = decrypt_padded(encrypted.as_bytes(), &key).unwrap();
            assert_eq!(decrypted.len(), expected_wire_len);
            assert_eq!(&decrypted[..logical_len], &input[..]);
            assert!(decrypted[logical_len..].iter().all(|byte| *byte == 0));
        }
        assert!(matches!(
            encrypt_padded(&[1; 9], &key),
            Ok(OwnedTeaCiphertext { .. })
        ));
        assert!(matches!(
            decrypt_padded(&[0; 9], &key),
            Err(TeaError::UnalignedLength { length: 9 })
        ));
    }

    #[test]
    fn stream_decoder_retains_tails_and_handles_bytewise_fragments() {
        let key: TeaKey = *b"stream-key-12345";
        let plaintext = b"fragmented legacy frame";
        let encrypted = encrypt_padded(plaintext, &key).unwrap();
        let mut decoder = LegacyTeaDecoder::new(key, 64).unwrap();
        let mut output = Vec::new();
        for byte in encrypted.as_bytes() {
            output.extend_from_slice(&decoder.feed(&[*byte]).unwrap());
        }
        assert_eq!(output, decrypt_padded(encrypted.as_bytes(), &key).unwrap());
        assert_eq!(output.len(), encrypted.wire_len());
        assert_eq!(&output[..plaintext.len()], plaintext);
        assert!(output[plaintext.len()..].iter().all(|byte| *byte == 0));
        assert_eq!(decoder.pending_len(), 0);
        assert!(!decoder.has_tail());
        decoder.finish().unwrap();
    }

    #[test]
    fn stream_decoder_handles_coalesced_frames_and_does_not_trim_padding() {
        let key: TeaKey = *b"stream-key-12345";
        let first = encrypt_padded(b"first", &key).unwrap();
        let second = encrypt_padded(b"second frame", &key).unwrap();
        let mut ciphertext = first.as_bytes().to_vec();
        ciphertext.extend_from_slice(second.as_bytes());
        let mut decoder = LegacyTeaDecoder::new(key, ciphertext.len()).unwrap();
        let mut expected = decrypt_padded(first.as_bytes(), &key).unwrap();
        expected.extend_from_slice(&decrypt_padded(second.as_bytes(), &key).unwrap());
        assert_eq!(decoder.feed(&ciphertext).unwrap(), expected);
        assert!(decoder.finish().is_ok());
    }

    #[test]
    fn stream_decoder_limit_and_tail_errors_are_typed_and_non_consuming() {
        let key: TeaKey = *b"stream-key-12345";
        assert_eq!(
            LegacyTeaDecoder::new(key, TEA_BLOCK_SIZE - 1),
            Err(TeaStreamError::InvalidLimit {
                maximum: TEA_BLOCK_SIZE - 1
            })
        );
        let mut decoder = LegacyTeaDecoder::new(key, TEA_BLOCK_SIZE).unwrap();
        assert_eq!(
            decoder.feed(&[0; TEA_BLOCK_SIZE + 1]),
            Err(TeaStreamError::PendingLimitExceeded {
                attempted: TEA_BLOCK_SIZE + 1,
                maximum: TEA_BLOCK_SIZE
            })
        );
        assert_eq!(decoder.pending_len(), 0);
        assert!(decoder.feed(&[1, 2, 3]).unwrap().is_empty());
        assert_eq!(decoder.pending_len(), 3);
        assert!(decoder.has_tail());
        assert_eq!(
            decoder.finish(),
            Err(TeaStreamError::IncompleteTail { pending: 3 })
        );
        assert_eq!(decoder.feed(&[4; 5]).unwrap().len(), 8);
        decoder.finish().unwrap();
    }

    #[test]
    fn all_ones_and_extreme_words_round_trip_through_wrapping_arithmetic() {
        let key: TeaKey = [0xff; TEA_KEY_SIZE];
        let plaintext: TeaBlock = [0xff; TEA_BLOCK_SIZE];
        let encrypted = encrypt_block(&plaintext, &key);
        assert_eq!(decrypt_block(&encrypted, &key), plaintext);
        assert_eq!(decrypt_block(&encrypt_block(&[0; 8], &key), &key), [0; 8]);
    }

    #[test]
    fn aligned_helpers_accept_empty_and_aligned_inputs_and_reject_padding() {
        let key: TeaKey = *b"fedcba9876543210";
        assert_eq!(encrypt_aligned(&[], &key).unwrap(), Vec::<u8>::new());
        assert_eq!(decrypt_aligned(&[], &key).unwrap(), Vec::<u8>::new());

        let mut plaintext = Vec::with_capacity(24);
        for index in 0..24 {
            plaintext.push(u8::try_from(index).unwrap_or(0).wrapping_mul(17));
        }
        let original = plaintext.clone();
        let ciphertext = encrypt_aligned(&plaintext, &key).unwrap();
        assert_eq!(plaintext, original);
        assert_eq!(ciphertext.len(), original.len());
        assert_eq!(decrypt_aligned(&ciphertext, &key).unwrap(), original);

        for length in 1..TEA_BLOCK_SIZE {
            assert_eq!(
                encrypt_aligned(&original[..length], &key),
                Err(TeaError::UnalignedLength { length })
            );
            assert_eq!(
                decrypt_aligned(&original[..length], &key),
                Err(TeaError::UnalignedLength { length })
            );
        }
        assert_eq!(
            encrypt_aligned(&original[..9], &key),
            Err(TeaError::UnalignedLength { length: 9 })
        );
    }

    #[test]
    fn explicit_little_endian_word_order_is_not_host_layout() {
        let plaintext: TeaBlock = [0x78, 0x56, 0x34, 0x12, 0xef, 0xbe, 0xad, 0xde];
        let key: TeaKey = [
            0x31, 0x32, 0x33, 0x34, 0x61, 0x62, 0x63, 0x64, 0x35, 0x36, 0x37, 0x38, 0x65, 0x66,
            0x67, 0x68,
        ];
        let encrypted = encrypt_block(&plaintext, &key);
        // The first four bytes encode the source word `0x12345678` in little
        // endian order; this golden output pins that byte order explicitly.
        assert_eq!(encrypted, [0xa3, 0x5e, 0x82, 0xb4, 0xcd, 0x95, 0xfe, 0xce]);
        assert_eq!(decrypt_block(&encrypted, &key), plaintext);
    }
}
