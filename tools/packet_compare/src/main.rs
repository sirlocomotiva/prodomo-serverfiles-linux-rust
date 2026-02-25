use clap::Parser;
use protocol::cg::*;
use protocol::gc::*;
use protocol::gg::*;
use protocol::PacketSerialize;
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    file1: PathBuf,
    file2: PathBuf,
    #[arg(short, long)]
    verbose: bool,
}

#[derive(Debug)]
struct PacketComparison {
    packet_type: String,
    header: u8,
    passed: bool,
    differences: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PacketDirection {
    CG,
    GC,
    GG,
}
fn identify_packet(header: u8, direction: PacketDirection) -> &'static str {
    match direction {
        PacketDirection::CG => match header {
            HEADER_CG_HANDSHAKE => "CG_HANDSHAKE",
            HEADER_CG_PONG => "CG_PONG",
            HEADER_CG_TIME_SYNC => "CG_TIME_SYNC",
            HEADER_CG_KEY_AGREEMENT => "CG_KEY_AGREEMENT",
            HEADER_CG_LOGIN => "CG_LOGIN",
            HEADER_CG_ATTACK => "CG_ATTACK",
            HEADER_CG_CHAT => "CG_CHAT",
            HEADER_CG_CHARACTER_CREATE => "CG_CHARACTER_CREATE",
            HEADER_CG_CHARACTER_DELETE => "CG_CHARACTER_DELETE",
            HEADER_CG_CHARACTER_SELECT => "CG_CHARACTER_SELECT",
            HEADER_CG_MOVE => "CG_MOVE",
            HEADER_CG_SYNC_POSITION => "CG_SYNC_POSITION",
            HEADER_CG_ENTERGAME => "CG_ENTERGAME",
            HEADER_CG_ITEM_USE => "CG_ITEM_USE",
            HEADER_CG_ITEM_DROP => "CG_ITEM_DROP",
            HEADER_CG_ITEM_MOVE => "CG_ITEM_MOVE",
            HEADER_CG_ITEM_PICKUP => "CG_ITEM_PICKUP",
            HEADER_CG_QUICKSLOT_ADD => "CG_QUICKSLOT_ADD",
            HEADER_CG_QUICKSLOT_DEL => "CG_QUICKSLOT_DEL",
            HEADER_CG_QUICKSLOT_SWAP => "CG_QUICKSLOT_SWAP",
            HEADER_CG_WHISPER => "CG_WHISPER",
            HEADER_CG_ITEM_DROP2 => "CG_ITEM_DROP2",
            HEADER_CG_ITEM_DESTROY => "CG_ITEM_DESTROY",
            HEADER_CG_ITEM_SELL => "CG_ITEM_SELL",
            HEADER_CG_ON_CLICK => "CG_ON_CLICK",
            HEADER_CG_EXCHANGE => "CG_EXCHANGE",
            HEADER_CG_CHARACTER_POSITION => "CG_CHARACTER_POSITION",
            HEADER_CG_SCRIPT_ANSWER => "CG_SCRIPT_ANSWER",
            HEADER_CG_QUEST_INPUT_STRING => "CG_QUEST_INPUT_STRING",
            HEADER_CG_QUEST_CONFIRM => "CG_QUEST_CONFIRM",
            HEADER_CG_REQUEST_EVENT_QUEST => "CG_REQUEST_EVENT_QUEST",
            HEADER_CG_SHOP => "CG_SHOP",
            HEADER_CG_FLY_TARGETING => "CG_FLY_TARGETING",
            HEADER_CG_USE_SKILL => "CG_USE_SKILL",
            HEADER_CG_ADD_FLY_TARGETING => "CG_ADD_FLY_TARGETING",
            HEADER_CG_SHOOT => "CG_SHOOT",
            HEADER_CG_MYSHOP => "CG_MYSHOP",
            HEADER_CG_ITEM_USE_TO_ITEM => "CG_ITEM_USE_TO_ITEM",
            HEADER_CG_TARGET => "CG_TARGET",
            HEADER_CG_TEXT => "CG_TEXT",
            HEADER_CG_WARP => "CG_WARP",
            HEADER_CG_SCRIPT_BUTTON => "CG_SCRIPT_BUTTON",
            HEADER_CG_MESSENGER => "CG_MESSENGER",
            HEADER_CG_MALL_CHECKOUT => "CG_MALL_CHECKOUT",
            HEADER_CG_SAFEBOX_CHECKIN => "CG_SAFEBOX_CHECKIN",
            HEADER_CG_SAFEBOX_CHECKOUT => "CG_SAFEBOX_CHECKOUT",
            HEADER_CG_PARTY_INVITE => "CG_PARTY_INVITE",
            HEADER_CG_PARTY_INVITE_ANSWER => "CG_PARTY_INVITE_ANSWER",
            HEADER_CG_PARTY_REMOVE => "CG_PARTY_REMOVE",
            HEADER_CG_PARTY_SET_STATE => "CG_PARTY_SET_STATE",
            HEADER_CG_PARTY_USE_SKILL => "CG_PARTY_USE_SKILL",
            HEADER_CG_SAFEBOX_ITEM_MOVE => "CG_SAFEBOX_ITEM_MOVE",
            HEADER_CG_PARTY_PARAMETER => "CG_PARTY_PARAMETER",
            HEADER_CG_GUILD => "CG_GUILD",
            HEADER_CG_ANSWER_MAKE_GUILD => "CG_ANSWER_MAKE_GUILD",
            HEADER_CG_FISHING => "CG_FISHING",
            HEADER_CG_ITEM_GIVE => "CG_ITEM_GIVE",
            HEADER_CG_EMPIRE => "CG_EMPIRE",
            HEADER_CG_REFINE => "CG_REFINE",
            HEADER_CG_MARK_LOGIN => "CG_MARK_LOGIN",
            HEADER_CG_MARK_CRCLIST => "CG_MARK_CRCLIST",
            HEADER_CG_MARK_UPLOAD => "CG_MARK_UPLOAD",
            HEADER_CG_MARK_IDXLIST => "CG_MARK_IDXLIST",
            HEADER_CG_HACK => "CG_HACK",
            HEADER_CG_CHANGE_NAME => "CG_CHANGE_NAME",
            HEADER_CG_LOGIN2 => "CG_LOGIN2",
            HEADER_CG_DUNGEON => "CG_DUNGEON",
            HEADER_CG_LOGIN3 => "CG_LOGIN3",
            HEADER_CG_GUILD_SYMBOL_UPLOAD => "CG_GUILD_SYMBOL_UPLOAD",
            HEADER_CG_SYMBOL_CRC => "CG_SYMBOL_CRC",
            HEADER_CG_SCRIPT_SELECT_ITEM => "CG_SCRIPT_SELECT_ITEM",
            HEADER_CG_REQUEST_EVENT_DATA => "CG_REQUEST_EVENT_DATA",
            HEADER_CG_SWITCHBOT => "CG_SWITCHBOT",
            HEADER_CG_AURA => "CG_AURA",
            HEADER_CG_DAILY_GIFT => "CG_DAILY_GIFT",
            HEADER_CG_DRAGON_SOUL_REFINE => "CG_DRAGON_SOUL_REFINE",
            HEADER_CG_STATE_CHECKER => "CG_STATE_CHECKER",
            HEADER_CG_CUBE_RENEWAL => "CG_CUBE_RENEWAL",
            HEADER_CG_PRIVATE_SHOP => "CG_PRIVATE_SHOP",
            HEADER_CG_CHANGE_LANGUAGE => "CG_CHANGE_LANGUAGE",
            HEADER_CG_WHISPER_DETAILS => "CG_WHISPER_DETAILS",
            HEADER_CG_GAYA_SYSTEM => "CG_GAYA_SYSTEM",
            HEADER_CG_CLIENT_VERSION => "CG_CLIENT_VERSION",
            HEADER_CG_CLIENT_VERSION2 => "CG_CLIENT_VERSION2",
            HEADER_CG_TARGET_INFO_LOAD => "CG_TARGET_INFO_LOAD",
            _ => "CG_UNKNOWN",
        },
        PacketDirection::GC => match header {
            HEADER_GC_CHARACTER_ADD => "GC_CHARACTER_ADD",
            HEADER_GC_CHARACTER_DEL => "GC_CHARACTER_DEL",
            HEADER_GC_CHARACTER_MOVE => "GC_CHARACTER_MOVE",
            HEADER_GC_CHAT => "GC_CHAT",
            HEADER_GC_WHISPER => "GC_WHISPER",
            HEADER_GC_CHARACTER_UPDATE => "GC_CHARACTER_UPDATE",
            HEADER_GC_ITEM_SET => "GC_ITEM_SET",
            HEADER_GC_ITEM_USE => "GC_ITEM_USE",
            HEADER_GC_ITEM_DROP => "GC_ITEM_DROP",
            HEADER_GC_ITEM_UPDATE => "GC_ITEM_UPDATE",
            HEADER_GC_ITEM_GROUND_ADD => "GC_ITEM_GROUND_ADD",
            HEADER_GC_ITEM_GROUND_DEL => "GC_ITEM_GROUND_DEL",
            HEADER_GC_QUICKSLOT_ADD => "GC_QUICKSLOT_ADD",
            HEADER_GC_QUICKSLOT_DEL => "GC_QUICKSLOT_DEL",
            HEADER_GC_QUICKSLOT_SWAP => "GC_QUICKSLOT_SWAP",
            HEADER_GC_MOTION => "GC_MOTION",
            HEADER_GC_SHOP => "GC_SHOP",
            HEADER_GC_SHOP_SIGN => "GC_SHOP_SIGN",
            HEADER_GC_DUEL_START => "GC_DUEL_START",
            HEADER_GC_PVP => "GC_PVP",
            HEADER_GC_EXCHANGE => "GC_EXCHANGE",
            HEADER_GC_CHARACTER_POSITION => "GC_CHARACTER_POSITION",
            HEADER_GC_PING => "GC_PING",
            HEADER_GC_SCRIPT => "GC_SCRIPT",
            HEADER_GC_QUEST_CONFIRM => "GC_QUEST_CONFIRM",
            HEADER_GC_MOUNT => "GC_MOUNT",
            HEADER_GC_OWNERSHIP => "GC_OWNERSHIP",
            HEADER_GC_TARGET => "GC_TARGET",
            HEADER_GC_WARP => "GC_WARP",
            HEADER_GC_ADD_FLY_TARGETING => "GC_ADD_FLY_TARGETING",
            HEADER_GC_CREATE_FLY => "GC_CREATE_FLY",
            HEADER_GC_FLY_TARGETING => "GC_FLY_TARGETING",
            HEADER_GC_SKILL_LEVEL => "GC_SKILL_LEVEL",
            HEADER_GC_MESSENGER => "GC_MESSENGER",
            HEADER_GC_GUILD => "GC_GUILD",
            HEADER_GC_PARTY_INVITE => "GC_PARTY_INVITE",
            HEADER_GC_PARTY_ADD => "GC_PARTY_ADD",
            HEADER_GC_PARTY_UPDATE => "GC_PARTY_UPDATE",
            HEADER_GC_PARTY_REMOVE => "GC_PARTY_REMOVE",
            HEADER_GC_PARTY_LINK => "GC_PARTY_LINK",
            HEADER_GC_PARTY_UNLINK => "GC_PARTY_UNLINK",
            HEADER_GC_PARTY_PARAMETER => "GC_PARTY_PARAMETER",
            HEADER_GC_SAFEBOX_SET => "GC_SAFEBOX_SET",
            HEADER_GC_SAFEBOX_DEL => "GC_SAFEBOX_DEL",
            HEADER_GC_SAFEBOX_WRONG_PASSWORD => "GC_SAFEBOX_WRONG_PASSWORD",
            HEADER_GC_SAFEBOX_SIZE => "GC_SAFEBOX_SIZE",
            HEADER_GC_FISHING => "GC_FISHING",
            HEADER_GC_DUNGEON => "GC_DUNGEON",
            HEADER_GC_TIME => "GC_TIME",
            HEADER_GC_CHANGE_NAME => "GC_CHANGE_NAME",
            HEADER_GC_AUTH_SUCCESS => "GC_AUTH_SUCCESS",
            HEADER_GC_CHANNEL => "GC_CHANNEL",
            HEADER_GC_MALL_OPEN => "GC_MALL_OPEN",
            HEADER_GC_TARGET_UPDATE => "GC_TARGET_UPDATE",
            HEADER_GC_TARGET_DELETE => "GC_TARGET_DELETE",
            HEADER_GC_TARGET_CREATE => "GC_TARGET_CREATE",
            HEADER_GC_AFFECT_ADD => "GC_AFFECT_ADD",
            HEADER_GC_AFFECT_REMOVE => "GC_AFFECT_REMOVE",
            HEADER_GC_MALL_SET => "GC_MALL_SET",
            HEADER_GC_MALL_DEL => "GC_MALL_DEL",
            HEADER_GC_LAND_LIST => "GC_LAND_LIST",
            HEADER_GC_LOVER_INFO => "GC_LOVER_INFO",
            HEADER_GC_LOVE_POINT_UPDATE => "GC_LOVE_POINT_UPDATE",
            HEADER_GC_DIG_MOTION => "GC_DIG_MOTION",
            HEADER_GC_HANDSHAKE => "GC_HANDSHAKE",
            HEADER_GC_HANDSHAKE_OK => "GC_HANDSHAKE_OK",
            HEADER_GC_HYBRIDCRYPT_KEYS => "GC_HYBRIDCRYPT_KEYS",
            HEADER_GC_HYBRIDCRYPT_SDB => "GC_HYBRIDCRYPT_SDB",
            HEADER_GC_SPECIFIC_EFFECT => "GC_SPECIFIC_EFFECT",
            HEADER_GC_DRAGON_SOUL_REFINE => "GC_DRAGON_SOUL_REFINE",
            HEADER_GC_RESPOND_CHANNELSTATUS => "GC_RESPOND_CHANNELSTATUS",
            _ => "GC_UNKNOWN",
        },
        PacketDirection::GG => match header {
            HEADER_GG_LOGIN => "GG_LOGIN",
            HEADER_GG_LOGOUT => "GG_LOGOUT",
            HEADER_GG_RELAY => "GG_RELAY",
            HEADER_GG_NOTICE => "GG_NOTICE",
            HEADER_GG_SHUTDOWN => "GG_SHUTDOWN",
            HEADER_GG_GUILD => "GG_GUILD",
            HEADER_GG_DISCONNECT => "GG_DISCONNECT",
            HEADER_GG_SHOUT => "GG_SHOUT",
            HEADER_GG_SETUP => "GG_SETUP",
            HEADER_GG_MESSENGER_ADD => "GG_MESSENGER_ADD",
            HEADER_GG_MESSENGER_REMOVE => "GG_MESSENGER_REMOVE",
            HEADER_GG_MESSENGER_MOBILE => "GG_MESSENGER_MOBILE",
            HEADER_GG_FIND_POSITION => "GG_FIND_POSITION",
            HEADER_GG_WARP_CHARACTER => "GG_WARP_CHARACTER",
            HEADER_GG_GUILD_WAR_ZONE_MAP_INDEX => "GG_GUILD_WAR_ZONE_MAP_INDEX",
            HEADER_GG_TRANSFER => "GG_TRANSFER",
            HEADER_GG_XMAS_WARP_SANTA => "GG_XMAS_WARP_SANTA",
            HEADER_GG_XMAS_WARP_SANTA_REPLY => "GG_XMAS_WARP_SANTA_REPLY",
            HEADER_GG_RELOAD_CRC_LIST => "GG_RELOAD_CRC_LIST",
            HEADER_GG_LOGIN_PING => "GG_LOGIN_PING",
            HEADER_GG_CHECK_CLIENT_VERSION => "GG_CHECK_CLIENT_VERSION",
            HEADER_GG_BLOCK_CHAT => "GG_BLOCK_CHAT",
            HEADER_GG_SIEGE => "GG_SIEGE",
            _ => "GG_UNKNOWN",
        },
    }
}

