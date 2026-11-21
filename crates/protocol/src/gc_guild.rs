//! The `HEADER_GC_GUILD` (75) records: the guild envelope and the creation walk's five
//! sub-packets (`server/server/game/packet.h:2186-2200`, `server/server/game/guild.h:45-82`).
//!
//! Every record here is a `#pragma pack(1)` structure. The envelope is 4 bytes — header,
//! WORD size, subheader (`TPacketGCGuild`, `packet.h:2186-2191`) — and `size` counts the
//! envelope itself. The creation walk sends five of them: the `GUILD_NAME` broadcast to every
//! PC (`G/guild.cpp:32-56`), then the master's four self packets — grades, INFO, LIST and
//! skills — from `LoginMember` (`G/guild.cpp:270-296`).
//!
//! The subheader values are the `GUILD_SUBHEADER_GC_*` enum (`packet.h:2143-2165`), which
//! starts at 0: `LOGIN` is 0, `LOGOUT` 1, `LIST` 2, `GRADE` 3, `ADD` 4, `REMOVE` 5,
//! `GRADE_NAME` 6, `GRADE_AUTH` 7, `INFO` 8, `COMMENTS` 9, `CHANGE_EXP` 10,
//! `CHANGE_MEMBER_GRADE` 11, `SKILL_INFO` 12, `CHANGE_MEMBER_GENERAL` 13, `GUILD_INVITE` 14,
//! `WAR` 15, `GUILD_NAME` 16, `GUILD_WAR_LIST` 17, `WAR_END_LIST` 18, `WAR_SCORE` 19.

use std::fmt;

/// Wire byte of `HEADER_GC_GUILD` (75).
pub const HEADER_GC_GUILD: u8 = 0x4b;

/// `GUILD_SUBHEADER_GC_LOGIN` (`packet.h:2144`): 0.
pub const GUILD_SUBHEADER_GC_LOGIN: u8 = 0;
/// `GUILD_SUBHEADER_GC_LOGOUT` (`packet.h:2145`): 1.
pub const GUILD_SUBHEADER_GC_LOGOUT: u8 = 1;
/// `GUILD_SUBHEADER_GC_LIST` (`packet.h:2146`): 2.
pub const GUILD_SUBHEADER_GC_LIST: u8 = 2;
/// `GUILD_SUBHEADER_GC_GRADE` (`packet.h:2147`): 3.
pub const GUILD_SUBHEADER_GC_GRADE: u8 = 3;
/// `GUILD_SUBHEADER_GC_REMOVE` (`packet.h:2149`): 5.
pub const GUILD_SUBHEADER_GC_REMOVE: u8 = 5;
/// `GUILD_SUBHEADER_GC_GRADE_NAME` (`packet.h:2150`): 6.
pub const GUILD_SUBHEADER_GC_GRADE_NAME: u8 = 6;
/// `GUILD_SUBHEADER_GC_GRADE_AUTH` (`packet.h:2151`): 7.
pub const GUILD_SUBHEADER_GC_GRADE_AUTH: u8 = 7;
/// `GUILD_SUBHEADER_GC_INFO` (`packet.h:2152`): 8.
pub const GUILD_SUBHEADER_GC_INFO: u8 = 8;
/// `GUILD_SUBHEADER_GC_COMMENTS` (`packet.h:2153`): 9.
pub const GUILD_SUBHEADER_GC_COMMENTS: u8 = 9;
/// `GUILD_SUBHEADER_GC_CHANGE_EXP` (`packet.h:2154`): 10.
pub const GUILD_SUBHEADER_GC_CHANGE_EXP: u8 = 10;
/// `GUILD_SUBHEADER_GC_CHANGE_MEMBER_GRADE` (`packet.h:2155`): 11.
pub const GUILD_SUBHEADER_GC_CHANGE_MEMBER_GRADE: u8 = 11;
/// `GUILD_SUBHEADER_GC_SKILL_INFO` (`packet.h:2156`): 12.
pub const GUILD_SUBHEADER_GC_SKILL_INFO: u8 = 12;
/// `GUILD_SUBHEADER_GC_CHANGE_MEMBER_GENERAL` (`packet.h:2157`): 13.
pub const GUILD_SUBHEADER_GC_CHANGE_MEMBER_GENERAL: u8 = 13;
/// `GUILD_SUBHEADER_GC_GUILD_INVITE` (`packet.h:2158`): 14.
pub const GUILD_SUBHEADER_GC_GUILD_INVITE: u8 = 14;
/// `GUILD_SUBHEADER_GC_WAR` (`packet.h:2159`): 15.
pub const GUILD_SUBHEADER_GC_WAR: u8 = 15;
/// `GUILD_SUBHEADER_GC_GUILD_NAME` (`packet.h:2160`): 16.
pub const GUILD_SUBHEADER_GC_GUILD_NAME: u8 = 16;
/// `GUILD_SUBHEADER_GC_GUILD_WAR_LIST` (`packet.h:2161`): 17.
pub const GUILD_SUBHEADER_GC_GUILD_WAR_LIST: u8 = 17;
/// `GUILD_SUBHEADER_GC_WAR_END_LIST` (`packet.h:2162`): 18.
pub const GUILD_SUBHEADER_GC_WAR_END_LIST: u8 = 18;
/// `GUILD_SUBHEADER_GC_WAR_SCORE` (`packet.h:2163`): 19.
pub const GUILD_SUBHEADER_GC_WAR_SCORE: u8 = 19;

