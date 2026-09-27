//! Metin2 packet protocol definitions.
//!
//! This crate contains the legacy protocol inventory, framing codecs, and
//! transitional record declarations used by the game and database servers.
//! Wire compatibility is defined by the active legacy x86 profile and the
//! `#pragma pack(1)` field order, not by the host platform or a Rust memory
//! layout. New record codecs therefore use explicit fixed-width,
//! little-endian field operations. The older `repr(C, packed)` declarations
//! remain an audit surface until they are replaced by safe domain/wire types.

#![warn(missing_docs)]

use std::io;

pub mod cg_account;
pub mod cg_arg_u8;
pub mod cg_attack;
pub mod cg_change_look;
pub mod cg_chat;
pub mod cg_client_version;
pub mod cg_cube_renewal;
pub mod cg_dragon_soul;
pub mod cg_exchange;
pub mod cg_fly_target;
pub mod cg_gaya_system;
pub mod cg_guild_answer;
pub mod cg_hack;
pub mod cg_handshake;
pub mod cg_header_only;
pub mod cg_inventory;
pub mod cg_item_destroy;
pub mod cg_item_drop;
pub mod cg_item_drop2;
pub mod cg_item_give;
pub mod cg_item_move;
pub mod cg_item_pickup;
pub mod cg_item_use;
pub mod cg_item_use_to_item;
pub mod cg_login;
pub mod cg_login3;
pub mod cg_mark;
pub mod cg_micro;
pub mod cg_move;
pub mod cg_name;
pub mod cg_party;
pub mod cg_party_skill;
pub mod cg_pick;
pub mod cg_position;
pub mod cg_quest_text;
pub mod cg_quickslot_add;
pub mod cg_quickslot_del;
pub mod cg_quickslot_swap;
pub mod cg_refine;
pub mod cg_safebox;
pub mod cg_safebox_move;
pub mod cg_sash;
pub mod cg_shoot;
pub mod cg_use_skill;
pub mod cg_variable;
pub mod cg_vid;
pub mod cg_wire;
pub mod gc;
pub mod gc_actors;
pub mod gc_channel_status;
pub mod gc_chat;
pub mod gc_entity;
pub mod gc_fields;
pub mod gc_inventory;
pub mod gc_nested;
pub mod gc_npc_position;
pub mod gc_position;
pub mod gc_small;
pub mod gc_vid;
pub mod item_pos;
pub mod simple_player;
pub mod tea;
/// Compatibility module name for the legacy client-to-game wire codec.
pub use cg_wire as client_wire;

// ============================================================================
// Size constants from length.h
// ============================================================================

/// Maximum login name length
pub const LOGIN_MAX_LEN: usize = 30;
/// Maximum password length
pub const PASSWD_MAX_LEN: usize = 16;
/// Number of players per account
pub const PLAYER_PER_ACCOUNT: usize = 4;
/// Maximum account status string length
pub const ACCOUNT_STATUS_MAX_LEN: usize = 8;
/// Maximum character name length
pub const CHARACTER_NAME_MAX_LEN: usize = 24;
/// Maximum shop sign length
pub const SHOP_SIGN_MAX_LEN: usize = 32;
/// Maximum chat message length
pub const CHAT_MAX_LEN: usize = 512;
/// Maximum guild name length
pub const GUILD_NAME_MAX_LEN: usize = 12;
/// Maximum quest name length
pub const QUEST_NAME_MAX_NUM: usize = 64;
/// Maximum social ID length
pub const SOCIAL_ID_MAX_LEN: usize = 18;
/// Maximum IP address length
pub const IP_ADDRESS_LENGTH: usize = 15;
/// Maximum host length
pub const MAX_HOST_LENGTH: usize = 15;
/// Maximum number of character points
pub const POINT_MAX_NUM: usize = 255;
/// Maximum number of skills
pub const SKILL_MAX_NUM: usize = 255;
/// Maximum number of quickslots
pub const QUICKSLOT_MAX_NUM: usize = 36;
/// Maximum number of item sockets
pub const ITEM_SOCKET_MAX_NUM: usize = 6;
/// Maximum number of item attributes
pub const ITEM_ATTRIBUTE_MAX_NUM: usize = 7;
/// Maximum number of refine materials
pub const REFINE_MATERIAL_MAX_NUM: usize = 5;
/// Map allow limit
pub const MAP_ALLOW_LIMIT: usize = 32;
/// Dragon soul slot max
pub const DS_SLOT_MAX: usize = 6;
/// Dragon soul refine grid size
pub const DRAGON_SOUL_REFINE_GRID_SIZE: usize = 15;
/// Maximum number of shop host items
pub const SHOP_HOST_ITEM_MAX_NUM: usize = 40;
/// Maximum shop tab name length
pub const SHOP_TAB_NAME_MAX: usize = 32;
/// Maximum quest notice arguments
pub const MAX_QUEST_NOTICE_ARGS: usize = 5;
/// Maximum title length for private shops
pub const TITLE_MAX_LEN: usize = 32;
/// Maximum number of private shop host items
pub const PRIVATE_SHOP_HOST_ITEM_MAX_NUM: usize = 128;
/// Maximum number of selected items for shop search
pub const SELECTED_ITEM_MAX_NUM: usize = 10;
/// Maximum inventory protected password length
pub const INVENTORY_PROTECTED_PASSWORD_MAX_LEN: usize = 6;
/// Number of days in a week for daily gift system
pub const DAILY_GIFT_WEEK_DAYS: usize = 7;
/// Maximum effect file name length
pub const MAX_EFFECT_FILE_NAME: usize = 128;
/// Music name length for BGM packets
pub const MUSIC_NAME_LEN: usize = 24;

