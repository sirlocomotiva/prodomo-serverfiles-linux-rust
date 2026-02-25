//! Explicit codecs for fixed game-to-game setup, lookup, warp, and map-index
//! records.
//!
//! The active x86 setup record is packed as one header byte, a little-endian
//! `u16` port, and one opaque channel byte. The position lookup record contains
//! one header byte followed by two little-endian `u32` process IDs. The warp
//! record contains one header byte, a little-endian `u32` PID, and two
//! little-endian signed `i32` coordinates. The guild-war map-index record
//! contains one header byte, two little-endian `u32` guild IDs, and one
//! little-endian signed `i32` map index. None of these records has a length
//! prefix, handle, or padding. This module is a transport-free record boundary
//! only; it does not register a connector, configure a descriptor, authenticate
//! a peer, look up a process or map, or perform gameplay work.

#![warn(missing_docs)]

use std::error::Error;
use std::fmt;

/// Legacy `HEADER_GG_SETUP` value in `server/server/game/packet.h`.
pub const HEADER_GG_SETUP: u8 = 9;

/// Complete packed setup wire size, including the one-byte header.
pub const GG_SETUP_WIRE_SIZE: usize = 4;

/// Legacy `HEADER_GG_FIND_POSITION` value in `server/server/game/packet.h`.
pub const HEADER_GG_FIND_POSITION: u8 = 12;

/// Complete packed position lookup wire size, including the one-byte header.
pub const GG_FIND_POSITION_WIRE_SIZE: usize = 9;

/// Legacy `HEADER_GG_WARP_CHARACTER` value in `server/server/game/packet.h`.
pub const HEADER_GG_WARP_CHARACTER: u8 = 13;

/// Complete packed warp-character wire size, including the one-byte header.
pub const GG_WARP_CHARACTER_WIRE_SIZE: usize = 13;

/// Legacy `HEADER_GG_GUILD_WAR_ZONE_MAP_INDEX` value in
/// `server/server/game/packet.h`.
pub const HEADER_GG_GUILD_WAR_ZONE_MAP_INDEX: u8 = 15;

/// Complete packed guild-war map-index wire size, including the one-byte
/// header.
pub const GG_GUILD_WAR_MAP_INDEX_WIRE_SIZE: usize = 13;

/// One fixed game-to-game peer setup record.
///
/// `channel` is an opaque source byte. The codec accepts every possible
/// channel value and does not infer a host, peer identity, or connection
/// state from this record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GgSetup {
    /// Port copied from the source `wPort` field.
    pub port: u16,
    /// Opaque source `bChannel` field.
    pub channel: u8,
}

impl GgSetup {
    /// Construct a setup record without interpreting its port or channel.
    #[must_use]
    pub const fn new(port: u16, channel: u8) -> Self {
        Self { port, channel }
    }

    /// Encode the exact four-byte packed record in source field order.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(GG_SETUP_WIRE_SIZE);
        bytes.push(HEADER_GG_SETUP);
        bytes.extend_from_slice(&self.port.to_le_bytes());
        bytes.push(self.channel);
        bytes
    }

    /// Decode one exact complete four-byte setup record.
    ///
    /// Length is checked before the header and fields are read. Therefore a
    /// short or overlong slice is reported as a length error even when its
    /// first byte is not [`HEADER_GG_SETUP`].
    ///
    /// # Errors
    ///
    /// Returns [`GgSetupError::Truncated`] for fewer than four bytes,
    /// [`GgSetupError::LengthMismatch`] for more than four bytes, and
    /// [`GgSetupError::InvalidHeader`] for a complete record with another
    /// header.
    pub fn decode(data: &[u8]) -> Result<Self, GgSetupError> {
        check_exact(data)?;
        let header = data[0];
        if header != HEADER_GG_SETUP {
            return Err(GgSetupError::InvalidHeader { actual: header });
        }
        Ok(Self {
            port: u16::from_le_bytes([data[1], data[2]]),
            channel: data[3],
        })
    }
}

/// One fixed game-to-game position lookup request.
///
/// The process IDs are opaque unsigned values. This record does not establish
/// identity, authorization, target existence, or map eligibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GgFindPosition {
    /// Source `dwFromPID` field.
    pub from_pid: u32,
    /// Source `dwTargetPID` field.
    pub target_pid: u32,
}

impl GgFindPosition {
    /// Construct a position lookup request without semantic PID validation.
    #[must_use]
    pub const fn new(from_pid: u32, target_pid: u32) -> Self {
        Self {
            from_pid,
            target_pid,
        }
    }