/// `GUILD_NAME_MAX_LEN` (`server/server/common/length.h:42`): 12.
///
/// The creation broadcast writes exactly 12 name bytes (`FGuildNameSender`,
/// `G/guild.cpp:32-56`); the stored name field is 13 (`+ 1` for the NUL the client sends).
pub const GUILD_NAME_MAX_LEN: usize = 12;
/// The stored guild-name field: `GUILD_NAME_MAX_LEN + 1` (13). The INFO packet's name is
/// this wide (`TPacketGCGuildInfo`, `guild.h:59-72`).
pub const GUILD_NAME_FIELD_SIZE: usize = GUILD_NAME_MAX_LEN + 1;
/// `GUILD_GRADE_NAME_MAX_LEN` (`G/guild.h:10`): 8. The grade name field is 9.
pub const GUILD_GRADE_NAME_MAX_LEN: usize = 8;
/// The stored grade-name field: `GUILD_GRADE_NAME_MAX_LEN + 1` (9).
pub const GUILD_GRADE_NAME_FIELD_SIZE: usize = GUILD_GRADE_NAME_MAX_LEN + 1;
/// `GUILD_GRADE_COUNT` (`G/guild.h:11`): 15.
pub const GUILD_GRADE_COUNT: usize = 15;
/// `GUILD_SKILL_COUNT` (`G/skill.h:200`): `GUILD_SKILL_END - GUILD_SKILL_START + 1`,
/// `162 - 151 + 1 = 12`.
pub const GUILD_SKILL_COUNT: usize = 12;
/// `CHARACTER_NAME_MAX_LEN` (`server/server/common/length.h:15`): 24. The LIST member name
/// field is 25 (`TGuildMemberPacketData`, `guild.h:45-57`).
pub const GUILD_MEMBER_NAME_FIELD_SIZE: usize = 25;

/// The envelope's packed width: header + WORD size + subheader (`TPacketGCGuild`).
pub const GC_GUILD_ENVELOPE_WIRE_SIZE: usize = 4;
/// `TPacketGCGuildName`: the envelope + DWORD id + 12 name bytes (`packet.h:2192-2200`).
pub const GC_GUILD_NAME_WIRE_SIZE: usize = GC_GUILD_ENVELOPE_WIRE_SIZE + 4 + GUILD_NAME_MAX_LEN;
/// `TPacketGCGuildInfo`: 2 + 2 + 4 + 4 + 4 + 1 + 13 + 4 + 1 = 35 (`guild.h:59-72`).
pub const GC_GUILD_INFO_BODY_WIRE_SIZE: usize = 35;
/// `TGuildMemberPacketData`: 4 + 1 + 1 + 1 + 1 + 4 + 1 + 25 = 38 (`guild.h:45-57`).
pub const GC_GUILD_MEMBER_WIRE_SIZE: usize = 38;
/// `GUILD_COMMENT_MAX_LEN` (`G/guild.h:13`): 50. A comment row's content field is 51.
pub const GUILD_COMMENT_MAX_LEN: usize = 50;
/// A comment row's content field: `GUILD_COMMENT_MAX_LEN + 1` (51).
pub const GUILD_COMMENT_FIELD_SIZE: usize = GUILD_COMMENT_MAX_LEN + 1;
/// One COMMENTS row: DWORD id + 25 name bytes + 51 content bytes (`RefreshCommentForce`,
/// `G/guild.cpp:1097-1141`).
pub const GC_GUILD_COMMENT_WIRE_SIZE: usize =
    4 + GUILD_MEMBER_NAME_FIELD_SIZE + GUILD_COMMENT_FIELD_SIZE;
/// `TGuildGrade`: 9 + 1 = 10 (`guild.h:74-77`).
pub const GC_GUILD_GRADE_WIRE_SIZE: usize = 10;
/// `TPacketGCGuildWar`: the envelope + DWORD self + DWORD opp + type + state = 14
/// (`packet.h:2201-2208`).
pub const GC_GUILD_WAR_WIRE_SIZE: usize = GC_GUILD_ENVELOPE_WIRE_SIZE + 10;

/// One member row of a LIST packet (`TGuildMemberPacketData`, `guild.h:45-57`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcGuildMember {
    /// `pid`: the player's id.
    pub pid: u32,
    /// `grade`: 1 to 15.
    pub grade: u8,
    /// `is_general`.
    pub is_general: u8,
    /// `job`.
    pub job: u8,
    /// `level`.
    pub level: u8,
    /// `offer`: the offered experience.
    pub offer: u32,
    /// `name_flag`: legacy writes `_dummy = 1` before sending (`G/guild.cpp:411,444`).
    pub name_flag: u8,
    /// The raw `char[CHARACTER_NAME_MAX_LEN + 1]` field, NUL padding included.
    pub name: [u8; GUILD_MEMBER_NAME_FIELD_SIZE],
}

impl GcGuildMember {
    /// Append the 38 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.pid.to_le_bytes());
        out.push(self.grade);
        out.push(self.is_general);
        out.push(self.job);
        out.push(self.level);
        out.extend_from_slice(&self.offer.to_le_bytes());
        out.push(self.name_flag);
        out.extend_from_slice(&self.name);
    }
}

/// The INFO packet's body (`TPacketGCGuildInfo`, `guild.h:59-72`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcGuildInfo {
    /// `member_count` (`GetMemberCount`).
    pub member_count: u16,
    /// `max_member_count` (`GetMaxMemberCount`: `32 + 2 * (level - 1) + bonus`,
    /// `G/guild.cpp:1870-1878`).
    pub max_member_count: u16,
    /// `guild_id`.
    pub guild_id: u32,
    /// `master_pid`.
    pub master_pid: u32,
    /// `exp`.
    pub exp: u32,
    /// `level`.
    pub level: u8,
    /// The raw `char[GUILD_NAME_MAX_LEN + 1]` field, NUL padding included.
    pub name: [u8; GUILD_NAME_FIELD_SIZE],
    /// `gold`.
    pub gold: u32,
    /// `has_land` (`HasLand`, `G/guild.cpp:1990-1993`).
    pub has_land: u8,
}