fn compare_bytes(name: &str, a: &[u8], b: &[u8]) -> Vec<String> {
    let mut diffs = Vec::new();
    if a.len() != b.len() {
        diffs.push(format!("{}: length mismatch ({} vs {})", name, a.len(), b.len()));
        return diffs;
    }
    for (i, (a_byte, b_byte)) in a.iter().zip(b.iter()).enumerate() {
        if a_byte != b_byte {
            diffs.push(format!("{}[{}]: 0x{:02x} vs 0x{:02x}", name, i, a_byte, b_byte));
        }
    }
    diffs
}

fn compare_u8(name: &str, a: u8, b: u8) -> Vec<String> {
    if a != b {
        vec![format!("{}: {} vs {}", name, a, b)]
    } else {
        vec![]
    }
}

fn compare_u16(name: &str, a: u16, b: u16) -> Vec<String> {
    if a != b {
        vec![format!("{}: {} vs {}", name, a, b)]
    } else {
        vec![]
    }
}

fn compare_u32(name: &str, a: u32, b: u32) -> Vec<String> {
    if a != b {
        vec![format!("{}: {} vs {}", name, a, b)]
    } else {
        vec![]
    }
}

fn compare_i32(name: &str, a: i32, b: i32) -> Vec<String> {
    if a != b {
        vec![format!("{}: {} vs {}", name, a, b)]
    } else {
        vec![]
    }
}

