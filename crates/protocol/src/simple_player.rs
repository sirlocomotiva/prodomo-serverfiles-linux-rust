//! `TSimplePlayer`, the 70-byte character summary the client receives.
//!
//! The record is embedded four times in `TPacketGCLoginSuccess` (header 32)
//! and once in `TPacketGCPlayerCreateSuccess`. Its layout is the active x86
//! build of `server/server/common/tables.h` under `#pragma pack(1)`, with the
//! sash part enabled by `prodomodefines.h`. Every field is written little
//! endian in source order; the Rust struct layout is never the wire layout.

use crate::CHARACTER_NAME_MAX_LEN;

/// Raw width of `szName[CHARACTER_NAME_MAX_LEN + 1]`.
pub const CHARACTER_NAME_BYTES: usize = CHARACTER_NAME_MAX_LEN + 1;

/// Packed x86 size of `TSimplePlayer`.
pub const SIMPLE_PLAYER_WIRE_SIZE: usize = 70;

/// `TSimplePlayer`: a character summary in `TAccountTable`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SimplePlayerRecord {
    /// `dwID`.
    pub id: u32,
    /// `szName[CHARACTER_NAME_MAX_LEN + 1]`.
    pub name: [u8; CHARACTER_NAME_BYTES],
    /// `byJob`.
    pub job: u8,
    /// `byLevel`.
    pub level: u8,
    /// `dwPlayMinutes`.
    pub play_minutes: u32,
    /// `byST`.
    pub st: u8,
    /// `byHT`.
    pub ht: u8,
    /// `byDX`.
    pub dx: u8,
    /// `byIQ`.
    pub iq: u8,
    /// `wMainPart`.
    pub main_part: u16,
    /// `bChangeName`.
    pub change_name: u8,
    /// `wHairPart`.
    pub hair_part: u16,
    /// `wSashPart` (the active legacy build enables the sash system).
    pub sash_part: u16,
    /// `bDummy[4]`.
    pub dummy: [u8; 4],
    /// `x` (x86 `long`).
    pub x: i32,
    /// `y` (x86 `long`).
    pub y: i32,
    /// `lAddr` (x86 `long`).
    pub addr: i32,
    /// `wPort`.
    pub port: u16,
    /// `skill_group`.
    pub skill_group: u8,
    /// `byConquerorLevel`.
    pub conqueror_level: u8,
    /// `bySungmaStr`.
    pub sungma_str: u8,
    /// `bySungmaHp`.
    pub sungma_hp: u8,
    /// `bySungmaMove`.
    pub sungma_move: u8,
    /// `bySungmaImmune`.
    pub sungma_immune: u8,
}

impl SimplePlayerRecord {
    /// Exact packed x86 wire size (`sizeof(TSimplePlayer)`).
    pub const WIRE_SIZE: usize = SIMPLE_PLAYER_WIRE_SIZE;

    /// Encode the 70 packed x86 bytes in legacy field order.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(SIMPLE_PLAYER_WIRE_SIZE);
        out.extend_from_slice(&self.id.to_le_bytes());
        out.extend_from_slice(&self.name);
        out.push(self.job);
        out.push(self.level);
        out.extend_from_slice(&self.play_minutes.to_le_bytes());
        out.push(self.st);
        out.push(self.ht);
        out.push(self.dx);
        out.push(self.iq);
        out.extend_from_slice(&self.main_part.to_le_bytes());
        out.push(self.change_name);
        out.extend_from_slice(&self.hair_part.to_le_bytes());
        out.extend_from_slice(&self.sash_part.to_le_bytes());
        out.extend_from_slice(&self.dummy);
        out.extend_from_slice(&self.x.to_le_bytes());
        out.extend_from_slice(&self.y.to_le_bytes());
        out.extend_from_slice(&self.addr.to_le_bytes());
        out.extend_from_slice(&self.port.to_le_bytes());
        out.push(self.skill_group);
        out.push(self.conqueror_level);
        out.push(self.sungma_str);
        out.push(self.sungma_hp);
        out.push(self.sungma_move);
        out.push(self.sungma_immune);
        debug_assert_eq!(out.len(), SIMPLE_PLAYER_WIRE_SIZE);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_player_has_golden_offsets() {
        let mut name = [0; CHARACTER_NAME_BYTES];
        name[..4].copy_from_slice(b"hero");
        name[CHARACTER_NAME_BYTES - 1] = 0xee;
        let record = SimplePlayerRecord {
            id: 0x0102_0304,
            name,
            job: 0x11,
            level: 0x12,
            play_minutes: 0x1314_1516,
            st: 0x21,
            ht: 0x22,
            dx: 0x23,
            iq: 0x24,
            main_part: 0x3132,
            change_name: 0x33,
            hair_part: 0x3435,
            sash_part: 0x3637,
            dummy: [0x41, 0x42, 0x43, 0x44],
            x: 0x5152_5354,
            y: -2,
            addr: 0x6162_6364,
            port: 0x7172,
            skill_group: 0x81,
            conqueror_level: 0x82,
            sungma_str: 0x83,
            sungma_hp: 0x84,
            sungma_move: 0x85,
            sungma_immune: 0x86,
        };
        let bytes = record.encode();
        assert_eq!(bytes.len(), SIMPLE_PLAYER_WIRE_SIZE);
        assert_eq!(bytes[0..4], [0x04, 0x03, 0x02, 0x01]);
        assert_eq!(bytes[4..8], *b"hero");
        assert_eq!(bytes[28], 0xee);
        assert_eq!(bytes[29..31], [0x11, 0x12]);
        assert_eq!(bytes[31..35], [0x16, 0x15, 0x14, 0x13]);
        assert_eq!(bytes[35..39], [0x21, 0x22, 0x23, 0x24]);
        assert_eq!(bytes[39..41], [0x32, 0x31]);
        assert_eq!(bytes[41], 0x33);
        assert_eq!(bytes[42..46], [0x35, 0x34, 0x37, 0x36]);
        assert_eq!(bytes[46..50], [0x41, 0x42, 0x43, 0x44]);
        assert_eq!(bytes[50..54], [0x54, 0x53, 0x52, 0x51]);
        assert_eq!(bytes[54..58], [0xfe, 0xff, 0xff, 0xff]);
        assert_eq!(bytes[58..62], [0x64, 0x63, 0x62, 0x61]);
        assert_eq!(bytes[62..64], [0x72, 0x71]);
        assert_eq!(bytes[64..70], [0x81, 0x82, 0x83, 0x84, 0x85, 0x86]);
    }
}
