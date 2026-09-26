//! Source-derived legacy descriptor cipher boundary.
//!
//! The legacy non-improved encryption path in
//! `server/server/game/desc.cpp:304-345,397-477,494-542,928-963` keeps a
//! plaintext input buffer before header dispatch, decrypts only the largest
//! eight-byte prefix of the incoming encrypted stream, and encrypts each
//! output packet separately with zero padding. This module models that state
//! transition without a socket, descriptor, input processor, or packet
//! analyzer.
//!
//! `DESC::Setup` copies the 16-byte ASCII default key into both TEA key
//! arrays. After the phase record is routed through plaintext, `SetPhase`
//! enables TEA with that raw pair. Later, `DESC::SetSecurityKey` copies the
//! four client-key words to the decryption key and encrypts those 16 bytes
//! with the source's `GetKey_20050304Myevan() + 37` byte offset. The table is
//! reproduced with explicit little-endian bytes; no host pointer alignment or
//! layout is used.
//!
//! The improved `_IMPROVED_PACKET_ENCRYPTION_` DH2/key-agreement path is not
//! implemented or inferred here. Calling this an adapter does not make it a
//! live encrypted session.

#![warn(missing_docs)]

use std::error::Error;
use std::fmt;

use protocol::tea::{
    encrypt_aligned, encrypt_padded, LegacyTeaDecoder, TeaError, TeaKey, TeaStreamError,
    TEA_BLOCK_SIZE,
};

/// The 16-byte key installed by the legacy descriptor setup path.
pub const LEGACY_DEFAULT_KEY: TeaKey = *b"1234abcd5678efgh";

/// Number of `DWORD` entries in the source Myevan key table.
pub const LEGACY_MYEVAN_TABLE_WORDS: usize = 1938;

/// Byte offset used by `GetKey_20050304Myevan() + 37` in `desc.cpp`.
pub const LEGACY_MYEVAN_KEY_OFFSET: usize = 37;

/// The default Rust plaintext-buffer policy based on source `MAX_INPUT_LEN`.
///
/// Legacy `MAX_INPUT_LEN` is an initial/growth capacity, not a hard semantic
/// wire maximum. This adapter deliberately treats it as a bounded security
/// policy so an input fragment cannot grow memory without limit.
pub const LEGACY_DEFAULT_INPUT_LIMIT: usize = 65_536;

/// A source-derived pair of legacy descriptor keys.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LegacyDescriptorKeys {
    decryption: TeaKey,
    encryption: TeaKey,
}

impl LegacyDescriptorKeys {
    /// Return the client-key bytes used for incoming TEA decryption.
    #[must_use]
    pub const fn decryption_key(self) -> TeaKey {
        self.decryption
    }

    /// Return the derived server key used for outgoing TEA encryption.
    #[must_use]
    pub const fn encryption_key(self) -> TeaKey {
        self.encryption
    }
}

/// Return the source `GetKey_20050304Myevan() + 37` key bytes.
///
/// The C++ loop tests `BYTE(seed)` after each seed update. The table is
/// represented as bytes here so the deliberately unaligned source pointer is
/// never recreated with a Rust reference or host-layout assumption.
#[must_use]
pub fn legacy_myevan_key() -> TeaKey {
    legacy_myevan_key_and_count().0
}

fn legacy_myevan_key_and_count() -> (TeaKey, usize) {
    const TABLE_BYTES: usize = LEGACY_MYEVAN_TABLE_WORDS * 4;
    let mut seed = 1_491_971_513_u32;
    let mut table = [0_u8; TABLE_BYTES];

    let mut index = 0;
    while index < LEGACY_MYEVAN_TABLE_WORDS {
        // The source evaluates BYTE(seed) again on every loop iteration.
        let limit = u8::try_from(seed & 0xff).unwrap_or(0);
        if index >= usize::from(limit) {
            break;
        }
        seed ^= 2_148_941_891;
        seed = seed.wrapping_add(3_592_385_981);
        let start = index * 4;
        table[start..start + 4].copy_from_slice(&seed.to_le_bytes());
        index += 1;
    }

    let mut key = [0_u8; 16];
    let start = LEGACY_MYEVAN_KEY_OFFSET;
    key.copy_from_slice(&table[start..start + 16]);
    (key, index)
}

