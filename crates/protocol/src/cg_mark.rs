//! Transport-free codecs for three small fixed-width CG **login-phase** records.
//!
//! These are the fixed-size members of the guild mark and guild symbol family
//! that `CInputLogin` dispatches as a contiguous run of cases at
//! `input_login.cpp:1207`-`:1234`. They are grouped by family and by phase, not
//! by width, and the phase is the fact that matters most: all three are reached
//! before a character exists.
//!
//! | record | header | wire | payload | declaration | case |
//! |---|---|---|---|---|---|
//! | `TPacketCGMarkLogin` | 100 `0x64` | 9 | 8 | `packet.h:2231-2236` | `:1207` |
//! | `TPacketCGMarkIDXList` | 104 `0x68` | 1 | 0 | `packet.h:2244-2248` | `:1218` |
//! | `TPacketCGSymbolCRC` | 113 `0x71` | 13 | 12 | `packet.h:2282-2288` | `:1234` |
//!
//! Their registrations are `packet_info.cpp:101`, `:102`, and `:106`, all
//! unconditional and outside the single `#ifdef _IMPROVED_PACKET_ENCRYPTION_`
//! that guards the only conditional registration in the file
//! (`HEADER_CG_KEY_AGREEMENT` at `:191`). All three lie inside the
//! `#pragma pack(1)` that opens at `packet.h:274` and closes at `:3540`.
//!
//! # The rest of the family, and why it is not here
//!
//! Four further mark cases are adjacent in the same run and are deliberately out
//! of scope:
//!
//! - `TPacketCGMarkCRCList` (101, 322 bytes) and `TPacketCGMarkUpload` (102, 773
//!   bytes) are fixed but large, and each carries a bulk array
//!   (`DWORD crclist[80]` and `BYTE image[16*12*4]`).
//! - `HEADER_CG_GUILD_SYMBOL_UPLOAD` at `:1229` is **variable length**. It is
//!   the only case in the run that checks for failure and propagates it:
//!   `if ((iExtraLen = GuildSymbolUpload(d, c_pData, m_iBufferLeft)) < 0)
//!   return -1;`. It consumes the rest of the buffer, so it is a different shape
//!   class from every fixed sibling and needs its own section.
//! - `HEADER_CG_HACK` (105, 257 bytes) at `:1239` is dispatched to `break;`.
//!   It is a live no-op, not dead code, and must never be treated as
//!   implemented.
//!
//! # Three cross-tree facts that the widths alone do not tell you
//!
//! - **The enumerator for 113 is named differently in each tree.** The server
//!   calls it `HEADER_CG_SYMBOL_CRC`; the client calls the same value
//!   `HEADER_CG_GUILD_SYMBOL_CRC` at `client/Client/Packet.h:77` and sends it
//!   from `CGuildMarkDownloader::__SendSymbolCRCList` at
//!   `GuildMarkDownloader.cpp:483`. Searching the client for the server's name
//!   returns nothing at all. The value, the struct, and the layout agree; only
//!   the constant's name differs.
//! - **`TPacketCGSymbolCRC` field names have zero overlap between the trees.**
//!   The server declares `guild_id`, `crc`, `size` at `packet.h:2285`; the
//!   client declares `dwGuildID`, `dwCRC`, `dwSize` at `Packet.h:374`. The
//!   widths and order still match, so the wire bytes are identical, but any
//!   reading that matches the trees by field name fails on this record.
//! - **`TPacketCGMarkIDXList` and `TPacketCGMarkLogin` agree exactly**, member
//!   for member and name for name. That is recorded as a fact rather than
//!   assumed, because it is not the usual outcome in this repository.
//!
//! # Why every field here stays opaque
//!
//! `CGuildMarkDownloader::__SendSymbolCRCList` at `GuildMarkDownloader.cpp:478`
//! computes all three `SymbolCRC` values on the client, in a loop over its
//! guild-id vector, from a file on the client's own disk: `dwCRC` is
//! `GetFileCRC32` of that file and `dwSize` is `GetFileSize` of it. The server
//! receives a CRC, a size, and a guild id that the client asserts, with nothing
//! proving the client holds the image the guild actually owns. That is a trust
//! question for the guild-symbol service, not a framing question, so
//! [`CgSymbolCrc::guild_id`], [`CgSymbolCrc::crc`] and [`CgSymbolCrc::size`]
//! stay opaque little-endian `u32`s here.
//!
//! `CgMarkLogin` carries `handle` and `random_key`, the same two words that
//! terminate the 357-byte `GcLoginSuccess` record, so the DB can attribute the
//! mark to a session. They stay opaque too: this codec does not generate,
//! compare, or correlate them.
//!
//! `CgMarkIdxList` has no payload field at all. The one-byte record is still a
//! real registered record that the frame layer must validate, and the handler
//! resolves the whole mark set from the session.

use crate::cg_inventory::{
    CgHeader, HEADER_CG_MARK_CRCLIST, HEADER_CG_MARK_IDXLIST, HEADER_CG_MARK_LOGIN,
    HEADER_CG_MARK_UPLOAD, HEADER_CG_SYMBOL_CRC,
};
use crate::cg_wire::ClientFrame;

/// The full legacy `TPacketCGMarkLogin` record, header byte included.
pub const CG_MARK_LOGIN_WIRE_SIZE: usize = 9;
/// The framed payload of `TPacketCGMarkLogin`.
pub const CG_MARK_LOGIN_PAYLOAD_SIZE: usize = 8;
/// The full legacy `TPacketCGMarkIDXList` record. It is a header and nothing else.
pub const CG_MARK_IDXLIST_WIRE_SIZE: usize = 1;
/// The framed payload of `TPacketCGMarkIDXList`, which is empty by construction.
pub const CG_MARK_IDXLIST_PAYLOAD_SIZE: usize = 0;
/// The full legacy `TPacketCGSymbolCRC` record, header byte included.
pub const CG_SYMBOL_CRC_WIRE_SIZE: usize = 13;
/// The framed payload of `TPacketCGSymbolCRC`.
pub const CG_SYMBOL_CRC_PAYLOAD_SIZE: usize = 12;
/// The full legacy `TPacketCGMarkCRCList` record, header byte included.
pub const CG_MARK_CRCLIST_WIRE_SIZE: usize = 322;
/// The framed payload of `TPacketCGMarkCRCList`.
pub const CG_MARK_CRCLIST_PAYLOAD_SIZE: usize = 321;
/// The full legacy `TPacketCGMarkUpload` record, header byte included.
pub const CG_MARK_UPLOAD_WIRE_SIZE: usize = 773;
/// The framed payload of `TPacketCGMarkUpload`.
pub const CG_MARK_UPLOAD_PAYLOAD_SIZE: usize = 772;
/// The number of CRC words in `TPacketCGMarkCRCList::crclist`.
pub const CG_MARK_CRCLIST_WORDS: usize = 80;
/// The byte width of `TPacketCGMarkUpload::image`, which is `16 * 12 * 4`.
pub const CG_MARK_UPLOAD_IMAGE_SIZE: usize = 768;
/// The number of 32-bit pixels in `TPacketCGMarkUpload::image`, which is
/// `SGuildMark::SIZE` at `MarkImage.h`, namely `WIDTH * HEIGHT` = `16 * 12`.
pub const CG_MARK_UPLOAD_PIXELS: usize = 192;