impl GcGuildInfo {
    /// Append the 35 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.member_count.to_le_bytes());
        out.extend_from_slice(&self.max_member_count.to_le_bytes());
        out.extend_from_slice(&self.guild_id.to_le_bytes());
        out.extend_from_slice(&self.master_pid.to_le_bytes());
        out.extend_from_slice(&self.exp.to_le_bytes());
        out.push(self.level);
        out.extend_from_slice(&self.name);
        out.extend_from_slice(&self.gold.to_le_bytes());
        out.push(self.has_land);
    }
}

/// One grade row of a GRADE packet (`TGuildGrade`, `guild.h:74-77`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcGuildGrade {
    /// The raw `char[GUILD_GRADE_NAME_MAX_LEN + 1]` field, NUL padding included.
    pub name: [u8; GUILD_GRADE_NAME_FIELD_SIZE],
    /// `auth_flag`.
    pub auth: u8,
}

impl GcGuildGrade {
    /// Append the 10 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.name);
        out.push(self.auth);
    }
}

/// The `SKILL_INFO` packet's body (`SendSkillInfoPacket`, `G/guild.cpp:1419-1437`): the skill
/// point byte, the 12 skill bytes, then power and max power as 2 bytes each — the struct's
/// `int` halves, `BufferedPacket(&m_data.power, 2)` and `Packet(&m_data.max_power, 2)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcGuildSkillInfo {
    /// `skill_point`.
    pub skill_point: u8,
    /// `abySkill[GUILD_SKILL_COUNT]`.
    pub skills: [u8; GUILD_SKILL_COUNT],
    /// The low 2 bytes of `power`.
    pub power: u16,
    /// The low 2 bytes of `max_power`.
    pub max_power: u16,
}

impl GcGuildSkillInfo {
    /// Append the 17 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.skill_point);
        out.extend_from_slice(&self.skills);
        out.extend_from_slice(&self.power.to_le_bytes());
        out.extend_from_slice(&self.max_power.to_le_bytes());
    }
}

/// Pad `name` with NULs to a wire name field. Bytes past the field are dropped, as `strlcpy`
/// into the packet struct drops them.
fn pad<const N: usize>(name: &[u8]) -> [u8; N] {
    let mut field = [0; N];
    let len = name.len().min(N);
    field[..len].copy_from_slice(&name[..len]);
    field
}

/// Append the 4-byte envelope (`TPacketGCGuild`) to `out`. `size` counts the envelope.
fn envelope(out: &mut Vec<u8>, subheader: u8, size: usize) {
    out.push(HEADER_GC_GUILD);
    out.extend_from_slice(&u16::try_from(size).unwrap_or(u16::MAX).to_le_bytes());
    out.push(subheader);
}

/// The `GUILD_NAME` creation broadcast (`FGuildNameSender`, `G/guild.cpp:32-56`): the
/// envelope, the DWORD id, and the 12 raw name bytes (no NUL room: `Packet(name,
/// GUILD_NAME_MAX_LEN)`).
#[must_use]
pub fn guild_name_record(id: u32, name: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(GC_GUILD_NAME_WIRE_SIZE);
    envelope(
        &mut out,
        GUILD_SUBHEADER_GC_GUILD_NAME,
        GC_GUILD_NAME_WIRE_SIZE,
    );
    out.extend_from_slice(&id.to_le_bytes());
    let padded: [u8; GUILD_NAME_MAX_LEN] = pad(name);
    out.extend_from_slice(&padded);
    out
}

/// The LOGIN packet for `pid` (`SendLoginPacket`, `G/guild.cpp:491-514`): the envelope
/// plus the DWORD.
#[must_use]
pub fn guild_login_record(pid: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(GC_GUILD_ENVELOPE_WIRE_SIZE + 4);
    envelope(
        &mut out,
        GUILD_SUBHEADER_GC_LOGIN,
        GC_GUILD_ENVELOPE_WIRE_SIZE + 4,
    );
    out.extend_from_slice(&pid.to_le_bytes());
    out
}

/// The LOGOUT packet for `pid` (`SendLogoutPacket`, `G/guild.cpp:522-545`): the envelope
/// plus the DWORD.
#[must_use]
pub fn guild_logout_record(pid: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(GC_GUILD_ENVELOPE_WIRE_SIZE + 4);
    envelope(
        &mut out,
        GUILD_SUBHEADER_GC_LOGOUT,
        GC_GUILD_ENVELOPE_WIRE_SIZE + 4,
    );
    out.extend_from_slice(&pid.to_le_bytes());
    out
}

/// The GRADE self packet (`SendAllGradePacket`, `G/guild.cpp:359-384`): the envelope, the
/// count byte 15, then 15 × (grade byte + grade struct). The count is a literal (which the
/// size math proves against `GUILD_GRADE_COUNT`); the loop's `u8` count is, too.
#[must_use]
pub fn guild_grade_record(grades: &[GcGuildGrade; GUILD_GRADE_COUNT]) -> Vec<u8> {
    let size = GC_GUILD_ENVELOPE_WIRE_SIZE + 1 + GUILD_GRADE_COUNT * (1 + GC_GUILD_GRADE_WIRE_SIZE);
    let mut out = Vec::with_capacity(size);
    envelope(&mut out, GUILD_SUBHEADER_GC_GRADE, size);
    debug_assert_eq!(
        GUILD_GRADE_COUNT, 15,
        "the count byte below is the literal 15"
    );
    out.push(15);
    let mut grade_number: u8 = 1;
    for grade in grades {
        out.push(grade_number);
        grade_number = grade_number.wrapping_add(1);
        grade.encode_into(&mut out);
    }
    out
}