fn compare_i64(name: &str, a: i64, b: i64) -> Vec<String> {
    if a != b {
        vec![format!("{}: {} vs {}", name, a, b)]
    } else {
        vec![]
    }
}

fn compare_titempos(name: &str, a: &TItemPos, b: &TItemPos) -> Vec<String> {
    let mut diffs = Vec::new();
    diffs.extend(compare_u8(&format!("{}.window_type", name), a.window_type, b.window_type));
    diffs.extend(compare_u16(&format!("{}.cell", name), a.cell, b.cell));
    diffs
}

fn compare_tquickslot(name: &str, a: &TQuickslot, b: &TQuickslot) -> Vec<String> {
    let mut diffs = Vec::new();
    diffs.extend(compare_u8(&format!("{}.b_type", name), a.b_type, b.type_));
    diffs.extend(compare_u8(&format!("{}.b_pos", name), a.b_pos, b.b_pos));
    diffs
}

fn compare_tplayerskill(name: &str, a: &TPlayerSkill, b: &TPlayerSkill) -> Vec<String> {
    let mut diffs = Vec::new();
    diffs.extend(compare_u8(&format!("{}.b_master_type", name), a.b_master_type, b.b_master_type));
    diffs.extend(compare_u8(&format!("{}.b_level", name), a.b_level, b.b_level));
    diffs.extend(compare_i64(&format!("{}.t_next_read", name), a.t_next_read, b.t_next_read));
    diffs
}