// ============================================================================
// Serialization traits for packed C-compatible structs
// ============================================================================

/// Trait for types that can be serialized to/from bytes in C packed format
pub trait PacketSerialize: Sized {
    /// Serialize this struct to a byte vector
    fn to_bytes(&self) -> Vec<u8>;
    /// Deserialize from a byte slice.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the input is truncated or malformed.
    fn from_bytes(data: &[u8]) -> io::Result<Self>;
    /// Expected size in bytes
    fn packed_size() -> usize;
}

// Helper functions for reading/writing primitive types in little-endian
fn write_u8(buf: &mut Vec<u8>, val: u8) {
    buf.push(val);
}

fn write_u16(buf: &mut Vec<u8>, val: u16) {
    buf.extend_from_slice(&val.to_le_bytes());
}

fn write_u32(buf: &mut Vec<u8>, val: u32) {
    buf.extend_from_slice(&val.to_le_bytes());
}

fn write_i32(buf: &mut Vec<u8>, val: i32) {
    buf.extend_from_slice(&val.to_le_bytes());
}

#[allow(dead_code)]
fn write_i64(buf: &mut Vec<u8>, val: i64) {
    buf.extend_from_slice(&val.to_le_bytes());
}

#[allow(dead_code)]
fn write_u64(buf: &mut Vec<u8>, val: u64) {
    buf.extend_from_slice(&val.to_le_bytes());
}

fn write_bytes(buf: &mut Vec<u8>, data: &[u8]) {
    buf.extend_from_slice(data);
}

fn read_u8(data: &[u8], offset: &mut usize) -> io::Result<u8> {
    let end = offset
        .checked_add(1)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "u8 offset overflow"))?;
    if end > data.len() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "read_u8"));
    }
    let val = data[*offset];
    *offset = end;
    Ok(val)
}

fn read_u16(data: &[u8], offset: &mut usize) -> io::Result<u16> {
    let end = offset
        .checked_add(2)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "u16 offset overflow"))?;
    if end > data.len() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "read_u16"));
    }
    let val = u16::from_le_bytes([data[*offset], data[*offset + 1]]);
    *offset = end;
    Ok(val)
}

fn read_u32(data: &[u8], offset: &mut usize) -> io::Result<u32> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "u32 offset overflow"))?;
    if end > data.len() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "read_u32"));
    }
    let val = u32::from_le_bytes([
        data[*offset],
        data[*offset + 1],
        data[*offset + 2],
        data[*offset + 3],
    ]);
    *offset = end;
    Ok(val)
}

fn read_i32(data: &[u8], offset: &mut usize) -> io::Result<i32> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "i32 offset overflow"))?;
    if end > data.len() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "read_i32"));
    }
    let val = i32::from_le_bytes([
        data[*offset],
        data[*offset + 1],
        data[*offset + 2],
        data[*offset + 3],
    ]);
    *offset = end;
    Ok(val)
}

#[allow(dead_code)]
fn read_i64(data: &[u8], offset: &mut usize) -> io::Result<i64> {
    let end = offset
        .checked_add(8)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "i64 offset overflow"))?;
    if end > data.len() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "read_i64"));
    }
    let val = i64::from_le_bytes([
        data[*offset],
        data[*offset + 1],
        data[*offset + 2],
        data[*offset + 3],
        data[*offset + 4],
        data[*offset + 5],
        data[*offset + 6],
        data[*offset + 7],
    ]);
    *offset = end;
    Ok(val)
}