/// The INFO self packet (`SendGuildInfoPacket`, `G/guild.cpp:910-939`).
#[must_use]
pub fn guild_info_record(info: &GcGuildInfo) -> Vec<u8> {
    let mut out = Vec::with_capacity(GC_GUILD_ENVELOPE_WIRE_SIZE + GC_GUILD_INFO_BODY_WIRE_SIZE);
    envelope(
        &mut out,
        GUILD_SUBHEADER_GC_INFO,
        GC_GUILD_ENVELOPE_WIRE_SIZE + GC_GUILD_INFO_BODY_WIRE_SIZE,
    );
    info.encode_into(&mut out);
    out
}

/// The LIST-one fan for `member` (`OfferExp`'s fan, `G/guild.cpp:985-995`): the envelope
/// plus 13 bytes — the row's first `sizeof(DWORD) * 3 + 1` (`pid`, `grade`, `is_general`,
/// `job`), not the whole row. An honest client consumes the declared 17.
#[must_use]
pub fn guild_list_one_record(member: &GcGuildMember) -> Vec<u8> {
    let size = GC_GUILD_ENVELOPE_WIRE_SIZE + 13;
    let mut out = Vec::with_capacity(size);
    envelope(&mut out, GUILD_SUBHEADER_GC_LIST, size);
    out.extend_from_slice(&member.pid.to_le_bytes());
    out.push(member.grade);
    out.push(member.is_general);
    out.push(member.job);
    out.extend_from_slice(&member.level.to_le_bytes());
    out.extend_from_slice(&member.offer.to_le_bytes());
    out.push(member.name_flag);
    debug_assert_eq!(out.len(), size, "the LIST-one fan is 13 body bytes");
    out
}

/// The LIST self packet for `members` (`SendListPacket`, `G/guild.cpp:427-459`): the envelope
/// then one 38-byte row each, in the container's order.
#[must_use]
pub fn guild_list_record(members: &[GcGuildMember]) -> Vec<u8> {
    let size = GC_GUILD_ENVELOPE_WIRE_SIZE + members.len() * GC_GUILD_MEMBER_WIRE_SIZE;
    let mut out = Vec::with_capacity(size);
    envelope(&mut out, GUILD_SUBHEADER_GC_LIST, size);
    for member in members {
        member.encode_into(&mut out);
    }
    out
}

/// One comment row of a COMMENTS packet (`RefreshCommentForce`,
/// `G/guild.cpp:1097-1141`): the DWORD id, the 25 name bytes, then the 51 content bytes.
/// The row query orders `notice DESC, id DESC` and takes `GUILD_COMMENT_MAX_COUNT` (12),
/// so notices ride first, newest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcGuildComment {
    /// The comment row's id.
    pub id: u32,
    /// The raw `char[CHARACTER_NAME_MAX_LEN + 1]` field, NUL padding included.
    pub name: [u8; GUILD_MEMBER_NAME_FIELD_SIZE],
    /// The raw `char[GUILD_COMMENT_MAX_LEN + 1]` field, NUL padding included.
    pub content: [u8; GUILD_COMMENT_FIELD_SIZE],
}

impl GcGuildComment {
    /// Append the 80 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.id.to_le_bytes());
        out.extend_from_slice(&self.name);
        out.extend_from_slice(&self.content);
    }
}

/// The COMMENTS self packet (`RefreshCommentForce`, `G/guild.cpp:1097-1141`): the
/// envelope, the count byte, then one row each. An empty board sends the count 0.
///
/// # Panics
///
/// Panics when `comments` holds 256 rows or more; the caller caps the board at 12.
#[must_use]
pub fn guild_comments_record(comments: &[GcGuildComment]) -> Vec<u8> {
    let count = u8::try_from(comments.len()).expect("a COMMENTS packet holds under 256 rows");
    let size = GC_GUILD_ENVELOPE_WIRE_SIZE + 1 + comments.len() * GC_GUILD_COMMENT_WIRE_SIZE;
    let mut out = Vec::with_capacity(size);
    envelope(&mut out, GUILD_SUBHEADER_GC_COMMENTS, size);
    out.push(count);
    for comment in comments {
        comment.encode_into(&mut out);
    }
    out
}

/// The `CHANGE_EXP` fan (`GuildPointChange`'s exp arm, `G/guild.cpp:1640-1663`): the
/// envelope, the level byte, then the DWORD exp. It fans on every offer, even when a
/// level step fired.
#[must_use]
pub fn guild_change_exp_record(level: u8, exp: u32) -> Vec<u8> {
    let size = GC_GUILD_ENVELOPE_WIRE_SIZE + 5;
    let mut out = Vec::with_capacity(size);
    envelope(&mut out, GUILD_SUBHEADER_GC_CHANGE_EXP, size);
    out.push(level);
    out.extend_from_slice(&exp.to_le_bytes());
    out
}

/// The `SKILL_INFO` self packet (`SendSkillInfoPacket`, `G/guild.cpp:1419-1437`).
#[must_use]
pub fn guild_skill_info_record(info: &GcGuildSkillInfo) -> Vec<u8> {
    let size = GC_GUILD_ENVELOPE_WIRE_SIZE + 6 + GUILD_SKILL_COUNT;
    let mut out = Vec::with_capacity(size);
    envelope(&mut out, GUILD_SUBHEADER_GC_SKILL_INFO, size);
    info.encode_into(&mut out);
    out
}