fn compare_tplayeritemattribute(name: &str, a: &TPlayerItemAttribute, b: &TPlayerItemAttribute) -> Vec<String> {
    let mut diffs = Vec::new();
    diffs.extend(compare_u8(&format!("{}.b_type", name), a.b_type, b.b_type));
    diffs.extend(compare_i16(&format!("{}.s_value", name), a.s_value, b.s_value));
    diffs
}

fn compare_i16(name: &str, a: i16, b: i16) -> Vec<String> {
    if a != b {
        vec![format!("{}: {} vs {}", name, a, b)]
    } else {
        vec![]
    }
}

fn compare_tsimpleplayer(name: &str, a: &TSimplePlayer, b: &TSimplePlayer) -> Vec<String> {
    let mut diffs = Vec::new();
    diffs.extend(compare_u32(&format!("{}.dw_id", name), a.dw_id, b.dw_id));
    diffs.extend(compare_bytes(&format!("{}.sz_name", name), &a.sz_name, &b.sz_name));
    diffs.extend(compare_u8(&format!("{}.by_job", name), a.by_job, b.by_job));
    diffs.extend(compare_u8(&format!("{}.by_level", name), a.by_level, b.by_level));
    diffs.extend(compare_u32(&format!("{}.dw_play_minutes", name), a.dw_play_minutes, b.dw_play_minutes));
    diffs.extend(compare_u8(&format!("{}.by_st", name), a.by_st, b.by_st));
    diffs.extend(compare_u8(&format!("{}.by_ht", name), a.by_ht, b.by_ht));
    diffs.extend(compare_u8(&format!("{}.by_dx", name), a.by_dx, b.by_dx));
    diffs.extend(compare_u8(&format!("{}.by_iq", name), a.by_iq, b.by_iq));
    diffs.extend(compare_u16(&format!("{}.w_main_part", name), a.w_main_part, b.w_main_part));
    diffs.extend(compare_u8(&format!("{}.b_change_name", name), a.b_change_name, b.b_change_name));
    diffs.extend(compare_u16(&format!("{}.w_hair_part", name), a.w_hair_part, b.w_hair_part));
    diffs.extend(compare_u16(&format!("{}.w_sash_part", name), a.w_sash_part, b.w_sash_part));
    diffs.extend(compare_bytes(&format!("{}.b_dummy", name), &a.b_dummy, &b.b_dummy));
    diffs.extend(compare_i32(&format!("{}.x", name), a.x, b.x));
    diffs.extend(compare_i32(&format!("{}.y", name), a.y, b.y));
    diffs.extend(compare_i32(&format!("{}.l_addr", name), a.l_addr, b.l_addr));
    diffs.extend(compare_u16(&format!("{}.w_port", name), a.w_port, b.w_port));
    diffs.extend(compare_u8(&format!("{}.skill_group", name), a.skill_group, b.skill_group));
    diffs.extend(compare_u8(&format!("{}.by_conqueror_level", name), a.by_conqueror_level, b.by_conqueror_level));
    diffs.extend(compare_u8(&format!("{}.by_sungma_str", name), a.by_sungma_str, b.by_sungma_str));
    diffs.extend(compare_u8(&format!("{}.by_sungma_hp", name), a.by_sungma_hp, b.by_sungma_hp));
    diffs.extend(compare_u8(&format!("{}.by_sungma_move", name), a.by_sungma_move, b.by_sungma_move));
    diffs.extend(compare_u8(&format!("{}.by_sungma_immune", name), a.by_sungma_immune, b.by_sungma_immune));
    diffs
}