    /// Encode the exact nine-byte packed record in source field order.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(GG_FIND_POSITION_WIRE_SIZE);
        bytes.push(HEADER_GG_FIND_POSITION);
        bytes.extend_from_slice(&self.from_pid.to_le_bytes());
        bytes.extend_from_slice(&self.target_pid.to_le_bytes());
        bytes
    }

    /// Decode one exact complete nine-byte position lookup request.
    ///
    /// Length is checked before the header and fields are read. Therefore a
    /// short or overlong slice is reported as a length error even when its
    /// first byte is not [`HEADER_GG_FIND_POSITION`].
    ///
    /// # Errors
    ///
    /// Returns [`GgFindPositionError::Truncated`] for fewer than nine bytes,
    /// [`GgFindPositionError::LengthMismatch`] for more than nine bytes, and
    /// [`GgFindPositionError::InvalidHeader`] for a complete record with
    /// another header.
    pub fn decode(data: &[u8]) -> Result<Self, GgFindPositionError> {
        check_find_position_exact(data)?;
        let header = data[0];
        if header != HEADER_GG_FIND_POSITION {
            return Err(GgFindPositionError::InvalidHeader { actual: header });
        }
        Ok(Self {
            from_pid: u32::from_le_bytes([data[1], data[2], data[3], data[4]]),
            target_pid: u32::from_le_bytes([data[5], data[6], data[7], data[8]]),
        })
    }
}

/// One fixed game-to-game warp-character record.
///
/// The PID and coordinates are copied as opaque record fields. This codec does
/// not establish target existence, map eligibility, peer identity, ownership,
/// authorization, or permission to warp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GgWarpCharacter {
    /// Source `pid` field, an unsigned 32-bit process ID on the active x86
    /// profile.
    pub pid: u32,
    /// Source `x` field, a signed 32-bit coordinate on the active x86 profile.
    pub x: i32,
    /// Source `y` field, a signed 32-bit coordinate on the active x86 profile.
    pub y: i32,
}

impl GgWarpCharacter {
    /// Construct a warp record without semantic PID or coordinate validation.
    #[must_use]
    pub const fn new(pid: u32, x: i32, y: i32) -> Self {
        Self { pid, x, y }
    }

    /// Encode the exact thirteen-byte packed record in source field order.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(GG_WARP_CHARACTER_WIRE_SIZE);
        bytes.push(HEADER_GG_WARP_CHARACTER);
        bytes.extend_from_slice(&self.pid.to_le_bytes());
        bytes.extend_from_slice(&self.x.to_le_bytes());
        bytes.extend_from_slice(&self.y.to_le_bytes());
        bytes
    }

    /// Decode one exact complete thirteen-byte warp record.
    ///
    /// Length is checked before the header and fields are read. Therefore a
    /// short or overlong slice is reported as a length error even when its
    /// first byte is not [`HEADER_GG_WARP_CHARACTER`].
    ///
    /// # Errors
    ///
    /// Returns [`GgWarpCharacterError::Truncated`] for fewer than thirteen
    /// bytes, [`GgWarpCharacterError::LengthMismatch`] for more than thirteen
    /// bytes, and [`GgWarpCharacterError::InvalidHeader`] for a complete
    /// record with another header.
    pub fn decode(data: &[u8]) -> Result<Self, GgWarpCharacterError> {
        check_warp_character_exact(data)?;
        let header = data[0];
        if header != HEADER_GG_WARP_CHARACTER {
            return Err(GgWarpCharacterError::InvalidHeader { actual: header });
        }
        Ok(Self {
            pid: u32::from_le_bytes([data[1], data[2], data[3], data[4]]),
            x: i32::from_le_bytes([data[5], data[6], data[7], data[8]]),
            y: i32::from_le_bytes([data[9], data[10], data[11], data[12]]),
        })
    }
}

/// One fixed game-to-game guild-war map-index record.
///
/// The guild IDs and map index are copied as opaque source fields. This codec
/// does not establish guild existence, map eligibility, peer identity,
/// authorization, or permission to change a guild-war map index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GgGuildWarMapIndex {
    /// Source `dwGuildID1` field.
    pub guild_id1: u32,
    /// Source `dwGuildID2` field.
    pub guild_id2: u32,
    /// Source `lMapIndex` field, a signed 32-bit value on the active x86
    /// profile.
    pub map_index: i32,
}