/// The REMOVE fan for `pid` (`SendOnlineRemoveOnePacket`, `G/guild.cpp:336-355`): the
/// envelope plus the DWORD.
#[must_use]
pub fn guild_remove_record(pid: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(GC_GUILD_ENVELOPE_WIRE_SIZE + 4);
    envelope(
        &mut out,
        GUILD_SUBHEADER_GC_REMOVE,
        GC_GUILD_ENVELOPE_WIRE_SIZE + 4,
    );
    out.extend_from_slice(&pid.to_le_bytes());
    out
}

/// The invite to the invitee (`CGuild::Invite`, `G/guild.cpp:2102-2112`): the envelope,
/// the DWORD guild id, and the 13 name bytes (`GUILD_NAME_MAX_LEN + 1` — one wider
/// than the broadcast's 12).
#[must_use]
pub fn guild_invite_record(id: u32, name: &[u8]) -> Vec<u8> {
    let size = GC_GUILD_ENVELOPE_WIRE_SIZE + 4 + GUILD_NAME_FIELD_SIZE;
    let mut out = Vec::with_capacity(size);
    envelope(&mut out, GUILD_SUBHEADER_GC_GUILD_INVITE, size);
    out.extend_from_slice(&id.to_le_bytes());
    let padded: [u8; GUILD_NAME_FIELD_SIZE] = pad(name);
    out.extend_from_slice(&padded);
    out
}

/// The grade-name fan (`CGuild::ChangeGradeName`, `G/guild.cpp:836-857`): the envelope,
/// the 1-based grade byte, and the 9 name bytes (`TOneGradeNamePacket`,
/// `G/guild.h:79-83`).
#[must_use]
pub fn guild_grade_name_record(grade: u8, name: &[u8]) -> Vec<u8> {
    let size = GC_GUILD_ENVELOPE_WIRE_SIZE + 1 + GUILD_GRADE_NAME_FIELD_SIZE;
    let mut out = Vec::with_capacity(size);
    envelope(&mut out, GUILD_SUBHEADER_GC_GRADE_NAME, size);
    out.push(grade);
    let padded: [u8; GUILD_GRADE_NAME_FIELD_SIZE] = pad(name);
    out.extend_from_slice(&padded);
    out
}

/// The grade-auth fan (`CGuild::ChangeGradeAuth`, `G/guild.cpp:891-913`): the envelope,
/// the 1-based grade byte, and the auth byte (`TOneGradeAuthPacket`,
/// `G/guild.h:85-89`).
#[must_use]
pub fn guild_grade_auth_record(grade: u8, auth: u8) -> Vec<u8> {
    let size = GC_GUILD_ENVELOPE_WIRE_SIZE + 2;
    let mut out = Vec::with_capacity(size);
    envelope(&mut out, GUILD_SUBHEADER_GC_GRADE_AUTH, size);
    out.push(grade);
    out.push(auth);
    out
}

/// The member-grade fan (`CGuild::ChangeMemberGrade`, `G/guild.cpp:1199-1217`): the
/// envelope, the DWORD pid, and the grade byte.
#[must_use]
pub fn guild_change_member_grade_record(pid: u32, grade: u8) -> Vec<u8> {
    let size = GC_GUILD_ENVELOPE_WIRE_SIZE + 5;
    let mut out = Vec::with_capacity(size);
    envelope(&mut out, GUILD_SUBHEADER_GC_CHANGE_MEMBER_GRADE, size);
    out.extend_from_slice(&pid.to_le_bytes());
    out.push(grade);
    out
}

/// The member-general fan (`CGuild::ChangeMemberGeneral`, `G/guild.cpp:1168-1186`): the
/// envelope, the DWORD pid, and the 0/1 byte.
#[must_use]
pub fn guild_change_member_general_record(pid: u32, is_general: u8) -> Vec<u8> {
    let size = GC_GUILD_ENVELOPE_WIRE_SIZE + 5;
    let mut out = Vec::with_capacity(size);
    envelope(&mut out, GUILD_SUBHEADER_GC_CHANGE_MEMBER_GENERAL, size);
    out.extend_from_slice(&pid.to_le_bytes());
    out.push(is_general);
    out
}

/// The war fan (`CGuild::GuildWarPacket`, `G/guild_war.cpp:28-58`): the envelope plus
/// `TPacketGCGuildWar` — the DWORD self id, the DWORD opp id, the war type byte, and
/// the war state byte (`packet.h:2201-2208`).
#[must_use]
pub fn guild_war_record(self_id: u32, opp_id: u32, war_type: u8, war_state: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(GC_GUILD_WAR_WIRE_SIZE);
    envelope(&mut out, GUILD_SUBHEADER_GC_WAR, GC_GUILD_WAR_WIRE_SIZE);
    out.extend_from_slice(&self_id.to_le_bytes());
    out.extend_from_slice(&opp_id.to_le_bytes());
    out.push(war_type);
    out.push(war_state);
    out
}

/// The `GUILD_WAR_LIST` send (`FSendWarList` and `CGuildManager::SendGuildWar`,
/// `G/guild_manager.cpp:597-823`): the envelope plus one (lo, hi) DWORD pair per war, in
/// the pairs' ascending order. The enter send carries every live pair; the start fan
/// carries one.
#[must_use]
pub fn guild_war_list_record(pairs: &[(u32, u32)]) -> Vec<u8> {
    let size = GC_GUILD_ENVELOPE_WIRE_SIZE + pairs.len() * 8;
    let mut out = Vec::with_capacity(size);
    envelope(&mut out, GUILD_SUBHEADER_GC_GUILD_WAR_LIST, size);
    for &(lo, hi) in pairs {
        out.extend_from_slice(&lo.to_le_bytes());
        out.extend_from_slice(&hi.to_le_bytes());
    }
    out
}

