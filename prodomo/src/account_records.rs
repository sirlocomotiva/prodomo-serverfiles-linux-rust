//! Account and character records the login path hands to the reducer.
//!
//! These carry the fields of the legacy `TAccountTable`, `TPlayerTable`, and
//! their request records (`server/server/common/tables.h`). In the legacy
//! server they crossed the DB-peer socket as packed records; in the Rewrite
//! the account store and the Channels share one process (ADR-0002), so they
//! are plain values with no wire layout. The fixed byte-array widths are kept
//! because the client-visible fields (login, character name, IP text) keep
//! their legacy limits.

use protocol::simple_player::SimplePlayerRecord;

/// Maximum number of fixed-width name bytes, including the NUL terminator.
pub const LEGACY_LOGIN_BYTES: usize = 31;
/// Maximum number of fixed-width password bytes, including the NUL terminator.
pub const LEGACY_PASSWORD_BYTES: usize = 17;
/// Maximum number of fixed-width social-ID bytes, including the NUL terminator.
pub const LEGACY_SOCIAL_ID_BYTES: usize = 19;
/// Maximum number of fixed-width account-status bytes, including the NUL terminator.
pub const LEGACY_ACCOUNT_STATUS_BYTES: usize = 9;
/// Maximum number of fixed-width character-name bytes, including the NUL terminator.
pub const LEGACY_CHARACTER_NAME_BYTES: usize = 25;
/// Maximum number of fixed-width IP/host bytes, including the NUL terminator.
pub const LEGACY_IP_BYTES: usize = 16;
/// Number of character summaries in an account result.
pub const LEGACY_PLAYER_PER_ACCOUNT: usize = 4;
/// Number of skills in a player result.
pub const LEGACY_SKILL_MAX_NUM: usize = 255;
/// Number of quickslots in a player result.
pub const LEGACY_QUICKSLOT_MAX_NUM: usize = 36;
/// Number of equipment-part words in a player result.
///
/// This is the value of the C++ `PART_MAX_NUM` sentinel in `EParts`.
/// `PART_WEAPON_SUB` follows the sentinel and is not serialized.
pub const LEGACY_PART_MAX_NUM: usize = 6;
/// Number of premium counters in a player result.
pub const LEGACY_PREMIUM_MAX_NUM: usize = 9;
/// Number of fish-event slots in a player result.
pub const LEGACY_FISH_EVENT_SLOTS_NUM: usize = 24;
/// Number of battle-pass records in a player result.
pub const LEGACY_BATTLEPASS_MISSIONS_PER_PLAYER: usize = 10;
/// `TAccountTable`: the account result sent for a successful login.
///
/// A `LOGIN_BY_KEY` request is followed by a database lookup.  The eventual
/// `HEADER_DG_LOGIN_SUCCESS` payload is this record, not the GD request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoginAccountRecord {
    /// `id`.
    pub id: u32,
    /// `login[LOGIN_MAX_LEN + 1]`.
    pub login: [u8; LEGACY_LOGIN_BYTES],
    /// `passwd[PASSWD_MAX_LEN + 1]`.
    pub passwd: [u8; LEGACY_PASSWORD_BYTES],
    /// `social_id[SOCIAL_ID_MAX_LEN + 1]`.
    pub social_id: [u8; LEGACY_SOCIAL_ID_BYTES],
    /// `status[ACCOUNT_STATUS_MAX_LEN + 1]`.
    pub status: [u8; LEGACY_ACCOUNT_STATUS_BYTES],
    /// `bEmpire`.
    pub empire: u8,
    /// `players[PLAYER_PER_ACCOUNT]`.
    pub players: [SimplePlayerRecord; LEGACY_PLAYER_PER_ACCOUNT],
    /// `bLanguage` (the active legacy build enables multi-language support).
    pub language: u8,
}

/// `TPacketDGLoginAlready`: the account name returned when login is already active.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoginAlreadyRecord {
    /// Raw `szLogin[LOGIN_MAX_LEN + 1]` bytes, including any source tail.
    pub login: [u8; LEGACY_LOGIN_BYTES],
}

/// `TPacketGDLoginByKey`: the GD request that starts the key-based login.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoginByKeyRequest {
    /// `szLogin[LOGIN_MAX_LEN + 1]`.
    pub login: [u8; LEGACY_LOGIN_BYTES],
    /// `dwLoginKey`.
    pub login_key: u32,
    /// `adwClientKey[4]`.
    pub client_key: [u32; 4],
    /// `szIP[MAX_HOST_LENGTH + 1]`.
    pub ip: [u8; LEGACY_IP_BYTES],
}

