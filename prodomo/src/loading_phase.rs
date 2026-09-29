//! Selecting a character, the loading burst, and entering the game.
//!
//! This module builds the records for the two transitions that take a descriptor from the
//! select screen to a live character, and the verdicts for the two client records that ask
//! for them. It holds no descriptor, no store and no world: the caller supplies the character
//! the store read and the neighbourhood the world knows, and sends what comes back.
//!
//! | step | legacy handler | sends |
//! |---|---|---|
//! | `CG_CHARACTER_SELECT` (6) | `CInputLogin::CharacterSelect` (`G/input_login.cpp:265-306`) | nothing; asks for the load |
//! | the load answer | `CInputDB::PlayerLoad` (`G/input_db.cpp:414-455`) | the loading burst |
//! | `CG_ENTER_GAME` (10) | `CInputLogin::Entergame` (`G/input_login.cpp:562-790`) | the enter-game burst |
//!
//! # The two bursts, in legacy order
//!
//! Each burst is returned in the two halves its legacy handler writes in, because each handler
//! changes the descriptor phase or takes a branch part-way through. Neither burst returns the
//! `GC_PHASE` record: in both handlers that record is a `DESC::SetPhase` call, so the
//! descriptor writes it when it moves the phase.
//!
//! Loading, from [`loading_burst`], as a [`LoadingBurst`]:
//!
//! 1. `GC_PHASE` (253) with `PHASE_LOADING` (4) &mdash; the caller's phase transition
//! 2. `before_map_test`: `GC_ENTITY` (249), every other online character, then
//!    `GC_MAIN_CHARACTER` (113), the 46-byte empire variant
//! 3. &mdash; the `map_allow_find` test, which closes the descriptor on a map the Channel does
//!    not host
//! 4. `after_map_test`: one `GC_QUICKSLOT_ADD` (28) per non-empty quickslot, then
//!    `GC_CHARACTER_GOLD` (224), `GC_CHARACTER_POINTS` (16), and `GC_SKILL_LEVEL` (76)
//!
//! Entering the game, from [`enter_game_burst`], as an [`EnterGameBurst`]:
//!
//! 1. `before_phase`: `GC_CHARACTER_ADD` (68) and `GC_CHAR_ADDITIONAL_INFO` (136) for the
//!    entering character, then the same pair for every character already in view; then
//!    `GC_NPC_POSITION` (115), only when the map has NPCs; then `GC_AFFECT_ADD` (126) for the
//!    five-second revive-invisible affect
//! 2. &mdash; `GC_PHASE` (253) with `PHASE_GAME` (5), the caller's phase transition
//! 3. `after_phase`: `GC_PVP` (41), one per live duel; `GC_LAND_LIST` (130), only for a map
//!    with guild land; then `GC_TIME` (106), `GC_CHANNEL` (121), and one `GC_CHAT` (4) line,
//!    `letters_event 0`
//!
//! # Legacy records this burst does not send, and why
//!
//! Each of these is a reachability question rather than a missing codec. They are listed so a
//! later ledger does not re-derive them.
//!
//! - `GC_HYBRIDCRYPT_SDB` (153), the package SDB, is a **loading** record, not an enter-game
//!   one. `SendClientPackageSDBToLoadMap` writes only when the package crypt knows the map's
//!   SDB stream, and `legacy/` has no `package_info.txt`, so legacy sends it for no map.
//! - The BGM variants of the main-character record (137 and 138) need
//!   `CHARACTER_AddBGMInfo`, fed by the legacy `map_bgm_info` table. `legacy/sql/` has no such
//!   table, so byte 113 is the only variant this deployment can send.
//! - `BroadcastEventFlagOnLogin` sends one `GC_CHAT` per non-zero quest event flag
//!   (`worldboss`, `xmas_snow`, `xmas_boom`, `xmas_tree`, `DayMode`, `newyear_boom`). Every
//!   flag is 0 in a fresh database, so none is sent.
//! - `SendGreetMessage` sends one `GC_CHAT` per legacy DB `string` row named `GREET`. The
//!   snapshot has no such row, so none is sent. A hard-coded welcome would be a Divergence.
//! - The guild, party, messenger, marriage, premium, mount and horse-skill records belong to
//!   systems the Rewrite has not ported; each belongs to its own ledger.
//!
//! # Where the Rewrite differs from legacy, and why
//!
//! - **The client-version check is not reproduced.** Legacy ends `Entergame` with
//!   `g_bCheckClientVersion`, which `config.cpp:86-87` defaults to `true` against
//!   `g_stClientVersion = "1215955205"`, and `legacy/config/ch1/core1/CONFIG` sets neither
//!   `check_version_server` nor `check_version_value`. So every client whose client version
//!   differs gets a notice chat and `DelayedDisconnect(0)`, an immediate close
//!   (`G/input_login.cpp:743-771`). The `if (!d->GetClientVersion())` arm above it is dead
//!   code: `GetClientVersion()` returns `std::string::c_str()`, which is never null, so the
//!   ten-second branch cannot be taken either. That is a Defect and a Divergence: the Rewrite
//!   lets a client into the game and records the client version it saw.
//! - **The two uninitialised point slots are not reproduced.** Legacy's `PointsPacket`
//!   declares `TPacketGCPoints pack;` on the stack and never writes slot 0 (`POINT_NONE`) or
//!   slot 2 (`POINT_VOICE`), so 16 bytes of stack garbage reach the client
//!   (`G/char.cpp:2033-2086`). The Rewrite writes every one of the 255 slots, with
//!   `POINT_VOICE` holding the character's stored voice. That is a Defect.
//! - **The select index is checked before the slot is read.** Legacy reads
//!   `c_r.players[pinfo->index].dwID` before it tests `pinfo->index >= PLAYER_PER_ACCOUNT`
//!   (`G/input_login.cpp:265-274`), an out-of-bounds read a client can trigger with one byte.
//!   The Rewrite tests the index first; see [`judge_select`].
//! - **A position the map cannot take is not a disconnect.** Legacy logs and falls back to
//!   the empire recall point (`G/input_login.cpp:572-585`); the Rewrite does the same. The
//!   Rewrite does not reproduce the stale `z` that fallback leaves behind, because the Rewrite
//!   has no `z` yet: the world owns height.

use std::sync::Arc;

use common::levels;
use common::point_slot as point;
use db::players::{Character, PLAYER_SLOTS};
use gamedata::mob_proto::{CHAR_TYPE_NPC, CHAR_TYPE_PC};
use protocol::gc_actors::{
    GcCharacterAdd, GcCharacterAdditionalInfo, GcMainCharacter2Empire, NAME_LEN,
};
use protocol::gc_chat::{GcChat, CHAT_TYPE_COMMAND};
use protocol::gc_entity::{GcEntity, GcEntityInfo, ENTITY_PART_NUM};
use protocol::gc_fields::{GcGold, GcPoints, GC_POINT_SLOT_COUNT};
use protocol::gc_inventory::{HEADER_GC_CHANNEL, HEADER_GC_PLAYER_POINT_CHANGE, HEADER_GC_TIME};
use protocol::gc_nested::{
    GcAffectAdd, GcAffectElement, GcPointChange, GcSkill, GcSkillLevelNew,
    GC_POINT_CHANGE_WIRE_SIZE, GC_SKILL_SLOT_COUNT,
};
use protocol::gc_npc_position::{GcNpcPosition, GcNpcPositionEntry};
use protocol::gc_small::GcHeaderAndByte;
use protocol::gc_vid::GcHeaderAndDword;
use world::character::{PointRecord, Points, PointsRow};
use world::npc::{MapNpcs, Npc, NpcPosition};

/// The legacy `PHASE_LOADING` byte (`EPhase`, `G/packet.h:796`).
pub const PHASE_LOADING: u8 = 4;

/// The legacy `PHASE_GAME` byte (`EPhase`, `G/packet.h:797`).
pub const PHASE_GAME: u8 = 5;

/// `AffectType` 215 (`G/affect.h:40`), the affect `ReviveInvisible` adds.
pub const AFFECT_REVIVE_INVISIBLE: u32 = 215;

/// `AFFECT_FLAG` 28 (`G/affect.h:243`), the flag that affect carries.
pub const AFF_REVIVE_INVISIBLE: u32 = 28;

/// The seconds `Entergame` gives the revive-invisible affect.
pub const REVIVE_INVISIBLE_SECONDS: i32 = 5;

/// The chat line `SEventLetters` always sends at the end of `Entergame`
/// (`G/input_login.cpp:69-82`): the `letters_event` flag is 0 or 1, so exactly one arm runs
/// and the line is `letters_event 0` for a character that is not in the event.
pub const LETTERS_EVENT_CHAT: &[u8] = b"letters_event 0";

/// The angle `EncodeInsertPacket` reports for a character that is not moving.
///
/// `CHARACTER::updatePacket` starts from the character's own `lX`/`lY`, and its `m_posDest`
/// equals the current position, so `iDur` stays 0 and the angle stays 0
/// (`G/char.cpp:1097-1105`).
pub const IDLE_ANGLE_BITS: u32 = 0;

/// The `bMovingSpeed` and `bAttackSpeed` a character has with no affect on it:
/// `SetPoint(POINT_MOV_SPEED, 100)` and `SetPoint(POINT_ATT_SPEED, 100)` plus the haste
/// bonus, which is 0 with no party (`G/char.cpp:2969-2972`).
///
/// The entering character's own insert record reads its speeds from its points. A
/// [`VisibleCharacter`] is described with these, because the Rewrite does not hold another
/// character's points yet.
pub const BASE_SPEED: u8 = 100;

/// `PK_MODE_FREE` (`EPKModes`, `G/char.h:348-356`), the mode `CHARACTER::SetProto` gives every
/// mob (`G/char.cpp:2474`).
pub const PK_MODE_FREE: u8 = 2;

/// What the descriptor should do with a `CG_CHARACTER_SELECT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectVerdict {
    /// The descriptor holds no account table: legacy sends nothing and does not close.
    Ignore,
    /// The slot holds no character: legacy answers `SetPhase(PHASE_CLOSE)`.
    Close,
    /// The account holds a character in that slot: load this one.
    Load {
        /// The character ID to load.
        player: u32,
    },
}