fn compare_cg_packet(header: u8, data1: &[u8], data2: &[u8]) -> PacketComparison {
    let packet_type = identify_packet(header, PacketDirection::CG);
    let mut differences = Vec::new();

    match header {
        HEADER_CG_HANDSHAKE => {
            if let (Ok(p1), Ok(p2)) = (
                TPacketCGHandshake::from_bytes(data1),
                TPacketCGHandshake::from_bytes(data2),
            ) {
                differences.extend(compare_u8("b_header", p1.b_header, p2.b_header));
                differences.extend(compare_u32("dw_handshake", p1.dw_handshake, p2.dw_handshake));
                differences.extend(compare_u32("dw_time", p1.dw_time, p2.dw_time));
                differences.extend(compare_i32("l_delta", p1.l_delta, p2.l_delta));
            } else {
                differences.push("Failed to parse packet".to_string());
            }
        }
        HEADER_CG_LOGIN => {
            if let (Ok(p1), Ok(p2)) = (
                TPacketCGLogin::from_bytes(data1),
                TPacketCGLogin::from_bytes(data2),
            ) {
                differences.extend(compare_u8("header", p1.header, p2.header));
                differences.extend(compare_bytes("login", &p1.login, &p2.login));
                differences.extend(compare_bytes("passwd", &p1.passwd, &p2.passwd));
            } else {
                differences.push("Failed to parse packet".to_string());
            }
        }
        HEADER_CG_LOGIN2 => {
            if let (Ok(p1), Ok(p2)) = (
                TPacketCGLogin2::from_bytes(data1),
                TPacketCGLogin2::from_bytes(data2),
            ) {
                differences.extend(compare_u8("header", p1.header, p2.header));
                differences.extend(compare_bytes("login", &p1.login, &p2.login));
                differences.extend(compare_u32("dw_login_key", p1.dw_login_key, p2.dw_login_key));
                for i in 0..4 {
                    differences.extend(compare_u32(&format!("adw_client_key[{}]", i), p1.adw_client_key[i], p2.adw_client_key[i]));
                }
            } else {
                differences.push("Failed to parse packet".to_string());
            }
        }
        HEADER_CG_LOGIN3 => {
            if let (Ok(p1), Ok(p2)) = (
                TPacketCGLogin3::from_bytes(data1),
                TPacketCGLogin3::from_bytes(data2),
            ) {
                differences.extend(compare_u8("header", p1.header, p2.header));
                differences.extend(compare_bytes("login", &p1.login, &p2.login));
                differences.extend(compare_bytes("passwd", &p1.passwd, &p2.passwd));
                for i in 0..4 {
                    differences.extend(compare_u32(&format!("adw_client_key[{}]", i), p1.adw_client_key[i], p2.adw_client_key[i]));
                }
                differences.extend(compare_u8("b_language", p1.b_language, p2.b_language));
            } else {
                differences.push("Failed to parse packet".to_string());
            }
        }
        HEADER_CG_ATTACK => {
            if let (Ok(p1), Ok(p2)) = (
                TPacketCGAttack::from_bytes(data1),
                TPacketCGAttack::from_bytes(data2),
            ) {
                differences.extend(compare_u8("b_header", p1.b_header, p2.b_header));
                differences.extend(compare_u8("b_type", p1.b_type, p2.b_type));
                differences.extend(compare_u32("dw_vid", p1.dw_vid, p2.dw_vid));
                differences.extend(compare_u8("b_crc_magic_cube_proc_piece", p1.b_crc_magic_cube_proc_piece, p2.b_crc_magic_cube_proc_piece));
                differences.extend(compare_u8("b_crc_magic_cube_file_piece", p1.b_crc_magic_cube_file_piece, p2.b_crc_magic_cube_file_piece));
            } else {
                differences.push("Failed to parse packet".to_string());
            }
        }
        HEADER_CG_MOVE => {
            if let (Ok(p1), Ok(p2)) = (
                TPacketCGMove::from_bytes(data1),
                TPacketCGMove::from_bytes(data2),
            ) {
                differences.extend(compare_u8("b_header", p1.b_header, p2.b_header));
                differences.extend(compare_u8("b_func", p1.b_func, p2.b_func));
                differences.extend(compare_u8("b_arg", p1.b_arg, p2.b_arg));
                differences.extend(compare_u8("b_rot", p1.b_rot, p2.b_rot));
                differences.extend(compare_i32("l_x", p1.l_x, p2.l_x));
                differences.extend(compare_i32("l_y", p1.l_y, p2.l_y));
                differences.extend(compare_u32("dw_time", p1.dw_time, p2.dw_time));
            } else {
                differences.push("Failed to parse packet".to_string());
            }
        }
        HEADER_CG_PLAYER_SELECT => {
            if let (Ok(p1), Ok(p2)) = (
                TPacketCGPlayerSelect::from_bytes(data1),
                TPacketCGPlayerSelect::from_bytes(data2),
            ) {
                differences.extend(compare_u8("header", p1.header, p2.header));
                differences.extend(compare_u8("index", p1.index, p2.index));
            } else {
                differences.push("Failed to parse packet".to_string());
            }
        }
        HEADER_CG_PLAYER_DELETE => {
            if let (Ok(p1), Ok(p2)) = (
                TPacketCGPlayerDelete::from_bytes(data1),
                TPacketCGPlayerDelete::from_bytes(data2),
            ) {
                differences.extend(compare_u8("header", p1.header, p2.header));
                differences.extend(compare_u8("index", p1.index, p2.index));
                differences.extend(compare_bytes("private_code", &p1.private_code, &p2.private_code));
            } else {
                differences.push("Failed to parse packet".to_string());
            }
        }
        HEADER_CG_PLAYER_CREATE => {
            if let (Ok(p1), Ok(p2)) = (
                TPacketCGPlayerCreate::from_bytes(data1),
                TPacketCGPlayerCreate::from_bytes(data2),
            ) {
                differences.extend(compare_u8("header", p1.header, p2.header));
                differences.extend(compare_u8("index", p1.index, p2.index));
                differences.extend(compare_bytes("name", &p1.name, &p2.name));
                differences.extend(compare_u16("job", p1.job, p2.job));
                differences.extend(compare_u8("shape", p1.shape, p2.shape));
                differences.extend(compare_u8("con", p1.con, p2.con));
                differences.extend(compare_u8("int_", p1.int_, p2.int_));
                differences.extend(compare_u8("str_", p1.str_, p2.str_));
                differences.extend(compare_u8("dex", p1.dex, p2.dex));
            } else {
                differences.push("Failed to parse packet".to_string());
            }
        }
        HEADER_CG_ITEM_USE => {
            if let (Ok(p1), Ok(p2)) = (
                TPacketCGItemUse::from_bytes(data1),
                TPacketCGItemUse::from_bytes(data2),
            ) {
                differences.extend(compare_u8("header", p1.header, p2.header));
                differences.extend(compare_titempos("cell", &p1.cell, &p2.cell));
            } else {
                differences.push("Failed to parse packet".to_string());
            }
        }
        HEADER_CG_ITEM_USE_TO_ITEM => {
            if let (Ok(p1), Ok(p2)) = (
                TPacketCGItemUseToItem::from_bytes(data1),
                TPacketCGItemUseToItem::from_bytes(data2),
            ) {
                differences.extend(compare_u8("header", p1.header, p2.header));
                differences.extend(compare_titempos("cell", &p1.cell, &p2.cell));
                differences.extend(compare_titempos("target_cell", &p1.target_cell, &p2.target_cell));
            } else {
                differences.push("Failed to parse packet".to_string());
            }
        }
        HEADER_CG_ENTERGAME => {
            if let (Ok(p1), Ok(p2)) = (
                TPacketCGEnterGame::from_bytes(data1),
                TPacketCGEnterGame::from_bytes(data2),
            ) {
                differences.extend(compare_u8("header", p1.header, p2.header));
            } else {
                differences.push("Failed to parse packet".to_string());
            }
        }
        _ => {
            differences.extend(compare_bytes("raw_data", data1, data2));
        }
    }

    PacketComparison {
        packet_type: packet_type.to_string(),
        header,
        passed: differences.is_empty(),
        differences,
    }
}