/// Derive the exact legacy decryption/encryption key pair.
///
/// The client key is retained verbatim as the decryption key. The encryption
/// key is the 16-byte TEA encryption of that client key under the source
/// Myevan key.
///
/// # Errors
///
/// Returns [`DescriptorCryptoError::Tea`] if the fixed-size allocation for
/// the source-derived transform cannot be created.
pub fn derive_legacy_descriptor_keys(
    client_key: TeaKey,
) -> Result<LegacyDescriptorKeys, DescriptorCryptoError> {
    let source_key = legacy_myevan_key();
    let encryption =
        encrypt_aligned(&client_key, &source_key).map_err(DescriptorCryptoError::Tea)?;
    Ok(LegacyDescriptorKeys {
        decryption: client_key,
        encryption: encryption
            .try_into()
            .map_err(|_| DescriptorCryptoError::Tea(TeaError::LengthOverflow))?,
    })
}

/// Return the raw default key pair installed by `DESC::Setup`.
///
/// Unlike [`derive_legacy_descriptor_keys`], this does not derive the
/// server-side TEA key. The legacy setup copies the same 16 ASCII bytes to
/// both key arrays; the Myevan transform is used only by `SetSecurityKey`.
#[must_use]
pub fn legacy_default_descriptor_keys() -> LegacyDescriptorKeys {
    LegacyDescriptorKeys {
        decryption: LEGACY_DEFAULT_KEY,
        encryption: LEGACY_DEFAULT_KEY,
    }
}

/// The explicit cipher state selected by a descriptor boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescriptorCryptoMode {
    /// The initial handshake boundary, before `SetPhase` enables TEA.
    Plaintext,
    /// The post-phase boundary using the raw default key pair.
    ///
    /// `SetPhase` enables TEA before `SetSecurityKey` installs the
    /// client-specific pair. The phase record itself is still written through
    /// the preceding plaintext output boundary by the caller.
    LegacyTeaDefault,
    /// The legacy non-improved TEA boundary after `SetSecurityKey`.
    LegacyTea,
}

/// A rejected descriptor cipher operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DescriptorCryptoError {
    /// The configured input bound is smaller than one TEA block.
    InvalidInputLimit {
        /// Configured maximum input bytes.
        maximum: usize,
    },
    /// Feeding or retaining input would exceed the configured bound.
    InputLimitExceeded {
        /// Prospective combined plaintext input length after this fragment.
        attempted: usize,
        /// Configured maximum input bytes.
        maximum: usize,
    },
    /// A key change was requested while plaintext input remained unconsumed.
    InputNotEmpty {
        /// Number of unconsumed plaintext bytes.
        pending: usize,
    },
    /// A client key was requested before the phase transition enabled TEA.
    KeyInstallBeforeTea,
    /// The descriptor already has a client-specific key installed.
    KeyAlreadyInstalled,
    /// A key change would reinterpret an incomplete ciphertext tail.
    PendingCiphertext {
        /// Number of retained ciphertext bytes.
        pending: usize,
    },
    /// A caller attempted to consume more plaintext than is available.
    ConsumeOutOfRange {
        /// Requested consumption length.
        requested: usize,
        /// Available plaintext length.
        available: usize,
    },
    /// A bounded plaintext buffer could not be reserved.
    AllocationFailed,
    /// A fixed-size legacy TEA operation failed.
    Tea(TeaError),
    /// The incremental ciphertext decoder failed.
    TeaStream(TeaStreamError),
}