/// Judge a `CG_CHARACTER_SELECT` (`G/input_login.cpp:265-306`).
///
/// # Where this differs from legacy
///
/// Legacy's first test is `c_r.players[pinfo->index].dwID == 0`, which reads the slot array
/// *before* it checks `pinfo->index >= PLAYER_PER_ACCOUNT` one line later. A client that sends
/// an index of 4 or more makes the server read past the end of a four-element array. That is a
/// Defect; here the index is range-checked first, so an index past the last slot is
/// [`SelectVerdict::Ignore`] rather than a close or a load.
#[must_use]
pub fn judge_select(index: u8, account: Option<&[u32]>) -> SelectVerdict {
    let Some(slots) = account else {
        return SelectVerdict::Ignore;
    };
    if usize::from(index) >= PLAYER_SLOTS {
        return SelectVerdict::Ignore;
    }
    let Some(&player) = slots.get(usize::from(index)) else {
        return SelectVerdict::Ignore;
    };
    if player == 0 {
        return SelectVerdict::Close;
    }
    SelectVerdict::Load { player }
}

/// What the descriptor should do with a `CG_ENTER_GAME`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnterGameVerdict {
    /// The descriptor holds no character: legacy answers `SetPhase(PHASE_CLOSE)`
    /// (`G/input_login.cpp:566-570`).
    Close,
    /// The character is on the descriptor: enter the game.
    Enter,
}

/// Judge a `CG_ENTER_GAME`.
///
/// `TPacketCGEnterGame` is a single header byte (`G/packet.h:594-597`), so there is nothing
/// else to check. Legacy's one guard is whether the descriptor has a character.
#[must_use]
pub fn judge_enter_game(has_character: bool) -> EnterGameVerdict {
    if has_character {
        EnterGameVerdict::Enter
    } else {
        EnterGameVerdict::Close
    }
}

/// The map the `map_allow_find` test uses.
///
/// `CInputDB::PlayerLoad` folds an instance map onto its base map before the test:
/// `lPublicMapIndex = lMapIndex >= 10000 ? lMapIndex / 10000 : lMapIndex`
/// (`G/input_db.cpp:427`). A Channel's configured map list holds base maps, so testing the
/// un-folded index would refuse an instance map the legacy deployment hosts.
#[must_use]
pub fn public_map_index(map_index: i32) -> i32 {
    if map_index >= INSTANCE_MAP_BASE {
        map_index / INSTANCE_MAP_BASE
    } else {
        map_index
    }
}

/// The map index at which a `lMapIndex` is treated as an instance map
/// (`lMapIndex >= 10000` in `G/input_db.cpp:427`).
pub const INSTANCE_MAP_BASE: i32 = 10_000;

/// `map_allow_find` for the maps a Channel hosts (`G/config.cpp:174-184`).
///
/// Legacy's test is membership in the `MAP_ALLOW` set the process was started with. A Channel's
/// configured map list is exactly the union of that Channel's legacy `MAP_ALLOW` lists
/// (`config/prodomo.toml.example`), so the same test is membership in the Channel's list.
///
/// Legacy's first arm, `if (g_bAuthServer) return false`, has no equivalent here: every game
/// listener in the Rewrite hosts a Channel, and the auth port never reaches this call.
#[must_use]
pub fn map_is_allowed(public_map: i32, channel_maps: &[u32]) -> bool {
    channel_maps
        .iter()
        .any(|&map| i64::from(map) == i64::from(public_map))
}

/// A character the world can see, and the records it is described by.
///
/// The loading burst describes a visible character with `GC_ENTITY`, which is five fields; the
/// enter-game burst describes the same character with `GC_CHARACTER_ADD` and
/// `GC_CHAR_ADDITIONAL_INFO`, which need its Name, level and race. One type carries all of
/// them so a caller cannot build a list that one burst can send and the other cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisibleCharacter {
    /// The character's VID.
    pub vid: u32,
    /// The race, which the client turns into a `dwRaceVNum`.
    pub job: u8,
    /// The Name, for `GC_CHAR_ADDITIONAL_INFO`.
    pub name: [u8; NAME_LEN],
    /// The level, for `GC_CHAR_ADDITIONAL_INFO`.
    pub level: u8,
    /// The conqueror level.
    pub conqueror_level: u8,
    /// The empire.
    pub empire: u8,
    /// The six equipment parts, in `ECharacterEquipmentPart` order.
    pub parts: [u16; ENTITY_PART_NUM],
    /// The x the character stands at.
    pub x: i32,
    /// The y the character stands at.
    pub y: i32,
    /// Its descriptor's language, which its `GC_CHAR_ADDITIONAL_INFO` carries
    /// (`GetDesc()->GetLanguage()`, `G/char.cpp:1174`).
    pub language: u8,
}

impl VisibleCharacter {
    /// The five-field description `GC_ENTITY` carries.
    #[must_use]
    pub fn entity(&self) -> GcEntityInfo {
        GcEntityInfo::new(self.vid, self.job.into(), self.parts, self.x, self.y)
    }

    /// The `GC_CHARACTER_ADD` for this character standing still.
    #[must_use]
    pub fn add(&self) -> GcCharacterAdd {
        GcCharacterAdd::new(
            self.vid,
            f32::from_bits(IDLE_ANGLE_BITS),
            self.x,
            self.y,
            0,
            CHAR_TYPE_PC,
            u16::from(self.job),
            BASE_SPEED,
            BASE_SPEED,
            0,
            [0, 0],
        )
    }

    /// The `GC_CHAR_ADDITIONAL_INFO` for this character, with no guild, mount or alignment.
    #[must_use]
    pub fn additional(&self) -> GcCharacterAdditionalInfo {
        GcCharacterAdditionalInfo::new(
            self.vid,
            self.name,
            self.parts,
            self.empire,
            0,
            u32::from(self.level),
            u32::from(self.conqueror_level),
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            self.language,
        )
    }
}

/// The characters and NPCs the world can see from a position.
///
/// The characters are empty until the world holds another character's points. The NPCs are the
/// map's: the Rewrite's view is the whole map, where legacy's is the sectrees around the
/// character (a Divergence), so every NPC on the map is in view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Neighbourhood {
    /// The other characters in view.
    pub characters: Vec<VisibleCharacter>,
    /// The map's NPCs and its mini-map list.
    pub npcs: Arc<MapNpcs>,
}

/// The `GC_CHARACTER_ADD` for an NPC standing where it spawned
/// (`CHARACTER::EncodeInsertPacket`, `G/char.cpp:1060-1110`).
///
/// The type is the proto's, the angle its rotation, the z its spawn's, and the race the vnum's
/// low 16 bits. The speeds are its limited points. It has no affect, and no state flag: a regen
/// NPC spawns without the spawn motion, and `Show` clears that bit once the record is written.
#[must_use]
pub fn npc_add(npc: &Npc) -> GcCharacterAdd {
    GcCharacterAdd::new(
        npc.vid,
        f32::from(npc.rotation),
        npc.x,
        npc.y,
        npc.z,
        npc.char_type,
        npc.race,
        npc.moving_speed,
        npc.attack_speed,
        0,
        [0, 0],
    )
}

/// The `GC_CHAR_ADDITIONAL_INFO` for an NPC, which legacy sends only for `CHAR_TYPE_NPC`: a warp
/// or a goto gets `None` (`G/char.cpp:1111`).
///
/// The Name is `GetName()`, the `LOCALE_YMIR` mob name of the race. `ENABLE_SHOWNPCLEVEL` is not
/// defined, so the level is 0, and an NPC has no parts, guild, mount, alignment or language.
#[must_use]
pub fn npc_additional(npc: &Npc) -> Option<GcCharacterAdditionalInfo> {
    (npc.char_type == CHAR_TYPE_NPC).then(|| {
        GcCharacterAdditionalInfo::new(
            npc.vid,
            name_bytes_field(&npc.name),
            [0; ENTITY_PART_NUM],
            npc.empire,
            0,
            0,
            0,
            0,
            PK_MODE_FREE,
            0,
            0,
            0,
            0,
            0,
            0,
        )
    })
}

/// A mini-map entry as `SendNPCPosition` writes it (`G/sectree_manager.cpp:1110-1117`).
#[must_use]
pub fn npc_position(position: &NpcPosition) -> GcNpcPositionEntry {
    GcNpcPositionEntry::new(
        position.char_type,
        name_bytes_field(&position.name),
        position.x,
        position.y,
    )
}

/// The points engine of a loaded character, as the points half of `CHARACTER::SetPlayerProto`
/// fills it (`G/char.cpp:2233-2420`).
///
/// `map_index` is the atlas index of the saved position, which is `GetMapIndex()` for the
/// loaded character and which decides the conqueror will of the map. The store keeps no
/// inventory unlock count, so `POINT_INVEN` starts at 0, the same value the item load searches
/// with.
#[must_use]
pub fn load_points(character: &Character, map_index: i32) -> Points {
    Points::load(&PointsRow {
        race: character.job,
        level: character.level,
        conqueror_level: character.conqueror_level,
        st: character.st,
        ht: character.ht,
        dx: character.dx,
        iq: character.iq,
        sungma: [
            character.sungma_str,
            character.sungma_hp,
            character.sungma_move,
            character.sungma_immune,
        ],
        hp: character.hp,
        sp: character.sp,
        stamina: character.stamina,
        inven_point: 0,
        map_index,
        part_base: character.part_base,
        hair_part: character.hair_part,
        sash_part: character.sash_part,
    })
}