impl GgGuildWarMapIndex {
    /// Construct a map-index record without semantic guild or map validation.
    #[must_use]
    pub const fn new(guild_id1: u32, guild_id2: u32, map_index: i32) -> Self {
        Self {
            guild_id1,
            guild_id2,
            map_index,
        }
    }

    /// Encode the exact thirteen-byte packed record in source field order.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(GG_GUILD_WAR_MAP_INDEX_WIRE_SIZE);
        bytes.push(HEADER_GG_GUILD_WAR_ZONE_MAP_INDEX);
        bytes.extend_from_slice(&self.guild_id1.to_le_bytes());
        bytes.extend_from_slice(&self.guild_id2.to_le_bytes());
        bytes.extend_from_slice(&self.map_index.to_le_bytes());
        bytes
    }

    /// Decode one exact complete thirteen-byte guild-war map-index record.
    ///
    /// Length is checked before the header and fields are read. Therefore a
    /// short or overlong slice is reported as a length error even when its
    /// first byte is not [`HEADER_GG_GUILD_WAR_ZONE_MAP_INDEX`].
    ///
    /// # Errors
    ///
    /// Returns [`GgGuildWarMapIndexError::Truncated`] for fewer than thirteen
    /// bytes, [`GgGuildWarMapIndexError::LengthMismatch`] for more than
    /// thirteen bytes, and [`GgGuildWarMapIndexError::InvalidHeader`] for a
    /// complete record with another header.
    pub fn decode(data: &[u8]) -> Result<Self, GgGuildWarMapIndexError> {
        check_guild_war_map_index_exact(data)?;
        let header = data[0];
        if header != HEADER_GG_GUILD_WAR_ZONE_MAP_INDEX {
            return Err(GgGuildWarMapIndexError::InvalidHeader { actual: header });
        }
        Ok(Self {
            guild_id1: u32::from_le_bytes([data[1], data[2], data[3], data[4]]),
            guild_id2: u32::from_le_bytes([data[5], data[6], data[7], data[8]]),
            map_index: i32::from_le_bytes([data[9], data[10], data[11], data[12]]),
        })
    }
}

/// A malformed game-to-game setup record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GgSetupError {
    /// The complete record was shorter than its packed wire size.
    Truncated {
        /// Required complete wire size, including the header.
        needed: usize,
        /// Bytes supplied by the caller.
        available: usize,
    },
    /// The complete record had bytes beyond its packed wire size.
    LengthMismatch {
        /// Exact complete wire size, including the header.
        expected: usize,
        /// Bytes supplied by the caller.
        actual: usize,
    },
    /// The record used a header other than [`HEADER_GG_SETUP`].
    InvalidHeader {
        /// Unsupported one-byte header supplied by the caller.
        actual: u8,
    },
}

impl fmt::Display for GgSetupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => write!(
                formatter,
                "GG setup is truncated: need {needed} bytes, have {available}"
            ),
            Self::LengthMismatch { expected, actual } => write!(
                formatter,
                "GG setup has {actual} bytes; expected exactly {expected}"
            ),
            Self::InvalidHeader { actual } => write!(
                formatter,
                "expected GG setup header 0x09, got 0x{actual:02x}"
            ),
        }
    }
}

impl Error for GgSetupError {}

/// A malformed game-to-game position lookup request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GgFindPositionError {
    /// The complete record was shorter than its packed wire size.
    Truncated {
        /// Required complete wire size, including the header.
        needed: usize,
        /// Bytes supplied by the caller.
        available: usize,
    },
    /// The complete record had bytes beyond its packed wire size.
    LengthMismatch {
        /// Exact complete wire size, including the header.
        expected: usize,
        /// Bytes supplied by the caller.
        actual: usize,
    },
    /// The record used a header other than [`HEADER_GG_FIND_POSITION`].
    InvalidHeader {
        /// Unsupported one-byte header supplied by the caller.
        actual: u8,
    },
}

impl fmt::Display for GgFindPositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => write!(
                formatter,
                "GG position lookup is truncated: need {needed} bytes, have {available}"
            ),
            Self::LengthMismatch { expected, actual } => write!(
                formatter,
                "GG position lookup has {actual} bytes; expected exactly {expected}"
            ),
            Self::InvalidHeader { actual } => write!(
                formatter,
                "expected GG position lookup header 0x0c, got 0x{actual:02x}"
            ),
        }
    }
}

impl Error for GgFindPositionError {}