impl fmt::Display for DescriptorCryptoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInputLimit { maximum } => write!(
                formatter,
                "descriptor input limit {maximum} is smaller than one TEA block"
            ),
            Self::InputLimitExceeded { attempted, maximum } => write!(
                formatter,
                "descriptor input length {attempted} exceeds configured limit {maximum}"
            ),
            Self::InputNotEmpty { pending } => write!(
                formatter,
                "cannot change descriptor keys with {pending} unconsumed plaintext bytes"
            ),
            Self::KeyInstallBeforeTea => {
                formatter.write_str("client key installation requires the TEA phase boundary")
            }
            Self::KeyAlreadyInstalled => {
                formatter.write_str("descriptor client key is already installed")
            }
            Self::PendingCiphertext { pending } => write!(
                formatter,
                "cannot change descriptor keys with {pending} pending ciphertext bytes"
            ),
            Self::ConsumeOutOfRange {
                requested,
                available,
            } => write!(
                formatter,
                "descriptor input consumption {requested} exceeds available {available}"
            ),
            Self::AllocationFailed => formatter.write_str("descriptor buffer allocation failed"),
            Self::Tea(error) => write!(formatter, "descriptor TEA operation failed: {error}"),
            Self::TeaStream(error) => write!(formatter, "descriptor TEA stream failed: {error}"),
        }
    }
}

impl Error for DescriptorCryptoError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Tea(error) => Some(error),
            Self::TeaStream(error) => Some(error),
            _ => None,
        }
    }
}

/// A transport-free legacy descriptor cipher and input buffer.
///
/// Incoming bytes are appended to a bounded plaintext buffer only after the
/// configured cipher boundary has processed them. A caller parses complete
/// frames from [`Self::input`] and reports exactly how many bytes its input
/// processor consumed with [`Self::consume_input`]. A security-key change keeps
/// already-decrypted bytes in this plaintext buffer and starts the new decoder
/// for subsequent fragments, matching the legacy temporary-buffer boundary.
/// The module does not parse packet headers or invoke a phase-specific analyzer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DescriptorCrypto {
    mode: DescriptorCryptoMode,
    keys: Option<LegacyDescriptorKeys>,
    decoder: Option<LegacyTeaDecoder>,
    input: Vec<u8>,
    max_input_bytes: usize,
}

impl DescriptorCrypto {
    /// Construct a plaintext descriptor boundary with an explicit input limit.
    ///
    /// The limit must be at least one eight-byte TEA block so a later key
    /// installation can use the source-aligned stream decoder.
    ///
    /// # Errors
    ///
    /// Returns [`DescriptorCryptoError::InvalidInputLimit`] when
    /// `max_input_bytes` is smaller than one TEA block.
    pub fn new(max_input_bytes: usize) -> Result<Self, DescriptorCryptoError> {
        if max_input_bytes < TEA_BLOCK_SIZE {
            return Err(DescriptorCryptoError::InvalidInputLimit {
                maximum: max_input_bytes,
            });
        }
        Ok(Self {
            mode: DescriptorCryptoMode::Plaintext,
            keys: None,
            decoder: None,
            input: Vec::new(),
            max_input_bytes,
        })
    }

    /// Construct a boundary using the source `MAX_INPUT_LEN` value.
    ///
    /// # Errors
    ///
    /// Returns [`DescriptorCryptoError::InvalidInputLimit`] only if the
    /// compile-time default ever ceases to satisfy the TEA-block invariant.
    pub fn with_default_limit() -> Result<Self, DescriptorCryptoError> {
        Self::new(LEGACY_DEFAULT_INPUT_LIMIT)
    }

    /// Return the currently selected cipher boundary.
    #[must_use]
    pub const fn mode(&self) -> DescriptorCryptoMode {
        self.mode
    }

    /// Return the installed source-derived key pair, if any.
    #[must_use]
    pub const fn keys(&self) -> Option<LegacyDescriptorKeys> {
        self.keys
    }