fn compare_gc_packet(header: u8, data1: &[u8], data2: &[u8]) -> PacketComparison {
    let packet_type = identify_packet(header, PacketDirection::GC);
    let mut differences = Vec::new();

    // For GC packets, we'll compare raw bytes since we don't have all the structures
    // In a full implementation, you'd add specific comparisons for each packet type
    differences.extend(compare_bytes("raw_data", data1, data2));

    PacketComparison {
        packet_type: packet_type.to_string(),
        header,
        passed: differences.is_empty(),
        differences,
    }
}

fn compare_gg_packet(header: u8, data1: &[u8], data2: &[u8]) -> PacketComparison {
    let packet_type = identify_packet(header, PacketDirection::GG);
    let mut differences = Vec::new();

    // For GG packets, we'll compare raw bytes since we don't have all the structures
    differences.extend(compare_bytes("raw_data", data1, data2));

    PacketComparison {
        packet_type: packet_type.to_string(),
        header,
        passed: differences.is_empty(),
        differences,
    }
}

fn parse_packets(data: &[u8], direction: PacketDirection) -> Vec<(u8, Vec<u8>)> {
    let mut packets = Vec::new();
    let mut offset = 0;

    while offset < data.len() {
        if offset >= data.len() {
            break;
        }

        let header = data[offset];
        offset += 1;

        // Determine packet size based on header
        let packet_size = match direction {
            PacketDirection::CG => match header {
                HEADER_CG_HANDSHAKE => TPacketCGHandshake::packed_size() - 1,
                HEADER_CG_LOGIN => TPacketCGLogin::packed_size() - 1,
                HEADER_CG_LOGIN2 => TPacketCGLogin2::packed_size() - 1,
                HEADER_CG_LOGIN3 => TPacketCGLogin3::packed_size() - 1,
                HEADER_CG_ATTACK => TPacketCGAttack::packed_size() - 1,
                HEADER_CG_MOVE => TPacketCGMove::packed_size() - 1,
                HEADER_CG_PLAYER_SELECT => TPacketCGPlayerSelect::packed_size() - 1,
                HEADER_CG_PLAYER_DELETE => TPacketCGPlayerDelete::packed_size() - 1,
                HEADER_CG_PLAYER_CREATE => TPacketCGPlayerCreate::packed_size() - 1,
                HEADER_CG_ITEM_USE => TPacketCGItemUse::packed_size() - 1,
                HEADER_CG_ITEM_USE_TO_ITEM => TPacketCGItemUseToItem::packed_size() - 1,
                HEADER_CG_ENTERGAME => TPacketCGEnterGame::packed_size() - 1,
                HEADER_CG_CHAT => {
                    // Variable length: read size from next 2 bytes
                    if offset + 2 > data.len() {
                        break;
                    }
                    let size = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
                    size
                }
                HEADER_CG_WHISPER => {
                    // Variable length: read size from next 2 bytes
                    if offset + 2 > data.len() {
                        break;
                    }
                    let size = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
                    size
                }
                HEADER_CG_SYNC_POSITION => {
                    // Variable length: read size from next 2 bytes
                    if offset + 2 > data.len() {
                        break;
                    }
                    let size = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
                    size
                }
                _ => {
                    // Unknown packet, try to read next header
                    // This is a heuristic - in practice you'd need proper packet boundaries
                    0
                }
            },
            PacketDirection::GC => {
                // For GC packets, we'll use a simple heuristic
                // In practice, you'd need proper packet size definitions
                match header {
                    HEADER_GC_CHAT | HEADER_GC_WHISPER => {
                        if offset + 2 > data.len() {
                            break;
                        }
                        let size = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
                        size
                    }
                    _ => 0,
                }
            }
            PacketDirection::GG => 0,
        };

        if packet_size == 0 {
            // Unknown packet size, skip
            break;
        }

        if offset + packet_size > data.len() {
            break;
        }

        let packet_data = data[offset..offset + packet_size].to_vec();
        packets.push((header, packet_data));
        offset += packet_size;
    }

    packets
}

