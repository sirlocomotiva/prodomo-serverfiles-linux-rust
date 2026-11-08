//! `HEADER_GC_WORLD_BOSS` (148): the world boss's answers
//! (`CWorldBoss::Process` and `CWorldBoss::TempWorldBossRanking`,
//! `server/server/game/worldboss.cpp:639-695`, `:886-907`).
//!
//! # Provenance
//!
//! `server/server/game/packet.h:3343-3361` declares the head and the two bodies
//! every world-boss send writes:
//!
//! ```ignore
//! typedef struct SPacketGCWorldBoss
//! {
//!     BYTE bHeader;
//!     WORD wSize;
//!     BYTE bSubHeader;
//! } TPacketGCWorldBoss;
//!
//! typedef struct SPacketGCWorldBossProcess
//! {
//!     SPacketGCWorldBossProcess()
//!         : bState(0), dwRunTimeLeft(0), dwCooldownTimeLeft(0), dwTotalDamage(0), dwMinDamage(0) {}
//!     DWORD bState, dwRunTimeLeft, dwCooldownTimeLeft, dwTotalDamage, dwMinDamage;
//! } TPacketGCWorldBossProcess;
//!
//! typedef struct SPacketCGWorldBoss
//! {
//!     BYTE bHeader;
//!     BYTE bSubHeader;
//! } TPacketCGWorldBoss;
//! ```
//!
//! The subheaders are the enum at `packet.h:3332-3341`: `GC_INFO` (0),
//! `GC_DAMAGE` (1) and `GC_RANKING` (2), with the `CG_INFO` (0), `CG_REWARD`
//! (1) and `CG_RANKING` (2) asks on the matching client byte.
//!
//! # Width
//!
//! `wSize` counts the whole record (`:686`). The info answer is 4 + 20 = 24
//! bytes, the damage answer 4 + 4 = 8, and a bare head with no body 4 (an
//! unknown subheader, `:693-694`). A ranking row is one
//! `TPacketGDTempWorldBossRanking` (`tables.h:2259-2265`): the 25-byte player
//! name, the 13-byte guild name, the empire byte and the record word, 43
//! bytes; the ranking answer is 4 + 43 per row.

use super::gc_inventory::HEADER_GC_WORLD_BOSS;

/// `WORLD_BOSS_SUBHEADER_GC_INFO`.
pub const SUB_INFO: u8 = 0;

/// `WORLD_BOSS_SUBHEADER_GC_DAMAGE`.
pub const SUB_DAMAGE: u8 = 1;

/// `WORLD_BOSS_SUBHEADER_GC_RANKING`.
pub const SUB_RANKING: u8 = 2;

/// The head's width: the header byte, the size word and the subheader.
pub const HEAD_LEN: usize = 4;

/// The info body's width: five words (`TPacketGCWorldBossProcess`).
pub const INFO_BODY_LEN: usize = 20;

/// The info answer's width: the head and the info body.
pub const INFO_LEN: usize = HEAD_LEN + INFO_BODY_LEN;

/// The damage answer's width: the head and the one damage word.
pub const DAMAGE_LEN: usize = HEAD_LEN + 4;

/// The player name's width (`CHARACTER_NAME_MAX_LEN + 1`, `common/tables.h`).
pub const PLAYER_NAME_LEN: usize = 25;

/// The guild name's width (`GUILD_NAME_MAX_LEN + 1`, `common/tables.h`).
pub const GUILD_NAME_LEN: usize = 13;

/// One ranking row's width: the names, the empire byte and the record word.
pub const RANKING_ROW_LEN: usize = PLAYER_NAME_LEN + GUILD_NAME_LEN + 1 + 4;