    /// Return the current bounded plaintext input buffer.
    #[must_use]
    pub fn input(&self) -> &[u8] {
        &self.input
    }

    /// Return the number of unconsumed plaintext bytes.
    #[must_use]
    pub fn input_len(&self) -> usize {
        self.input.len()
    }

    /// Return the number of ciphertext bytes retained for a future fragment.
    #[must_use]
    pub fn pending_ciphertext_len(&self) -> usize {
        self.decoder
            .as_ref()
            .map_or(0, LegacyTeaDecoder::pending_len)
    }

    /// Select the post-phase legacy TEA boundary with the raw setup keys.
    ///
    /// This corresponds to `DESC::SetPhase` selecting the Login/Auth input
    /// processor. The phase record must be routed through the preceding
    /// plaintext output boundary before the caller invokes this method.
    /// `SetSecurityKey` is a later, separate operation.
    ///
    /// # Errors
    ///
    /// Returns [`DescriptorCryptoError::InputNotEmpty`] if plaintext remains,
    /// or [`DescriptorCryptoError::KeyAlreadyInstalled`] if TEA is already
    /// selected.
    pub fn enable_default_legacy_tea(&mut self) -> Result<(), DescriptorCryptoError> {
        if self.mode != DescriptorCryptoMode::Plaintext {
            return Err(DescriptorCryptoError::KeyAlreadyInstalled);
        }
        if !self.input.is_empty() {
            return Err(DescriptorCryptoError::InputNotEmpty {
                pending: self.input.len(),
            });
        }
        self.install_key_pair(
            legacy_default_descriptor_keys(),
            DescriptorCryptoMode::LegacyTeaDefault,
        )
    }

    /// Install the source-derived client keys for `SetSecurityKey`.
    ///
    /// The source performs this operation after the phase has already selected
    /// TEA. Already-decrypted bytes in the temporary plaintext buffer remain
    /// available and are not reinterpreted; the replacement decoder applies to
    /// the next ciphertext fragment. A pending incomplete ciphertext tail is
    /// rejected because it cannot be safely assigned to two key epochs.
    ///
    /// A client key replaces an earlier one, as legacy `SetSecurityKey`
    /// (`desc.cpp:951-963`) overwrites both keys on every `LOGIN2`.
    ///
    /// # Errors
    ///
    /// Returns [`DescriptorCryptoError::KeyInstallBeforeTea`] before the phase
    /// transition, or [`DescriptorCryptoError::PendingCiphertext`] when a
    /// 1..7-byte tail remains.
    pub fn install_legacy_key(&mut self, client_key: TeaKey) -> Result<(), DescriptorCryptoError> {
        if self.mode == DescriptorCryptoMode::Plaintext {
            return Err(DescriptorCryptoError::KeyInstallBeforeTea);
        }
        if self.pending_ciphertext_len() != 0 {
            return Err(DescriptorCryptoError::PendingCiphertext {
                pending: self.pending_ciphertext_len(),
            });
        }
        let keys = derive_legacy_descriptor_keys(client_key)?;
        self.install_key_pair(keys, DescriptorCryptoMode::LegacyTea)
    }

    fn install_key_pair(
        &mut self,
        keys: LegacyDescriptorKeys,
        mode: DescriptorCryptoMode,
    ) -> Result<(), DescriptorCryptoError> {
        let decoder = LegacyTeaDecoder::new(keys.decryption_key(), self.max_input_bytes)
            .map_err(DescriptorCryptoError::TeaStream)?;
        self.keys = Some(keys);
        self.decoder = Some(decoder);
        self.mode = mode;
        Ok(())
    }