fn compare_captures(data1: &[u8], data2: &[u8], direction: PacketDirection, verbose: bool) -> Vec<PacketComparison> {
    let packets1 = parse_packets(data1, direction);
    let packets2 = parse_packets(data2, direction);

    let mut results = Vec::new();
    let max_packets = packets1.len().max(packets2.len());

    for i in 0..max_packets {
        match (packets1.get(i), packets2.get(i)) {
            (Some((h1, d1)), Some((h2, d2))) => {
                if h1 != h2 {
                    results.push(PacketComparison {
                        packet_type: format!("MISMATCH (0x{:02x} vs 0x{:02x})", h1, h2),
                        header: *h1,
                        passed: false,
                        differences: vec![format!("Header mismatch at packet {}", i)],
                    });
                } else {
                    let comparison = match direction {
                        PacketDirection::CG => compare_cg_packet(*h1, d1, d2),
                        PacketDirection::GC => compare_gc_packet(*h1, d1, d2),
                        PacketDirection::GG => compare_gg_packet(*h1, d1, d2),
                    };
                    results.push(comparison);
                }
            }
            (Some((h, _)), None) => {
                results.push(PacketComparison {
                    packet_type: identify_packet(*h, direction).to_string(),
                    header: *h,
                    passed: false,
                    differences: vec!["Packet only in first capture".to_string()],
                });
            }
            (None, Some((h, _))) => {
                results.push(PacketComparison {
                    packet_type: identify_packet(*h, direction).to_string(),
                    header: *h,
                    passed: false,
                    differences: vec!["Packet only in second capture".to_string()],
                });
            }
            (None, None) => break,
        }
    }

    results
}