/// The info body (`TPacketGCWorldBossProcess`, `packet.h:3350-3355`): the
/// state, the run and cooldown seconds left, the asker's total damage and the
/// minimum damage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcWorldBossProcess {
    /// `bState`: the boss state (a `DWORD` on the wire).
    pub state: u32,
    /// `dwRunTimeLeft`.
    pub run_time_left: u32,
    /// `dwCooldownTimeLeft`.
    pub cooldown_left: u32,
    /// `dwTotalDamage`: the asker's accumulated damage on the boss.
    pub total_damage: u32,
    /// `dwMinDamage`: the config's minimum damage.
    pub min_damage: u32,
}

/// One ranking row (`TPacketGDTempWorldBossRanking`, `tables.h:2259-2265`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcWorldBossRankingRow {
    /// `szPlayerName`: the 25-byte name, zero-padded.
    pub player: [u8; PLAYER_NAME_LEN],
    /// `szGuildName`: the 13-byte guild name, zero-padded.
    pub guild: [u8; GUILD_NAME_LEN],
    /// `bEmpire`.
    pub empire: u8,
    /// `dwRecord`: the damage.
    pub record: u32,
}

/// Encode the info answer (`Process`'s `GC_INFO` arm, `worldboss.cpp:651-665`):
/// the header, the size word counting the whole 24-byte record, the `INFO`
/// subheader, and the five little-endian words.
///
/// # Panics
///
/// Never: the 24-byte answer fits the size word.
#[must_use]
pub fn encode_info(body: GcWorldBossProcess) -> [u8; INFO_LEN] {
    let size = u16::try_from(INFO_LEN).expect("the info answer is short");
    let [lo, hi] = size.to_le_bytes();
    let mut out = [0u8; INFO_LEN];
    out[0] = HEADER_GC_WORLD_BOSS.value();
    out[1] = lo;
    out[2] = hi;
    out[3] = SUB_INFO;
    out[4..8].copy_from_slice(&body.state.to_le_bytes());
    out[8..12].copy_from_slice(&body.run_time_left.to_le_bytes());
    out[12..16].copy_from_slice(&body.cooldown_left.to_le_bytes());
    out[16..20].copy_from_slice(&body.total_damage.to_le_bytes());
    out[20..24].copy_from_slice(&body.min_damage.to_le_bytes());
    out
}

/// Encode the damage answer (`Process`'s `GC_DAMAGE` arm, `:668-681`): the
/// header, the size word counting the whole 8-byte record, the `DAMAGE`
/// subheader, and the little-endian damage word.
///
/// # Panics
///
/// Never: the 8-byte answer fits the size word.
#[must_use]
pub fn encode_damage(damage: u32) -> [u8; DAMAGE_LEN] {
    let size = u16::try_from(DAMAGE_LEN).expect("the damage answer is short");
    let [lo, hi] = size.to_le_bytes();
    let mut out = [0u8; DAMAGE_LEN];
    out[0] = HEADER_GC_WORLD_BOSS.value();
    out[1] = lo;
    out[2] = hi;
    out[3] = SUB_DAMAGE;
    out[4..8].copy_from_slice(&damage.to_le_bytes());
    out
}

/// Encode a bare head (an unknown subheader, `worldboss.cpp:693-694`): the
/// header, the size word counting the 4-byte record, and the subheader.
///
/// # Panics
///
/// Never: the 4-byte head fits the size word.
#[must_use]
pub fn encode_head(sub: u8) -> [u8; HEAD_LEN] {
    let size = u16::try_from(HEAD_LEN).expect("the head is short");
    let [lo, hi] = size.to_le_bytes();
    [HEADER_GC_WORLD_BOSS.value(), lo, hi, sub]
}

/// Encode one ranking row: the padded names, the empire byte and the
/// little-endian record.
#[must_use]
pub fn encode_ranking_row(row: GcWorldBossRankingRow) -> [u8; RANKING_ROW_LEN] {
    let mut out = [0u8; RANKING_ROW_LEN];
    out[0..PLAYER_NAME_LEN].copy_from_slice(&row.player);
    out[PLAYER_NAME_LEN..PLAYER_NAME_LEN + GUILD_NAME_LEN].copy_from_slice(&row.guild);
    out[PLAYER_NAME_LEN + GUILD_NAME_LEN] = row.empire;
    out[PLAYER_NAME_LEN + GUILD_NAME_LEN + 1..].copy_from_slice(&row.record.to_le_bytes());
    out
}