/// Every way one of these three decoders can refuse a byte slice.
///
/// The three failure modes are identical across the three, so they share one
/// error type rather than three near-identical ones. `InvalidHeader` carries the
/// expected value so the message still names the right record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgMarkError {
    /// Fewer bytes than the fixed record needs.
    Truncated {
        /// The fixed width the decoder requires.
        needed: usize,
        /// How many bytes were actually offered.
        available: usize,
    },
    /// A complete-length slice that is not the fixed width.
    LengthMismatch {
        /// The one width this record has.
        expected: usize,
        /// The width that was offered.
        actual: usize,
    },
    /// The right number of bytes, but the header byte is not this record's.
    InvalidHeader {
        /// The header byte this record requires.
        expected: u8,
        /// The header byte that was actually present.
        actual: u8,
    },
}

impl std::fmt::Display for CgMarkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(f, "mark record needs {needed} bytes, got {available}")
            }
            Self::LengthMismatch { expected, actual } => {
                write!(
                    f,
                    "mark record must be exactly {expected} bytes, got {actual}"
                )
            }
            Self::InvalidHeader { expected, actual } => {
                write!(f, "mark header {actual} is not {expected}")
            }
        }
    }
}

impl std::error::Error for CgMarkError {}

/// Read one little-endian `u32`. Explicit, never a packed struct.
fn read_u32_le(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Append one little-endian `u32`.
fn write_u32_le(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Check a complete record width, then its header.
fn decode_parts(bytes: &[u8], wire_size: usize, expected: u8) -> Result<(), CgMarkError> {
    check_exact(bytes.len(), wire_size)?;
    check_header(bytes[0], expected)
}

/// Check a header-less frame payload width, then its header.
fn decode_frame_parts(
    frame: &ClientFrame,
    payload_size: usize,
    expected: u8,
) -> Result<(), CgMarkError> {
    check_exact(frame.payload.len(), payload_size)?;
    check_header(frame.header, expected)
}

fn check_exact(len: usize, size: usize) -> Result<(), CgMarkError> {
    if len < size {
        return Err(CgMarkError::Truncated {
            needed: size,
            available: len,
        });
    }
    if len > size {
        return Err(CgMarkError::LengthMismatch {
            expected: size,
            actual: len,
        });
    }
    Ok(())
}

fn check_header(actual: u8, expected: u8) -> Result<(), CgMarkError> {
    if actual != expected {
        return Err(CgMarkError::InvalidHeader { expected, actual });
    }
    Ok(())
}

/// The transport-free `TPacketCGMarkLogin` record, header 100.
///
/// Both words are opaque. The client fills them in
/// `CGuildMarkDownloader` and `CGuildMarkUploader`; nothing in this codec
/// validates a handle, and nothing here ties the pair to a login session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgMarkLogin {
    /// The account handle. Opaque little-endian `u32`.
    pub handle: u32,
    /// The random key issued with the login. Opaque little-endian `u32`.
    pub random_key: u32,
}

impl CgMarkLogin {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> CgHeader {
        HEADER_CG_MARK_LOGIN
    }

    /// Build the record from its two opaque words.
    pub const fn new(handle: u32, random_key: u32) -> Self {
        Self { handle, random_key }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        write_u32_le(out, self.handle);
        write_u32_le(out, self.random_key);
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_MARK_LOGIN_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame, header byte and payload.
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_MARK_LOGIN_PAYLOAD_SIZE);
        write_u32_le(&mut payload, self.handle);
        write_u32_le(&mut payload, self.random_key);
        ClientFrame::new(Self::header().value(), payload)
    }

    /// # Errors
    ///
    /// Returns [`CgMarkError::Truncated`] for a short input,
    /// [`CgMarkError::LengthMismatch`] for a long input, and
    /// [`CgMarkError::InvalidHeader`] when the header is not 100. The length is
    /// checked before the header, and the header before any payload byte.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgMarkError> {
        decode_parts(bytes, CG_MARK_LOGIN_WIRE_SIZE, Self::header().value())?;
        Ok(Self {
            handle: read_u32_le(bytes, 1),
            random_key: read_u32_le(bytes, 5),
        })
    }

    /// # Errors
    ///
    /// As [`CgMarkLogin::decode`], except that the **payload** width is checked
    /// before the frame header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgMarkError> {
        decode_frame_parts(frame, CG_MARK_LOGIN_PAYLOAD_SIZE, Self::header().value())?;
        Ok(Self {
            handle: read_u32_le(&frame.payload, 0),
            random_key: read_u32_le(&frame.payload, 4),
        })
    }
}

/// The transport-free `TPacketCGMarkIDXList` record, header 104.
///
/// The record has no payload field: `packet.h:2244-2248` declares only
/// `BYTE header;`, and the handler `GuildMarkIDXList` resolves the requested
/// index list from the session rather than from the wire. The one-byte record is
/// still registered at `packet_info.cpp:102`, so the frame layer must validate
/// it, and that is the whole of this record's job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgMarkIdxList;

impl CgMarkIdxList {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> CgHeader {
        HEADER_CG_MARK_IDXLIST
    }

    /// Build the record. It carries no field.
    pub const fn new() -> Self {
        Self
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_MARK_IDXLIST_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame, which has an empty payload.
    pub fn to_frame(&self) -> ClientFrame {
        ClientFrame::new(Self::header().value(), [0_u8; 0])
    }

    /// # Errors
    ///
    /// Returns [`CgMarkError::Truncated`] for an empty input,
    /// [`CgMarkError::LengthMismatch`] for an input longer than one byte, and
    /// [`CgMarkError::InvalidHeader`] when the header is not 104.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgMarkError> {
        decode_parts(bytes, CG_MARK_IDXLIST_WIRE_SIZE, Self::header().value())?;
        Ok(Self)
    }

    /// # Errors
    ///
    /// As [`CgMarkIdxList::decode`], except that the frame payload must be
    /// **empty**. A one-byte payload is a [`CgMarkError::LengthMismatch`], not an
    /// accepted record, because the handler reads nothing and the legacy sender
    /// sends nothing.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgMarkError> {
        decode_frame_parts(frame, CG_MARK_IDXLIST_PAYLOAD_SIZE, Self::header().value())?;
        Ok(Self)
    }
}

/// The transport-free `TPacketCGSymbolCRC` record, header 113.
///
/// All three words are opaque, and the client's own sender computes every one of
/// them from a file on the client's disk. See the module documentation for why
/// that trust question is deliberately left open here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgSymbolCrc {
    /// The guild the client claims to hold a symbol for. Opaque `u32`.
    pub guild_id: u32,
    /// The client's own CRC of its local symbol file. Opaque `u32`.
    pub crc: u32,
    /// The client's own size of that file. Opaque `u32`.
    pub size: u32,
}