/// The `WAR_SCORE` send (`CGuild::SetWarScoreAgainstTo`'s field arm,
/// `G/guild_war.cpp:198-217`): the envelope, the gaining guild's DWORD, the opp's DWORD,
/// and the new total as a 4-byte `long`.
#[must_use]
pub fn guild_war_score_record(gain: u32, opp: u32, score: u32) -> Vec<u8> {
    let size = GC_GUILD_ENVELOPE_WIRE_SIZE + 12;
    let mut out = Vec::with_capacity(size);
    envelope(&mut out, GUILD_SUBHEADER_GC_WAR_SCORE, size);
    out.extend_from_slice(&gain.to_le_bytes());
    out.extend_from_slice(&opp.to_le_bytes());
    out.extend_from_slice(&score.to_le_bytes());
    out
}
/// The `WAR_END_LIST` send (`FSendWarList` with the end subheader,
/// `G/guild_manager.cpp:601-612,713`): the envelope plus exactly one `(lo, hi)` DWORD
/// pair, to every PC after the over notice.
#[must_use]
pub fn guild_war_end_list_record(lo: u32, hi: u32) -> Vec<u8> {
    let size = GC_GUILD_ENVELOPE_WIRE_SIZE + 8;
    let mut out = Vec::with_capacity(size);
    envelope(&mut out, GUILD_SUBHEADER_GC_WAR_END_LIST, size);
    out.extend_from_slice(&lo.to_le_bytes());
    out.extend_from_slice(&hi.to_le_bytes());
    out
}

/// Pad a character name to the 25-byte LIST field.
#[must_use]
pub fn pad_member_name(name: &[u8]) -> [u8; GUILD_MEMBER_NAME_FIELD_SIZE] {
    pad(name)
}

/// Pad a guild name to the 13-byte INFO field.
#[must_use]
pub fn pad_guild_name(name: &[u8]) -> [u8; GUILD_NAME_FIELD_SIZE] {
    pad(name)
}

/// Pad a grade name to the 9-byte GRADE field.
#[must_use]
pub fn pad_grade_name(name: &[u8]) -> [u8; GUILD_GRADE_NAME_FIELD_SIZE] {
    pad(name)
}

/// Pad comment content to the 51-byte COMMENTS field.
#[must_use]
pub fn pad_guild_comment(content: &[u8]) -> [u8; GUILD_COMMENT_FIELD_SIZE] {
    pad(content)
}

/// A `gc_guild` decode failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcGuildError {
    /// The buffer ends before the record does.
    Truncated {
        /// Which record was being read.
        context: &'static str,
        /// How many bytes it needs.
        needed: usize,
        /// How many bytes were there.
        got: usize,
    },
}

impl fmt::Display for GcGuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated {
                context,
                needed,
                got,
            } => write!(formatter, "{context} needs {needed} bytes, got {got}"),
        }
    }
}