/// Decode the info body: five little-endian words. `None` when the slice is
/// short.
#[must_use]
pub fn decode_process(body: &[u8]) -> Option<GcWorldBossProcess> {
    let body: [u8; INFO_BODY_LEN] = body.try_into().ok()?;
    Some(GcWorldBossProcess {
        state: u32::from_le_bytes(body[0..4].try_into().ok()?),
        run_time_left: u32::from_le_bytes(body[4..8].try_into().ok()?),
        cooldown_left: u32::from_le_bytes(body[8..12].try_into().ok()?),
        total_damage: u32::from_le_bytes(body[12..16].try_into().ok()?),
        min_damage: u32::from_le_bytes(body[16..20].try_into().ok()?),
    })
}

/// Decode the damage body: one little-endian word. `None` when short.
#[must_use]
pub fn decode_damage(body: &[u8]) -> Option<u32> {
    let body: [u8; 4] = body.try_into().ok()?;
    Some(u32::from_le_bytes(body))
}

/// Decode one ranking row. `None` when the slice is short.
#[must_use]
pub fn decode_ranking_row(row: &[u8]) -> Option<GcWorldBossRankingRow> {
    let row: [u8; RANKING_ROW_LEN] = row.try_into().ok()?;
    let mut player = [0u8; PLAYER_NAME_LEN];
    let mut guild = [0u8; GUILD_NAME_LEN];
    player.copy_from_slice(&row[0..PLAYER_NAME_LEN]);
    guild.copy_from_slice(&row[PLAYER_NAME_LEN..PLAYER_NAME_LEN + GUILD_NAME_LEN]);
    let empire = row[PLAYER_NAME_LEN + GUILD_NAME_LEN];
    let record = u32::from_le_bytes(
        row[PLAYER_NAME_LEN + GUILD_NAME_LEN + 1..]
            .try_into()
            .ok()?,
    );
    Some(GcWorldBossRankingRow {
        player,
        guild,
        empire,
        record,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The byte-exact info answer: header 148, size 24, sub 0, five words.
    #[test]
    fn the_info_answer_is_byte_exact() {
        let body = GcWorldBossProcess {
            state: 0x0201_0403,
            run_time_left: 0x0605_0807,
            cooldown_left: 0x0A09_0C0B,
            total_damage: 0x0E0D_100F,
            min_damage: 0x1211_1413,
        };
        let record = encode_info(body);
        assert_eq!(
            record,
            [
                148, 24, 0, 0, 3, 4, 1, 2, 7, 8, 5, 6, 11, 12, 9, 10, 15, 16, 13, 14, 19, 20, 17,
                18,
            ]
        );
        assert_eq!(decode_process(&record[4..]), Some(body));
    }

    /// The byte-exact damage answer: header 148, size 8, sub 1, one word.
    #[test]
    fn the_damage_answer_is_byte_exact() {
        let record = encode_damage(1234);
        assert_eq!(&record[0..4], &[148, 8, 0, 1]);
        assert_eq!(decode_damage(&record[4..]), Some(1234));
    }

    /// The bare head and a ranking row round-trip.
    #[test]
    fn the_head_and_a_ranking_row_round_trip() {
        assert_eq!(encode_head(9), [148, 4, 0, 9]);
        let row = GcWorldBossRankingRow {
            player: [b'A'; PLAYER_NAME_LEN],
            guild: [b'G'; GUILD_NAME_LEN],
            empire: 1,
            record: 777,
        };
        assert_eq!(decode_ranking_row(&encode_ranking_row(row)), Some(row));
        assert!(decode_process(&[0u8; 3]).is_none());
        assert!(decode_damage(&[0u8; 3]).is_none());
        assert!(decode_ranking_row(&[0u8; 3]).is_none());
    }
}