/// The 255 point slots of `GC_CHARACTER_POINTS`, as `CHARACTER::PointsPacket` fills them
/// (`G/char.cpp:2031-2087`).
///
/// # How each slot is decided
///
/// Legacy writes the named slots below `POINT_ST` from the character, then loops `POINT_ST` to
/// `POINT_MAX_NUM` writing `GetPoint(i)`, then overwrites the conqueror slots and
/// `POINT_MOV_SPEED`, which carries `GetLimitPoint` rather than `GetPoint`. The level,
/// experience, gold and voice come from the row; every other slot comes from `state`.
///
/// `POINT_INVEN` and `POINT_GAYA` are written before the loop and then overwritten by it,
/// because both sit above `POINT_ST`. The biologist and protected-inventory slots are written
/// after the loop from state the Rewrite does not store, so they stay at what `GetPoint`
/// answers, which is 0.
///
/// # The two slots legacy leaves alone
///
/// Slot 0 is `POINT_NONE` and slot 2 is `POINT_VOICE`, and legacy writes neither, so 16 bytes
/// of uninitialised stack reach the client. The Rewrite writes zero into slot 0 and the
/// character's stored voice into slot 2. See the module note.
#[must_use]
pub fn points(character: &Character, state: &Points) -> [i64; GC_POINT_SLOT_COUNT] {
    let mut slots = [0i64; GC_POINT_SLOT_COUNT];
    slots[point::POINT_LEVEL] = i64::from(character.level);
    slots[point::POINT_VOICE] = i64::from(character.voice);
    slots[point::POINT_EXP] = character.exp;
    slots[point::POINT_NEXT_EXP] = levels::next_exp(character.level);
    slots[point::POINT_HP] = i64::from(state.hp());
    slots[point::POINT_MAX_HP] = i64::from(state.max_hp());
    slots[point::POINT_SP] = i64::from(state.sp());
    slots[point::POINT_MAX_SP] = i64::from(state.max_sp());
    slots[point::POINT_GOLD] = character.gold;
    slots[point::POINT_STAMINA] = i64::from(state.stamina());
    slots[point::POINT_MAX_STAMINA] = i64::from(state.max_stamina());
    for (kind, slot) in slots.iter_mut().enumerate().skip(point::POINT_ST) {
        *slot = i64::from(state.get_point(kind));
    }
    slots[point::POINT_CONQUEROR_LEVEL] = i64::from(character.conqueror_level);
    slots[point::POINT_CONQUEROR_EXP] = character.conqueror_exp;
    slots[point::POINT_CONQUEROR_NEXT_EXP] = levels::conqueror_next_exp(character.conqueror_level);
    slots[point::POINT_MOV_SPEED] = i64::from(state.limit_point(point::POINT_MOV_SPEED));
    slots
}

/// The `GC_CHARACTER_POINT_CHANGE` records for the changes `state` reports, in order.
///
/// Legacy writes each record with its own `DESC::Packet` call from `UpdatePointsPacket`
/// (`G/char.cpp:2089-2118`), so each is its own frame. A broadcast record goes to every
/// descriptor that sees the character, and the character's own descriptor is one of them; no
/// other character can see it yet, so every record is written to the descriptor.
#[must_use]
pub fn point_changes(records: &[PointRecord], vid: u32) -> Vec<Vec<u8>> {
    records
        .iter()
        .map(|record| {
            let mut bytes = Vec::with_capacity(GC_POINT_CHANGE_WIRE_SIZE);
            GcPointChange {
                header: i32::from(HEADER_GC_PLAYER_POINT_CHANGE.value()),
                vid,
                change_type: record.kind,
                amount: record.amount,
                value: record.value,
            }
            .encode_into(&mut bytes);
            bytes
        })
        .collect()
}

/// The 255 skill slots of `GC_SKILL_LEVEL`, all empty: the Rewrite has no skill table yet.
///
/// Legacy writes `m_skills[255]`, and a character with no learned skill has a zero level and a
/// zero `tNextRead` in every slot (`CHARACTER::SkillLevelPacket`, `G/char_skill.cpp:170-186`).
#[must_use]
pub fn skill_levels() -> [GcSkill; GC_SKILL_SLOT_COUNT] {
    [GcSkill {
        master_type: 0,
        level: 0,
        next_read: 0,
    }; GC_SKILL_SLOT_COUNT]
}

/// A character's Name in its 25-byte wire field.
///
/// Legacy writes the Name with `strlcpy(pack.szName, ch->GetName(), sizeof(pack.szName))`, which
/// copies at most `sizeof - 1` bytes and always NUL-terminates. This does the same, so a Name
/// the store's `CHECK` already bounds to 24 ASCII bytes fills the field exactly, and a longer
/// one is truncated at 24 with the terminator left in place rather than filling all 25 bytes.
#[must_use]
pub fn name_field(name: &str) -> [u8; NAME_LEN] {
    name_bytes_field(name.as_bytes())
}

/// A Name's bytes in the 25-byte wire field, as `strlcpy` copies them: up to the first NUL, at
/// most 24 bytes, and always terminated.
#[must_use]
pub fn name_bytes_field(name: &[u8]) -> [u8; NAME_LEN] {
    let mut field = [0u8; NAME_LEN];
    let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    let kept = usize::min(end, NAME_LEN - 1);
    field[..kept].copy_from_slice(&name[..kept]);
    field
}

/// The 46-byte own-character record the server sends at byte 113
/// (`CHARACTER::MainCharacterPacket`, `G/char.cpp:2005-2029`).
#[must_use]
pub fn main_character(character: &Character, vid: u32) -> GcMainCharacter2Empire {
    GcMainCharacter2Empire::new(
        vid,
        u16::from(character.job),
        name_field(&character.name),
        character.x,
        character.y,
        0,
        character.empire,
        character.skill_group,
    )
}

/// The `GC_CHARACTER_ADD` for the character that is entering, standing still.
///
/// `bType` is `GetCharType()` (`G/char.cpp:1077`), which `SetPlayerProto` makes `CHAR_TYPE_PC`
/// for every PC (`G/char.cpp:2240`). Until ledger 223 the Rewrite sent 1, which is
/// `CHAR_TYPE_NPC`.
///
/// The two speeds are `GetLimitPoint(POINT_MOV_SPEED)` and `GetLimitPoint(POINT_ATT_SPEED)`
/// (`G/char.cpp:1089-1091`), which the limit holds to 0..200 and 0..170, so each fits the
/// record's byte.
#[must_use]
pub fn character_add(character: &Character, state: &Points, vid: u32) -> GcCharacterAdd {
    GcCharacterAdd::new(
        vid,
        f32::from_bits(IDLE_ANGLE_BITS),
        character.x,
        character.y,
        0,
        CHAR_TYPE_PC,
        u16::from(character.job),
        speed_byte(state.limit_point(point::POINT_MOV_SPEED)),
        speed_byte(state.limit_point(point::POINT_ATT_SPEED)),
        0,
        [0, 0],
    )
}

/// A limited speed as the record's byte. [`Points::limit_point`] holds both speeds inside a
/// byte, so the fall-back is never taken.
fn speed_byte(speed: i32) -> u8 {
    u8::try_from(speed).unwrap_or(u8::MAX)
}

/// The `GC_CHAR_ADDITIONAL_INFO` for the character that is entering, with no guild, mount or
/// alignment (`EncodeAdditionalInfo`, `G/char.cpp:1236-1300`).
///
/// The parts are the ones the last `ComputePoints` left (`GetPart`), which after the item load
/// are the worn armour, weapon, hair costume and sash over the base part.
#[must_use]
pub fn character_additional(
    character: &Character,
    state: &Points,
    vid: u32,
    language: u8,
) -> GcCharacterAdditionalInfo {
    GcCharacterAdditionalInfo::new(
        vid,
        name_field(&character.name),
        state.parts(),
        character.empire,
        0,
        u32::from(character.level),
        u32::from(character.conqueror_level),
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        language,
    )
}

/// The loading burst, split where `CInputDB::PlayerLoad` branches.
///
/// The split is not cosmetic. `CInputDB::PlayerLoad` writes the entity list and the
/// own-character records, then tests `map_allow_find`; a character standing on a map the
/// Channel does not host gets `SetPhase(PHASE_CLOSE)` there and never receives the gold,
/// points, or skill-level records (`G/input_db.cpp:414-455`). A caller that cannot see that
/// branch would send records legacy never sends on that path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadingBurst {
    /// `GC_ENTITY` and the own-character records, written before the map test.
    pub before_map_test: Vec<Vec<u8>>,
    /// The gold, points, and skill-level records, written only when the map is allowed.
    pub after_map_test: Vec<Vec<u8>>,
}

/// The records the loading burst sends, in legacy order.
///
/// `vid` is the character's own VID, the identity the rest of the session uses. The quickslot
/// records are not built here: the descriptor loads the stored slots and puts what
/// [`crate::quickslot::load`] sends at the front of `after_map_test`, where `PlayerLoad` sets
/// them (`G/input_db.cpp:436-443`).
///
/// `DESC::SetPhase(PHASE_LOADING)` is the burst's first record and is **not** returned here: the
/// descriptor writes it as a phase transition, because that is the legacy operation that
/// produces it (`G/input_db.cpp:414`). Returning it as content as well would put two
/// `GC_PHASE` records on the wire.
#[must_use]
pub fn loading_burst(
    character: &Character,
    state: &Points,
    vid: u32,
    view: &Neighbourhood,
) -> LoadingBurst {
    let entities: Vec<GcEntityInfo> = view
        .characters
        .iter()
        .map(VisibleCharacter::entity)
        .collect();
    let before_map_test = vec![
        encoded(
            &mut GcEntity::new(entities),
            "a character list from the world",
        ),
        encoded(
            &mut main_character(character, vid),
            "the own-character record",
        ),
    ];
    let mut after_map_test = points_packet(character, state);
    after_map_test.push(encoded(
        &mut GcSkillLevelNew {
            header: GcSkillLevelNew::header(),
            skills: skill_levels(),
        },
        "the skill-level record",
    ));
    LoadingBurst {
        before_map_test,
        after_map_test,
    }
}

/// The records `CHARACTER::PointsPacket` writes: the gold record, then the points record.
///
/// `ENABLE_REMOVE_LIMIT_GOLD` is on, so `PointsPacket` writes `GC_CHARACTER_GOLD` before
/// `GC_CHARACTER_POINTS` (`G/char.cpp:2078-2085`). The loading burst sends the pair once
/// from `PlayerLoad`, and the item load sends it again as its last step
/// (`G/input_db.cpp:1564`), whether or not the character owns an item.
///
/// # Panics
///
/// Panics when a record does not encode; both are fixed layouts, so that would be a bug in
/// this module.
#[must_use]
pub fn points_packet(character: &Character, state: &Points) -> Vec<Vec<u8>> {
    vec![
        encoded(&mut GcGold::new(gold(character)), "the gold record"),
        encoded(
            &mut GcPoints::new(points(character, state)),
            "the points record",
        ),
    ]
}