impl CgSymbolCrc {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> CgHeader {
        HEADER_CG_SYMBOL_CRC
    }

    /// Build the record from its three opaque words.
    pub const fn new(guild_id: u32, crc: u32, size: u32) -> Self {
        Self {
            guild_id,
            crc,
            size,
        }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        write_u32_le(out, self.guild_id);
        write_u32_le(out, self.crc);
        write_u32_le(out, self.size);
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_SYMBOL_CRC_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame, header byte and payload.
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_SYMBOL_CRC_PAYLOAD_SIZE);
        write_u32_le(&mut payload, self.guild_id);
        write_u32_le(&mut payload, self.crc);
        write_u32_le(&mut payload, self.size);
        ClientFrame::new(Self::header().value(), payload)
    }

    /// # Errors
    ///
    /// Returns [`CgMarkError::Truncated`] for a short input,
    /// [`CgMarkError::LengthMismatch`] for a long input, and
    /// [`CgMarkError::InvalidHeader`] when the header is not 113.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgMarkError> {
        decode_parts(bytes, CG_SYMBOL_CRC_WIRE_SIZE, Self::header().value())?;
        Ok(Self {
            guild_id: read_u32_le(bytes, 1),
            crc: read_u32_le(bytes, 5),
            size: read_u32_le(bytes, 9),
        })
    }

    /// # Errors
    ///
    /// As [`CgSymbolCrc::decode`], except that the **payload** width is checked
    /// before the frame header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgMarkError> {
        decode_frame_parts(frame, CG_SYMBOL_CRC_PAYLOAD_SIZE, Self::header().value())?;
        Ok(Self {
            guild_id: read_u32_le(&frame.payload, 0),
            crc: read_u32_le(&frame.payload, 4),
            size: read_u32_le(&frame.payload, 8),
        })
    }
}
/// The login-phase request for the mark blocks the client does not hold yet.
///
/// `CInputLogin::GuildMarkCRCList` at `input_login.cpp:1123` hands the 80 words
/// to `CGuildMarkManager::GetDiffBlocks` and replies with only the differing
/// blocks, so the 80 words are a **cache request key**, not a payload the
/// server stores. The codec keeps all 80 words opaque and preserves their
/// order; which blocks differ is mark-manager policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgMarkCRCList {
    /// The opaque image-slot index the client is asking about.
    pub img_idx: u8,
    /// The 80 opaque little-endian CRC words, in wire order.
    pub crc_list: [u32; CG_MARK_CRCLIST_WORDS],
}

impl CgMarkCRCList {
    /// The fixed header byte.
    pub const fn header() -> CgHeader {
        HEADER_CG_MARK_CRCLIST
    }

    /// Build the record from the image index and its 80 CRC words.
    #[must_use]
    pub const fn new(img_idx: u8, crc_list: [u32; CG_MARK_CRCLIST_WORDS]) -> Self {
        Self { img_idx, crc_list }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.push(self.img_idx);
        for word in &self.crc_list {
            write_u32_le(out, *word);
        }
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_MARK_CRCLIST_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame, header byte and payload.
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_MARK_CRCLIST_PAYLOAD_SIZE);
        payload.push(self.img_idx);
        for word in &self.crc_list {
            write_u32_le(&mut payload, *word);
        }
        ClientFrame::new(Self::header().value(), payload)
    }

    /// # Errors
    ///
    /// Returns [`CgMarkError::Truncated`] for a short input,
    /// [`CgMarkError::LengthMismatch`] for a long input, and
    /// [`CgMarkError::InvalidHeader`] when the header is not 101.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgMarkError> {
        decode_parts(bytes, CG_MARK_CRCLIST_WIRE_SIZE, Self::header().value())?;
        let mut crc_list = [0_u32; CG_MARK_CRCLIST_WORDS];
        for (index, slot) in crc_list.iter_mut().enumerate() {
            *slot = read_u32_le(bytes, 2 + index * 4);
        }
        Ok(Self {
            img_idx: bytes[1],
            crc_list,
        })
    }

    /// # Errors
    ///
    /// As [`CgMarkCRCList::decode`], except that the frame payload must be
    /// exactly 321 bytes.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgMarkError> {
        decode_frame_parts(frame, CG_MARK_CRCLIST_PAYLOAD_SIZE, Self::header().value())?;
        let mut crc_list = [0_u32; CG_MARK_CRCLIST_WORDS];
        for (index, slot) in crc_list.iter_mut().enumerate() {
            *slot = read_u32_le(&frame.payload, 1 + index * 4);
        }
        Ok(Self {
            img_idx: frame.payload[0],
            crc_list,
        })
    }
}

/// The login-phase upload of one 16x12 guild mark image.
///
/// The `image` field is **768 opaque bytes** and the codec keeps it that way.
/// The legacy handler at `input_login.cpp:1059` nonetheless reads it two
/// different ways, and both are load-bearing:
///
/// - the emptiness test reinterprets the field as
///   `CG_MARK_UPLOAD_PIXELS` **little-endian `u32` pixels**, because
///   `SGuildMark::SIZE` is `WIDTH * HEIGHT` = 192 and the loop steps a
///   `DWORD *` across it;
/// - `CGuildMarkManager::SaveMark(DWORD guildID, BYTE * pbMarkImage)` at
///   `MarkManager.h:49` takes the very same storage as `BYTE *`.
///
/// "Every word is zero" is mark-manager policy and is **not** encoded here. What
/// is documented is the grouping, because a future pixel-level implementation
/// needs to know that byte order and cannot recover it from the field name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgMarkUpload {
    /// The opaque guild id the image belongs to.
    pub gid: u32,
    /// The 768 opaque image bytes, in wire order.
    pub image: [u8; CG_MARK_UPLOAD_IMAGE_SIZE],
}

impl CgMarkUpload {
    /// The fixed header byte.
    pub const fn header() -> CgHeader {
        HEADER_CG_MARK_UPLOAD
    }