/// `TPlayerLoadPacket`: the GD request for one character.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlayerLoadRequest {
    /// `account_id`.
    pub account_id: u32,
    /// `player_id`.
    pub player_id: u32,
    /// `account_index`.
    pub account_index: u8,
}

/// `TPlayerSkill`: one packed x86 skill record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlayerSkillRecord {
    /// `bMasterType`.
    pub master_type: u8,
    /// `bLevel`.
    pub level: u8,
    /// `tNextRead` (x86 `time_t` is four bytes).
    pub next_read: i32,
}

/// `TQuickslot`: one packed quickslot record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QuickslotRecord {
    /// `type`.
    pub slot_type: u8,
    /// `pos`.
    pub pos: u8,
}

/// `TPlayerFishEventSlot`: one packed fish-event slot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FishEventSlotRecord {
    /// Raw one-byte C++ `bool` representation.
    pub is_main: u8,
    /// `bShape`.
    pub shape: u8,
}

/// `THorseInfo`: one packed horse record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HorseInfoRecord {
    /// `bLevel`.
    pub level: u8,
    /// `bRiding`.
    pub riding: u8,
    /// `sStamina`.
    pub stamina: i16,
    /// `sHealth`.
    pub health: i16,
    /// `dwHorseHealthDropTime`.
    pub health_drop_time: u32,
}

/// `TPlayerBattlePass`: one packed battle-pass record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BattlePassRecord {
    /// `missionID`.
    pub mission_id: u16,
    /// `progress`.
    pub progress: u32,
    /// `type`.
    pub battle_pass_type: u8,
    /// `endTime`.
    pub end_time: u32,
}

/// `TPlayerTable`: the `PLAYER_LOAD_SUCCESS` result.
///
/// The active production feature set is encoded here, including the x86
/// `long`/`time_t` widths, the ten battle-pass records, and the private-shop
/// unlocked-slot word.  No host-language struct layout is assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerResultRecord {
    /// `id`.
    pub id: u32,
    /// `name[CHARACTER_NAME_MAX_LEN + 1]`.
    pub name: [u8; LEGACY_CHARACTER_NAME_BYTES],
    /// `ip[IP_ADDRESS_LENGTH + 1]`.
    pub ip: [u8; LEGACY_IP_BYTES],
    /// `job`.
    pub job: u16,
    /// `voice`.
    pub voice: u8,
    /// `level`.
    pub level: u8,
    /// `level_step`.
    pub level_step: u8,
    /// `st`.
    pub st: i16,
    /// `ht`.
    pub ht: i16,
    /// `dx`.
    pub dx: i16,
    /// `iq`.
    pub iq: i16,
    /// `exp`.
    pub exp: u32,
    /// `gold` (`unsigned long long` is enabled in the active build).
    pub gold: u64,
    /// `gaya` (`ENABLE_GAYA_SYSTEM`).
    pub gaya: i32,
    /// `dir`.
    pub dir: u8,
    /// `x`.
    pub x: i32,
    /// `y`.
    pub y: i32,
    /// `z`.
    pub z: i32,
    /// `lMapIndex`.
    pub map_index: i32,
    /// `lExitX`.
    pub exit_x: i32,
    /// `lExitY`.
    pub exit_y: i32,
    /// `lExitMapIndex`.
    pub exit_map_index: i32,
    /// `hp`.
    pub hp: i32,
    /// `sp`.
    pub sp: i32,
    /// `sRandomHP`.
    pub random_hp: i16,
    /// `sRandomSP`.
    pub random_sp: i16,
    /// `playtime`.
    pub playtime: i32,
    /// `stat_point`.
    pub stat_point: i16,
    /// `skill_point`.
    pub skill_point: i16,
    /// `sub_skill_point`.
    pub sub_skill_point: i16,
    /// `horse_skill_point`.
    pub horse_skill_point: i16,
    /// `skills[SKILL_MAX_NUM]`.
    pub skills: [PlayerSkillRecord; LEGACY_SKILL_MAX_NUM],
    /// `quickslot[QUICKSLOT_MAX_NUM]`.
    pub quickslot: [QuickslotRecord; LEGACY_QUICKSLOT_MAX_NUM],
    /// `part_base`.
    pub part_base: u8,
    /// `parts[PART_MAX_NUM]`.
    pub parts: [u16; LEGACY_PART_MAX_NUM],
    /// `stamina`.
    pub stamina: i16,
    /// `skill_group`.
    pub skill_group: u8,
    /// `lAlignment` (x86 `long`).
    pub alignment: i32,
    /// `stat_reset_count`.
    pub stat_reset_count: i16,
    /// `horse`.
    pub horse: HorseInfoRecord,
    /// `logoff_interval`.
    pub logoff_interval: u32,
    /// `aiPremiumTimes[PREMIUM_MAX_NUM]`.
    pub premium_times: [i32; LEGACY_PREMIUM_MAX_NUM],
    /// `envanter` (`ENABLE_EXTEND_INVEN_SYSTEM`).
    pub envanter: i32,
    /// `fishEventUseCount`.
    pub fish_event_use_count: u32,
    /// `fishSlots[FISH_EVENT_SLOTS_NUM]`.
    pub fish_slots: [FishEventSlotRecord; LEGACY_FISH_EVENT_SLOTS_NUM],
    /// `premium`.
    pub premium: u8,
    /// `premium_time` (x86 `long`).
    pub premium_time: i32,
    /// `secured`.
    pub secured: u8,
    /// `secured_password`.
    pub secured_password: i32,
    /// `biologist_state`.
    pub biologist_state: u32,
    /// `biologist_items_taken`.
    pub biologist_items_taken: u32,
    /// `biologist_completed`.
    pub biologist_completed: u32,
    /// `conqueror_level`.
    pub conqueror_level: u8,
    /// `conqueror_level_step`.
    pub conqueror_level_step: u8,
    /// `sungma_str`.
    pub sungma_str: i16,
    /// `sungma_hp`.
    pub sungma_hp: i16,
    /// `sungma_move`.
    pub sungma_move: i16,
    /// `sungma_immune`.
    pub sungma_immune: i16,
    /// `conqueror_exp`.
    pub conqueror_exp: u32,
    /// `conqueror_point`.
    pub conqueror_point: i16,
    /// `battlePass[BATTLEPASS_MISSIONS_PER_PLAYER]`.
    pub battle_pass: [BattlePassRecord; LEGACY_BATTLEPASS_MISSIONS_PER_PLAYER],
    /// `wPrivateShopUnlockedSlot`.
    pub private_shop_unlocked_slot: u16,
}