/// The records the item load ends with: `CheckMaximumPoints`, then `PointsPacket`
/// (`G/input_db.cpp:1563-1564`).
///
/// A stored hit or spell point total above the maximum the load computed is brought down to
/// it, with one `GC_CHARACTER_POINT_CHANGE` each, and the gold and points records that
/// follow carry the clamped values. `state` keeps the clamp, so the caller's next save writes
/// it.
#[must_use]
pub fn item_load_points(character: &Character, state: &mut Points, vid: u32) -> Vec<Vec<u8>> {
    let mut frames = point_changes(&state.check_maximum_points(), vid);
    frames.extend(points_packet(character, state));
    frames
}

/// The gold as the record carries it.
///
/// Legacy's field is `unsigned long long` because `ENABLE_REMOVE_LIMIT_GOLD` is on, so the
/// value is never negative. The store's `CHECK` on `player.gold` refuses a negative row, so
/// this is a fall-back, not a path a row can reach.
fn gold(character: &Character) -> u64 {
    u64::try_from(character.gold).unwrap_or(0)
}

/// The enter-game burst, split where `CInputLogin::Entergame` calls `SetPhase`.
///
/// `SetPhase(PHASE_GAME)` sits in the middle of `Entergame`, after the revive-invisible affect
/// and before the time and Channel records (`G/input_login.cpp:592-606`). Legacy writes the
/// `GC_PHASE` record through the boundary the descriptor already has, so the record belongs to
/// the phase transition rather than to the content; the caller sends the first half, moves the
/// descriptor, then sends the second half.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnterGameBurst {
    /// The own-character pair, the visible characters' pairs, the NPCs, and the affect.
    pub before_phase: Vec<Vec<u8>>,
    /// The time, Channel, and `letters_event` records, written after the phase change.
    pub after_phase: Vec<Vec<u8>>,
}

/// The records the enter-game burst sends, in legacy order.
///
/// `state` is the character's points, which give its own insert record its two speeds.
/// `channel` is `g_bChannel`, and `now` is `get_global_time()`, which is `time(0)` plus a gap
/// the Rewrite does not yet carry. `language` is the entering descriptor's, which its own
/// summary carries; a visible character's summary carries its own descriptor's. The time and
/// channel records are unconditional; every other record here is about the character or about a
/// list that is empty today.
///
/// # Panics
///
/// Panics when a record does not encode. Every record here is a fixed layout this module
/// builds itself, so a failure is a defect in this module rather than anything a client
/// sends.
#[must_use]
pub fn enter_game_burst(
    character: &Character,
    state: &Points,
    vid: u32,
    view: &Neighbourhood,
    channel: u8,
    now: u32,
    language: u8,
) -> EnterGameBurst {
    // `ch->Show` sends the entering character's own pair, then the pair for every character
    // that can see it (`G/char.cpp:1905`, `G/entity_view.cpp:137`).
    let mut before = Vec::with_capacity(7);
    before.push(encoded(
        &mut character_add(character, state, vid),
        "the own-character insert",
    ));
    before.push(encoded(
        &mut character_additional(character, state, vid, language),
        "the own-character summary",
    ));
    for other in &view.characters {
        before.push(encoded(&mut other.add(), "a visible character insert"));
        before.push(encoded(
            &mut other.additional(),
            "a visible character summary",
        ));
    }
    // The NPCs arrive through the same view insert, in VID order after the characters.
    for npc in &view.npcs.npcs {
        before.push(encoded(&mut npc_add(npc), "an NPC insert"));
        if let Some(mut additional) = npc_additional(npc) {
            before.push(encoded(&mut additional, "an NPC summary"));
        }
    }
    // `SECTREE_MANAGER::SendNPCPosition` returns without writing when the map has no NPC, so
    // an empty list sends nothing at all (`G/sectree_manager.cpp:1089-1098`).
    if !view.npcs.positions.is_empty() {
        let entries = view.npcs.positions.iter().map(npc_position).collect();
        before.push(encoded(&mut GcNpcPosition::new(entries), "the NPC list"));
    }
    // `ch->ReviveInvisible(5)` adds an affect, and `AddAffect` sends `GC_AFFECT_ADD` for
    // every affect it takes on a PC (`G/char.cpp:7490-7493`, `G/char_affect.cpp:747-750`).
    before.push(encoded(
        &mut GcAffectAdd {
            header: GcAffectAdd::header(),
            element: GcAffectElement {
                affect_type: AFFECT_REVIVE_INVISIBLE,
                apply_on: 0,
                apply_value: 0,
                flag: AFF_REVIVE_INVISIBLE,
                duration: REVIVE_INVISIBLE_SECONDS,
                sp_cost: 0,
            },
        },
        "the revive-invisible affect",
    ));
    // `SetPhase(PHASE_GAME)` writes the `GC_PHASE` record; it is the caller's phase
    // transition, so it is the boundary between the two halves rather than a record here.
    let mut out = Vec::with_capacity(3);
    out.push(encoded(
        &mut GcHeaderAndDword::new(HEADER_GC_TIME.value(), now),
        "the time record",
    ));
    out.push(encoded(
        &mut GcHeaderAndByte::new(HEADER_GC_CHANNEL.value(), channel),
        "the Channel record",
    ));
    // `SEventLetters` always sends one line; the flag is 0 for a character not in the event.
    // `LETTERS_EVENT_CHAT` is a short constant, so the length check cannot fail.
    let mut line = GcChat::notice(CHAT_TYPE_COMMAND, character.empire, LETTERS_EVENT_CHAT)
        .expect("the letters_event line is shorter than the chat length limit");
    out.push(encoded(&mut line, "the letters_event line"));
    EnterGameBurst {
        before_phase: before,
        after_phase: out,
    }
}

/// Encode one record into its own frame.
///
/// Legacy writes each record with its own `DESC::Packet` call, so each is a separate frame on
/// the wire; a caller that concatenated them would hand the client one TEA unit where legacy
/// hands it several. The two fallible encoders refuse only when a list is long enough to
/// overflow a length field, which a world's character list cannot be, so the message names the
/// record rather than swallowing the error.
fn encoded<T>(record: &mut T, what: &str) -> Vec<u8>
where
    T: RecordFrame,
{
    let mut bytes = Vec::new();
    record
        .frame_into(&mut bytes)
        .unwrap_or_else(|()| panic!("{what} does not fit its wire length field"));
    bytes
}

/// A record that can write itself into one frame.
trait RecordFrame {
    /// Append the whole record, or report that it does not fit its length field.
    fn frame_into(&self, out: &mut Vec<u8>) -> Result<(), ()>;
}

impl RecordFrame for GcEntity {
    fn frame_into(&self, out: &mut Vec<u8>) -> Result<(), ()> {
        GcEntity::encode_into(self, out).map_err(|_| ())
    }
}

impl RecordFrame for GcMainCharacter2Empire {
    fn frame_into(&self, out: &mut Vec<u8>) -> Result<(), ()> {
        GcMainCharacter2Empire::encode_into(self, out);
        Ok(())
    }
}

impl RecordFrame for GcGold {
    fn frame_into(&self, out: &mut Vec<u8>) -> Result<(), ()> {
        GcGold::encode_into(self, out);
        Ok(())
    }
}

impl RecordFrame for GcPoints {
    fn frame_into(&self, out: &mut Vec<u8>) -> Result<(), ()> {
        GcPoints::encode_into(self, out);
        Ok(())
    }
}

impl RecordFrame for GcSkillLevelNew {
    fn frame_into(&self, out: &mut Vec<u8>) -> Result<(), ()> {
        GcSkillLevelNew::encode_into(self, out);
        Ok(())
    }
}

impl RecordFrame for GcCharacterAdd {
    fn frame_into(&self, out: &mut Vec<u8>) -> Result<(), ()> {
        GcCharacterAdd::encode_into(self, out);
        Ok(())
    }
}

impl RecordFrame for GcCharacterAdditionalInfo {
    fn frame_into(&self, out: &mut Vec<u8>) -> Result<(), ()> {
        GcCharacterAdditionalInfo::encode_into(self, out);
        Ok(())
    }
}

impl RecordFrame for GcNpcPosition {
    fn frame_into(&self, out: &mut Vec<u8>) -> Result<(), ()> {
        GcNpcPosition::encode_into(self, out).map_err(|_| ())
    }
}

impl RecordFrame for GcAffectAdd {
    fn frame_into(&self, out: &mut Vec<u8>) -> Result<(), ()> {
        GcAffectAdd::encode_into(self, out);
        Ok(())
    }
}

impl RecordFrame for GcHeaderAndDword {
    fn frame_into(&self, out: &mut Vec<u8>) -> Result<(), ()> {
        GcHeaderAndDword::encode_into(self, out);
        Ok(())
    }
}

impl RecordFrame for GcHeaderAndByte {
    fn frame_into(&self, out: &mut Vec<u8>) -> Result<(), ()> {
        GcHeaderAndByte::encode_into(self, out);
        Ok(())
    }
}