    /// Feed a caller-owned input fragment through the selected boundary.
    ///
    /// Legacy TEA decrypts only complete blocks and retains a 1..7-byte
    /// ciphertext tail. The returned count is the number of newly available
    /// plaintext bytes, which can be smaller than the fragment length.
    ///
    /// The bound is enforced against prospective plaintext, including bytes
    /// released from a pending ciphertext tail. On an allocation or TEA error
    /// the caller's plaintext bytes are not changed; the underlying decoder
    /// preserves its pending ciphertext on its documented failures.
    ///
    /// # Errors
    ///
    /// Returns a bounded-input, allocation, or TEA stream error as described
    /// by [`DescriptorCryptoError`].
    pub fn feed_input(&mut self, fragment: &[u8]) -> Result<usize, DescriptorCryptoError> {
        let prospective = if self.mode == DescriptorCryptoMode::Plaintext {
            fragment.len()
        } else {
            let pending = self.pending_ciphertext_len();
            let combined = pending.checked_add(fragment.len()).ok_or(
                DescriptorCryptoError::InputLimitExceeded {
                    attempted: usize::MAX,
                    maximum: self.max_input_bytes,
                },
            )?;
            combined - (combined % TEA_BLOCK_SIZE)
        };
        let attempted = self.input.len().checked_add(prospective).ok_or(
            DescriptorCryptoError::InputLimitExceeded {
                attempted: usize::MAX,
                maximum: self.max_input_bytes,
            },
        )?;
        if attempted > self.max_input_bytes {
            return Err(DescriptorCryptoError::InputLimitExceeded {
                attempted,
                maximum: self.max_input_bytes,
            });
        }
        if self.input.try_reserve(prospective).is_err() {
            return Err(DescriptorCryptoError::AllocationFailed);
        }

        if self.mode == DescriptorCryptoMode::Plaintext {
            self.input.extend_from_slice(fragment);
            return Ok(fragment.len());
        }

        let output = self
            .decoder
            .as_mut()
            .ok_or(DescriptorCryptoError::TeaStream(
                TeaStreamError::InvalidLimit {
                    maximum: self.max_input_bytes,
                },
            ))?
            .feed(fragment)
            .map_err(DescriptorCryptoError::TeaStream)?;
        let added = output.len();
        self.input.extend_from_slice(&output);
        Ok(added)
    }

    /// Consume bytes accepted by the caller's input processor.
    ///
    /// The legacy input buffer advances by the processor's reported byte
    /// count. Rejecting an out-of-range count prevents a malformed adapter
    /// from silently dropping or duplicating a frame.
    ///
    /// # Errors
    ///
    /// Returns [`DescriptorCryptoError::ConsumeOutOfRange`] when `count`
    /// exceeds the currently buffered plaintext length.
    pub fn consume_input(&mut self, count: usize) -> Result<(), DescriptorCryptoError> {
        if count > self.input.len() {
            return Err(DescriptorCryptoError::ConsumeOutOfRange {
                requested: count,
                available: self.input.len(),
            });
        }
        self.input.drain(..count);
        Ok(())
    }

    /// Check only the ciphertext block tail at an input-stream boundary.
    ///
    /// This does not inspect or drain unconsumed plaintext. The caller owns
    /// descriptor EOF/teardown and must handle that state separately.
    ///
    /// # Errors
    ///
    /// Returns [`DescriptorCryptoError::TeaStream`] when one to seven
    /// ciphertext bytes remain without a final block.
    pub fn finish_input(&self) -> Result<(), DescriptorCryptoError> {
        if let Some(decoder) = &self.decoder {
            decoder.finish().map_err(DescriptorCryptoError::TeaStream)?;
        }
        Ok(())
    }