fn main() -> io::Result<()> {
    let args = Args::parse();

    let data1 = fs::read(&args.file1)?;
    let data2 = fs::read(&args.file2)?;

    println!("Comparing packet captures:");
    println!("  File 1: {} ({} bytes)", args.file1.display(), data1.len());
    println!("  File 2: {} ({} bytes)", args.file2.display(), data2.len());
    println!();

    // Try all three directions
    let directions = [
        ("Client → Game (CG)", PacketDirection::CG),
        ("Game → Client (GC)", PacketDirection::GC),
        ("Game → Game (GG)", PacketDirection::GG),
    ];

    let mut total_passed = 0;
    let mut total_failed = 0;

    for (dir_name, direction) in &directions {
        let results = compare_captures(&data1, &data2, *direction, args.verbose);

        if results.is_empty() {
            continue;
        }

        println!("=== {} ===", dir_name);

        for result in &results {
            if result.passed {
                total_passed += 1;
                if args.verbose {
                    println!("  ✓ {} (header: 0x{:02x})", result.packet_type, result.header);
                }
            } else {
                total_failed += 1;
                println!("  ✗ {} (header: 0x{:02x})", result.packet_type, result.header);
                for diff in &result.differences {
                    println!("    - {}", diff);
                }
            }
        }
        println!();
    }

    println!("Summary:");
    println!("  Passed: {}", total_passed);
    println!("  Failed: {}", total_failed);
    println!("  Total:  {}", total_passed + total_failed);

    if total_failed > 0 {
        std::process::exit(1);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identify_cg_packets() {
        assert_eq!(identify_packet(HEADER_CG_LOGIN, PacketDirection::CG), "CG_LOGIN");
        assert_eq!(identify_packet(HEADER_CG_ATTACK, PacketDirection::CG), "CG_ATTACK");
        assert_eq!(identify_packet(HEADER_CG_MOVE, PacketDirection::CG), "CG_MOVE");
    }

    #[test]
    fn test_identify_gc_packets() {
        assert_eq!(identify_packet(HEADER_GC_CHARACTER_ADD, PacketDirection::GC), "GC_CHARACTER_ADD");
        assert_eq!(identify_packet(HEADER_GC_CHAT, PacketDirection::GC), "GC_CHAT");
    }

    #[test]
    fn test_identify_gg_packets() {
        assert_eq!(identify_packet(HEADER_GG_LOGIN, PacketDirection::GG), "GG_LOGIN");
        assert_eq!(identify_packet(HEADER_GG_LOGOUT, PacketDirection::GG), "GG_LOGOUT");
    }

    #[test]
    fn test_compare_bytes_equal() {
        let diffs = compare_bytes("test", &[1, 2, 3], &[1, 2, 3]);
        assert!(diffs.is_empty());
    }

    #[test]
    fn test_compare_bytes_different() {
        let diffs = compare_bytes("test", &[1, 2, 3], &[1, 2, 4]);
        assert_eq!(diffs.len(), 1);
        assert!(diffs[0].contains("test[2]"));
    }

    #[test]
    fn test_compare_u8_equal() {
        let diffs = compare_u8("test", 42, 42);
        assert!(diffs.is_empty());
    }

    #[test]
    fn test_compare_u8_different() {
        let diffs = compare_u8("test", 42, 43);
        assert_eq!(diffs.len(), 1);
    }
}