impl std::error::Error for GcGuildError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guild_name_matches_the_legacy_layout() {
        let record = guild_name_record(7, b"Knights");
        assert_eq!(record.len(), GC_GUILD_NAME_WIRE_SIZE);
        assert_eq!(record.len(), 20);
        assert_eq!(record[0], HEADER_GC_GUILD);
        assert_eq!(u16::from_le_bytes([record[1], record[2]]), 20);
        assert_eq!(record[3], GUILD_SUBHEADER_GC_GUILD_NAME);
        assert_eq!(u32::from_le_bytes(record[4..8].try_into().unwrap()), 7);
        assert_eq!(&record[8..15], b"Knights");
        assert!(record[15..20].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn grade_packet_counts_fifteen_grades() {
        let grades = [GcGuildGrade {
            name: pad_grade_name(b"Leader"),
            auth: 15,
        }; GUILD_GRADE_COUNT];
        let record = guild_grade_record(&grades);
        assert_eq!(record.len(), 4 + 1 + 15 * 11);
        assert_eq!(record[3], GUILD_SUBHEADER_GC_GRADE);
        assert_eq!(u16::from_le_bytes([record[1], record[2]]), 170);
        assert_eq!(record[4], 15);
        assert_eq!(record[5], 1);
        assert_eq!(&record[6..12], b"Leader");
        assert_eq!(record[15], 15);
        assert_eq!(record[16], 2);
    }

    #[test]
    fn info_packet_matches_the_legacy_layout() {
        let info = GcGuildInfo {
            member_count: 1,
            max_member_count: 32,
            guild_id: 9,
            master_pid: 41,
            exp: 0,
            level: 1,
            name: pad_guild_name(b"Knights"),
            gold: 0,
            has_land: 0,
        };
        let record = guild_info_record(&info);
        assert_eq!(record.len(), 4 + 35);
        assert_eq!(record[3], GUILD_SUBHEADER_GC_INFO);
        assert_eq!(u16::from_le_bytes([record[1], record[2]]), 39);
        assert_eq!(u16::from_le_bytes([record[4], record[5]]), 1);
        assert_eq!(u16::from_le_bytes([record[6], record[7]]), 32);
        assert_eq!(u32::from_le_bytes(record[8..12].try_into().unwrap()), 9);
        assert_eq!(u32::from_le_bytes(record[12..16].try_into().unwrap()), 41);
        assert_eq!(u32::from_le_bytes(record[16..20].try_into().unwrap()), 0);
        assert_eq!(record[20], 1);
        assert_eq!(&record[21..28], b"Knights");
        assert_eq!(u32::from_le_bytes(record[34..38].try_into().unwrap()), 0);
        assert_eq!(record[38], 0);
    }

    #[test]
    fn list_packet_holds_one_38_byte_row() {
        let member = GcGuildMember {
            pid: 41,
            grade: 1,
            is_general: 0,
            job: 3,
            level: 154,
            offer: 0,
            name_flag: 1,
            name: pad_member_name(b"Alpha"),
        };
        let record = guild_list_record(&[member]);
        assert_eq!(record.len(), 4 + 38);
        assert_eq!(record[3], GUILD_SUBHEADER_GC_LIST);
        assert_eq!(u32::from_le_bytes(record[4..8].try_into().unwrap()), 41);
        assert_eq!(record[8], 1);
        assert_eq!(record[9], 0);
        assert_eq!(record[10], 3);
        assert_eq!(record[11], 154);
        assert_eq!(u32::from_le_bytes(record[12..16].try_into().unwrap()), 0);
        assert_eq!(record[16], 1);
        assert_eq!(&record[17..22], b"Alpha");
    }

    #[test]
    fn skill_info_packet_sends_two_byte_powers() {
        let info = GcGuildSkillInfo {
            skill_point: 0,
            skills: [0; GUILD_SKILL_COUNT],
            power: 400,
            max_power: 400,
        };
        let record = guild_skill_info_record(&info);
        // `sizeof(pack) + 6 + GUILD_SKILL_COUNT` claims 22, but only 4 + 1 + 12 + 2 + 2 =
        // 21 bytes leave the socket: the `6` counts a 3-byte power pair as 6
        // (`G/guild.cpp:1429-1436`). An honest client consumes the declared 22.
        assert_eq!(record.len(), 21);
        assert_eq!(u16::from_le_bytes([record[1], record[2]]), 22);
        assert_eq!(record[3], GUILD_SUBHEADER_GC_SKILL_INFO);
        assert_eq!(record[4], 0);
        assert!(record[5..17].iter().all(|&byte| byte == 0));
        assert_eq!(u16::from_le_bytes([record[17], record[18]]), 400);
        assert_eq!(u16::from_le_bytes([record[19], record[20]]), 400);
    }

    #[test]
    fn remove_packet_carries_the_pid() {
        let record = guild_remove_record(41);
        assert_eq!(record.len(), 8);
        assert_eq!(record[0], HEADER_GC_GUILD);
        assert_eq!(u16::from_le_bytes([record[1], record[2]]), 8);
        assert_eq!(record[3], GUILD_SUBHEADER_GC_REMOVE);
        assert_eq!(u32::from_le_bytes(record[4..8].try_into().unwrap()), 41);
    }

    #[test]
    fn grade_name_packet_carries_the_grade_and_nine_name_bytes() {
        let record = guild_grade_name_record(3, b"Veteran");
        assert_eq!(record.len(), 4 + 1 + 9);
        assert_eq!(record[0], HEADER_GC_GUILD);
        assert_eq!(u16::from_le_bytes([record[1], record[2]]), 14);
        assert_eq!(record[3], GUILD_SUBHEADER_GC_GRADE_NAME);
        assert_eq!(record[4], 3);
        assert_eq!(&record[5..12], b"Veteran");
        assert!(record[12..14].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn grade_auth_packet_carries_the_grade_and_auth() {
        let record = guild_grade_auth_record(4, 7);
        assert_eq!(record.len(), 6);
        assert_eq!(record[0], HEADER_GC_GUILD);
        assert_eq!(u16::from_le_bytes([record[1], record[2]]), 6);
        assert_eq!(record[3], GUILD_SUBHEADER_GC_GRADE_AUTH);
        assert_eq!(record[4], 4);
        assert_eq!(record[5], 7);
    }

    #[test]
    fn change_exp_packet_carries_the_level_and_exp() {
        let record = guild_change_exp_record(3, 12_000);
        assert_eq!(record.len(), 9);
        assert_eq!(record[0], HEADER_GC_GUILD);
        assert_eq!(u16::from_le_bytes([record[1], record[2]]), 9);
        assert_eq!(record[3], GUILD_SUBHEADER_GC_CHANGE_EXP);
        assert_eq!(record[4], 3);
        assert_eq!(u32::from_le_bytes(record[5..9].try_into().unwrap()), 12_000);
    }

    #[test]
    fn comments_packet_counts_its_rows() {
        let comment = GcGuildComment {
            id: 9,
            name: pad_member_name(b"Score"),
            content: pad_guild_comment(b"!war at nine"),
        };
        let record = guild_comments_record(&[comment]);
        assert_eq!(record.len(), 4 + 1 + GC_GUILD_COMMENT_WIRE_SIZE);
        assert_eq!(
            u16::from_le_bytes([record[1], record[2]]),
            u16::try_from(record.len()).unwrap()
        );
        assert_eq!(record[3], GUILD_SUBHEADER_GC_COMMENTS);
        assert_eq!(record[4], 1);
        assert_eq!(u32::from_le_bytes(record[5..9].try_into().unwrap()), 9);
        assert_eq!(&record[9..14], b"Score");
        assert!(record[14..34].iter().all(|&byte| byte == 0));
        assert_eq!(&record[34..46], b"!war at nine");
        assert!(record[46..85].iter().all(|&byte| byte == 0));
        let empty = guild_comments_record(&[]);
        assert_eq!(empty.len(), 5);
        assert_eq!(empty[4], 0);
    }

    #[test]
    fn list_one_packet_carries_thirteen_row_bytes() {
        let member = GcGuildMember {
            pid: 41,
            grade: 15,
            is_general: 0,
            job: 6,
            level: 3,
            offer: 200,
            name_flag: 1,
            name: pad_member_name(b"East"),
        };
        let record = guild_list_one_record(&member);
        assert_eq!(record.len(), 17);
        assert_eq!(record[0], HEADER_GC_GUILD);
        assert_eq!(u16::from_le_bytes([record[1], record[2]]), 17);
        assert_eq!(record[3], GUILD_SUBHEADER_GC_LIST);
        assert_eq!(u32::from_le_bytes(record[4..8].try_into().unwrap()), 41);
        assert_eq!(record[8], 15);
        assert_eq!(record[9], 0);
        assert_eq!(record[10], 6);
        assert_eq!(record[11], 3);
        assert_eq!(u32::from_le_bytes(record[12..16].try_into().unwrap()), 200);
        assert_eq!(record[16], 1);
    }

    #[test]
    fn member_grade_packet_carries_the_pid_and_grade() {
        let record = guild_change_member_grade_record(41, 5);
        assert_eq!(record.len(), 9);
        assert_eq!(record[0], HEADER_GC_GUILD);
        assert_eq!(u16::from_le_bytes([record[1], record[2]]), 9);
        assert_eq!(record[3], GUILD_SUBHEADER_GC_CHANGE_MEMBER_GRADE);
        assert_eq!(u32::from_le_bytes(record[4..8].try_into().unwrap()), 41);
        assert_eq!(record[8], 5);
    }

    #[test]
    fn member_general_packet_carries_the_pid_and_flag() {
        let record = guild_change_member_general_record(41, 1);
        assert_eq!(record.len(), 9);
        assert_eq!(record[0], HEADER_GC_GUILD);
        assert_eq!(u16::from_le_bytes([record[1], record[2]]), 9);
        assert_eq!(record[3], GUILD_SUBHEADER_GC_CHANGE_MEMBER_GENERAL);
        assert_eq!(u32::from_le_bytes(record[4..8].try_into().unwrap()), 41);
        assert_eq!(record[8], 1);
    }

    #[test]
    fn invite_packet_carries_the_id_and_thirteen_name_bytes() {
        let record = guild_invite_record(7, b"Knights");
        assert_eq!(record.len(), 4 + 4 + 13);
        assert_eq!(record[0], HEADER_GC_GUILD);
        assert_eq!(u16::from_le_bytes([record[1], record[2]]), 21);
        assert_eq!(record[3], GUILD_SUBHEADER_GC_GUILD_INVITE);
        assert_eq!(u32::from_le_bytes(record[4..8].try_into().unwrap()), 7);
        assert_eq!(&record[8..15], b"Knights");
        assert!(record[15..21].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn war_packet_carries_self_opp_type_and_state() {
        let record = guild_war_record(7, 9, 0, 1);
        assert_eq!(record.len(), GC_GUILD_WAR_WIRE_SIZE);
        assert_eq!(record.len(), 14);
        assert_eq!(record[0], HEADER_GC_GUILD);
        assert_eq!(u16::from_le_bytes([record[1], record[2]]), 14);
        assert_eq!(record[3], GUILD_SUBHEADER_GC_WAR);
        assert_eq!(u32::from_le_bytes(record[4..8].try_into().unwrap()), 7);
        assert_eq!(u32::from_le_bytes(record[8..12].try_into().unwrap()), 9);
        assert_eq!(record[12], 0);
        assert_eq!(record[13], 1);
    }

    #[test]
    fn war_list_carries_each_pair_lo_then_hi() {
        let record = guild_war_list_record(&[(3, 11), (5, 7)]);
        assert_eq!(record.len(), 4 + 16);
        assert_eq!(record[0], HEADER_GC_GUILD);
        assert_eq!(u16::from_le_bytes([record[1], record[2]]), 20);
        assert_eq!(record[3], GUILD_SUBHEADER_GC_GUILD_WAR_LIST);
        assert_eq!(u32::from_le_bytes(record[4..8].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(record[8..12].try_into().unwrap()), 11);
        assert_eq!(u32::from_le_bytes(record[12..16].try_into().unwrap()), 5);
        assert_eq!(u32::from_le_bytes(record[16..20].try_into().unwrap()), 7);
    }

    #[test]
    fn war_list_with_no_wars_is_the_bare_envelope() {
        let record = guild_war_list_record(&[]);
        assert_eq!(record.len(), GC_GUILD_ENVELOPE_WIRE_SIZE);
        assert_eq!(u16::from_le_bytes([record[1], record[2]]), 4);
        assert_eq!(record[3], GUILD_SUBHEADER_GC_GUILD_WAR_LIST);
    }

    #[test]
    fn war_score_carries_gain_opp_and_total() {
        let record = guild_war_score_record(7, 9, 40);
        assert_eq!(record.len(), 16);
        assert_eq!(record[0], HEADER_GC_GUILD);
        assert_eq!(u16::from_le_bytes([record[1], record[2]]), 16);
        assert_eq!(record[3], GUILD_SUBHEADER_GC_WAR_SCORE);
        assert_eq!(u32::from_le_bytes(record[4..8].try_into().unwrap()), 7);
        assert_eq!(u32::from_le_bytes(record[8..12].try_into().unwrap()), 9);
        assert_eq!(u32::from_le_bytes(record[12..16].try_into().unwrap()), 40);
    }

    #[test]
    fn war_end_list_carries_one_pair_lo_then_hi() {
        let record = guild_war_end_list_record(3, 11);
        assert_eq!(record.len(), 4 + 8);
        assert_eq!(record[0], HEADER_GC_GUILD);
        assert_eq!(u16::from_le_bytes([record[1], record[2]]), 12);
        assert_eq!(record[3], GUILD_SUBHEADER_GC_WAR_END_LIST);
        assert_eq!(u32::from_le_bytes(record[4..8].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(record[8..12].try_into().unwrap()), 11);
    }
}