/// A malformed game-to-game warp-character record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GgWarpCharacterError {
    /// The complete record was shorter than its packed wire size.
    Truncated {
        /// Required complete wire size, including the header.
        needed: usize,
        /// Bytes supplied by the caller.
        available: usize,
    },
    /// The complete record had bytes beyond its packed wire size.
    LengthMismatch {
        /// Exact complete wire size, including the header.
        expected: usize,
        /// Bytes supplied by the caller.
        actual: usize,
    },
    /// The record used a header other than [`HEADER_GG_WARP_CHARACTER`].
    InvalidHeader {
        /// Unsupported one-byte header supplied by the caller.
        actual: u8,
    },
}

impl fmt::Display for GgWarpCharacterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => write!(
                formatter,
                "GG warp character is truncated: need {needed} bytes, have {available}"
            ),
            Self::LengthMismatch { expected, actual } => write!(
                formatter,
                "GG warp character has {actual} bytes; expected exactly {expected}"
            ),
            Self::InvalidHeader { actual } => write!(
                formatter,
                "expected GG warp character header 0x0d, got 0x{actual:02x}"
            ),
        }
    }
}

impl Error for GgWarpCharacterError {}

/// A malformed game-to-game guild-war map-index record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GgGuildWarMapIndexError {
    /// The complete record was shorter than its packed wire size.
    Truncated {
        /// Required complete wire size, including the header.
        needed: usize,
        /// Bytes supplied by the caller.
        available: usize,
    },
    /// The complete record had bytes beyond its packed wire size.
    LengthMismatch {
        /// Exact complete wire size, including the header.
        expected: usize,
        /// Bytes supplied by the caller.
        actual: usize,
    },
    /// The record used a header other than
    /// [`HEADER_GG_GUILD_WAR_ZONE_MAP_INDEX`].
    InvalidHeader {
        /// Unsupported one-byte header supplied by the caller.
        actual: u8,
    },
}

impl fmt::Display for GgGuildWarMapIndexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => write!(
                formatter,
                "GG guild-war map index is truncated: need {needed} bytes, have {available}"
            ),
            Self::LengthMismatch { expected, actual } => write!(
                formatter,
                "GG guild-war map index has {actual} bytes; expected exactly {expected}"
            ),
            Self::InvalidHeader { actual } => write!(
                formatter,
                "expected GG guild-war map-index header 0x0f, got 0x{actual:02x}"
            ),
        }
    }
}

impl Error for GgGuildWarMapIndexError {}

fn check_exact(data: &[u8]) -> Result<(), GgSetupError> {
    match data.len().cmp(&GG_SETUP_WIRE_SIZE) {
        std::cmp::Ordering::Less => Err(GgSetupError::Truncated {
            needed: GG_SETUP_WIRE_SIZE,
            available: data.len(),
        }),
        std::cmp::Ordering::Greater => Err(GgSetupError::LengthMismatch {
            expected: GG_SETUP_WIRE_SIZE,
            actual: data.len(),
        }),
        std::cmp::Ordering::Equal => Ok(()),
    }
}

fn check_find_position_exact(data: &[u8]) -> Result<(), GgFindPositionError> {
    match data.len().cmp(&GG_FIND_POSITION_WIRE_SIZE) {
        std::cmp::Ordering::Less => Err(GgFindPositionError::Truncated {
            needed: GG_FIND_POSITION_WIRE_SIZE,
            available: data.len(),
        }),
        std::cmp::Ordering::Greater => Err(GgFindPositionError::LengthMismatch {
            expected: GG_FIND_POSITION_WIRE_SIZE,
            actual: data.len(),
        }),
        std::cmp::Ordering::Equal => Ok(()),
    }
}

fn check_warp_character_exact(data: &[u8]) -> Result<(), GgWarpCharacterError> {
    match data.len().cmp(&GG_WARP_CHARACTER_WIRE_SIZE) {
        std::cmp::Ordering::Less => Err(GgWarpCharacterError::Truncated {
            needed: GG_WARP_CHARACTER_WIRE_SIZE,
            available: data.len(),
        }),
        std::cmp::Ordering::Greater => Err(GgWarpCharacterError::LengthMismatch {
            expected: GG_WARP_CHARACTER_WIRE_SIZE,
            actual: data.len(),
        }),
        std::cmp::Ordering::Equal => Ok(()),
    }
}