#[allow(dead_code)]
fn read_u64(data: &[u8], offset: &mut usize) -> io::Result<u64> {
    let end = offset
        .checked_add(8)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "u64 offset overflow"))?;
    if end > data.len() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "read_u64"));
    }
    let val = u64::from_le_bytes([
        data[*offset],
        data[*offset + 1],
        data[*offset + 2],
        data[*offset + 3],
        data[*offset + 4],
        data[*offset + 5],
        data[*offset + 6],
        data[*offset + 7],
    ]);
    *offset = end;
    Ok(val)
}

#[allow(dead_code)]
fn read_bytes(data: &[u8], offset: &mut usize, len: usize) -> io::Result<Vec<u8>> {
    let end = offset
        .checked_add(len)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "byte offset overflow"))?;
    if end > data.len() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "read_bytes"));
    }
    let val = data[*offset..end].to_vec();
    *offset = end;
    Ok(val)
}

fn read_array<const N: usize>(data: &[u8], offset: &mut usize) -> io::Result<[u8; N]> {
    let end = offset
        .checked_add(N)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "array offset overflow"))?;
    if end > data.len() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "read_array"));
    }
    let mut arr = [0u8; N];
    arr.copy_from_slice(&data[*offset..end]);
    *offset = end;
    Ok(arr)
}

fn ensure_exact_len(data: &[u8], expected: usize) -> io::Result<()> {
    match data.len().cmp(&expected) {
        std::cmp::Ordering::Equal => Ok(()),
        std::cmp::Ordering::Less => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!(
                "record is truncated: expected {expected}, got {}",
                data.len()
            ),
        )),
        std::cmp::Ordering::Greater => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "record has trailing bytes: expected {expected}, got {}",
                data.len()
            ),
        )),
    }
}

// ============================================================================
// Shared types from tables.h
// ============================================================================

/// Simple player info for character selection screen
/// C++: `TSimplePlayer` (70 bytes in the active packed x86 profile)
#[derive(Debug, Clone)]
pub struct TSimplePlayer {
    /// Player ID
    pub dw_id: u32,
    /// Character name (null-terminated)
    pub sz_name: [u8; CHARACTER_NAME_MAX_LEN + 1],
    /// Job class
    pub by_job: u8,
    /// Player level
    pub by_level: u8,
    /// Total play time in minutes
    pub dw_play_minutes: u32,
    /// STR stat
    pub by_st: u8,
    /// HT stat
    pub by_ht: u8,
    /// DEX stat
    pub by_dx: u8,
    /// INT stat
    pub by_iq: u8,
    /// Main equipment part
    pub w_main_part: u16,
    /// Change name flag
    pub b_change_name: u8,
    /// Hair part
    pub w_hair_part: u16,
    /// Sash part
    pub w_sash_part: u16,
    /// Dummy bytes
    pub b_dummy: [u8; 4],
    /// X position
    pub x: i32,
    /// Y position
    pub y: i32,
    /// Address
    pub l_addr: i32,
    /// Port
    pub w_port: u16,
    /// Skill group
    pub skill_group: u8,
    /// Conqueror level
    pub by_conqueror_level: u8,
    /// Sungma STR
    pub by_sungma_str: u8,
    /// Sungma HP
    pub by_sungma_hp: u8,
    /// Sungma Move
    pub by_sungma_move: u8,
    /// Sungma Immune
    pub by_sungma_immune: u8,
}

impl PacketSerialize for TSimplePlayer {
    fn packed_size() -> usize {
        4 + 25
            + 1
            + 1
            + 4
            + 1
            + 1
            + 1
            + 1
            + 2
            + 1
            + 2
            + 2
            + 4
            + 4
            + 4
            + 4
            + 2
            + 1
            + 1
            + 1
            + 1
            + 1
            + 1
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(Self::packed_size());
        write_u32(&mut buf, self.dw_id);
        write_bytes(&mut buf, &self.sz_name);
        write_u8(&mut buf, self.by_job);
        write_u8(&mut buf, self.by_level);
        write_u32(&mut buf, self.dw_play_minutes);
        write_u8(&mut buf, self.by_st);
        write_u8(&mut buf, self.by_ht);
        write_u8(&mut buf, self.by_dx);
        write_u8(&mut buf, self.by_iq);
        write_u16(&mut buf, self.w_main_part);
        write_u8(&mut buf, self.b_change_name);
        write_u16(&mut buf, self.w_hair_part);
        write_u16(&mut buf, self.w_sash_part);
        write_bytes(&mut buf, &self.b_dummy);
        write_i32(&mut buf, self.x);
        write_i32(&mut buf, self.y);
        write_i32(&mut buf, self.l_addr);
        write_u16(&mut buf, self.w_port);
        write_u8(&mut buf, self.skill_group);
        write_u8(&mut buf, self.by_conqueror_level);
        write_u8(&mut buf, self.by_sungma_str);
        write_u8(&mut buf, self.by_sungma_hp);
        write_u8(&mut buf, self.by_sungma_move);
        write_u8(&mut buf, self.by_sungma_immune);
        buf
    }