impl Default for PlayerResultRecord {
    fn default() -> Self {
        Self {
            id: 0,
            name: [0; LEGACY_CHARACTER_NAME_BYTES],
            ip: [0; LEGACY_IP_BYTES],
            job: 0,
            voice: 0,
            level: 0,
            level_step: 0,
            st: 0,
            ht: 0,
            dx: 0,
            iq: 0,
            exp: 0,
            gold: 0,
            gaya: 0,
            dir: 0,
            x: 0,
            y: 0,
            z: 0,
            map_index: 0,
            exit_x: 0,
            exit_y: 0,
            exit_map_index: 0,
            hp: 0,
            sp: 0,
            random_hp: 0,
            random_sp: 0,
            playtime: 0,
            stat_point: 0,
            skill_point: 0,
            sub_skill_point: 0,
            horse_skill_point: 0,
            skills: [PlayerSkillRecord::default(); LEGACY_SKILL_MAX_NUM],
            quickslot: [QuickslotRecord::default(); LEGACY_QUICKSLOT_MAX_NUM],
            part_base: 0,
            parts: [0; LEGACY_PART_MAX_NUM],
            stamina: 0,
            skill_group: 0,
            alignment: 0,
            stat_reset_count: 0,
            horse: HorseInfoRecord::default(),
            logoff_interval: 0,
            premium_times: [0; LEGACY_PREMIUM_MAX_NUM],
            envanter: 0,
            fish_event_use_count: 0,
            fish_slots: [FishEventSlotRecord::default(); LEGACY_FISH_EVENT_SLOTS_NUM],
            premium: 0,
            premium_time: 0,
            secured: 0,
            secured_password: 0,
            biologist_state: 0,
            biologist_items_taken: 0,
            biologist_completed: 0,
            conqueror_level: 0,
            conqueror_level_step: 0,
            sungma_str: 0,
            sungma_hp: 0,
            sungma_move: 0,
            sungma_immune: 0,
            conqueror_exp: 0,
            conqueror_point: 0,
            battle_pass: [BattlePassRecord::default(); LEGACY_BATTLEPASS_MISSIONS_PER_PLAYER],
            private_shop_unlocked_slot: 0,
        }
    }
}