fn check_guild_war_map_index_exact(data: &[u8]) -> Result<(), GgGuildWarMapIndexError> {
    match data.len().cmp(&GG_GUILD_WAR_MAP_INDEX_WIRE_SIZE) {
        std::cmp::Ordering::Less => Err(GgGuildWarMapIndexError::Truncated {
            needed: GG_GUILD_WAR_MAP_INDEX_WIRE_SIZE,
            available: data.len(),
        }),
        std::cmp::Ordering::Greater => Err(GgGuildWarMapIndexError::LengthMismatch {
            expected: GG_GUILD_WAR_MAP_INDEX_WIRE_SIZE,
            actual: data.len(),
        }),
        std::cmp::Ordering::Equal => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_bytes_preserve_header_source_order_and_little_endian_port() {
        let packet = GgSetup::new(0x1234, 0x56);
        assert_eq!(packet.encode(), vec![0x09, 0x34, 0x12, 0x56]);
        assert_eq!(packet.encode()[0], HEADER_GG_SETUP);
        assert_eq!(&packet.encode()[1..3], &[0x34, 0x12]);
        assert_eq!(packet.encode()[3], 0x56);
        assert_eq!(GgSetup::decode(&packet.encode()), Ok(packet));
    }

    #[test]
    fn zero_max_and_non_boundary_values_round_trip() {
        for packet in [
            GgSetup::new(0, 0),
            GgSetup::new(0x1234, 0x7f),
            GgSetup::new(0xabcd, 0x80),
            GgSetup::new(u16::MAX, u8::MAX),
        ] {
            let encoded = packet.encode();
            assert_eq!(encoded.len(), GG_SETUP_WIRE_SIZE);
            assert_eq!(GgSetup::decode(&encoded), Ok(packet));
        }
    }

    #[test]
    fn every_short_length_is_truncated_before_header_validation() {
        let complete = GgSetup::new(0x1234, 0x56).encode();
        for available in 0..GG_SETUP_WIRE_SIZE {
            let mut short = complete[..available].to_vec();
            if let Some(header) = short.first_mut() {
                *header = 0;
            }
            assert_eq!(
                GgSetup::decode(&short),
                Err(GgSetupError::Truncated {
                    needed: GG_SETUP_WIRE_SIZE,
                    available,
                })
            );
        }
    }

    #[test]
    fn trailing_bytes_and_wrong_headers_are_rejected() {
        let complete = GgSetup::new(1, 2).encode();

        let mut trailing = complete.clone();
        trailing.push(0);
        assert_eq!(
            GgSetup::decode(&trailing),
            Err(GgSetupError::LengthMismatch {
                expected: GG_SETUP_WIRE_SIZE,
                actual: GG_SETUP_WIRE_SIZE + 1,
            })
        );

        for actual in [0, 8, 10, u8::MAX] {
            let mut wrong_header = complete.clone();
            wrong_header[0] = actual;
            assert_eq!(
                GgSetup::decode(&wrong_header),
                Err(GgSetupError::InvalidHeader { actual })
            );
        }
    }

    #[test]
    fn error_precedence_is_length_before_header() {
        let mut short_wrong_header = vec![0xff, 0x34, 0x12, 0x56];
        short_wrong_header.truncate(2);
        assert_eq!(
            GgSetup::decode(&short_wrong_header),
            Err(GgSetupError::Truncated {
                needed: GG_SETUP_WIRE_SIZE,
                available: 2,
            })
        );

        let trailing_wrong_header = vec![0xff, 0x34, 0x12, 0x56, 0xaa];
        assert_eq!(
            GgSetup::decode(&trailing_wrong_header),
            Err(GgSetupError::LengthMismatch {
                expected: GG_SETUP_WIRE_SIZE,
                actual: 5,
            })
        );
    }

    #[test]
    fn exact_wire_size_is_four_bytes() {
        assert_eq!(GG_SETUP_WIRE_SIZE, 4);
        let encoded = GgSetup::new(0, 0).encode();
        assert_eq!(encoded.len(), 4);
        assert_eq!(GgSetup::decode(&encoded).unwrap().port, 0);
        assert_eq!(GgSetup::decode(&encoded).unwrap().channel, 0);
    }

    #[test]
    fn find_position_golden_bytes_preserve_header_source_order_and_little_endian_pids() {
        assert_eq!(HEADER_GG_FIND_POSITION, 12);
        assert_eq!(GG_FIND_POSITION_WIRE_SIZE, 9);

        let packet = GgFindPosition::new(0x1234_5678, 0xdead_beef);
        let encoded = packet.encode();
        assert_eq!(
            encoded,
            vec![0x0c, 0x78, 0x56, 0x34, 0x12, 0xef, 0xbe, 0xad, 0xde]
        );
        assert_eq!(encoded.len(), GG_FIND_POSITION_WIRE_SIZE);
        assert_eq!(GgFindPosition::decode(&encoded), Ok(packet));
    }

    #[test]
    fn find_position_fields_have_independent_offset_and_unsigned_boundary_round_trips() {
        for packet in [
            GgFindPosition::new(0, 0x0102_0304),
            GgFindPosition::new(0x89ab_cdef, 0),
            GgFindPosition::new(u32::MAX, 0x8000_0000),
            GgFindPosition::new(0x7fff_ffff, u32::MAX),
        ] {
            let encoded = packet.encode();
            assert_eq!(&encoded[1..5], &packet.from_pid.to_le_bytes());
            assert_eq!(&encoded[5..9], &packet.target_pid.to_le_bytes());
            assert_eq!(GgFindPosition::decode(&encoded), Ok(packet));
        }
    }

    #[test]
    fn every_short_find_position_length_is_truncated_before_header_validation() {
        let complete = GgFindPosition::new(0x1234_5678, 0xdead_beef).encode();
        for available in 0..GG_FIND_POSITION_WIRE_SIZE {
            let mut short = complete[..available].to_vec();
            if let Some(header) = short.first_mut() {
                *header = u8::MAX;
            }
            assert_eq!(
                GgFindPosition::decode(&short),
                Err(GgFindPositionError::Truncated {
                    needed: GG_FIND_POSITION_WIRE_SIZE,
                    available,
                })
            );
        }
    }

    #[test]
    fn find_position_trailing_bytes_and_exact_length_wrong_headers_are_rejected() {
        let complete = GgFindPosition::new(1, 2).encode();

        let mut trailing = complete.clone();
        trailing.push(0);
        assert_eq!(
            GgFindPosition::decode(&trailing),
            Err(GgFindPositionError::LengthMismatch {
                expected: GG_FIND_POSITION_WIRE_SIZE,
                actual: GG_FIND_POSITION_WIRE_SIZE + 1,
            })
        );

        for actual in [0, 9, 11, 13, u8::MAX] {
            let mut wrong_header = complete.clone();
            wrong_header[0] = actual;
            assert_eq!(
                GgFindPosition::decode(&wrong_header),
                Err(GgFindPositionError::InvalidHeader { actual })
            );
        }
    }

    #[test]
    fn find_position_error_precedence_is_length_before_header() {
        let mut short_wrong_header = vec![u8::MAX; GG_FIND_POSITION_WIRE_SIZE];
        short_wrong_header.truncate(GG_FIND_POSITION_WIRE_SIZE - 1);
        assert_eq!(
            GgFindPosition::decode(&short_wrong_header),
            Err(GgFindPositionError::Truncated {
                needed: GG_FIND_POSITION_WIRE_SIZE,
                available: GG_FIND_POSITION_WIRE_SIZE - 1,
            })
        );

        let mut trailing_wrong_header = vec![u8::MAX; GG_FIND_POSITION_WIRE_SIZE + 1];
        trailing_wrong_header[0] = 0;
        assert_eq!(
            GgFindPosition::decode(&trailing_wrong_header),
            Err(GgFindPositionError::LengthMismatch {
                expected: GG_FIND_POSITION_WIRE_SIZE,
                actual: GG_FIND_POSITION_WIRE_SIZE + 1,
            })
        );
    }

    #[test]
    fn warp_character_golden_bytes_preserve_header_source_order_and_signed_coordinates() {
        assert_eq!(HEADER_GG_WARP_CHARACTER, 13);
        assert_eq!(GG_WARP_CHARACTER_WIRE_SIZE, 13);

        let packet = GgWarpCharacter::new(0x1234_5678, -0x1234_5678i32, 0x7fff_fabc);
        let encoded = packet.encode();
        assert_eq!(
            encoded,
            vec![0x0d, 0x78, 0x56, 0x34, 0x12, 0x88, 0xa9, 0xcb, 0xed, 0xbc, 0xfa, 0xff, 0x7f]
        );
        assert_eq!(encoded.len(), GG_WARP_CHARACTER_WIRE_SIZE);
        assert_eq!(GgWarpCharacter::decode(&encoded), Ok(packet));
    }

    #[test]
    fn warp_character_fields_have_independent_offsets_and_signed_boundaries() {
        for packet in [
            GgWarpCharacter::new(0, i32::MIN, i32::MAX),
            GgWarpCharacter::new(u32::MAX, -1, 0),
            GgWarpCharacter::new(0x0102_0304, 0x7fff_ffff, -0x8000_0000),
            GgWarpCharacter::new(0x89ab_cdef, -123, 456),
        ] {
            let encoded = packet.encode();
            assert_eq!(&encoded[0..1], &[HEADER_GG_WARP_CHARACTER]);
            assert_eq!(&encoded[1..5], &packet.pid.to_le_bytes());
            assert_eq!(&encoded[5..9], &packet.x.to_le_bytes());
            assert_eq!(&encoded[9..13], &packet.y.to_le_bytes());
            assert_eq!(encoded.len(), GG_WARP_CHARACTER_WIRE_SIZE);
            assert_eq!(GgWarpCharacter::decode(&encoded), Ok(packet));
        }
    }

    #[test]
    fn every_short_warp_character_length_is_truncated_before_header_validation() {
        let complete = GgWarpCharacter::new(0x1234_5678, -1, 1).encode();
        for available in 0..GG_WARP_CHARACTER_WIRE_SIZE {
            let mut short = complete[..available].to_vec();
            if let Some(header) = short.first_mut() {
                *header = u8::MAX;
            }
            assert_eq!(
                GgWarpCharacter::decode(&short),
                Err(GgWarpCharacterError::Truncated {
                    needed: GG_WARP_CHARACTER_WIRE_SIZE,
                    available,
                })
            );
        }
    }

    #[test]
    fn warp_character_trailing_bytes_and_exact_length_wrong_headers_are_rejected() {
        let complete = GgWarpCharacter::new(1, -2, 3).encode();

        let mut trailing = complete.clone();
        trailing.push(0);
        assert_eq!(
            GgWarpCharacter::decode(&trailing),
            Err(GgWarpCharacterError::LengthMismatch {
                expected: GG_WARP_CHARACTER_WIRE_SIZE,
                actual: GG_WARP_CHARACTER_WIRE_SIZE + 1,
            })
        );

        for actual in [0, 9, 12, 14, 15, u8::MAX] {
            let mut wrong_header = complete.clone();
            wrong_header[0] = actual;
            assert_eq!(
                GgWarpCharacter::decode(&wrong_header),
                Err(GgWarpCharacterError::InvalidHeader { actual })
            );
        }
    }

    #[test]
    fn warp_character_error_precedence_is_length_before_header() {
        let mut short_wrong_header = vec![u8::MAX; GG_WARP_CHARACTER_WIRE_SIZE];
        short_wrong_header.truncate(GG_WARP_CHARACTER_WIRE_SIZE - 1);
        assert_eq!(
            GgWarpCharacter::decode(&short_wrong_header),
            Err(GgWarpCharacterError::Truncated {
                needed: GG_WARP_CHARACTER_WIRE_SIZE,
                available: GG_WARP_CHARACTER_WIRE_SIZE - 1,
            })
        );

        let mut trailing_wrong_header = vec![u8::MAX; GG_WARP_CHARACTER_WIRE_SIZE + 1];
        trailing_wrong_header[0] = 0;
        assert_eq!(
            GgWarpCharacter::decode(&trailing_wrong_header),
            Err(GgWarpCharacterError::LengthMismatch {
                expected: GG_WARP_CHARACTER_WIRE_SIZE,
                actual: GG_WARP_CHARACTER_WIRE_SIZE + 1,
            })
        );
    }

    #[test]
    fn guild_war_map_index_golden_bytes_preserve_header_source_order_and_little_endian() {
        assert_eq!(HEADER_GG_GUILD_WAR_ZONE_MAP_INDEX, 15);
        assert_eq!(GG_GUILD_WAR_MAP_INDEX_WIRE_SIZE, 13);

        let packet = GgGuildWarMapIndex::new(0x1234_5678, 0xdead_beef, -0x1234_5678i32);
        let encoded = packet.encode();
        assert_eq!(
            encoded,
            vec![0x0f, 0x78, 0x56, 0x34, 0x12, 0xef, 0xbe, 0xad, 0xde, 0x88, 0xa9, 0xcb, 0xed]
        );
        assert_eq!(encoded.len(), GG_GUILD_WAR_MAP_INDEX_WIRE_SIZE);
        assert_eq!(GgGuildWarMapIndex::decode(&encoded), Ok(packet));
    }

    #[test]
    fn guild_war_map_index_fields_have_independent_offsets_and_round_trip() {
        for packet in [
            GgGuildWarMapIndex::new(0, 0x89ab_cdef, 0),
            GgGuildWarMapIndex::new(0x0102_0304, 0, i32::MIN),
            GgGuildWarMapIndex::new(u32::MAX, 0x8000_0000, i32::MAX),
            GgGuildWarMapIndex::new(0x89ab_cdef, 0x0123_4567, -123),
        ] {
            let encoded = packet.encode();
            assert_eq!(&encoded[0..1], &[HEADER_GG_GUILD_WAR_ZONE_MAP_INDEX]);
            assert_eq!(&encoded[1..5], &packet.guild_id1.to_le_bytes());
            assert_eq!(&encoded[5..9], &packet.guild_id2.to_le_bytes());
            assert_eq!(&encoded[9..13], &packet.map_index.to_le_bytes());
            assert_eq!(encoded.len(), GG_GUILD_WAR_MAP_INDEX_WIRE_SIZE);
            assert_eq!(GgGuildWarMapIndex::decode(&encoded), Ok(packet));
        }
    }

    #[test]
    fn guild_war_map_index_signed_boundaries_round_trip() {
        for map_index in [i32::MIN, -1, 0, 1, i32::MAX] {
            let packet = GgGuildWarMapIndex::new(0x1234_5678, 0x89ab_cdef, map_index);
            let encoded = packet.encode();
            assert_eq!(&encoded[9..13], &map_index.to_le_bytes());
            assert_eq!(GgGuildWarMapIndex::decode(&encoded), Ok(packet));
        }
    }

    #[test]
    fn every_short_guild_war_map_index_length_is_truncated_before_header_validation() {
        let complete = GgGuildWarMapIndex::new(0x1234_5678, 0xdead_beef, -1).encode();
        for available in 0..GG_GUILD_WAR_MAP_INDEX_WIRE_SIZE {
            let mut short = complete[..available].to_vec();
            if let Some(header) = short.first_mut() {
                *header = u8::MAX;
            }
            assert_eq!(
                GgGuildWarMapIndex::decode(&short),
                Err(GgGuildWarMapIndexError::Truncated {
                    needed: GG_GUILD_WAR_MAP_INDEX_WIRE_SIZE,
                    available,
                })
            );
        }
    }

    #[test]
    fn guild_war_map_index_trailing_and_long_inputs_are_rejected() {
        let complete = GgGuildWarMapIndex::new(1, 2, 3).encode();
        for extra in [1, 4, 64] {
            let mut overlong = complete.clone();
            overlong.extend(std::iter::repeat_n(0xa5, extra));
            assert_eq!(
                GgGuildWarMapIndex::decode(&overlong),
                Err(GgGuildWarMapIndexError::LengthMismatch {
                    expected: GG_GUILD_WAR_MAP_INDEX_WIRE_SIZE,
                    actual: GG_GUILD_WAR_MAP_INDEX_WIRE_SIZE + extra,
                })
            );
        }
    }

    #[test]
    fn guild_war_map_index_exact_length_wrong_headers_are_rejected() {
        let complete = GgGuildWarMapIndex::new(1, 2, 3).encode();
        for actual in [0, 9, 12, 13, 14, 16, u8::MAX] {
            let mut wrong_header = complete.clone();
            wrong_header[0] = actual;
            assert_eq!(
                GgGuildWarMapIndex::decode(&wrong_header),
                Err(GgGuildWarMapIndexError::InvalidHeader { actual })
            );
        }
    }

    #[test]
    fn guild_war_map_index_error_precedence_is_length_before_header() {
        let mut short_wrong_header = vec![u8::MAX; GG_GUILD_WAR_MAP_INDEX_WIRE_SIZE];
        short_wrong_header.truncate(GG_GUILD_WAR_MAP_INDEX_WIRE_SIZE - 1);
        assert_eq!(
            GgGuildWarMapIndex::decode(&short_wrong_header),
            Err(GgGuildWarMapIndexError::Truncated {
                needed: GG_GUILD_WAR_MAP_INDEX_WIRE_SIZE,
                available: GG_GUILD_WAR_MAP_INDEX_WIRE_SIZE - 1,
            })
        );

        let mut trailing_wrong_header = vec![u8::MAX; GG_GUILD_WAR_MAP_INDEX_WIRE_SIZE + 1];
        trailing_wrong_header[0] = 0;
        assert_eq!(
            GgGuildWarMapIndex::decode(&trailing_wrong_header),
            Err(GgGuildWarMapIndexError::LengthMismatch {
                expected: GG_GUILD_WAR_MAP_INDEX_WIRE_SIZE,
                actual: GG_GUILD_WAR_MAP_INDEX_WIRE_SIZE + 1,
            })
        );
    }
}