    fn from_bytes(data: &[u8]) -> io::Result<Self> {
        ensure_exact_len(data, Self::packed_size())?;
        let mut offset = 0;
        Ok(Self {
            dw_id: read_u32(data, &mut offset)?,
            sz_name: read_array(data, &mut offset)?,
            by_job: read_u8(data, &mut offset)?,
            by_level: read_u8(data, &mut offset)?,
            dw_play_minutes: read_u32(data, &mut offset)?,
            by_st: read_u8(data, &mut offset)?,
            by_ht: read_u8(data, &mut offset)?,
            by_dx: read_u8(data, &mut offset)?,
            by_iq: read_u8(data, &mut offset)?,
            w_main_part: read_u16(data, &mut offset)?,
            b_change_name: read_u8(data, &mut offset)?,
            w_hair_part: read_u16(data, &mut offset)?,
            w_sash_part: read_u16(data, &mut offset)?,
            b_dummy: read_array(data, &mut offset)?,
            x: read_i32(data, &mut offset)?,
            y: read_i32(data, &mut offset)?,
            l_addr: read_i32(data, &mut offset)?,
            w_port: read_u16(data, &mut offset)?,
            skill_group: read_u8(data, &mut offset)?,
            by_conqueror_level: read_u8(data, &mut offset)?,
            by_sungma_str: read_u8(data, &mut offset)?,
            by_sungma_hp: read_u8(data, &mut offset)?,
            by_sungma_move: read_u8(data, &mut offset)?,
            by_sungma_immune: read_u8(data, &mut offset)?,
        })
    }
}

/// Item attribute for player items
/// C++: `TPlayerItemAttribute` (3 bytes with `#pragma pack(1)`)
#[derive(Debug, Clone, Copy)]
pub struct TPlayerItemAttribute {
    /// Attribute type
    pub b_type: u8,
    /// Attribute value
    pub s_value: i16,
}

impl PacketSerialize for TPlayerItemAttribute {
    fn packed_size() -> usize {
        3
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(3);
        write_u8(&mut buf, self.b_type);
        buf.extend_from_slice(&self.s_value.to_le_bytes());
        buf
    }

    fn from_bytes(data: &[u8]) -> io::Result<Self> {
        ensure_exact_len(data, Self::packed_size())?;
        let mut offset = 0;
        let b_type = read_u8(data, &mut offset)?;
        let s_value = i16::from_le_bytes([data[offset], data[offset + 1]]);
        Ok(Self { b_type, s_value })
    }
}

/// Quickslot entry
/// C++: `TQuickslot` (2 bytes with `#pragma pack(1)`)
///
/// `PartialEq`, `Eq`, `Hash`, and `Default` are derived so that a containing
/// fixed record can expose the usual value semantics. The wire domain is
/// unchanged: both fields stay raw `u8` and neither is range-checked here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TQuickslot {
    /// Slot type
    pub b_type: u8,
    /// Slot position
    pub b_pos: u8,
}

impl PacketSerialize for TQuickslot {
    fn packed_size() -> usize {
        2
    }

    fn to_bytes(&self) -> Vec<u8> {
        vec![self.b_type, self.b_pos]
    }

    fn from_bytes(data: &[u8]) -> io::Result<Self> {
        ensure_exact_len(data, Self::packed_size())?;
        Ok(Self {
            b_type: data[0],
            b_pos: data[1],
        })
    }
}

/// Player skill entry
/// C++: `TPlayerSkill` (6 bytes in the active packed x86 profile)
#[derive(Debug, Clone, Copy)]
pub struct TPlayerSkill {
    /// Master type
    pub b_master_type: u8,
    /// Skill level
    pub b_level: u8,
    /// Next read time (`time_t` on the legacy x86 target)
    pub t_next_read: i32,
}