    /// Encrypt one output packet using the active descriptor boundary.
    ///
    /// In legacy TEA mode the source `TEA_Encrypt` zero-pads each packet to an
    /// eight-byte multiple. The returned vector is the complete wire payload;
    /// the logical packet length remains the caller's responsibility.
    ///
    /// # Errors
    ///
    /// Returns [`DescriptorCryptoError::AllocationFailed`] or
    /// [`DescriptorCryptoError::Tea`] if the packet cannot be prepared.
    pub fn encrypt_output(&self, plaintext: &[u8]) -> Result<Vec<u8>, DescriptorCryptoError> {
        match self.mode {
            DescriptorCryptoMode::Plaintext => {
                let mut output = Vec::new();
                if output.try_reserve(plaintext.len()).is_err() {
                    return Err(DescriptorCryptoError::AllocationFailed);
                }
                output.extend_from_slice(plaintext);
                Ok(output)
            }
            DescriptorCryptoMode::LegacyTeaDefault | DescriptorCryptoMode::LegacyTea => {
                let keys = self
                    .keys
                    .ok_or(DescriptorCryptoError::Tea(TeaError::LengthOverflow))?;
                encrypt_padded(plaintext, &keys.encryption_key())
                    .map(protocol::tea::OwnedTeaCiphertext::into_bytes)
                    .map_err(DescriptorCryptoError::Tea)
            }
        }
    }