    /// Build the record from the guild id and its 768 image bytes.
    #[must_use]
    pub const fn new(gid: u32, image: [u8; CG_MARK_UPLOAD_IMAGE_SIZE]) -> Self {
        Self { gid, image }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        write_u32_le(out, self.gid);
        out.extend_from_slice(&self.image);
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_MARK_UPLOAD_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame, header byte and payload.
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_MARK_UPLOAD_PAYLOAD_SIZE);
        write_u32_le(&mut payload, self.gid);
        payload.extend_from_slice(&self.image);
        ClientFrame::new(Self::header().value(), payload)
    }

    /// # Errors
    ///
    /// Returns [`CgMarkError::Truncated`] for a short input,
    /// [`CgMarkError::LengthMismatch`] for a long input, and
    /// [`CgMarkError::InvalidHeader`] when the header is not 102.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgMarkError> {
        decode_parts(bytes, CG_MARK_UPLOAD_WIRE_SIZE, Self::header().value())?;
        let mut image = [0_u8; CG_MARK_UPLOAD_IMAGE_SIZE];
        image.copy_from_slice(&bytes[5..]);
        Ok(Self {
            gid: read_u32_le(bytes, 1),
            image,
        })
    }

    /// # Errors
    ///
    /// As [`CgMarkUpload::decode`], except that the frame payload must be
    /// exactly 772 bytes.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgMarkError> {
        decode_frame_parts(frame, CG_MARK_UPLOAD_PAYLOAD_SIZE, Self::header().value())?;
        let mut image = [0_u8; CG_MARK_UPLOAD_IMAGE_SIZE];
        image.copy_from_slice(&frame.payload[4..]);
        Ok(Self {
            gid: read_u32_le(&frame.payload, 0),
            image,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// One record's identity and encodings, so the shared assertions can walk
    /// all three without repeating the dispatch three times.
    struct Sample {
        name: &'static str,
        header: CgHeader,
        /// The encoding with every field zero. For the fieldless record this is
        /// the whole one-byte record.
        low: Vec<u8>,
        /// The encoding with every field at its maximum.
        high: Vec<u8>,
        wire_size: usize,
        payload_size: usize,
    }

    impl Sample {
        /// Decode `bytes` with whichever decoder matches this record's name.
        fn decode(&self, bytes: &[u8]) -> Result<Decoded, CgMarkError> {
            match self.name {
                "CgMarkLogin" => CgMarkLogin::decode(bytes).map(Decoded::Login),
                "CgMarkIdxList" => CgMarkIdxList::decode(bytes).map(|_| Decoded::IdxList),
                _ => CgSymbolCrc::decode(bytes).map(Decoded::SymbolCrc),
            }
        }

        /// The frame projection of the all-zero record.
        fn zero_frame(&self) -> ClientFrame {
            match self.name {
                "CgMarkLogin" => CgMarkLogin::new(0, 0).to_frame(),
                "CgMarkIdxList" => CgMarkIdxList::new().to_frame(),
                _ => CgSymbolCrc::new(0, 0, 0).to_frame(),
            }
        }
    }

    /// The decoded value of any one of the three, so `decode` above can return a
    /// single error type without losing which record was checked.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Decoded {
        Login(CgMarkLogin),
        IdxList,
        SymbolCrc(CgSymbolCrc),
    }

    fn samples() -> Vec<Sample> {
        vec![
            Sample {
                name: "CgMarkLogin",
                header: HEADER_CG_MARK_LOGIN,
                low: CgMarkLogin::new(0, 0).encode(),
                high: CgMarkLogin::new(u32::MAX, u32::MAX).encode(),
                wire_size: CG_MARK_LOGIN_WIRE_SIZE,
                payload_size: CG_MARK_LOGIN_PAYLOAD_SIZE,
            },
            Sample {
                name: "CgMarkIdxList",
                header: HEADER_CG_MARK_IDXLIST,
                low: CgMarkIdxList::new().encode(),
                high: CgMarkIdxList::new().encode(),
                wire_size: CG_MARK_IDXLIST_WIRE_SIZE,
                payload_size: CG_MARK_IDXLIST_PAYLOAD_SIZE,
            },
            Sample {
                name: "CgSymbolCrc",
                header: HEADER_CG_SYMBOL_CRC,
                low: CgSymbolCrc::new(0, 0, 0).encode(),
                high: CgSymbolCrc::new(u32::MAX, u32::MAX, u32::MAX).encode(),
                wire_size: CG_SYMBOL_CRC_WIRE_SIZE,
                payload_size: CG_SYMBOL_CRC_PAYLOAD_SIZE,
            },
        ]
    }

    /// Byte patterns that must survive a round trip untouched, so "opaque" is
    /// shown to mean all 256 byte positions rather than "not obviously signed".
    fn edge_words() -> Vec<u32> {
        vec![
            0,
            1,
            0x0000_00FF,
            0x0000_FF00,
            0x00FF_0000,
            0xFF00_0000,
            0x7FFF_FFFF,
            0x8000_0000,
            0xFFFF_FFFE,
            u32::MAX,
        ]
    }

    #[test]
    fn golden_bytes_match_the_legacy_layout() {
        assert_eq!(CgMarkIdxList::new().encode(), vec![0x68]);
        assert_eq!(
            CgMarkLogin::new(0, 0).encode(),
            vec![0x64, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            CgMarkLogin::new(0x1122_3344, 0xAABB_CCDD).encode(),
            vec![0x64, 0x44, 0x33, 0x22, 0x11, 0xDD, 0xCC, 0xBB, 0xAA]
        );
        assert_eq!(
            CgSymbolCrc::new(0, 0, 0).encode(),
            vec![0x71, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            CgSymbolCrc::new(1, 2, 3).encode(),
            vec![0x71, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0]
        );
    }

    #[test]
    fn the_declared_widths_are_the_registered_widths() {
        assert_eq!(CG_MARK_LOGIN_WIRE_SIZE, 9);
        assert_eq!(CG_MARK_LOGIN_PAYLOAD_SIZE, 8);
        assert_eq!(CG_MARK_IDXLIST_WIRE_SIZE, 1);
        assert_eq!(CG_MARK_IDXLIST_PAYLOAD_SIZE, 0);
        assert_eq!(CG_SYMBOL_CRC_WIRE_SIZE, 13);
        assert_eq!(CG_SYMBOL_CRC_PAYLOAD_SIZE, 12);
        for s in samples() {
            assert_eq!(s.low.len(), s.wire_size, "{} low width", s.name);
            assert_eq!(s.high.len(), s.wire_size, "{} high width", s.name);
            assert_eq!(s.wire_size, s.payload_size + 1, "{} header byte", s.name);
            assert_eq!(s.low[0], s.header.value(), "{} header", s.name);
        }
    }

    #[test]
    fn the_three_headers_are_distinct_and_are_the_inventory_values() {
        let values: BTreeSet<u8> = samples().iter().map(|s| s.header.value()).collect();
        assert_eq!(values.len(), 3, "the three headers must not alias");
        assert_eq!(CgMarkLogin::header().value(), 100);
        assert_eq!(CgMarkIdxList::header().value(), 104);
        assert_eq!(CgSymbolCrc::header().value(), 113);
    }

    #[test]
    fn every_record_round_trips_its_own_zero_and_maximum_encodings() {
        for s in samples() {
            for bytes in [&s.low, &s.high] {
                assert!(
                    s.decode(bytes).is_ok(),
                    "{} should accept its own bytes",
                    s.name
                );
            }
        }
        assert_eq!(
            CgMarkLogin::decode(&CgMarkLogin::new(u32::MAX, u32::MAX).encode()).unwrap(),
            CgMarkLogin::new(u32::MAX, u32::MAX)
        );
        assert_eq!(
            CgSymbolCrc::decode(&CgSymbolCrc::new(1, 2, 3).encode()).unwrap(),
            CgSymbolCrc::new(1, 2, 3)
        );
    }

    #[test]
    fn every_opaque_word_value_round_trips() {
        for w in edge_words() {
            assert_eq!(
                CgMarkLogin::decode(&CgMarkLogin::new(w, w).encode()).unwrap(),
                CgMarkLogin::new(w, w)
            );
            assert_eq!(
                CgSymbolCrc::decode(&CgSymbolCrc::new(w, w, w).encode()).unwrap(),
                CgSymbolCrc::new(w, w, w)
            );
        }
    }

    #[test]
    fn every_byte_value_in_every_word_position_round_trips() {
        for b in 0..=u8::MAX {
            let w = u32::from(b);
            assert_eq!(
                CgMarkLogin::decode(&CgMarkLogin::new(w, w).encode()).unwrap(),
                CgMarkLogin::new(w, w),
                "byte {b} must survive both words"
            );
            assert_eq!(
                CgSymbolCrc::decode(&CgSymbolCrc::new(w, w, w).encode()).unwrap(),
                CgSymbolCrc::new(w, w, w),
                "byte {b} must survive all three words"
            );
        }
    }

    #[test]
    fn words_are_little_endian_and_never_swapped() {
        let bytes = CgMarkLogin::new(0x0102_0304, 0x0506_0708).encode();
        assert_eq!(&bytes[1..5], &[0x04, 0x03, 0x02, 0x01]);
        assert_eq!(&bytes[5..9], &[0x08, 0x07, 0x06, 0x05]);
        let crc = CgSymbolCrc::new(0x0102_0304, 0x0506_0708, 0x090A_0B0C).encode();
        assert_eq!(&crc[1..5], &[0x04, 0x03, 0x02, 0x01]);
        assert_eq!(&crc[5..9], &[0x08, 0x07, 0x06, 0x05]);
        assert_eq!(&crc[9..13], &[0x0C, 0x0B, 0x0A, 0x09]);
    }

    #[test]
    fn the_two_words_of_mark_login_are_not_interchangeable() {
        let a = CgMarkLogin::new(0x1111_1111, 0x2222_2222);
        let b = CgMarkLogin::new(0x2222_2222, 0x1111_1111);
        assert_ne!(a, b, "handle and random_key must not be the same field");
        assert_ne!(a.encode(), b.encode());
        assert_eq!(CgMarkLogin::decode(&a.encode()).unwrap(), a);
        assert_eq!(CgMarkLogin::decode(&b.encode()).unwrap(), b);
    }

    #[test]
    fn every_encode_into_appends_and_none_of_them_clears() {
        let mut out = vec![0xAA, 0xBB];
        let before = out.len();
        CgMarkLogin::new(1, 2).encode_into(&mut out);
        assert_eq!(out.len(), before + CG_MARK_LOGIN_WIRE_SIZE);
        assert_eq!(&out[..2], &[0xAA, 0xBB], "the prefix must survive");
        CgMarkIdxList::new().encode_into(&mut out);
        assert_eq!(out.len(), before + CG_MARK_LOGIN_WIRE_SIZE + 1);
        assert_eq!(&out[..2], &[0xAA, 0xBB], "the prefix must still survive");
        CgSymbolCrc::new(1, 2, 3).encode_into(&mut out);
        assert_eq!(
            out.len(),
            before + CG_MARK_LOGIN_WIRE_SIZE + 1 + CG_SYMBOL_CRC_WIRE_SIZE
        );
        assert_eq!(
            &out[..2],
            &[0xAA, 0xBB],
            "the prefix must survive all three"
        );
    }

    #[test]
    fn encode_into_into_an_empty_buffer_equals_encode() {
        for s in samples() {
            let mut out = Vec::new();
            let expected = match s.name {
                "CgMarkLogin" => CgMarkLogin::new(0, 0).encode(),
                "CgMarkIdxList" => CgMarkIdxList::new().encode(),
                _ => CgSymbolCrc::new(0, 0, 0).encode(),
            };
            match s.name {
                "CgMarkLogin" => CgMarkLogin::new(0, 0).encode_into(&mut out),
                "CgMarkIdxList" => CgMarkIdxList::new().encode_into(&mut out),
                _ => CgSymbolCrc::new(0, 0, 0).encode_into(&mut out),
            }
            assert_eq!(out, expected, "{} into an empty buffer", s.name);
            assert_eq!(out, s.low, "{} must match the sample bytes", s.name);
        }
    }

    #[test]
    fn every_record_rejects_every_other_header_at_its_own_width() {
        for s in samples() {
            for h in 0..=u8::MAX {
                let mut bytes = s.low.clone();
                bytes[0] = h;
                match s.decode(&bytes) {
                    Ok(_) if h == s.header.value() => {}
                    Ok(_) => panic!("{} accepted foreign header {h}", s.name),
                    Err(CgMarkError::InvalidHeader { expected, actual }) => {
                        assert_eq!(expected, s.header.value(), "{} expected", s.name);
                        assert_eq!(actual, h, "{} actual", s.name);
                    }
                    Err(e) => panic!("{} gave {e:?} for header {h}, want InvalidHeader", s.name),
                }
            }
        }
    }

    #[test]
    fn a_complete_width_with_a_foreign_header_is_a_header_error_not_a_length_error() {
        assert_eq!(
            CgMarkIdxList::decode(&[0x64]),
            Err(CgMarkError::InvalidHeader {
                expected: 104,
                actual: 100
            })
        );
        // The last byte is the most significant one, so this pins the order.
        assert_eq!(
            CgMarkLogin::decode(&[0x64, 0, 0, 0, 0, 0, 0, 0, 1]),
            Ok(CgMarkLogin::new(0, 0x0100_0000))
        );
    }

    #[test]
    fn short_inputs_are_truncated_and_long_inputs_are_a_length_mismatch() {
        for s in samples() {
            for n in 0..s.wire_size {
                let err = s.decode(&s.low[..n]).unwrap_err();
                assert_eq!(
                    err,
                    CgMarkError::Truncated {
                        needed: s.wire_size,
                        available: n
                    },
                    "{} truncated at {n}",
                    s.name
                );
            }
            for extra in [1_usize, 2, 7] {
                let mut bytes = s.low.clone();
                bytes.extend(std::iter::repeat_n(s.header.value(), extra));
                assert_eq!(
                    s.decode(&bytes).unwrap_err(),
                    CgMarkError::LengthMismatch {
                        expected: s.wire_size,
                        actual: s.wire_size + extra
                    },
                    "{} overlong by {extra}",
                    s.name
                );
            }
        }
    }

    #[test]
    fn length_is_checked_before_the_header() {
        // Two bytes: the header is already wrong, so a length-first check must
        // be what produces the error rather than InvalidHeader.
        // Two bytes is short for the 9-byte login record and long for the
        // 1-byte index list, so the same slice yields two different variants
        // purely from the width.
        assert_eq!(
            CgMarkLogin::decode(&[0x00, 0x00]),
            Err(CgMarkError::Truncated {
                needed: 9,
                available: 2
            })
        );
        assert_eq!(
            CgMarkIdxList::decode(&[0x00, 0x00]),
            Err(CgMarkError::LengthMismatch {
                expected: 1,
                actual: 2
            })
        );
        assert_eq!(
            CgSymbolCrc::decode(&[0x00, 0x00]),
            Err(CgMarkError::Truncated {
                needed: 13,
                available: 2
            })
        );
    }

    #[test]
    fn to_frame_carries_exactly_the_payload_width_and_no_header() {
        for s in samples() {
            let frame = s.zero_frame();
            assert_eq!(frame.header, s.header.value(), "{} frame header", s.name);
            assert_eq!(
                frame.payload.len(),
                s.payload_size,
                "{} frame payload width",
                s.name
            );
        }
        let idx_frame = CgMarkIdxList::new().to_frame();
        assert!(
            idx_frame.payload.is_empty(),
            "the index list is header-only"
        );
    }

    #[test]
    fn decode_frame_agrees_with_decode_for_all_three() {
        for s in samples() {
            assert!(matches!(s.zero_frame().header, _));
            let frame = s.zero_frame();
            let accepted = match s.name {
                "CgMarkLogin" => CgMarkLogin::decode_frame(&frame).map(Decoded::Login),
                "CgMarkIdxList" => CgMarkIdxList::decode_frame(&frame).map(|_| Decoded::IdxList),
                _ => CgSymbolCrc::decode_frame(&frame).map(Decoded::SymbolCrc),
            };
            assert_eq!(accepted, s.decode(&s.low), "{} frame vs slice", s.name);
        }
    }

    #[test]
    fn decode_frame_checks_the_payload_width_and_not_the_record_width() {
        for s in samples() {
            let mut frame = s.zero_frame();
            frame.payload.push(0x00);
            let err = match s.name {
                "CgMarkLogin" => CgMarkLogin::decode_frame(&frame).err(),
                "CgMarkIdxList" => CgMarkIdxList::decode_frame(&frame).err(),
                _ => CgSymbolCrc::decode_frame(&frame).err(),
            };
            assert_eq!(
                err,
                Some(CgMarkError::LengthMismatch {
                    expected: s.payload_size,
                    actual: s.payload_size + 1
                }),
                "{} must reject a one-byte-longer payload",
                s.name
            );
        }
    }

    #[test]
    fn decode_frame_rejects_a_short_payload_before_reading_the_header() {
        for s in samples() {
            if s.payload_size == 0 {
                continue;
            }
            let mut frame = s.zero_frame();
            frame.payload.truncate(s.payload_size - 1);
            let err = match s.name {
                "CgMarkLogin" => CgMarkLogin::decode_frame(&frame).err(),
                _ => CgSymbolCrc::decode_frame(&frame).err(),
            };
            assert_eq!(
                err,
                Some(CgMarkError::Truncated {
                    needed: s.payload_size,
                    available: s.payload_size - 1
                }),
                "{} must reject a short payload",
                s.name
            );
        }
    }

    #[test]
    fn the_index_list_frame_must_be_header_only() {
        // The shared-frame equivalent of CgWarp's empty payload: a payload here
        // is not a shorter record, it is a different one.
        let frame = ClientFrame::new(CgMarkIdxList::header().value(), vec![0x00]);
        assert_eq!(
            CgMarkIdxList::decode_frame(&frame),
            Err(CgMarkError::LengthMismatch {
                expected: 0,
                actual: 1
            })
        );
    }

    #[test]
    fn a_frame_with_a_foreign_header_is_rejected_at_the_right_payload_width() {
        let mut frame = CgSymbolCrc::new(1, 2, 3).to_frame();
        frame.header = HEADER_CG_MARK_LOGIN.value();
        assert_eq!(
            CgSymbolCrc::decode_frame(&frame),
            Err(CgMarkError::InvalidHeader {
                expected: 113,
                actual: 100
            })
        );
    }

    #[test]
    fn cross_decoding_is_always_a_width_failure_because_the_widths_differ() {
        // The three widths are 1, 9 and 13, all distinct, and the length check
        // runs before the header check. So no cross-decoding among these three
        // can ever reach `InvalidHeader`: the width always decides first. That
        // is worth pinning, because a reader who assumed the header would be
        // reported would otherwise "fix" the order and lose this property.
        let encoded = [
            CgMarkLogin::new(1, 2).encode(),
            CgMarkIdxList::new().encode(),
            CgSymbolCrc::new(1, 2, 3).encode(),
        ];
        let expected: Vec<CgMarkError> = vec![
            // login (9) given the 1-byte index list
            CgMarkError::Truncated {
                needed: 9,
                available: 1,
            },
            // login (9) given the 13-byte symbol CRC
            CgMarkError::LengthMismatch {
                expected: 9,
                actual: 13,
            },
            // index list (1) given the 9-byte login
            CgMarkError::LengthMismatch {
                expected: 1,
                actual: 9,
            },
            // index list (1) given the 13-byte symbol CRC
            CgMarkError::LengthMismatch {
                expected: 1,
                actual: 13,
            },
            // symbol CRC (13) given the 9-byte login
            CgMarkError::Truncated {
                needed: 13,
                available: 9,
            },
            // symbol CRC (13) given the 1-byte index list
            CgMarkError::Truncated {
                needed: 13,
                available: 1,
            },
        ];
        let mut got = Vec::new();
        for row in 0..3 {
            for (col, foreign) in encoded.iter().enumerate() {
                if row == col {
                    continue;
                }
                let decoded = match row {
                    0 => CgMarkLogin::decode(foreign).err(),
                    1 => CgMarkIdxList::decode(foreign).err(),
                    _ => CgSymbolCrc::decode(foreign).err(),
                };
                got.push(decoded.expect("a foreign record must not decode"));
            }
        }
        assert_eq!(got.len(), 6);
        assert_eq!(got, expected, "cross-decoding must name the width");
        assert!(
            !got.iter()
                .any(|e| matches!(e, CgMarkError::InvalidHeader { .. })),
            "distinct widths mean the header is never the thing that fails"
        );
    }

    #[test]
    fn a_foreign_header_is_reported_when_the_widths_happen_to_agree() {
        // The complement of the test above: pad the 1-byte index list to the
        // 9-byte login width and the header error finally surfaces, which shows
        // the width check really is what was hiding it.
        assert_eq!(
            CgMarkLogin::decode(&[0x68, 0, 0, 0, 0, 0, 0, 0, 0]),
            Err(CgMarkError::InvalidHeader {
                expected: 100,
                actual: 104
            })
        );
        assert_eq!(
            CgSymbolCrc::decode(&[0x64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            Err(CgMarkError::InvalidHeader {
                expected: 113,
                actual: 100
            })
        );
    }

    #[test]
    fn frame_and_slice_encodings_agree_byte_for_byte() {
        let login = CgMarkLogin::new(0xDEAD_BEEF, 0x0BAD_F00D);
        let mut rebuilt = login.to_frame().payload.clone();
        rebuilt.insert(0, CgMarkLogin::header().value());
        assert_eq!(rebuilt, login.encode());
        let crc = CgSymbolCrc::new(0xFEED_FACE, 0x0BAD_F00D, 0x1234_5678);
        let mut rebuilt = crc.to_frame().payload.clone();
        rebuilt.insert(0, CgSymbolCrc::header().value());
        assert_eq!(rebuilt, crc.encode());
        let idx = CgMarkIdxList::new();
        let mut rebuilt = idx.to_frame().payload.clone();
        rebuilt.insert(0, CgMarkIdxList::header().value());
        assert_eq!(rebuilt, idx.encode());
    }

    #[test]
    fn the_records_are_plain_data_and_compare_by_value() {
        let a = CgMarkLogin::new(1, 2);
        let mut b = a;
        b.random_key = 3;
        assert_ne!(a, b);
        assert_eq!(a, a);
        assert_eq!(CgMarkIdxList::new(), CgMarkIdxList);
        assert_eq!(CgSymbolCrc::default(), CgSymbolCrc::new(0, 0, 0));
    }

    #[test]
    fn the_error_messages_name_the_right_widths_and_headers() {
        let e = CgMarkError::Truncated {
            needed: 9,
            available: 4,
        };
        assert_eq!(e.to_string(), "mark record needs 9 bytes, got 4");
        let e = CgMarkError::LengthMismatch {
            expected: 13,
            actual: 12,
        };
        assert_eq!(
            e.to_string(),
            "mark record must be exactly 13 bytes, got 12"
        );
        let e = CgMarkError::InvalidHeader {
            expected: 104,
            actual: 100,
        };
        assert_eq!(e.to_string(), "mark header 100 is not 104");
        let _: &dyn std::error::Error = &e;
    }

    #[test]
    fn decode_never_panics_on_arbitrary_slices() {
        // A cheap fuzz: every length from 0 to the widest record, with a header
        // byte that is never valid, must return rather than index out of range.
        let poison = [0x00_u8; CG_SYMBOL_CRC_WIRE_SIZE];
        for n in 0..=CG_SYMBOL_CRC_WIRE_SIZE {
            let slice = &poison[..n];
            let _ = CgMarkLogin::decode(slice);
            let _ = CgMarkIdxList::decode(slice);
            let _ = CgSymbolCrc::decode(slice);
        }
    }

    #[test]
    fn a_zero_length_frame_payload_is_only_valid_for_the_index_list() {
        assert!(CgMarkIdxList::decode_frame(&ClientFrame::new(
            CgMarkIdxList::header().value(),
            Vec::new()
        ))
        .is_ok());
        assert_eq!(
            CgMarkLogin::decode_frame(&ClientFrame::new(CgMarkLogin::header().value(), Vec::new())),
            Err(CgMarkError::Truncated {
                needed: 8,
                available: 0
            })
        );
        assert_eq!(
            CgSymbolCrc::decode_frame(&ClientFrame::new(CgSymbolCrc::header().value(), Vec::new())),
            Err(CgMarkError::Truncated {
                needed: 12,
                available: 0
            })
        );
    }

    // ---- CgMarkCRCList -----------------------------------------------

    fn sample_crc_list() -> [u32; CG_MARK_CRCLIST_WORDS] {
        let mut words = [0_u32; CG_MARK_CRCLIST_WORDS];
        for (index, slot) in words.iter_mut().enumerate() {
            *slot = 0xA5A5_0000 | u32::try_from(index).expect("index fits in u32");
        }
        words
    }

    #[test]
    fn crclist_header_is_101() {
        assert_eq!(CgMarkCRCList::header().value(), 0x65);
    }

    #[test]
    fn crclist_sizes_are_322_and_321() {
        assert_eq!(CG_MARK_CRCLIST_WIRE_SIZE, 322);
        assert_eq!(CG_MARK_CRCLIST_PAYLOAD_SIZE, 321);
        assert_eq!(CG_MARK_CRCLIST_WORDS, 80);
    }

    #[test]
    fn crclist_width_is_header_byte_plus_eighty_words() {
        assert_eq!(1 + 1 + 4 * CG_MARK_CRCLIST_WORDS, CG_MARK_CRCLIST_WIRE_SIZE);
    }

    #[test]
    fn crclist_encodes_to_322_bytes() {
        assert_eq!(CgMarkCRCList::new(1, sample_crc_list()).encode().len(), 322);
    }

    #[test]
    fn crclist_puts_the_index_at_offset_one() {
        let b = CgMarkCRCList::new(7, sample_crc_list()).encode();
        assert_eq!(b[0], 0x65);
        assert_eq!(b[1], 7);
    }

    #[test]
    fn crclist_words_are_little_endian_at_offset_two() {
        let mut words = [0_u32; CG_MARK_CRCLIST_WORDS];
        words[0] = 0x1122_3344;
        let b = CgMarkCRCList::new(0, words).encode();
        assert_eq!(&b[2..6], &[0x44, 0x33, 0x22, 0x11]);
    }

    #[test]
    fn crclist_preserves_word_order() {
        let words = sample_crc_list();
        let r = CgMarkCRCList::decode(&CgMarkCRCList::new(0, words).encode())
            .expect("a 322-byte record must decode");
        assert_eq!(r.crc_list, words);
        assert_eq!(r.crc_list[0], 0xA5A5_0000);
        assert_eq!(r.crc_list[79], 0xA5A5_004F);
    }

    #[test]
    fn crclist_roundtrips_through_a_frame() {
        let words = sample_crc_list();
        let r = CgMarkCRCList::decode_frame(&CgMarkCRCList::new(9, words).to_frame())
            .expect("a 321-byte payload must decode");
        assert_eq!(r.img_idx, 9);
        assert_eq!(r.crc_list, words);
    }

    #[test]
    fn crclist_frame_payload_is_321_bytes() {
        assert_eq!(CgMarkCRCList::new(0, [0; 80]).to_frame().payload.len(), 321);
    }

    #[test]
    fn crclist_rejects_a_short_slice_as_truncated() {
        let mut b = CgMarkCRCList::new(0, [0; 80]).encode();
        b.pop();
        assert_eq!(
            CgMarkCRCList::decode(&b),
            Err(CgMarkError::Truncated {
                needed: 322,
                available: 321
            })
        );
    }

    #[test]
    fn crclist_rejects_a_long_slice() {
        let mut b = CgMarkCRCList::new(0, [0; 80]).encode();
        b.push(0);
        assert_eq!(
            CgMarkCRCList::decode(&b),
            Err(CgMarkError::LengthMismatch {
                expected: 322,
                actual: 323
            })
        );
    }

    #[test]
    fn crclist_rejects_a_wrong_header_at_the_right_width() {
        let mut b = CgMarkCRCList::new(0, [0; 80]).encode();
        b[0] = 0x66;
        assert_eq!(
            CgMarkCRCList::decode(&b),
            Err(CgMarkError::InvalidHeader {
                expected: 0x65,
                actual: 0x66
            })
        );
    }

    #[test]
    fn crclist_frame_rejects_a_payload_of_320() {
        let f = ClientFrame::new(0x65, vec![0_u8; 320]);
        assert_eq!(
            CgMarkCRCList::decode_frame(&f),
            Err(CgMarkError::Truncated {
                needed: 321,
                available: 320
            })
        );
    }

    #[test]
    fn crclist_words_are_opaque_across_the_whole_u32_range() {
        let words = [u32::MAX, 0, 1, u32::MAX - 1];
        let mut full = [0_u32; CG_MARK_CRCLIST_WORDS];
        full[..4].copy_from_slice(&words);
        let r = CgMarkCRCList::decode(&CgMarkCRCList::new(0, full).encode())
            .expect("extreme words must survive");
        assert_eq!(&r.crc_list[..4], &words);
    }

    // ---- CgMarkUpload -----------------------------------------------

    fn sample_image() -> [u8; CG_MARK_UPLOAD_IMAGE_SIZE] {
        let mut image = [0_u8; CG_MARK_UPLOAD_IMAGE_SIZE];
        for (index, slot) in image.iter_mut().enumerate() {
            *slot = u8::try_from(index % 251).expect("remainder fits in u8");
        }
        image
    }

    #[test]
    fn upload_header_is_102() {
        assert_eq!(CgMarkUpload::header().value(), 0x66);
    }

    #[test]
    fn upload_sizes_are_773_and_772() {
        assert_eq!(CG_MARK_UPLOAD_WIRE_SIZE, 773);
        assert_eq!(CG_MARK_UPLOAD_PAYLOAD_SIZE, 772);
        assert_eq!(CG_MARK_UPLOAD_IMAGE_SIZE, 768);
    }

    #[test]
    fn upload_image_is_16_by_12_pixels_of_four_bytes() {
        assert_eq!(16 * 12 * 4, CG_MARK_UPLOAD_IMAGE_SIZE);
        assert_eq!(16 * 12, CG_MARK_UPLOAD_PIXELS);
    }

    #[test]
    fn upload_pixels_tile_the_image_without_a_remainder() {
        assert_eq!(CG_MARK_UPLOAD_PIXELS * 4, CG_MARK_UPLOAD_IMAGE_SIZE);
    }

    #[test]
    fn upload_encodes_to_773_bytes() {
        assert_eq!(CgMarkUpload::new(1, [0_u8; 768]).encode().len(), 773);
    }

    #[test]
    fn upload_puts_the_gid_little_endian_at_offset_one() {
        let b = CgMarkUpload::new(0x1122_3344, [0_u8; 768]).encode();
        assert_eq!(b[0], 0x66);
        assert_eq!(&b[1..5], &[0x44, 0x33, 0x22, 0x11]);
    }

    #[test]
    fn upload_image_starts_at_offset_five() {
        let mut image = [0_u8; CG_MARK_UPLOAD_IMAGE_SIZE];
        image[0] = 0xAB;
        let b = CgMarkUpload::new(0, image).encode();
        assert_eq!(b[5], 0xAB);
    }

    #[test]
    fn upload_preserves_every_image_byte() {
        let image = sample_image();
        let r = CgMarkUpload::decode(&CgMarkUpload::new(5, image).encode())
            .expect("a 773-byte record must decode");
        assert_eq!(r.gid, 5);
        assert_eq!(r.image, image);
    }

    #[test]
    fn upload_roundtrips_through_a_frame() {
        let image = sample_image();
        let r = CgMarkUpload::decode_frame(&CgMarkUpload::new(6, image).to_frame())
            .expect("a 772-byte payload must decode");
        assert_eq!(r.gid, 6);
        assert_eq!(r.image, image);
    }

    #[test]
    fn upload_frame_payload_is_772_bytes() {
        assert_eq!(CgMarkUpload::new(0, [0; 768]).to_frame().payload.len(), 772);
    }

    #[test]
    fn upload_keeps_an_all_zero_image_distinct_from_a_nonzero_one() {
        // The legacy handler treats an all-zero image as a deletion request, but
        // that is mark-manager policy. The codec must still round-trip both.
        let zero = CgMarkUpload::new(1, [0_u8; 768]);
        let mut image = [0_u8; CG_MARK_UPLOAD_IMAGE_SIZE];
        image[767] = 1;
        let one = CgMarkUpload::new(1, image);
        let zr = CgMarkUpload::decode(&zero.encode()).expect("zero image");
        let or = CgMarkUpload::decode(&one.encode()).expect("nonzero image");
        assert_eq!(zr, zero);
        assert_eq!(or, one);
        assert_ne!(zr, or);
    }

    #[test]
    fn upload_rejects_a_short_slice_as_truncated() {
        let mut b = CgMarkUpload::new(0, [0; 768]).encode();
        b.pop();
        assert_eq!(
            CgMarkUpload::decode(&b),
            Err(CgMarkError::Truncated {
                needed: 773,
                available: 772
            })
        );
    }

    #[test]
    fn upload_rejects_a_long_slice() {
        let mut b = CgMarkUpload::new(0, [0; 768]).encode();
        b.push(0);
        assert_eq!(
            CgMarkUpload::decode(&b),
            Err(CgMarkError::LengthMismatch {
                expected: 773,
                actual: 774
            })
        );
    }

    #[test]
    fn upload_rejects_a_wrong_header_at_the_right_width() {
        let mut b = CgMarkUpload::new(0, [0; 768]).encode();
        b[0] = 0x65;
        assert_eq!(
            CgMarkUpload::decode(&b),
            Err(CgMarkError::InvalidHeader {
                expected: 0x66,
                actual: 0x65
            })
        );
    }

    #[test]
    fn upload_frame_rejects_a_payload_of_768() {
        let f = ClientFrame::new(0x66, vec![0_u8; 768]);
        assert_eq!(
            CgMarkUpload::decode_frame(&f),
            Err(CgMarkError::Truncated {
                needed: 772,
                available: 768
            })
        );
    }

    // ---- the two records never decode as each other -------------------

    #[test]
    fn crclist_and_upload_reject_each_others_header() {
        let mut c = CgMarkCRCList::new(0, [0; 80]).encode();
        c[0] = 0x66;
        assert!(CgMarkCRCList::decode(&c).is_err());

        let mut u = CgMarkUpload::new(0, [0; 768]).encode();
        u[0] = 0x65;
        assert!(CgMarkUpload::decode(&u).is_err());
    }

    #[test]
    fn the_two_new_widths_are_distinct_from_the_three_old_ones() {
        let widths = [
            CG_MARK_LOGIN_WIRE_SIZE,
            CG_MARK_IDXLIST_WIRE_SIZE,
            CG_SYMBOL_CRC_WIRE_SIZE,
            CG_MARK_CRCLIST_WIRE_SIZE,
            CG_MARK_UPLOAD_WIRE_SIZE,
        ];
        let mut sorted = widths.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 5);
    }
}