impl PacketSerialize for TPlayerSkill {
    fn packed_size() -> usize {
        6
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(6);
        write_u8(&mut buf, self.b_master_type);
        write_u8(&mut buf, self.b_level);
        write_i32(&mut buf, self.t_next_read);
        buf
    }

    fn from_bytes(data: &[u8]) -> io::Result<Self> {
        ensure_exact_len(data, Self::packed_size())?;
        let mut offset = 0;
        Ok(Self {
            b_master_type: read_u8(data, &mut offset)?,
            b_level: read_u8(data, &mut offset)?,
            t_next_read: read_i32(data, &mut offset)?,
        })
    }
}

/// Item position (window type + cell)
/// C++: `TItemPos` (3 bytes with `#pragma pack(1)`)
#[derive(Debug, Clone, Copy)]
pub struct TItemPos {
    /// Window type
    pub window_type: u8,
    /// Cell position
    pub cell: u16,
}

impl PacketSerialize for TItemPos {
    fn packed_size() -> usize {
        3
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(3);
        write_u8(&mut buf, self.window_type);
        write_u16(&mut buf, self.cell);
        buf
    }

    fn from_bytes(data: &[u8]) -> io::Result<Self> {
        ensure_exact_len(data, Self::packed_size())?;
        let mut offset = 0;
        Ok(Self {
            window_type: read_u8(data, &mut offset)?,
            cell: read_u16(data, &mut offset)?,
        })
    }
}

/// Helper to create a null-terminated byte array from a string
pub fn str_to_bytes<const N: usize>(s: &str) -> [u8; N] {
    let mut buf = [0u8; N];
    let bytes = s.as_bytes();
    let len = bytes.len().min(N.saturating_sub(1));
    buf[..len].copy_from_slice(&bytes[..len]);
    buf
}

/// Helper to read a null-terminated string from a byte array
pub fn bytes_to_str(bytes: &[u8]) -> &str {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end]).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tplayer_item_attribute_roundtrip() {
        let attr = TPlayerItemAttribute {
            b_type: 5,
            s_value: 100,
        };
        let bytes = attr.to_bytes();
        assert_eq!(bytes.len(), TPlayerItemAttribute::packed_size());
        let decoded = TPlayerItemAttribute::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.b_type, attr.b_type);
        assert_eq!(decoded.s_value, attr.s_value);
    }

    #[test]
    fn test_tquickslot_roundtrip() {
        let slot = TQuickslot {
            b_type: 1,
            b_pos: 5,
        };
        let bytes = slot.to_bytes();
        assert_eq!(bytes.len(), TQuickslot::packed_size());
        let decoded = TQuickslot::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.b_type, slot.b_type);
        assert_eq!(decoded.b_pos, slot.b_pos);
    }

    #[test]
    fn test_titem_pos_roundtrip() {
        let pos = TItemPos {
            window_type: 1,
            cell: 42,
        };
        let bytes = pos.to_bytes();
        assert_eq!(bytes.len(), TItemPos::packed_size());
        let decoded = TItemPos::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.window_type, pos.window_type);
        assert_eq!(decoded.cell, pos.cell);
    }

    #[test]
    fn test_tplayer_skill_roundtrip() {
        let skill = TPlayerSkill {
            b_master_type: 2,
            b_level: 10,
            t_next_read: 1_234_567_890,
        };
        let bytes = skill.to_bytes();
        assert_eq!(bytes.len(), TPlayerSkill::packed_size());
        let decoded = TPlayerSkill::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.b_master_type, skill.b_master_type);
        assert_eq!(decoded.b_level, skill.b_level);
        assert_eq!(decoded.t_next_read, skill.t_next_read);
    }

    #[test]
    fn test_fixed_legacy_records_reject_wrong_lengths() {
        assert_eq!(
            TPlayerItemAttribute::from_bytes(&[1, 2])
                .unwrap_err()
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
        assert_eq!(
            TPlayerItemAttribute::from_bytes(&[1, 2, 3, 4])
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn test_str_to_bytes() {
        let bytes = str_to_bytes::<10>("hello");
        assert_eq!(&bytes[..5], b"hello");
        assert_eq!(bytes[5], 0);
    }

    #[test]
    fn test_zero_length_string_buffer_is_safe() {
        assert_eq!(str_to_bytes::<0>("ignored"), [0u8; 0]);
    }

    #[test]
    fn test_bytes_to_str() {
        let mut bytes = [0u8; 10];
        bytes[..5].copy_from_slice(b"hello");
        assert_eq!(bytes_to_str(&bytes), "hello");
    }
}