    /// Clear all buffered input, keys, and cipher state.
    pub fn clear(&mut self) {
        self.mode = DescriptorCryptoMode::Plaintext;
        self.keys = None;
        self.decoder = None;
        self.input.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::tea::{
        decrypt_aligned, decrypt_padded, encrypt_aligned, encrypt_padded, TEA_BLOCK_SIZE,
    };

    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write as _;

        let mut output = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
        }
        output
    }

    fn client_key() -> TeaKey {
        *b"client-key-12345"
    }

    #[test]
    fn source_key_table_reevaluates_the_dynamic_seed_limit() {
        let (key, generated_words) = legacy_myevan_key_and_count();
        assert_eq!(generated_words, 38);
        assert_eq!(hex(&key), "c928b68fff5d0c89356b6287eb9cb881");
        assert_eq!(key, legacy_myevan_key());
    }

    #[test]
    fn default_descriptor_keys_are_raw_on_both_sides() {
        let keys = legacy_default_descriptor_keys();
        assert_eq!(keys.decryption_key(), LEGACY_DEFAULT_KEY);
        assert_eq!(keys.encryption_key(), LEGACY_DEFAULT_KEY);
        assert_eq!(
            hex(&encrypt_aligned(b"12345678", &keys.encryption_key()).unwrap()),
            "1d28905a47e325c4"
        );
    }

    #[test]
    fn derives_the_source_client_and_server_keys() {
        let client = *b"0123456789abcdef";
        let keys = derive_legacy_descriptor_keys(client).unwrap();
        assert_eq!(keys.decryption_key(), client);
        assert_eq!(
            hex(&keys.encryption_key()),
            "e857888da816642dbe8a788df30a6049"
        );
    }

    #[test]
    fn plaintext_boundary_does_not_pad_or_install_keys() {
        let mut crypto = DescriptorCrypto::new(16).unwrap();
        assert_eq!(crypto.mode(), DescriptorCryptoMode::Plaintext);
        assert_eq!(crypto.feed_input(b"abc").unwrap(), 3);
        assert_eq!(crypto.input(), b"abc");
        assert_eq!(crypto.encrypt_output(b"xyz").unwrap(), b"xyz");
        assert!(crypto.keys().is_none());
        assert_eq!(crypto.pending_ciphertext_len(), 0);
        crypto.consume_input(2).unwrap();
        assert_eq!(crypto.input(), b"c");
    }

    #[test]
    fn phase_transition_uses_raw_default_keys_before_security_install() {
        let mut crypto = DescriptorCrypto::with_default_limit().unwrap();
        crypto.feed_input(b"old").unwrap();
        assert_eq!(
            crypto.enable_default_legacy_tea(),
            Err(DescriptorCryptoError::InputNotEmpty { pending: 3 })
        );
        crypto.consume_input(3).unwrap();
        crypto.enable_default_legacy_tea().unwrap();
        assert_eq!(crypto.mode(), DescriptorCryptoMode::LegacyTeaDefault);
        assert_eq!(crypto.keys(), Some(legacy_default_descriptor_keys()));

        let default_ciphertext = encrypt_padded(b"next", &LEGACY_DEFAULT_KEY).unwrap();
        assert_eq!(crypto.feed_input(default_ciphertext.as_bytes()).unwrap(), 8);
        assert_eq!(crypto.input(), b"next\0\0\0\0");

        let client = client_key();
        let derived = derive_legacy_descriptor_keys(client).unwrap();
        crypto.install_legacy_key(client).unwrap();
        assert_eq!(crypto.mode(), DescriptorCryptoMode::LegacyTea);
        assert_eq!(crypto.keys(), Some(derived));
        // Already-decrypted bytes remain available across the key epoch.
        assert_eq!(crypto.input(), b"next\0\0\0\0");
    }

    #[test]
    fn legacy_input_keeps_only_aligned_prefix_and_preserves_padding() {
        let client = client_key();
        let keys = derive_legacy_descriptor_keys(client).unwrap();
        let ciphertext =
            encrypt_padded(b"fragmented descriptor frame", &keys.decryption_key()).unwrap();
        let mut crypto = DescriptorCrypto::new(64).unwrap();
        crypto.enable_default_legacy_tea().unwrap();
        crypto.install_legacy_key(client).unwrap();
        assert_eq!(crypto.mode(), DescriptorCryptoMode::LegacyTea);

        for byte in ciphertext.as_bytes() {
            crypto.feed_input(&[*byte]).unwrap();
        }
        let expected = decrypt_padded(ciphertext.as_bytes(), &keys.decryption_key()).unwrap();
        assert_eq!(crypto.input(), expected.as_slice());
        assert_eq!(crypto.pending_ciphertext_len(), 0);
        assert!(crypto.finish_input().is_ok());
        assert_eq!(
            crypto.encrypt_output(b"out").unwrap().len() % TEA_BLOCK_SIZE,
            0
        );
    }

    #[test]
    fn rejects_a_key_before_tea_and_with_a_pending_tail() {
        let client = client_key();
        let mut crypto = DescriptorCrypto::with_default_limit().unwrap();
        assert_eq!(
            crypto.install_legacy_key(client),
            Err(DescriptorCryptoError::KeyInstallBeforeTea)
        );
        crypto.enable_default_legacy_tea().unwrap();
        crypto.feed_input(&[1, 2, 3, 4, 5, 6, 7]).unwrap();
        assert_eq!(crypto.pending_ciphertext_len(), 7);
        assert_eq!(
            crypto.install_legacy_key(client),
            Err(DescriptorCryptoError::PendingCiphertext { pending: 7 })
        );
        crypto.feed_input(&[0]).unwrap();
        assert_eq!(crypto.pending_ciphertext_len(), 0);
        crypto.install_legacy_key(client).unwrap();
        assert_eq!(crypto.mode(), DescriptorCryptoMode::LegacyTea);
    }

    #[test]
    fn a_second_client_key_replaces_the_first() {
        let first = client_key();
        let mut second = first;
        second[0] ^= 0x5a;
        let mut replaced = DescriptorCrypto::with_default_limit().unwrap();
        replaced.enable_default_legacy_tea().unwrap();
        replaced.install_legacy_key(first).unwrap();
        let under_first = replaced.encrypt_output(b"record!!").unwrap();
        replaced.install_legacy_key(second).unwrap();
        let mut fresh = DescriptorCrypto::with_default_limit().unwrap();
        fresh.enable_default_legacy_tea().unwrap();
        fresh.install_legacy_key(second).unwrap();
        let under_second = fresh.encrypt_output(b"record!!").unwrap();
        assert_ne!(under_first, under_second);
        assert_eq!(replaced.encrypt_output(b"record!!").unwrap(), under_second);
    }

    #[test]
    fn enforces_plaintext_bound_after_releasing_a_pending_tail() {
        let mut crypto = DescriptorCrypto::new(15).unwrap();
        crypto.enable_default_legacy_tea().unwrap();
        assert_eq!(crypto.feed_input(&[0; 7]).unwrap(), 0);
        assert_eq!(crypto.pending_ciphertext_len(), 7);
        assert_eq!(crypto.feed_input(&[0; 8]).unwrap(), 8);
        assert_eq!(crypto.input_len(), 8);
        assert_eq!(crypto.pending_ciphertext_len(), 7);

        let before_input = crypto.input().to_vec();
        assert_eq!(
            crypto.feed_input(&[0]),
            Err(DescriptorCryptoError::InputLimitExceeded {
                attempted: 16,
                maximum: 15
            })
        );
        assert_eq!(crypto.input(), before_input.as_slice());
        assert_eq!(crypto.pending_ciphertext_len(), 7);
    }

    #[test]
    fn enforces_plaintext_bound_and_reports_invalid_constructor_limit() {
        let mut crypto = DescriptorCrypto::new(8).unwrap();
        crypto.feed_input(b"12345678").unwrap();
        assert_eq!(
            crypto.feed_input(b"9"),
            Err(DescriptorCryptoError::InputLimitExceeded {
                attempted: 9,
                maximum: 8
            })
        );
        assert_eq!(
            DescriptorCrypto::new(TEA_BLOCK_SIZE - 1),
            Err(DescriptorCryptoError::InvalidInputLimit {
                maximum: TEA_BLOCK_SIZE - 1
            })
        );
    }

    #[test]
    fn output_zero_pads_each_legacy_unit_for_boundary_lengths() {
        let mut crypto = DescriptorCrypto::with_default_limit().unwrap();
        crypto.enable_default_legacy_tea().unwrap();
        for length in [0, 1, 7, 8, 9] {
            let plaintext = vec![0x5a; length];
            let ciphertext = crypto.encrypt_output(&plaintext).unwrap();
            let expected_wire_length = length.div_ceil(TEA_BLOCK_SIZE) * TEA_BLOCK_SIZE;
            assert_eq!(ciphertext.len(), expected_wire_length);
            let decrypted = decrypt_aligned(&ciphertext, &LEGACY_DEFAULT_KEY).unwrap();
            assert_eq!(&decrypted[..length], plaintext.as_slice());
            assert!(decrypted[length..].iter().all(|byte| *byte == 0));
        }
    }

    #[test]
    fn finish_reports_only_an_incomplete_ciphertext_tail() {
        let mut crypto = DescriptorCrypto::with_default_limit().unwrap();
        let client = *b"tail-key-1234567";
        let keys = derive_legacy_descriptor_keys(client).unwrap();
        crypto.enable_default_legacy_tea().unwrap();
        crypto.install_legacy_key(client).unwrap();
        crypto.feed_input(&[1, 2, 3]).unwrap();
        assert!(matches!(
            crypto.finish_input(),
            Err(DescriptorCryptoError::TeaStream(
                TeaStreamError::IncompleteTail { pending: 3 }
            ))
        ));
        assert_eq!(crypto.pending_ciphertext_len(), 3);
        assert_eq!(keys.decryption_key(), client);
    }

    #[test]
    fn clear_returns_to_a_fresh_plaintext_boundary() {
        let mut crypto = DescriptorCrypto::with_default_limit().unwrap();
        crypto.enable_default_legacy_tea().unwrap();
        crypto.feed_input(&[0; TEA_BLOCK_SIZE]).unwrap();
        crypto.clear();
        assert_eq!(crypto.mode(), DescriptorCryptoMode::Plaintext);
        assert!(crypto.input().is_empty());
        assert!(crypto.keys().is_none());
        assert_eq!(crypto.pending_ciphertext_len(), 0);
    }
}