impl RecordFrame for GcChat {
    fn frame_into(&self, out: &mut Vec<u8>) -> Result<(), ()> {
        GcChat::encode_into(self, out).map_err(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamedata::mob_proto::{CHAR_TYPE_GOTO, CHAR_TYPE_MONSTER, CHAR_TYPE_STONE, CHAR_TYPE_WARP};

    /// A character with values chosen so that every field's little-endian bytes are distinct:
    /// no field is byte-symmetric, so an endianness or byte-order slip cannot pass.
    fn hero() -> Character {
        Character {
            id: 0x1122_3344,
            slot: 1,
            name: "Arges".to_string(),
            empire: 2,
            job: 5,
            level: 9,
            exp: 0x0102_0304_0506,
            conqueror_level: 3,
            sungma_str: 21,
            sungma_hp: 22,
            sungma_move: 23,
            sungma_immune: 24,
            st: 11,
            ht: 13,
            dx: 17,
            iq: 19,
            hp: 0x0012_3456,
            sp: 0x00ab_cdef,
            stamina: 0x0011_2233,
            gold: 0x0012_3456_789a_bcde,
            conqueror_exp: 0x00fe_dcba,
            voice: 77,
            part_base: 1,
            main_part: 0x1122,
            hair_part: 0x3344,
            sash_part: 0x5566,
            x: 0x0012_3456,
            y: -0x0012_3457,
            skill_group: 6,
            playtime_minutes: 0x0a0b_0c0d,
            change_name: false,
        }
    }

    /// The hero's points on a map with no conqueror will.
    fn hero_points() -> Points {
        load_points(&hero(), 0)
    }

    fn empty_view() -> Neighbourhood {
        Neighbourhood::default()
    }

    fn neighbour() -> VisibleCharacter {
        VisibleCharacter {
            vid: 0x0a0b_0c0d,
            job: 7,
            name: name_field("Spectre"),
            level: 21,
            conqueror_level: 1,
            empire: 1,
            parts: [0x1111, 0x2222, 0x3333, 0x4444, 0x5555, 0x6666],
            x: 0x0000_1000,
            y: -0x0000_2000,
            language: 5,
        }
    }

    fn with_neighbour() -> Neighbourhood {
        Neighbourhood {
            characters: vec![neighbour()],
            npcs: Arc::default(),
        }
    }

    fn headers(burst: &[Vec<u8>]) -> Vec<u8> {
        burst.iter().map(|frame| frame[0]).collect()
    }

    /// Every record of a burst, in wire order across the split, for a test that is about a
    /// record's own bytes rather than about where the phase transition falls.
    fn all<'a>(before: &'a [Vec<u8>], after: &'a [Vec<u8>]) -> Vec<&'a Vec<u8>> {
        before.iter().chain(after.iter()).collect()
    }

    #[test]
    fn select_needs_an_account_table() {
        assert_eq!(judge_select(0, None), SelectVerdict::Ignore);
    }

    #[test]
    fn select_loads_the_character_in_the_slot() {
        assert_eq!(
            judge_select(2, Some(&[11, 0, 33, 0])),
            SelectVerdict::Load { player: 33 }
        );
    }

    #[test]
    fn select_closes_on_an_empty_slot() {
        assert_eq!(judge_select(1, Some(&[11, 0, 33, 0])), SelectVerdict::Close);
    }

    /// The out-of-bounds index legacy reads before it range-checks. The index must not load,
    /// and it must not close either: the Rewrite has nothing to say about a slot that does not
    /// exist, and a close would be a response to a byte legacy never meant to accept.
    #[test]
    fn select_ignores_an_index_past_the_last_slot() {
        for index in u8::try_from(PLAYER_SLOTS).expect("four slots fit in a u8")..=u8::MAX {
            assert_eq!(
                judge_select(index, Some(&[11, 22, 33, 44])),
                SelectVerdict::Ignore,
                "index {index} must not reach the slot array",
            );
        }
    }

    #[test]
    fn select_reaches_every_slot_inside_the_table() {
        for index in 0..u8::try_from(PLAYER_SLOTS).expect("four slots fit in a u8") {
            assert_eq!(
                judge_select(index, Some(&[11, 22, 33, 44])),
                SelectVerdict::Load {
                    player: 11 + u32::from(index) * 11
                },
                "slot {index} is inside the table and must load",
            );
        }
    }

    #[test]
    fn enter_game_closes_without_a_character() {
        assert_eq!(judge_enter_game(false), EnterGameVerdict::Close);
    }

    #[test]
    fn enter_game_needs_nothing_but_a_character() {
        assert_eq!(judge_enter_game(true), EnterGameVerdict::Enter);
    }

    /// The loading burst, one record per frame, in the order `CInputDB::PlayerLoad` writes
    /// them. `GC_PHASE` is the descriptor's own phase transition and is not in either half.
    #[test]
    fn loading_burst_headers_are_in_legacy_order() {
        let burst = loading_burst(&hero(), &hero_points(), 0x1122_3344, &empty_view());
        assert_eq!(
            headers(&burst.before_map_test),
            vec![249, 113],
            "GC_ENTITY, GC_MAIN_CHARACTER2_EMPIRE",
        );
        assert_eq!(
            headers(&burst.after_map_test),
            vec![224, 16, 76],
            "GC_CHARACTER_GOLD, GC_CHARACTER_POINTS, GC_SKILL_LEVEL",
        );
    }

    #[test]
    fn loading_burst_frame_widths_are_the_legacy_widths() {
        let burst = loading_burst(&hero(), &hero_points(), 0x1122_3344, &empty_view());
        let widths = |frames: &[Vec<u8>]| -> Vec<usize> { frames.iter().map(Vec::len).collect() };
        assert_eq!(
            widths(&burst.before_map_test),
            vec![3, 46],
            "the empty ENTITY is 3, MAIN_CHARACTER2_EMPIRE is 46",
        );
        assert_eq!(
            widths(&burst.after_map_test),
            vec![9, 2041, 1531],
            "GOLD is 9, POINTS is 1 + 255*8, SKILL_LEVEL is 1 + 255*6",
        );
    }

    /// Neither half carries a `GC_PHASE`. The burst's phase record is the caller's
    /// `SetPhase(PHASE_LOADING)`, so a caller that also wrote one would put two on the wire.
    #[test]
    fn no_loading_burst_half_carries_a_phase_record() {
        let burst = loading_burst(&hero(), &hero_points(), 7, &empty_view());
        for frame in burst
            .before_map_test
            .iter()
            .chain(burst.after_map_test.iter())
        {
            assert_ne!(
                frame[0], 253,
                "GC_PHASE is the descriptor's phase transition"
            );
        }
    }

    /// `GC_MAIN_CHARACTER2_EMPIRE` is byte 113, not byte 15. Byte 15 is the two-empire
    /// variant legacy never sends, and choosing the wrong one is a silent Parity break.
    #[test]
    fn main_character_uses_the_empire_variant_byte() {
        let mut bytes = Vec::new();
        main_character(&hero(), 0x1122_3344).encode_into(&mut bytes);
        assert_eq!(bytes[0], 113, "byte 113 is GC_MAIN_CHARACTER2_EMPIRE");
        assert_ne!(bytes[0], 15, "byte 15 is HEADER_GC_MAIN_CHARACTER_OLD");
        assert_eq!(bytes.len(), 46, "the empire variant is 46 packed bytes");
    }

    /// The field order of `TPacketGCMainCharacter` (`G/packet.h:1004-1013`): header, dwVID,
    /// wRaceNum, the 25-byte szName, the three longs, empire, skill group.
    #[test]
    fn main_character_bytes_are_in_legacy_field_order() {
        let hero = hero();
        let mut bytes = Vec::new();
        main_character(&hero, 0x1122_3344).encode_into(&mut bytes);
        assert_eq!(
            &bytes[1..5],
            &0x1122_3344u32.to_le_bytes(),
            "dwVID little-endian"
        );
        assert_eq!(&bytes[5..7], &5u16.to_le_bytes(), "wRaceNum is the job");
        assert_eq!(&bytes[7..12], b"Arges", "the Name, NUL-terminated");
        assert_eq!(&bytes[12..32], &[0u8; 20], "the rest of the Name is zero");
        assert_eq!(&bytes[32..36], &hero.x.to_le_bytes(), "lx");
        assert_eq!(&bytes[36..40], &hero.y.to_le_bytes(), "ly");
        assert_eq!(
            &bytes[40..44],
            &[0u8; 4],
            "lz is the world-owned height, zero here"
        );
        assert_eq!(bytes[44], 2, "empire");
        assert_eq!(bytes[45], 6, "skill_group");
    }

    /// `strlcpy` copies at most `sizeof - 1` bytes and always terminates, so the last field
    /// byte is the NUL even for a Name the store's `CHECK` would refuse.
    #[test]
    fn a_name_longer_than_the_field_is_truncated_and_terminated() {
        let mut hero = hero();
        hero.name = "A".repeat(40);
        let mut bytes = Vec::new();
        main_character(&hero, 1).encode_into(&mut bytes);
        assert_eq!(bytes.len(), 46, "the field never grows");
        assert_eq!(
            &bytes[7..31],
            &[b'A'; 24],
            "24 Name bytes, as strlcpy copies"
        );
        assert_eq!(bytes[31], 0, "the 25th field byte is the terminator");
    }

    #[test]
    fn a_name_of_exactly_24_characters_fills_the_field() {
        let mut hero = hero();
        hero.name = "B".repeat(24);
        let mut bytes = Vec::new();
        main_character(&hero, 1).encode_into(&mut bytes);
        assert_eq!(&bytes[7..31], &[b'B'; 24]);
        assert_eq!(bytes[31], 0);
    }

    #[test]
    fn gold_record_carries_the_full_64_bit_value() {
        let hero = hero();
        let mut bytes = Vec::new();
        GcGold::new(gold(&hero)).encode_into(&mut bytes);
        assert_eq!(bytes[0], 224, "GC_CHARACTER_GOLD");
        assert_eq!(&bytes[1..9], &0x0012_3456_789a_bcdeu64.to_le_bytes());
    }

    /// The points record is 2041 bytes: one header byte and 255 eight-byte slots. Legacy's
    /// `pack.points[255]` is the last field of `TPacketGCPoints`, so the array starts one byte
    /// after the header.
    #[test]
    fn points_record_is_2041_bytes_of_little_endian_slots() {
        let mut bytes = Vec::new();
        GcPoints::new(points(&hero(), &hero_points())).encode_into(&mut bytes);
        assert_eq!(bytes.len(), 2041, "1 header byte plus 255 eight-byte slots");
        assert_eq!(bytes[0], 16, "GC_CHARACTER_POINTS");
        assert_eq!(&bytes[9..17], &9i64.to_le_bytes(), "slot 1 is POINT_LEVEL");
    }

    /// The two slots legacy leaves uninitialised, pinned so the Defect cannot creep back in.
    #[test]
    fn points_writes_the_two_slots_legacy_leaves_uninitialised() {
        let slots = points(&hero(), &hero_points());
        assert_eq!(
            slots[0], 0,
            "POINT_NONE is written as zero, not stack garbage"
        );
        assert_eq!(
            slots[2], 77,
            "POINT_VOICE carries the character's stored voice"
        );
    }

    #[test]
    fn points_carry_the_stored_and_the_derived_values() {
        let hero = hero();
        let slots = points(&hero, &hero_points());
        assert_eq!(slots[point::POINT_EXP], hero.exp);
        assert_eq!(slots[point::POINT_NEXT_EXP], levels::next_exp(hero.level));
        assert_eq!(slots[point::POINT_HP], i64::from(hero.hp));
        assert_eq!(slots[point::POINT_SP], i64::from(hero.sp));
        assert_eq!(slots[point::POINT_STAMINA], i64::from(hero.stamina));
        assert_eq!(slots[point::POINT_GOLD], hero.gold);
        assert_eq!(slots[point::POINT_CONQUEROR_LEVEL], 3);
        assert_eq!(slots[point::POINT_CONQUEROR_EXP], hero.conqueror_exp);
        assert_eq!(
            slots[point::POINT_CONQUEROR_NEXT_EXP],
            levels::conqueror_next_exp(3),
        );
        assert_eq!(slots[point::POINT_ST], 11);
        assert_eq!(slots[point::POINT_HT], 13);
        assert_eq!(slots[point::POINT_DX], 17);
        assert_eq!(slots[point::POINT_IQ], 19);
        assert_eq!(slots[point::POINT_SUNGMA_STR], 21);
        assert_eq!(slots[point::POINT_SUNGMA_HP], 22);
        assert_eq!(slots[point::POINT_SUNGMA_MOVE], 23);
        assert_eq!(slots[point::POINT_SUNGMA_IMMUNE], 24);
    }

    /// The three base speeds are 100, which is `SetPoint` in `CHARACTER::Init` plus the haste
    /// bonus that is 0 with no party.
    #[test]
    fn points_carry_the_three_base_speeds() {
        let slots = points(&hero(), &hero_points());
        assert_eq!(slots[point::POINT_MOV_SPEED], 100);
        assert_eq!(slots[point::POINT_ATT_SPEED], 100);
        assert_eq!(slots[point::POINT_CASTING_SPEED], 100);
    }

    /// From `POINT_ST` on, every slot is `GetPoint`, except the three conqueror slots, which
    /// come from the row, and the movement speed, which is `GetLimitPoint`.
    #[test]
    fn every_slot_from_point_st_is_the_engine_value() {
        let hero = hero();
        let state = hero_points();
        let slots = points(&hero, &state);
        let overwritten = [
            point::POINT_CONQUEROR_LEVEL,
            point::POINT_CONQUEROR_EXP,
            point::POINT_CONQUEROR_NEXT_EXP,
            point::POINT_MOV_SPEED,
        ];
        for (kind, slot) in slots.iter().enumerate().skip(point::POINT_ST) {
            if overwritten.contains(&kind) {
                continue;
            }
            assert_eq!(*slot, i64::from(state.get_point(kind)), "point slot {kind}");
        }
        assert_eq!(
            slots[point::POINT_MOV_SPEED],
            i64::from(state.limit_point(point::POINT_MOV_SPEED)),
        );
    }

    /// A level-1 warrior with the creation attributes 6, 4, 3 and 3: the five grades
    /// `ComputeBattlePoints` gives it, and the three maxima of `ComputePoints`, each a hand sum
    /// of the warrior row (600 + 4 x 40, 200 + 3 x 20, 800 + 4 x 5).
    #[test]
    fn a_fresh_warrior_is_sent_its_grades_and_maxima() {
        let warrior = Character {
            job: 0,
            level: 1,
            conqueror_level: 0,
            st: 6,
            ht: 4,
            dx: 3,
            iq: 3,
            ..hero()
        };
        let slots = points(&warrior, &load_points(&warrior, 0));
        assert_eq!(slots[point::POINT_ATT_GRADE], 14);
        assert_eq!(slots[point::POINT_DEF_GRADE], 4);
        assert_eq!(slots[point::POINT_CLIENT_DEF_GRADE], 5);
        assert_eq!(slots[point::POINT_MAGIC_ATT_GRADE], 8);
        assert_eq!(slots[point::POINT_MAGIC_DEF_GRADE], 5);
        assert_eq!(slots[point::POINT_MAX_HP], 760);
        assert_eq!(slots[point::POINT_MAX_SP], 260);
        assert_eq!(slots[point::POINT_MAX_STAMINA], 820);
    }

    /// The slots whose state the Rewrite does not store are what `GetPoint` answers for them,
    /// which is 0: the inventory unlocks, the Gaya, the biologist and the protected inventory.
    #[test]
    fn the_slots_of_unstored_state_are_zero() {
        let slots = points(&hero(), &hero_points());
        for kind in [
            point::POINT_INVEN,
            point::POINT_GAYA,
            point::POINT_BIOLOGIST_STATE,
            point::POINT_BIOLOGIST_ITEMS_TAKEN,
            point::POINT_BIOLOGIST_COMPLETED,
            point::POINT_SECURED_STATE,
            point::POINT_SECURED_PASSWORD,
        ] {
            assert_eq!(slots[kind], 0, "point slot {kind}");
        }
    }

    /// The maxima are the engine's, from the assassin row: 650 + 13 x 40 hit points,
    /// 200 + 19 x 20 spell points and 800 + 13 x 5 stamina. The stored hit points are far
    /// above the maximum, and the loading burst sends them unclamped, as legacy does.
    #[test]
    fn point_maxima_are_derived_from_the_attributes() {
        let hero = hero();
        let slots = points(&hero, &hero_points());
        assert_eq!(slots[point::POINT_MAX_HP], 1170);
        assert_eq!(slots[point::POINT_MAX_SP], 580);
        assert_eq!(slots[point::POINT_MAX_STAMINA], 865);
        assert_eq!(slots[point::POINT_HP], 0x0012_3456);
    }

    /// Map 384 wants a movement will of 65 and a hit-point will of 60. The hero is a conqueror
    /// with 23 and 22, so the client is told half its speed and half its maximum hit points,
    /// in the points record and in its own insert record.
    #[test]
    fn a_weak_conqueror_is_sent_half_its_speed_and_hit_points() {
        let hero = hero();
        let state = load_points(&hero, 384);
        let slots = points(&hero, &state);
        assert_eq!(slots[point::POINT_MOV_SPEED], 50);
        assert_eq!(slots[point::POINT_MAX_HP], 585);
        let mut bytes = Vec::new();
        character_add(&hero, &state, 1).encode_into(&mut bytes);
        assert_eq!(bytes[24], 50, "bMovingSpeed");
        assert_eq!(bytes[25], 100, "bAttackSpeed");
    }

    /// `TPacketGCPointChange` (`G/packet.h:1064-1071`): a four-byte `int` header, the VID, the
    /// type byte, then the amount and the value as `long long`.
    #[test]
    fn a_point_change_is_25_bytes_in_legacy_field_order() {
        let record = PointRecord {
            kind: 0x05,
            amount: -0x0102_0304_0506_0708,
            value: 0x1112_1314_1516_1718,
            broadcast: false,
        };
        let frames = point_changes(&[record], 0x2122_2324);
        assert_eq!(frames.len(), 1);
        let mut expected = vec![0x11, 0, 0, 0, 0x24, 0x23, 0x22, 0x21, 0x05];
        expected.extend_from_slice(&(-0x0102_0304_0506_0708i64).to_le_bytes());
        expected.extend_from_slice(&[0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11]);
        assert_eq!(frames[0], expected);
    }

    /// The item load's tail: `CheckMaximumPoints` clamps the stored hit and spell points, one
    /// record each, and only then `PointsPacket` sends the gold and the clamped points.
    #[test]
    fn the_item_load_clamps_the_pools_before_the_points_packet() {
        let hero = hero();
        let mut state = hero_points();
        let frames = item_load_points(&hero, &mut state, 0x0a0b_0c0d);
        assert_eq!(headers(&frames), vec![0x11, 0x11, 224, 16]);
        let hp = GcPointChange::decode(&frames[0]).expect("a point change");
        assert_eq!(
            (hp.vid, hp.change_type, hp.amount, hp.value),
            (0x0a0b_0c0d, 5, 0, 1170),
        );
        let sp = GcPointChange::decode(&frames[1]).expect("a point change");
        assert_eq!((sp.change_type, sp.amount, sp.value), (7, 0, 580));
        assert_eq!((state.hp(), state.sp()), (1170, 580));
        let hp_slot = 1 + 8 * point::POINT_HP;
        assert_eq!(&frames[3][hp_slot..hp_slot + 8], &1170i64.to_le_bytes());
    }

    /// A character inside its maxima is not clamped, so the item load sends only the pair.
    #[test]
    fn the_item_load_sends_no_change_for_pools_inside_their_maxima() {
        let hero = Character {
            hp: 1170,
            sp: 1,
            ..hero()
        };
        let mut state = load_points(&hero, 0);
        let frames = item_load_points(&hero, &mut state, 1);
        assert_eq!(headers(&frames), vec![224, 16]);
    }

    #[test]
    fn skill_levels_are_255_empty_slots() {
        let skills = skill_levels();
        assert_eq!(skills.len(), 255);
        for (index, skill) in skills.iter().enumerate() {
            assert_eq!(skill.master_type, 0, "skill slot {index}");
            assert_eq!(skill.level, 0, "skill slot {index}");
            assert_eq!(skill.next_read, 0, "skill slot {index}");
        }
        let mut bytes = Vec::new();
        GcSkillLevelNew {
            header: GcSkillLevelNew::header(),
            skills,
        }
        .encode_into(&mut bytes);
        assert_eq!(bytes.len(), 1531, "1 header byte plus 255 six-byte slots");
    }

    #[test]
    fn the_loaded_parts_are_the_base_part_and_the_stored_hair_and_sash() {
        // `SetPlayerProto` puts the base part in the main part and reads the hair and the sash
        // from the row; the stored armour part (0x1122) is what the select screen shows, and
        // the load puts it back only by wearing the armour.
        let parts = hero_points().parts();
        assert_eq!(parts[0], 1, "the base part");
        assert_eq!(parts[1], 0, "weapon: nothing is worn");
        assert_eq!(parts[2], 0, "head");
        assert_eq!(parts[3], 0x3344, "hair");
        assert_eq!(parts[4], 0x5566, "sash");
        assert_eq!(parts[5], 0, "aura: not stored");
    }

    /// `TPacketGCCharacterAdd` is 35 packed bytes and its header is **byte 1**, not 68: byte
    /// 68 is `GC_CHARACTER_POSITION`.
    #[test]
    fn character_add_is_35_bytes_under_byte_one() {
        let hero = hero();
        let mut bytes = Vec::new();
        character_add(&hero, &hero_points(), 0x1122_3344).encode_into(&mut bytes);
        assert_eq!(bytes.len(), 35, "1 header byte plus the 34-byte payload");
        assert_eq!(bytes[0], 1, "HEADER_GC_CHARACTER_ADD is byte 1");
        assert_ne!(bytes[0], 68, "byte 68 is GC_CHARACTER_POSITION");
        assert_eq!(&bytes[1..5], &0x1122_3344u32.to_le_bytes(), "dwVID");
        assert_eq!(
            &bytes[5..9],
            &0u32.to_le_bytes(),
            "the angle is 0 when not moving"
        );
        assert_eq!(&bytes[9..13], &hero.x.to_le_bytes(), "x");
        assert_eq!(&bytes[13..17], &hero.y.to_le_bytes(), "y");
        assert_eq!(&bytes[17..21], &[0u8; 4], "z");
        assert_eq!(
            bytes[21], 6,
            "bType is GetCharType(), CHAR_TYPE_PC for every PC"
        );
        assert_eq!(&bytes[22..24], &5u16.to_le_bytes(), "wRaceNum");
        assert_eq!(bytes[24], BASE_SPEED, "bMovingSpeed");
        assert_eq!(bytes[25], BASE_SPEED, "bAttackSpeed");
        assert_eq!(bytes[26], 0, "bStateFlag");
        assert_eq!(&bytes[27..35], &[0u8; 8], "the two affect-flag words");
    }

    /// The summary record is 70 packed bytes: 1 header, 4 vid, 25 name, 12 parts, then the
    /// eleven trailing fields. `dwNewIsGuildName` is a `BYTE`, which is what makes 70 the right
    /// total and not 73.
    #[test]
    fn character_additional_is_70_bytes_over_byte_136() {
        let hero = hero();
        let mut bytes = Vec::new();
        character_additional(&hero, &hero_points(), 0x1122_3344, 3).encode_into(&mut bytes);
        assert_eq!(bytes.len(), 70);
        assert_eq!(bytes[0], 136, "GC_CHAR_ADDITIONAL_INFO");
        assert_eq!(&bytes[1..5], &0x1122_3344u32.to_le_bytes(), "dwVID");
        assert_eq!(&bytes[5..10], b"Arges", "szName");
        assert_eq!(&bytes[10..30], &[0u8; 20], "the rest of szName is zero");
        assert_eq!(
            &bytes[30..42],
            &[1, 0, 0, 0, 0, 0, 0x44, 0x33, 0x66, 0x55, 0, 0],
            "awPart is main, weapon, head, hair, sash, aura as six LE words",
        );
        assert_eq!(bytes[42], 2, "bEmpire");
        assert_eq!(
            &bytes[43..47],
            &0u32.to_le_bytes(),
            "dwGuildID: no guild is ported"
        );
        assert_eq!(&bytes[47..51], &9u32.to_le_bytes(), "dwLevel");
        assert_eq!(&bytes[51..55], &3u32.to_le_bytes(), "dwConquerorLevel");
        assert_eq!(&bytes[55..57], &0i16.to_le_bytes(), "sAlignment");
        assert_eq!(bytes[57], 0, "bPKMode");
        assert_eq!(&bytes[58..62], &0u32.to_le_bytes(), "dwMountVnum");
        assert_eq!(bytes[62], 0, "bRefineElementType");
        assert_eq!(bytes[63], 0, "dwNewIsGuildName is a BYTE");
        assert_eq!(bytes[64], 0, "byPremium");
        assert_eq!(&bytes[65..69], &0i32.to_le_bytes(), "iPremiumTime");
        assert_eq!(bytes[69], 3, "bLanguage comes from the descriptor");
    }

    /// The enter-game burst, one record per frame, in the order `CInputLogin::Entergame`
    /// writes them. With an empty map and nobody in view it is six records around the
    /// descriptor's own `SetPhase(PHASE_GAME)`.
    #[test]
    fn enter_game_burst_headers_are_in_legacy_order() {
        let burst = enter_game_burst(
            &hero(),
            &hero_points(),
            0x1122_3344,
            &empty_view(),
            1,
            1_600_000_000,
            3,
        );
        assert_eq!(
            headers(&burst.before_phase),
            vec![1, 136, 126],
            "CHARACTER_ADD, CHAR_ADDITIONAL_INFO, AFFECT_ADD",
        );
        assert_eq!(
            headers(&burst.after_phase),
            vec![106, 121, 4],
            "TIME, CHANNEL, CHAT, all after the phase change",
        );
    }

    /// Neither half carries a `GC_PHASE`: `Entergame`'s is the descriptor's phase transition,
    /// and writing it as content as well would put two on the wire.
    #[test]
    fn no_enter_game_half_carries_a_phase_record() {
        let burst = enter_game_burst(&hero(), &hero_points(), 1, &empty_view(), 1, 0, 0);
        for frame in all(&burst.before_phase, &burst.after_phase) {
            assert_ne!(
                frame[0], 253,
                "GC_PHASE is the descriptor's phase transition"
            );
        }
    }

    /// `CInputDB::PlayerLoad` folds an instance map onto its base map before the allow test.
    #[test]
    fn an_instance_map_folds_onto_its_base_map() {
        assert_eq!(public_map_index(10_000), 1, "10000 is instance 1");
        assert_eq!(public_map_index(10_501), 1, "the tail is dropped");
        assert_eq!(
            public_map_index(19_999),
            1,
            "the whole instance range folds"
        );
        assert_eq!(public_map_index(20_000), 2, "20000 is instance 2");
    }

    /// A map below the instance base is its own public map, unchanged.
    #[test]
    fn a_base_map_is_its_own_public_map() {
        for map in [0, 1, 56, 9999] {
            assert_eq!(public_map_index(map), map, "map {map} is unchanged");
        }
    }

    /// The allow test is membership in the Channel's configured maps, which are base maps.
    #[test]
    fn the_allow_test_is_membership_in_the_channel_maps() {
        let maps = [1, 3, 4, 21, 373];
        assert!(map_is_allowed(21, &maps), "a listed map is allowed");
        assert!(!map_is_allowed(2, &maps), "an unlisted map is refused");
        assert!(
            !map_is_allowed(21, &[]),
            "a Channel with no maps allows nothing"
        );
    }

    /// An instance map is allowed when its base map is listed, which is what legacy's fold is
    /// for. Testing the raw index would refuse it.
    #[test]
    fn an_instance_map_is_allowed_through_its_base_map() {
        let maps = [1, 21];
        assert!(
            map_is_allowed(public_map_index(10_501), &maps),
            "instance 10501 is instance 1, and 1 is listed"
        );
        assert!(
            !map_is_allowed(public_map_index(20_501), &maps),
            "instance 20501 is instance 2, which is not listed"
        );
    }

    /// The configured list is a `u32` per map and the map index is a signed `long`. A map
    /// index above `i32::MAX` is not a legacy map, but it must not wrap into a listed value.
    #[test]
    fn a_wide_map_index_cannot_wrap_into_a_listed_map() {
        assert!(
            !map_is_allowed(i32::MIN, &[u32::MAX >> 1]),
            "a negative index matches nothing"
        );
    }

    /// The revive-invisible affect is unconditional: `Entergame` calls `ReviveInvisible(5)`
    /// after the NPC records and before `SetPhase(PHASE_GAME)`.
    #[test]
    fn the_revive_invisible_affect_is_sent_once_with_five_seconds() {
        let burst = enter_game_burst(&hero(), &hero_points(), 1, &empty_view(), 1, 0, 0);
        let affects: Vec<&Vec<u8>> = all(&burst.before_phase, &burst.after_phase)
            .into_iter()
            .filter(|f| f[0] == 126)
            .collect();
        assert_eq!(affects.len(), 1, "exactly one GC_AFFECT_ADD");
        let frame = affects[0];
        assert_eq!(
            frame.len(),
            22,
            "1 header byte plus the 21-byte affect element"
        );
        assert_eq!(&frame[1..5], &215u32.to_le_bytes(), "AffectType 215");
        assert_eq!(frame[5], 0, "bPointIdxApplyOn");
        assert_eq!(&frame[6..10], &0i32.to_le_bytes(), "lApplyValue");
        assert_eq!(&frame[10..14], &28u32.to_le_bytes(), "AFFECT_FLAG 28");
        assert_eq!(
            &frame[14..18],
            &5i32.to_le_bytes(),
            "lDuration is five seconds"
        );
        assert_eq!(&frame[18..22], &0i32.to_le_bytes(), "lSPCost");
    }

    /// `Entergame` calls `SetPhase(PHASE_GAME)` after the revive-invisible affect, so the
    /// affect is the last record before the phase change and the time record is the first
    /// after it.
    #[test]
    fn the_affect_is_the_last_record_before_the_phase_change() {
        let burst = enter_game_burst(&hero(), &hero_points(), 1, &empty_view(), 1, 0, 0);
        assert_eq!(
            burst.before_phase.last().map(|f| f[0]),
            Some(126),
            "GC_AFFECT_ADD is written just before SetPhase(PHASE_GAME)"
        );
        assert_eq!(
            burst.after_phase.first().map(|f| f[0]),
            Some(106),
            "GC_TIME is the first record after it"
        );
    }

    #[test]
    fn time_then_channel_carry_their_whole_values() {
        let burst = enter_game_burst(
            &hero(),
            &hero_points(),
            1,
            &empty_view(),
            4,
            1_600_000_000,
            0,
        );
        let frames = all(&burst.before_phase, &burst.after_phase);
        let time_at = frames.iter().position(|f| f[0] == 106).unwrap();
        let channel_at = frames.iter().position(|f| f[0] == 121).unwrap();
        assert!(time_at < channel_at, "GC_TIME is written before GC_CHANNEL");
        assert_eq!(
            frames[time_at],
            &vec![106, 0x00, 0x10, 0x5e, 0x5f],
            "GC_TIME is 5 bytes"
        );
        assert_eq!(frames[channel_at], &vec![121, 4], "GC_CHANNEL is 2 bytes");
    }

    /// The chat line is the last record. Its `wSize` covers the whole record, the type is a
    /// command line, and the text is the unterminated tail.
    #[test]
    fn the_letters_event_line_is_the_last_record() {
        let burst = enter_game_burst(&hero(), &hero_points(), 1, &empty_view(), 1, 0, 0);
        let last = burst.after_phase.last().unwrap();
        assert_eq!(last[0], 4, "GC_CHAT is byte 4");
        assert_eq!(
            usize::from(last[1]) | (usize::from(last[2]) << 8),
            last.len(),
            "wSize"
        );
        assert_eq!(
            last[3], CHAT_TYPE_COMMAND,
            "the event line is a command line"
        );
        assert_eq!(&last[4..8], &0u32.to_le_bytes(), "id stays 0");
        assert_eq!(last[8], 2, "the character's empire");
        assert_eq!(
            last[9], 1,
            "bCanFormat stays true, as the constructor leaves it"
        );
        assert_eq!(&last[10..], b"letters_event 0");
    }

    /// The client trims the line on a space or a control byte, so the sent text is exactly
    /// `letters_event` with the `0` left for the client to show as a flag. Both are what
    /// `SEventLetters` writes; the sent bytes must keep the space.
    #[test]
    fn the_letters_event_line_keeps_its_space() {
        let burst = enter_game_burst(&hero(), &hero_points(), 1, &empty_view(), 1, 0, 0);
        let last = burst.after_phase.last().unwrap();
        let text = &last[10..];
        assert_eq!(text, b"letters_event 0");
        assert!(text.contains(&b' '), "the space is part of the sent bytes");
    }

    /// An empty NPC list sends nothing at all: `SendNPCPosition` returns before it writes.
    #[test]
    fn an_empty_npc_list_sends_no_record() {
        let burst = enter_game_burst(&hero(), &hero_points(), 1, &empty_view(), 1, 0, 0);
        assert!(
            !all(&burst.before_phase, &burst.after_phase)
                .into_iter()
                .any(|f| f[0] == 115),
            "GC_NPC_POSITION is byte 115 and must be absent",
        );
    }

    /// An NPC whose every field differs, so that no field can stand in for another.
    fn smith() -> Npc {
        Npc {
            vid: 0x8000_0007,
            vnum: 0x1_4e30,
            race: 0x4e30,
            char_type: CHAR_TYPE_NPC,
            x: 12_345,
            y: 23_456,
            z: 7,
            rotation: 135,
            empire: 3,
            moving_speed: 120,
            attack_speed: 90,
            name: b"Fierar".to_vec(),
        }
    }

    /// An NPC's insert carries each of its own fields, and nothing it does not have.
    #[test]
    fn an_npc_insert_carries_each_of_its_fields() {
        let add = npc_add(&smith());
        assert_eq!(add.dw_vid, 0x8000_0007);
        assert_eq!(
            add.angle.to_bits(),
            135f32.to_bits(),
            "the angle is its rotation"
        );
        assert_eq!((add.x, add.y, add.z), (12_345, 23_456, 7));
        assert_eq!((add.b_type, add.w_race_num), (CHAR_TYPE_NPC, 0x4e30));
        assert_eq!((add.b_moving_speed, add.b_attack_speed), (120, 90));
        assert_eq!((add.b_state_flag, add.dw_affect_flag), (0, [0, 0]));
        let warp = Npc {
            char_type: CHAR_TYPE_WARP,
            ..smith()
        };
        assert_eq!(
            npc_add(&warp).b_type,
            CHAR_TYPE_WARP,
            "the type is the proto's"
        );
    }

    /// An NPC's summary is its VID, name, empire and `PK_MODE_FREE`; a warp or goto has none.
    #[test]
    fn only_an_npc_of_type_npc_has_a_summary() {
        let summary = npc_additional(&smith()).unwrap();
        assert_eq!(summary.dw_vid, 0x8000_0007);
        assert_eq!(&summary.name[..7], b"Fierar\0");
        assert_eq!((summary.b_empire, summary.b_pk_mode), (3, PK_MODE_FREE));
        assert_eq!((summary.dw_level, summary.b_language), (0, 0));
        for char_type in [
            CHAR_TYPE_WARP,
            CHAR_TYPE_GOTO,
            CHAR_TYPE_MONSTER,
            CHAR_TYPE_STONE,
        ] {
            let other = Npc {
                char_type,
                ..smith()
            };
            assert!(npc_additional(&other).is_none(), "type {char_type}");
        }
    }

    /// A mini-map entry carries its own type, name and position.
    #[test]
    fn a_mini_map_entry_carries_its_own_fields() {
        let position = NpcPosition {
            char_type: CHAR_TYPE_GOTO,
            name: b"Poarta".to_vec(),
            x: 62_200,
            y: 55_600,
        };
        let entry = npc_position(&position);
        assert_eq!(entry.npc_type, CHAR_TYPE_GOTO);
        assert_eq!(&entry.name[..7], b"Poarta\0");
        assert_eq!((entry.x, entry.y), (62_200, 55_600));
    }

    /// A name is cut at its first NUL and at 24 bytes, and the field always ends with a NUL.
    #[test]
    fn a_name_field_is_cut_as_strlcpy_cuts_it() {
        assert_eq!(&name_bytes_field(b"Ab\0cd")[..4], b"Ab\0\0");
        let long = name_bytes_field(&[b'x'; 30]);
        assert_eq!(long[..24], [b'x'; 24]);
        assert_eq!(long[24], 0, "the 25th byte is the terminator");
    }

    /// A character already in view adds two records, its insert and its summary, after the
    /// entering character's own pair.
    #[test]
    fn a_visible_character_adds_its_pair_after_the_own_pair() {
        let burst = enter_game_burst(&hero(), &hero_points(), 1, &with_neighbour(), 1, 0, 3);
        assert_eq!(
            headers(&burst.before_phase),
            vec![1, 136, 1, 136, 126],
            "the neighbour's pair sits between the own pair and the affect",
        );
        assert_eq!(
            burst.before_phase[2][21], CHAR_TYPE_PC,
            "every PC's insert carries CHAR_TYPE_PC, the neighbour's too"
        );
        let summary = &burst.before_phase[3];
        assert_eq!(
            &summary[1..5],
            &0x0a0b_0c0du32.to_le_bytes(),
            "the neighbour's VID"
        );
        assert_eq!(&summary[5..12], b"Spectre");
        assert_eq!(summary[69], 5, "the neighbour's own descriptor's language");
        assert_eq!(
            burst.before_phase[1][69], 3,
            "the entering descriptor's language"
        );
    }

    /// A second character in view is described by `GC_ENTITY` at load. `wSize` is a `WORD`
    /// holding the whole record, and each `TPacketEntityInfo` is 28 bytes.
    #[test]
    fn a_visible_character_reaches_the_loading_burst() {
        let burst = loading_burst(&hero(), &hero_points(), 1, &with_neighbour());
        let entity = &burst.before_map_test[0];
        assert_eq!(entity[0], 249, "GC_ENTITY is byte 249");
        assert_eq!(entity.len(), 31, "3 header bytes plus one 28-byte entry");
        assert_eq!(
            usize::from(entity[1]) | (usize::from(entity[2]) << 8),
            entity.len(),
            "wSize is the whole record",
        );
        assert_eq!(&entity[3..7], &0x0a0b_0c0du32.to_le_bytes(), "dwVID");
        assert_eq!(&entity[7..11], &7u32.to_le_bytes(), "dwRaceVNum is a DWORD");
        assert_eq!(
            &entity[11..23],
            &[0x11, 0x11, 0x22, 0x22, 0x33, 0x33, 0x44, 0x44, 0x55, 0x55, 0x66, 0x66],
            "wPart[CHR_EQUIPPART_NUM] is six little-endian words",
        );
        assert_eq!(&entity[23..27], &0x0000_1000i32.to_le_bytes(), "xPos");
        assert_eq!(&entity[27..31], &(-0x0000_2000i32).to_le_bytes(), "yPos");
    }

    /// `GC_ENTITY` is still sent when nobody is in view: the client expects the record, and
    /// legacy writes it with a size of 3 and no entries.
    #[test]
    fn an_empty_view_still_sends_the_entity_record() {
        let burst = loading_burst(&hero(), &hero_points(), 1, &empty_view());
        assert_eq!(
            burst.before_map_test[0],
            vec![249, 3, 0],
            "a size of 3 and no entries"
        );
    }

    /// The quickslot records come from the store, which the descriptor reads, so the burst
    /// built from the row alone holds none.
    #[test]
    fn the_burst_leaves_the_quickslot_records_to_the_descriptor() {
        let burst = loading_burst(&hero(), &hero_points(), 1, &empty_view());
        assert!(
            !all(&burst.before_map_test, &burst.after_map_test)
                .into_iter()
                .any(|f| f[0] == 28),
            "GC_QUICKSLOT_ADD is byte 28 and must be absent",
        );
    }

    /// `GC_HYBRIDCRYPT_SDB` is a loading record, not an enter-game one, and `legacy/` has no
    /// `package_info.txt`, so legacy sends it for no map. It must be absent from both bursts.
    #[test]
    fn the_package_sdb_is_absent_from_both_bursts() {
        let loading = loading_burst(&hero(), &hero_points(), 1, &with_neighbour());
        let entering = enter_game_burst(&hero(), &hero_points(), 1, &with_neighbour(), 1, 0, 0);
        assert!(
            !all(&loading.before_map_test, &loading.after_map_test)
                .into_iter()
                .any(|f| f[0] == 153),
            "absent from the loading burst"
        );
        assert!(
            !all(&entering.before_phase, &entering.after_phase)
                .into_iter()
                .any(|f| f[0] == 153),
            "absent from enter-game"
        );
    }

    /// A hard-coded welcome would be a Divergence: the snapshot has no `GREET` row.
    #[test]
    fn no_hard_coded_greeting_is_sent() {
        let burst = enter_game_burst(&hero(), &hero_points(), 1, &empty_view(), 1, 0, 0);
        let lines: Vec<&Vec<u8>> = all(&burst.before_phase, &burst.after_phase)
            .into_iter()
            .filter(|f| f[0] == 4)
            .collect();
        assert_eq!(lines.len(), 1, "only the letters_event line");
        assert_eq!(&lines[0][10..], b"letters_event 0");
    }

    /// Each record is its own frame. Concatenating them would hand the client one TEA unit
    /// where legacy hands it several.
    #[test]
    fn every_record_is_its_own_frame() {
        let loading = loading_burst(&hero(), &hero_points(), 1, &with_neighbour());
        let entering = enter_game_burst(&hero(), &hero_points(), 1, &with_neighbour(), 1, 0, 0);
        for half in [
            &loading.before_map_test,
            &loading.after_map_test,
            &entering.before_phase,
            &entering.after_phase,
        ] {
            assert!(half.iter().all(|frame| !frame.is_empty()));
            assert!(!half.is_empty(), "every half of a burst has a record");
        }
    }
}
