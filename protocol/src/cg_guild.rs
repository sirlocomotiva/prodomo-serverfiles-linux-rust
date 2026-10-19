//! The `HEADER_CG_GUILD` (0x50) frame: a 2-byte base plus a subheader-gated body
//! (`TPacketCGGuild`, `server/server/game/packet.h:2211-2216`, and `GetSubPacketSize`,
//! `G/input_main.cpp:2773-2795`).
//!
//! The base is the header byte plus the subheader byte. What follows depends on the
//! subheader: 15 arms from 0 to 8 bytes, and an unknown subheader is 0 bytes (`return 0` at
//! the switch's end, `:2794`). `CInputMain::Guild` (`:2797-2814`) refuses a frame shorter
//! than the base, then a body shorter than the arm's size, so the frame width is always
//! `2 + sub_packet_size(subheader)`.
//!
//! Only the sizes live here. The world ports each arm as its system lands; the sizes are
//! the dispatch contract every arm shares.

use crate::cg_inventory::{CgHeader, HEADER_CG_GUILD};

/// The base width: the header byte plus the subheader byte (`TPacketCGGuild`).
pub const CG_GUILD_BASE_WIRE_SIZE: usize = 2;

/// `GUILD_SUBHEADER_CG_ADD_MEMBER` (`packet.h:2167`): 0.
pub const GUILD_SUBHEADER_CG_ADD_MEMBER: u8 = 0;
/// `GUILD_SUBHEADER_CG_REMOVE_MEMBER` (`packet.h:2168`): 1.
pub const GUILD_SUBHEADER_CG_REMOVE_MEMBER: u8 = 1;
/// `GUILD_SUBHEADER_CG_CHANGE_GRADE_NAME` (`packet.h:2169`): 2.
pub const GUILD_SUBHEADER_CG_CHANGE_GRADE_NAME: u8 = 2;
/// `GUILD_SUBHEADER_CG_CHANGE_GRADE_AUTHORITY` (`packet.h:2170`): 3.
pub const GUILD_SUBHEADER_CG_CHANGE_GRADE_AUTHORITY: u8 = 3;
/// `GUILD_SUBHEADER_CG_OFFER` (`packet.h:2171`): 4.
pub const GUILD_SUBHEADER_CG_OFFER: u8 = 4;
/// `GUILD_SUBHEADER_CG_POST_COMMENT` (`packet.h:2172`): 5.
pub const GUILD_SUBHEADER_CG_POST_COMMENT: u8 = 5;
/// `GUILD_SUBHEADER_CG_DELETE_COMMENT` (`packet.h:2173`): 6.
pub const GUILD_SUBHEADER_CG_DELETE_COMMENT: u8 = 6;
/// `GUILD_SUBHEADER_CG_REFRESH_COMMENT` (`packet.h:2174`): 7.
pub const GUILD_SUBHEADER_CG_REFRESH_COMMENT: u8 = 7;
/// `GUILD_SUBHEADER_CG_CHANGE_MEMBER_GRADE` (`packet.h:2175`): 8.
pub const GUILD_SUBHEADER_CG_CHANGE_MEMBER_GRADE: u8 = 8;
/// `GUILD_SUBHEADER_CG_USE_SKILL` (`packet.h:2176`): 9.
pub const GUILD_SUBHEADER_CG_USE_SKILL: u8 = 9;
/// `GUILD_SUBHEADER_CG_CHANGE_MEMBER_GENERAL` (`packet.h:2177`): 10.
pub const GUILD_SUBHEADER_CG_CHANGE_MEMBER_GENERAL: u8 = 10;
/// `GUILD_SUBHEADER_CG_GUILD_INVITE_ANSWER` (`packet.h:2178`): 11.
pub const GUILD_SUBHEADER_CG_GUILD_INVITE_ANSWER: u8 = 11;
/// `GUILD_SUBHEADER_CG_CHARGE_GSP` (`packet.h:2179`): 12.
pub const GUILD_SUBHEADER_CG_CHARGE_GSP: u8 = 12;
/// `GUILD_SUBHEADER_CG_DEPOSIT_MONEY` (`packet.h:2180`): 13.
pub const GUILD_SUBHEADER_CG_DEPOSIT_MONEY: u8 = 13;
/// `GUILD_SUBHEADER_CG_WITHDRAW_MONEY` (`packet.h:2181`): 14.
pub const GUILD_SUBHEADER_CG_WITHDRAW_MONEY: u8 = 14;

/// `GetSubPacketSize` (`G/input_main.cpp:2773-2795`): the body width after the 2-byte base
/// for `subheader`. Unknown subheaders are 0 bytes (`:2794`).
#[must_use]
pub const fn sub_packet_size(subheader: u8) -> usize {
    match subheader {
        // `sizeof(int)` / `sizeof(DWORD)`: six arms share the 4-byte body
        // (`G/input_main.cpp:2776-2783`).
        GUILD_SUBHEADER_CG_DEPOSIT_MONEY
        | GUILD_SUBHEADER_CG_WITHDRAW_MONEY
        | GUILD_SUBHEADER_CG_ADD_MEMBER
        | GUILD_SUBHEADER_CG_REMOVE_MEMBER
        | GUILD_SUBHEADER_CG_OFFER
        | GUILD_SUBHEADER_CG_CHARGE_GSP
        | GUILD_SUBHEADER_CG_DELETE_COMMENT => 4,
        GUILD_SUBHEADER_CG_CHANGE_GRADE_NAME => 10,
        GUILD_SUBHEADER_CG_CHANGE_GRADE_AUTHORITY => 2,
        GUILD_SUBHEADER_CG_POST_COMMENT => 1,
        // `sizeof(DWORD) + sizeof(BYTE)`: three arms share the 5-byte body (`:2787-2791`).
        GUILD_SUBHEADER_CG_CHANGE_MEMBER_GRADE
        | GUILD_SUBHEADER_CG_CHANGE_MEMBER_GENERAL
        | GUILD_SUBHEADER_CG_GUILD_INVITE_ANSWER => 5,
        GUILD_SUBHEADER_CG_USE_SKILL => 8,
        // `GUILD_SUBHEADER_CG_REFRESH_COMMENT` and the unknowns are 0 (`:2786`, `:2794`).
        _ => 0,
    }
}

/// The legacy header byte.
#[must_use]
pub const fn header() -> CgHeader {
    HEADER_CG_GUILD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_arm_matches_get_sub_packet_size() {
        // In `GUILD_SUBHEADER_CG` order (`packet.h:2166-2184`), with the
        // `GetSubPacketSize` arm beside each (`G/input_main.cpp:2777-2791`).
        let arms = [
            (GUILD_SUBHEADER_CG_ADD_MEMBER, 4),
            (GUILD_SUBHEADER_CG_REMOVE_MEMBER, 4),
            (GUILD_SUBHEADER_CG_CHANGE_GRADE_NAME, 10),
            (GUILD_SUBHEADER_CG_CHANGE_GRADE_AUTHORITY, 2),
            (GUILD_SUBHEADER_CG_OFFER, 4),
            (GUILD_SUBHEADER_CG_POST_COMMENT, 1),
            (GUILD_SUBHEADER_CG_DELETE_COMMENT, 4),
            (GUILD_SUBHEADER_CG_REFRESH_COMMENT, 0),
            (GUILD_SUBHEADER_CG_CHANGE_MEMBER_GRADE, 5),
            (GUILD_SUBHEADER_CG_USE_SKILL, 8),
            (GUILD_SUBHEADER_CG_CHANGE_MEMBER_GENERAL, 5),
            (GUILD_SUBHEADER_CG_GUILD_INVITE_ANSWER, 5),
            (GUILD_SUBHEADER_CG_CHARGE_GSP, 4),
            (GUILD_SUBHEADER_CG_DEPOSIT_MONEY, 4),
            (GUILD_SUBHEADER_CG_WITHDRAW_MONEY, 4),
        ];
        for (subheader, size) in arms {
            assert_eq!(sub_packet_size(subheader), size, "subheader {subheader}");
        }
        assert_eq!(sub_packet_size(15), 0, "unknown subheaders are 0 bytes");
        assert_eq!(sub_packet_size(255), 0, "unknown subheaders are 0 bytes");
    }
}
