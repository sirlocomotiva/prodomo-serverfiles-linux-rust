//! A player's skills: `UseSkill`'s entry gates, SP cost and cooldown stamp, the
//! melee-skill damage arm on monsters (ledger 230d), the shoot path, the SPLASH
//! walk, and the RANGE/MAGIC pipes (ledger 230e), and skill-up, the self buffs
//! and the grandmaster wire (ledger 230f).
//!
//! A melee skill lands in two client steps (`G/input_main.cpp:1918-2009`,
//! `G/char_skill.cpp:2896-3135`). `CG_USE_SKILL` runs [`GameState::use_skill`],
//! which checks `CanUseSkill`, the stun gate (`CanMove` on the stun affect
//! only — no shop, riding, or polymorph is ported), the row, the level,
//! charges the SP (`kSPCostPoly` on `POINT_SP`; no owner PC row sets
//! `USE_HP_AS_COST`) and stamps the cooldown (`TSkillUseInfo::UseSkill`).
//! `CG_ATTACK` with `bType > 0` runs [`GameState::skill_attack`], which
//! checks `CanUseSkill` again, refuses the 14 shoot-list skills, checks
//! `CheckSkillHitCount` and the 1500 ms skill-term gate, then runs
//! `ComputeSkill`'s `FuncSplashDamage` arm on the monster victim.
//!
//! A shoot-listed skill lands in three: `CG_FLY_TARGETING` stores the main VID
//! and `CG_ADD_FLY_TARGETING` appends more ([`GameState::fly_target`], each
//! relayed around but the shooter), then `CG_SHOOT` runs [`GameState::shoot`]
//! (`CFuncShoot` over the main VID first, then the extras, clearing both):
//! the plain bow arm (type 0, assassins only), the arrow-skill arm (vnum 47),
//! the MELEE/MAGIC skill arms (20/35/81/91), and `YONGBI`'s `OnMove` only.
//!
//! Clocks: legacy stamps wall ms (`get_dword_time()`). The Rewrite stamps
//! Pulses: the cooldown is `ComputeCooltime(cool*1000)` ms counted in Pulses
//! (rounding up, 40 ms a Pulse), and the 1500 ms term gate fires past 37
//! Pulses (`1500*25/1000 = 37.5`, so `delta > 37`). A Divergence under V17(f).
//!
//! Only the 230d melee rows and the 230e splash/range/magic rows cast through
//! `skill_attack`/`shoot` (see [`gamedata::pc_skill_data::PcSkill::is_castable_melee`]
//! and its 230e siblings); the 230f buffs and the heal cast through `use_skill`'s
//! dispatch tail, 230g fans the party-flagged buffs to the near members, and 230h
//! ports the combo toggle and the charge pair; MUYEONG's row is refused at load,
//! so its self-compute arm stays documented (the hide's readers live in
//! `mob_battle::find_victim`).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use common::cfloat::{f32_to_i32, f64_to_f32, f64_to_i32};
use common::enums::EWearPositions;
use common::point_slot as point;
use common::vid::Vid;
use gamedata::item_kind::{ITEM_WEAPON, WEAPON_ARROW, WEAPON_BOW};
use gamedata::mob_proto::AIFLAG_NOMOVE;
use gamedata::mob_skill_data::{
    SKILL_ATTR_MAGIC, SKILL_ATTR_RANGE, SKILL_FLAG_ATTACK, SKILL_FLAG_CRUSH, SKILL_FLAG_CRUSH_LONG,
    SKILL_FLAG_DISABLE_BY_POINT_UP, SKILL_FLAG_FIRE_CONT, SKILL_FLAG_IGNORE_TARGET_RATING,
    SKILL_FLAG_PARTY, SKILL_FLAG_PENETRATE, SKILL_FLAG_POISON, SKILL_FLAG_REMOVE_BAD_AFFECT,
    SKILL_FLAG_SELFONLY, SKILL_FLAG_SPLASH, SKILL_FLAG_STUN, SKILL_FLAG_TOGGLE,
    SKILL_FLAG_USE_ARROW_DAMAGE,
};
use gamedata::pc_skill_data::SHOOT_LIST;
use world::character::{
    attack_rating, battle_damage_floor, calc_arrow_skill_atk, calc_melee_skill_atk, damage_magic,
    damage_melee, damage_normal_range, damage_range, number, skill_magic_attr, skill_melee_attr,
    skill_range_attr, AttackRatingInput, Equipment, Points, Striker, Struck,
};
use world::npc::Npc;

use super::fight::{is_struck, AggroDamage, AFF_STUN_WORD};
use crate::chat_line::Arg;
use crate::item_move::Mover;
use protocol::gc_chat::{CHAT_TYPE_COMMAND, CHAT_TYPE_INFO, CHAT_TYPE_TALKING};

/// `SKILL_YEONSA` (`G/skill.h:79`): the shoot arm's arrow-skill list opens here.
const SKILL_YEONSA: u32 = 46;
/// `SKILL_KWANKYEOK`: 47, the only hosted arrow-skill row.
const SKILL_KWANKYEOK: u32 = 47;
/// `SKILL_HWAJO`: 48.
const SKILL_HWAJO: u32 = 48;
/// `SKILL_GIGUNG` (`G/skill.h:84`): 50 (49 is `SKILL_GYEONGGONG`, not a shoot arm).
const SKILL_GIGUNG: u32 = 50;
/// `SKILL_HORSE_WILDATTACK_RANGE`: 140.
pub(super) const SKILL_HORSE_WILDATTACK_RANGE: u32 = 140;
/// `SKILL_YONGBI` (`G/skill.h:109`): 92, whose shoot arm runs `OnMove(true)` only.
const SKILL_YONGBI: u32 = 92;
/// `SKILL_CHAIN` (`G/skill.h:121`): 108.
const SKILL_CHAIN: u32 = 108;
/// `JOB_ASSASSIN`: the only job the plain bow arm shoots for.
pub(super) const JOB_ASSASSIN: u8 = 1;
/// The plain bow shot's SP charge, skipped by skill group 2
/// (`G/char_battle.cpp:3086-3090`).
const BOW_SHOT_SP: i32 = 5;
/// `SKILL_NORMAL` (`C/length.h:761`): master type 0.
pub(super) const SKILL_NORMAL: u8 = 0;
/// `SKILL_MASTER` (`C/length.h:762`): master type 1.
const SKILL_MASTER: u8 = 1;
/// `SKILL_GRAND_MASTER` (`C/length.h:763`): master type 2.
pub(super) const SKILL_GRAND_MASTER: u8 = 2;
/// `SKILL_PERFECT_MASTER` (`C/length.h:764`): master type 3.
const SKILL_PERFECT_MASTER: u8 = 3;
/// `SKILL_ADD_HP` (`G/skill.h:159`): 141, the max-HP passive.
pub(super) const SKILL_ADD_HP: u32 = 141;
/// `SKILL_RESIST_PENETRATE` (`G/skill.h:160`): 142, the resist-penetrate passive.
pub(super) const SKILL_RESIST_PENETRATE: u32 = 142;
/// `SKILL_7_A_ANTI_TANHWAN` (`G/skill.h:129`): 112, opening the 7/8 anti range.
pub(super) const SKILL_ANTI_FIRST: u32 = 112;
/// `SKILL_8_D_ANTI_BYEURAK` (`G/skill.h:137`): 119, closing it.
pub(super) const SKILL_ANTI_LAST: u32 = 119;
/// `SKILL_ANTI_PALBANG` (`G/skill.h:182`): 221, opening the compiled-in
/// `__7AND8TH_SKILLS__` `do_skillup` cases (221-228, `G/cmd_general.cpp:953-960`).
pub(super) const SKILL_NEW_FIRST: u32 = 221;
/// `SKILL_ANTI_BYEURAK` (`G/skill.h:189`): 228, closing them.
pub(super) const SKILL_NEW_LAST: u32 = 228;
/// `SKILL_HELP_PALBANG` (`G/skill.h:190`): 236, opening the help cases
/// (236-243, `G/cmd_general.cpp:961-968`).
pub(super) const SKILL_HELP_FIRST: u32 = 236;
/// `SKILL_HELP_BYEURAK` (`G/skill.h:197`): 243, closing them.
pub(super) const SKILL_HELP_LAST: u32 = 243;
/// `SKILL_MAX_NUM` (`C/length.h:72`): the skill array's width.
pub(super) const SKILL_MAX_NUM: u32 = 255;
/// `SKILL_TERROR` (`G/skill.h:92`): 64, whose cooling refusal pre-empts the charge.
const SKILL_TERROR: u32 = 64;
/// `SKILL_EUNHYUNG` (`G/skill.h:74`): 34.
const SKILL_EUNHYUNG: u32 = 34;
/// `SKILL_MUYEONG` (`G/skill.h:100`): 78.
const SKILL_MUYEONG: u32 = 78;
/// `SKILL_TUSOK` (`G/skill.h:103`): 80, the cure's last removal.
const SKILL_TUSOK: u32 = 80;
/// `SKILL_CHUNKEON` (`G/skill.h:65`): 19, whose buff carries
/// `CHEONGEUN_WITH_FALL` below GRAND.
const SKILL_CHUNKEON: u32 = 19;
/// `AFF_CHEONGEUN_WITH_FALL` (`G/affect.h:249`): flag 34, bit 1 of word 1.
const AFF_CHEONGEUN_WITH_FALL: u32 = 34;
/// `IMMUNE_STUN` (`C/length.h:715`). `IMMUNE_SLOW` has no reader: SLOW's only hosted
/// row (80) is refused, so the SLOW affect arm is documented, not ported (230e).
const IMMUNE_STUN: u32 = 1 << 0;
/// `AFFECT_FIRE` (`G/affect.h:41`): 216, counting from `AFFECT_MOV_SPEED = 200`.
const AFFECT_FIRE: u32 = 216;
/// `AFF_FIRE` (`G/affect.h:244`): flag 30, counting from `AFF_NONE`.
const AFF_FIRE: u32 = 30;
/// The CRUSH slide's length for a player caster (`G/char_skill.cpp:1904-1905`).
const CRUSH_SLIDE_PC: f32 = 200.0;
/// `SKILLBOOK_DELAY_MIN/MAX` (`C/length.h:73-74`): the book-read delay window,
/// 18-30 h in `get_global_time()` seconds.
const SKILLBOOK_DELAY_MIN: i32 = 64800;
/// `SKILLBOOK_DELAY_MAX` (see [`SKILLBOOK_DELAY_MIN`]).
const SKILLBOOK_DELAY_MAX: i32 = 108_000;
/// The book read's experience cost (`:484-490`): 20,000 below the max level.
const BOOK_NEED_EXP: i64 = 20_000;
/// `aiGrandMasterSkillBookCountForLevelUp` (`G/constants.cpp:921`): the `number(1, count)`
/// width by `min(9, level - 30)`; the read climbs on a 2.
const GRAND_BOOK_COUNT: [i32; 10] = [3, 3, 5, 5, 7, 7, 10, 10, 10, 20];
/// `aiGrandMasterSkillBookMinCount` (`G/constants.cpp:926`): below this count the read
/// always fails.
const GRAND_BOOK_MIN: [i32; 10] = [1, 1, 1, 2, 2, 3, 3, 4, 5, 6];
/// `aiGrandMasterSkillBookMaxCount` (`G/constants.cpp:933`): past this count the read
/// always climbs.
const GRAND_BOOK_MAX: [i32; 10] = [5, 7, 9, 11, 13, 15, 20, 25, 30, 35];
/// `SKILLBOOK_NEXTREAD_MIN/MAX` (`G/config.cpp:58-59`): the GRAND read's stamp window,
/// 8-12 h. The config keys are dropped (our config is fixed); the compiled-in legacy
/// defaults are hardcoded (230g).
const GRAND_READ_DELAY_MIN: i32 = 28800;
/// `SKILLBOOK_NEXTREAD_MAX` (see [`GRAND_READ_DELAY_MIN`]).
const GRAND_READ_DELAY_MAX: i32 = 43200;
/// `AFFECT_SKILL_BOOK_BONUS` (`G/affect.h:75`): 512. No item grants it (only
/// the book arms consume it), so the consume arm is coded and dead.
pub(super) const AFFECT_SKILL_BOOK_BONUS: u32 = 512;
/// `AFFECT_SKILL_NO_BOOK_DELAY` (`G/affect.h:76`): 513, likewise ungranted.
const AFFECT_SKILL_NO_BOOK_DELAY: u32 = 513;
/// `SKILL_LEADERSHIP` (`G/skill.h:142`): 121.
const SKILL_LEADERSHIP: u32 = 121;
/// `SKILL_COMBO` (`G/skill.h:143`): 122.
const SKILL_COMBO: u32 = 122;
/// `SKILL_CREATE` (`G/skill.h:144`): 123.
const SKILL_CREATE: u32 = 123;
/// `SKILL_MINING` (`G/skill.h:145`): 124.
const SKILL_MINING: u32 = 124;
/// `SKILL_POLYMORPH` (`G/skill.h:150`): 129.
const SKILL_POLYMORPH: u32 = 129;
/// `SKILL_HORSE` (`G/skill.h:152`): 130.
const SKILL_HORSE: u32 = 130;
/// `SKILL_HORSE_WILDATTACK` (`G/skill.h:154`): 137.
const SKILL_HORSE_WILDATTACK: u32 = 137;
/// `SKILL_HORSE_CHARGE` (`G/skill.h:155`): 138.
const SKILL_HORSE_CHARGE: u32 = 138;
/// `SKILL_TANHWAN` (`G/skill.h:57`): 5, auto-incremented after
/// `SKILL_GEOMKYUNG` = 4. The owner's vnum-5 row is this skill: the "no owner
/// PC row charges" claim 230d/e left in two comments is wrong (230h).
const SKILL_TANHWAN: u32 = 5;
/// `AFF_TANHWAN_DASH` (`G/affect.h:247`): flag 32, bit 31 of word 0.
const AFF_TANHWAN_DASH: u32 = 32;
/// `SKILL_HORSE_ESCAPE` (`G/skill.h:156`): 139.
const SKILL_HORSE_ESCAPE: u32 = 139;
/// `SKILL_LANGUAGE1` (`G/skill.h:147`): 126, opening the language trio the
/// 50311-50313 arm teaches (the empire gate reads `skill - 126 + 1`).
const SKILL_LANGUAGE1: u32 = 126;
use super::motion::{calculate_duration, distance_sqrt, rotation_to_xy};
use super::GameState;
use crate::sync_position::{SyncPositionVictim, SyncPositionVictimKind};

/// Wall ms per Pulse: the cooldown stamp's clock (`PASSES_PER_SEC(1)`).
const MS_PER_PULSE: u64 = 40;

/// The skill-term gate in Pulses: `1500ms * 25/1000 = 37.5`, and the `>` fires
/// past 37 (`G/char_battle.cpp:297-304`).
const SKILL_TERM_PULSES: u64 = 37;

/// `SKILL_FLAG_USE_HP_AS_COST` (`G/skill.h:14`): bit 5, matching the schema's
/// 6th `setFlag` member.
const SKILL_FLAG_USE_HP_AS_COST: u32 = 1 << 5;

/// `TSkillUseInfo` (`G/char.h:509-533`): one skill's cooldown stamp and hit
/// counts. Pulses, not wall ms (see the module docs).
#[derive(Debug, Clone, Default)]
pub(super) struct SkillUse {
    /// `bUsed`.
    used: bool,
    /// `dwNextSkillUsableTime`, in Pulses; `None` past a zero cooldown.
    next_usable: Option<u64>,
    /// `iHitCount`.
    hit_count: i32,
    /// `dwVID`: the main target's VID.
    main_vid: u32,
    /// `TargetVIDMap`: how many `CG_ATTACK`s of this skill each victim's VID
    /// has seen since the cast.
    hits_per_victim: HashMap<u32, u32>,
    /// `isGrandMaster`: the stamp's `bUseGrandMaster` (the packet path passes
    /// true, the `UseSkill` default, `G/char.h:1485`; ledger 230f).
    grand: bool,
}

impl SkillUse {
    /// `TSkillUseInfo::UseSkill` (`G/char_skill.cpp:3229-3258`): false past an
    /// unexpired stamp (zeroing the hit count), else stamped with `cool`
    /// (`None` past a zero cooldown), `hit_count`, and the grand bit.
    fn stamp(
        &mut self,
        now: u64,
        main_vid: u32,
        cool: Option<u64>,
        hit_count: i32,
        grand: bool,
    ) -> bool {
        if self.used && self.next_usable.is_some_and(|due| due > now) {
            self.hit_count = 0;
            return false;
        }
        self.used = true;
        self.next_usable = cool.map(|pulses| now.saturating_add(pulses));
        self.hit_count = hit_count;
        self.main_vid = main_vid;
        self.grand = grand;
        true
    }
    /// Whether the stamp is cooling at `now` (`bUsed` with an unexpired due).
    fn cooling(&self, now: u64) -> bool {
        self.used && self.next_usable.is_some_and(|due| due > now)
    }

    /// `HitOnce` (`G/char_skill.cpp:3216-3227`): false unused, false past an
    /// expired stamp (`MUYEONG` 78 and `HORSE_WILDATTACK` 137 excepted — neither
    /// casts here), true on `-1`, else decremented while open.
    fn hit_once(&mut self, now: u64, vnum: u32) -> bool {
        if !self.used {
            return false;
        }
        if self.next_usable.is_some_and(|due| due < now) && vnum != 78 && vnum != 137 {
            return false;
        }
        if self.hit_count == -1 {
            return true;
        }
        if self.hit_count > 0 {
            self.hit_count -= 1;
            return true;
        }
        false
    }
}

/// What a player keeps of its skills: its levels, its use stamps, and its fly targets.
#[derive(Debug, Clone)]
pub(super) struct PcSkillState {
    /// `m_pSkillLevels[].bLevel` (`G/char.cpp:2275`): all 0 until a skill-up
    /// lands (the store blob, ledger 230f).
    pub(super) levels: [u8; 255],
    /// `m_pSkillLevels[].tNextRead`: the book-climb delay, 0 until a book sets
    /// one (no book is ported; the wire slot carries it, ledger 230f).
    pub(super) next_read: [u32; 255],
    /// `m_SkillUseInfo`, by skill vnum.
    uses: HashMap<u32, SkillUse>,
    /// `m_dwLastSkillTime`, in Pulses.
    last_skill: Option<u64>,
    /// `m_dwFlyTargetID` (`G/char.h:1439`): the main fly VID `CG_FLY_TARGETING`
    /// stored, 0 while none is held.
    fly_main: u32,
    /// `m_vec_dwFlyTargets`: the extra fly VIDs `CG_ADD_FLY_TARGETING`
    /// appended, in arrival order.
    fly_extra: Vec<u32>,
    /// `m_dwAffectedEunhyungLevel` (`G/char.h:1566`): the skill power of 34 at
    /// the last cloaked attack, 0 while uncloaked (ledger 230f).
    affected_eunhyung: u32,
    /// `m_bComboIndex` (`G/char.h:1431`): the combo toggle's level, 0 while off.
    combo_index: u8,
}

impl Default for PcSkillState {
    fn default() -> Self {
        Self {
            levels: [0; 255],
            next_read: [0; 255],
            uses: HashMap::new(),
            last_skill: None,
            fly_main: 0,
            fly_extra: Vec::new(),
            affected_eunhyung: 0,
            combo_index: 0,
        }
    }
}

/// Which typed skill pipe `skill_damage_typed` runs: the one shared MELEE/RANGE/MAGIC
/// branch (`damage_range`/`damage_magic`), or the dodge-gated NORMAL body of the
/// plain bow shot (`damage_normal_range`, which also answers the stolen gold).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkillDamageKind {
    /// `DAMAGE_TYPE_RANGE`.
    Range,
    /// `DAMAGE_TYPE_MAGIC`.
    Magic,
    /// `DAMAGE_TYPE_NORMAL_RANGE`.
    NormalRange,
}
/// `SkillLevelUp`'s method (`G/char.h:1469-1474`): `BY_POINT` spends, `BY_BOOK`
/// climbs a MASTER (or any type-0), `BY_QUEST` climbs a GRAND. `BY_TRAIN` has
/// no caller anywhere under `server/` and never reaches here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SkillUpMethod {
    /// `SKILL_UP_BY_POINT`.
    Point,
    /// `SKILL_UP_BY_BOOK`.
    Book,
    /// `SKILL_UP_BY_QUEST`.
    Quest,
}

/// `LearnGrandMasterSkill(vnum)` (`G/char_skill.cpp:341-463`): the 7/8th-skills
/// quest's GRAND read, run by `pc.learn_grand_master_skill` while the script that
/// asked still waits for the answer. `total` is the read count including this one,
/// which the bridge already stamped in the quest flags. The read-time gate is
/// legacy's commented-out block: never read. A type-0 answers 949, a non-GRAND
/// 950/951, below the min count the roll always fails, past the max it always
/// climbs, else `number(1, count) == 2` climbs, and the stamp is always
/// `now + number(28800, 43200)` (`SKILLBOOK_NEXTREAD_MIN/MAX`, `G/config.cpp:58-59`).
/// The tail talks 954/955/956 on a climb, 952/953 else. Answers whether the skill
/// climbed; a climb is queued on the session for the store's `store_skill`.
pub(super) fn grand_master_read(
    session: &mut super::quests::Session<'_>,
    vnum: u32,
    total: i32,
) -> bool {
    let tables = session.skill_tables();
    let Some(row) = tables.get(vnum).map(|skill| skill.record) else {
        return false;
    };
    if !session.learnable(&row) {
        session.say(CHAT_TYPE_INFO, b"[LS;948]");
        return false;
    }
    if row.skill_type == 0 {
        session.say(CHAT_TYPE_INFO, b"[LS;949]");
        return false;
    }
    if master_of(session.slot(vnum)) != SKILL_GRAND_MASTER {
        let over = master_of(session.slot(vnum)) > SKILL_GRAND_MASTER;
        let notice: &[u8] = if over { b"[LS;950]" } else { b"[LS;951]" };
        session.say(CHAT_TYPE_INFO, notice);
        return false;
    }
    let last_level = session.slot(vnum);
    let index = usize::from(last_level.saturating_sub(30).min(9));
    let mut book_count = GRAND_BOOK_COUNT[index];
    if session.has_affect(AFFECT_SKILL_BOOK_BONUS) {
        book_count = if book_count & 1 == 1 {
            book_count / 2 + 1
        } else {
            book_count / 2
        };
        session.take_affect(AFFECT_SKILL_BOOK_BONUS);
    }
    let mut climbed = number(session.dice_mut(), 1, book_count) == 2;
    if total < GRAND_BOOK_MIN[index] {
        climbed = false;
    }
    if total > GRAND_BOOK_MAX[index] {
        climbed = true;
    }
    if climbed {
        session.climb_grand(vnum, &row);
    }
    let delay = number(
        session.dice_mut(),
        GRAND_READ_DELAY_MIN,
        GRAND_READ_DELAY_MAX,
    );
    let stamp = GameState::global_time().saturating_add(u32::try_from(delay).unwrap_or(0));
    session.stamp(vnum, stamp);
    if last_level == session.slot(vnum) {
        session.say(CHAT_TYPE_TALKING, b"[LS;952]");
        session.say(CHAT_TYPE_INFO, b"[LS;953]");
        return false;
    }
    session.say(CHAT_TYPE_TALKING, b"[LS;954]");
    session.say(CHAT_TYPE_TALKING, b"[LS;955]");
    session.say(CHAT_TYPE_INFO, b"[LS;956]");
    true
}

impl GameState {
    /// Share the hosted PC skill rows boot loaded. Without them every cast refuses
    /// (as an unknown vnum does).
    #[must_use]
    pub fn with_pc_skills(mut self, skills: Arc<gamedata::pc_skill_data::PcSkills>) -> Self {
        self.pc_skill_tables = skills;
        self
    }

    /// Share the hosted guild skill rows boot loaded. Without them every up and cast
    /// refuses (as an unknown vnum does).
    #[must_use]
    pub fn with_guild_skills(
        mut self,
        skills: Arc<gamedata::guild_skill_data::GuildSkills>,
    ) -> Self {
        self.guild_skill_tables = skills;
        self
    }

    /// The skill state of the player under `vid`, creating it.
    fn pc_skill_state(&mut self, vid: u32) -> &mut PcSkillState {
        self.pc_skills.entry(vid).or_default()
    }

    /// The skill level of the player under `vid` for `vnum`
    /// (`GetSkillLevel`, `G/char_skill.cpp:3198-3210`): 0 past vnum 254,
    /// clamped to 40.
    pub(super) fn pc_skill_level(&self, vid: u32, vnum: u32) -> u8 {
        if vnum >= 255 {
            return 0;
        }
        self.pc_skills
            .get(&vid)
            .map_or(0, |state| state.levels[vnum as usize].min(40))
    }

    /// Set a skill level in memory (tests only, until skill-up lands).
    #[cfg(test)]
    pub(super) fn set_pc_skill_level(&mut self, vid: u32, vnum: u32, level: u8) {
        if vnum < 255 {
            self.pc_skill_state(vid).levels[vnum as usize] = level.min(40);
        }
    }
    /// `do_skillup` + ``SkillLevelUp``(`vnum`, `SKILL_UP_BY_POINT`)
    /// (`G/cmd_general.cpp:904-972`, `G/char_skill.cpp:723-907`): the `/skillup`
    /// chat line's vnum climbs one level when every gate passes. `rest` is the
    /// line after the command word.
    ///
    /// The parse is `one_argument` + `str_to_number<DWORD>`: the first argument's
    /// leading digits, wrapping mod 2^32 (32-bit `strtoul`), a non-digit start
    /// reading 0. An empty arg returns silently.
    ///
    /// The gates, in legacy order: `CanUseSkill` or the else-switch (horse
    /// 137-140, the 7/8 anti 112-119, 141/142, the 7/8 anti/help 221-228/236-243
    /// — `__7AND8TH_SKILLS__` is ON in `prodomodefines.h:165`), the 112-119
    /// zero-level refusal (221-243 never refuse: the quest/grand shapes are
    /// ported, not the unhosted 7/8 types), the unknown-row refusal (a hosted
    /// row, so the switch's 137-140 and the anti vnums refuse here: none is
    /// hosted), the `>= 255` refusal,
    /// `IsLearnableSkill`, the GRAND/PERFECT master refusals, the NORMAL +
    /// `DISABLE_BY_POINT_UP` point-method gate, the level limit, the prereq arm,
    /// the `FIX_SECONDARY_SKILL` group gate, the spend (`SUB_SKILL` for type 0,
    /// `SKILL` for 1-4/6, `HORSE_SKILL` for 5), the climb (`SetSkillLevel`'s
    /// ladder), and the typed 17/30/40 jump draws (`ENABLE_FORCE2MASTERSKILL` is
    /// ON, so the NORMAL draw jumps straight to 20; the quest-flag arm is
    /// compiled out). `g_bSkillDisable` is a dead test global, always false; no
    /// polymorph is ported, so that arm is vacuous. A landed up spends the point
    /// (its point-change record goes out), climbs the level, recomputes the
    /// points, and sends byte 76 with all 255 slots (`SkillLevelPacket`).
    /// Returns the vnum and its new level when the up lands, for the store's
    /// `store_skill`.
    pub(super) fn skill_up(&mut self, vid: Vid, rest: &[u8]) -> Option<(u8, u8)> {
        let vnum = skill_up_vnum(rest)?;
        let (job, group) = match self.bodies.get(&vid) {
            Some(body) => (body.card.job, body.card.skill_group),
            None => return None,
        };
        self.skill_level_up(vid, job, group, vnum, SkillUpMethod::Point)
    }

    /// `SkillLevelUp`'s gate half: the level the up would climb to, or `None`
    /// when any gate refuses.
    fn skill_up_level(
        &self,
        vid: Vid,
        job: u8,
        group: u8,
        vnum: u32,
        method: SkillUpMethod,
    ) -> Option<u8> {
        let usable = gamedata::pc_skill_data::can_use(job, group, false, vnum)
            || matches!(
            vnum,
            137..=140 | SKILL_ANTI_FIRST..=SKILL_ANTI_LAST | SKILL_ADD_HP | SKILL_RESIST_PENETRATE | SKILL_NEW_FIRST..=SKILL_NEW_LAST | SKILL_HELP_FIRST..=SKILL_HELP_LAST,
            );
        if !usable {
            return None;
        }
        if (SKILL_ANTI_FIRST..=SKILL_ANTI_LAST).contains(&vnum)
            && self.pc_skill_level(vid.raw(), vnum) == 0
        {
            return None;
        }
        let row = self.pc_skill_tables.get(vnum).map(|skill| skill.record)?;
        if vnum >= SKILL_MAX_NUM {
            return None;
        }
        if !self.is_learnable_skill(vid, job, group, &row) {
            return None;
        }
        let level = self.pc_skill_level(vid.raw(), vnum);
        // The master gates (`:773-799`): a typed GRAND climbs by quest only, a
        // PERFECT never; BY_POINT climbs a NORMAL without the disable flag, BY_BOOK
        // a typed MASTER (any type-0 rides the book), BY_QUEST a typed GRAND.
        if row.skill_type != 0 {
            match master_of(level) {
                SKILL_GRAND_MASTER if method != SkillUpMethod::Quest => return None,
                SKILL_PERFECT_MASTER => return None,
                _ => {}
            }
        }
        match method {
            SkillUpMethod::Point => {
                if master_of(level) != SKILL_NORMAL
                    || row.flag & SKILL_FLAG_DISABLE_BY_POINT_UP != 0
                {
                    return None;
                }
            }
            SkillUpMethod::Book => {
                if row.skill_type != 0 && master_of(level) != SKILL_MASTER {
                    return None;
                }
            }
            SkillUpMethod::Quest => {
                if row.skill_type != 0 && master_of(level) != SKILL_GRAND_MASTER {
                    return None;
                }
                if row.skill_type == 0 {
                    return None;
                }
            }
        }
        let pc_level = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .map_or(0, world::character::Points::level);
        if pc_level < row.level_limit {
            return None;
        }
        if row.pre_skill_vnum != 0
            && master_of(self.pc_skill_level(vid.raw(), row.pre_skill_vnum)) == SKILL_NORMAL
            && self.pc_skill_level(vid.raw(), row.pre_skill_vnum) < row.pre_skill_level
        {
            return None;
        }
        if row.skill_type != 0 && group == 0 {
            return None;
        }
        Some(level)
    }

    /// `SkillLevelUp(vnum, method)`'s spend-and-climb half (`:802-907`): `BY_POINT`
    /// spends the typed point, every method climbs the level with the typed
    /// jump draws, recomputes 141, and sends byte 76.
    fn skill_level_up(
        &mut self,
        vid: Vid,
        job: u8,
        group: u8,
        vnum: u32,
        method: SkillUpMethod,
    ) -> Option<(u8, u8)> {
        let level = self.skill_up_level(vid, job, group, vnum, method)?;
        let row = self.pc_skill_tables.get(vnum).map(|skill| skill.record)?;
        if method == SkillUpMethod::Point {
            let spend = match row.skill_type {
                0 => point::POINT_SUB_SKILL,
                1..=4 | 6 => point::POINT_SKILL,
                5 => point::POINT_HORSE_SKILL,
                _ => return None,
            };
            let charged = {
                let points = self.pc_points_mut(vid.raw())?;
                if points.get_point(spend) < 1 {
                    return None;
                }
                points
                    .point_change(spend, -1, false, false)
                    .unwrap_or_default()
                    .clone()
            };
            self.send_point_records(vid, &charged);
        }
        let mut climbed = level.saturating_add(1).min(40);
        if row.skill_type != 0 {
            match master_of(climbed) {
                SKILL_NORMAL if climbed >= 17 => {
                    climbed = 20;
                }
                SKILL_MASTER if climbed >= 30 => {
                    let window = 31 - i32::from(climbed.min(30));
                    if number(&mut self.dice, 1, window) == 1 {
                        climbed = 30;
                    }
                }
                SKILL_GRAND_MASTER if climbed >= 40 => {
                    climbed = 40;
                }
                _ => {}
            }
        }
        self.pc_skill_state(vid.raw()).levels[vnum as usize] = climbed;
        if vnum == SKILL_ADD_HP {
            self.refresh_passive_hp(vid);
        }
        self.send_skill_levels(vid);
        let slot = u8::try_from(vnum).ok()?;
        Some((slot, climbed))
    }

    /// `IsLearnableSkill` (`G/char_skill.cpp:260-339`): the row is known, below
    /// 40, a type-0 below its max, a type-5 (140 needs an assassin), else a
    /// grouped row of the job's type, a type-6 anti vnum with no sibling
    /// learned, or a learned new/help vnum (221-228/236-243, the compiled-in
    /// `__7AND8TH_SKILLS__` arm). No type-5/6 row is hosted, so those arms
    /// never fire here.
    fn is_learnable_skill(
        &self,
        vid: Vid,
        job: u8,
        group: u8,
        row: &gamedata::records::SkillTableRecord,
    ) -> bool {
        let level = self.pc_skill_level(vid.raw(), row.vnum);
        if level >= 40 {
            return false;
        }
        if row.skill_type == 0 {
            return level < row.max_level;
        }
        if row.skill_type == 5 {
            return row.vnum != SKILL_HORSE_WILDATTACK_RANGE || job == JOB_ASSASSIN;
        }
        if group == 0 {
            return false;
        }
        if row.skill_type.wrapping_sub(1) == job {
            return true;
        }
        if row.skill_type == 6 {
            if (SKILL_ANTI_FIRST..SKILL_ANTI_FIRST + 4).contains(&row.vnum) {
                return (SKILL_ANTI_FIRST..SKILL_ANTI_FIRST + 4)
                    .filter(|sibling| *sibling != row.vnum)
                    .all(|sibling| self.pc_skill_level(vid.raw(), sibling) == 0);
            }
            if (SKILL_ANTI_FIRST + 4..=SKILL_ANTI_LAST).contains(&row.vnum) {
                return (SKILL_ANTI_FIRST + 4..=SKILL_ANTI_LAST)
                    .filter(|sibling| *sibling != row.vnum)
                    .all(|sibling| self.pc_skill_level(vid.raw(), sibling) == 0);
            }
            if (SKILL_NEW_FIRST..=SKILL_NEW_LAST).contains(&row.vnum)
                || (SKILL_HELP_FIRST..=SKILL_HELP_LAST).contains(&row.vnum)
            {
                return level != 0;
            }
        }
        false
    }
    /// `LearnSkillByBook(vnum, prob)` (`:466-645`): the gates (row, learnable,
    /// 20k exp below the max level, typed MASTER, the read-time stamp with the
    /// no-delay consume), then the `prob` arm (bonus-inflated `number(1,100)`
    /// roll, climb by book) or the training arm (exp spend, flag read, bonus
    /// zeroing, the 65 % train roll, the count ladder, `SkillLevelUp` by book,
    /// flag write). `test_server` is always false here, `ENABLE_MASTER_SKILL_
    /// BOOK_NO_STEPS` is off (see `prodomodefines.h`), and the read-time stamp
    /// moves only in the item arm, not here.
    ///
    /// The first answer is whether the read passed its gates: legacy returns
    /// true (and the item arm consumes the book and stamps the read time) even
    /// when the roll fails and nothing climbs (`:633-644` fall through to
    /// `return true`). The second is the climbed slot, for the store's
    /// `store_skill`, or `None` when nothing climbed.
    fn learn_skill_by_book(
        &mut self,
        vid: Vid,
        mover: Mover,
        vnum: u32,
        prob: u8,
    ) -> (bool, Option<(u8, u8)>) {
        let Some(row) = self.pc_skill_tables.get(vnum).map(|skill| skill.record) else {
            return (false, None);
        };
        let (job, group) = match self.bodies.get(&vid) {
            Some(body) => (body.card.job, body.card.skill_group),
            None => return (false, None),
        };
        if !self.is_learnable_skill(vid, job, group, &row) {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;948]", &[]);
            return (false, None);
        }
        let pc_level = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .map_or(0, world::character::Points::level);
        let need_exp = if i32::from(pc_level) < self.exp_limits.max_level {
            BOOK_NEED_EXP
        } else {
            0
        };
        let have_exp = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .map_or(0, world::character::Points::exp);
        if have_exp < need_exp {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;957]", &[]);
            return (false, None);
        }
        if row.skill_type != 0 && master_of(self.pc_skill_level(vid.raw(), vnum)) != SKILL_MASTER {
            let over = master_of(self.pc_skill_level(vid.raw(), vnum)) > SKILL_MASTER;
            let notice: &[u8] = if over { b"[LS;959]" } else { b"[LS;960]" };
            self.tell(vid, mover, CHAT_TYPE_INFO, notice, &[]);
            return (false, None);
        }
        let now = Self::global_time();
        let next = self
            .pc_skills
            .get(&vid.raw())
            .map_or(0, |state| state.next_read[vnum as usize]);
        if now < next
            && !self.affect_events.get(&vid).is_some_and(|event| {
                Self::has_book_affect(&event.affects, AFFECT_SKILL_NO_BOOK_DELAY)
            })
        {
            self.skill_learn_wait_more_time_message(vid, mover, next - now);
            return (false, None);
        }
        if now < next {
            let _gone = self.remove_pc_affect(vid, AFFECT_SKILL_NO_BOOK_DELAY);
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;377]", &[]);
        }
        let last_level = self.pc_skill_level(vid.raw(), vnum);
        if prob != 0 {
            let mut chance = prob;
            if Self::has_book_affect(
                &self
                    .affect_events
                    .get(&vid)
                    .map_or(Vec::new(), |event| event.affects.clone()),
                AFFECT_SKILL_BOOK_BONUS,
            ) {
                chance = chance.saturating_add(prob / 2);
                let _gone = self.remove_pc_affect(vid, AFFECT_SKILL_BOOK_BONUS);
            }
            if number(&mut self.dice, 1, 100) <= i32::from(chance) {
                let climbed = self.skill_level_up(vid, job, group, vnum, SkillUpMethod::Book);
                return (true, climbed);
            }
            self.book_read_tail(vid, mover, vnum, last_level);
            return (true, None);
        }
        self.train_master_skill(vid, mover, vnum, last_level, need_exp)
    }
    /// The `prob == 0` training arm (`:552-632`): spends the exp, reads the
    /// `traning_master_skill.{vnum}.read_count` flag (0 unset), zeroes the
    /// percent on the book bonus, rolls `number(1,100) > percent`, climbs on a
    /// full count and otherwise bumps the count with a 961-963 talking line and
    /// the 995 remainder. Without quests (`quests: None`) the flag reads 0 and
    /// the count never persists, so a sub-max MASTER read trains but never
    /// climbs — the no-quest shape of legacy's NULL-`GetPC` arm.
    fn train_master_skill(
        &mut self,
        vid: Vid,
        mover: Mover,
        vnum: u32,
        last_level: u8,
        need_exp: i64,
    ) -> (bool, Option<(u8, u8)>) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return (false, None);
        };
        let pid = character.player_id();
        if need_exp > 0 {
            let lost = {
                let max_level = self.exp_limits.max_level;
                let Some(points) = self.pc_points_mut(vid.raw()) else {
                    return (false, None);
                };
                points
                    .lose_exp(need_exp, max_level)
                    .map(|record| vec![record])
            };
            if let Some(records) = lost {
                self.send_point_records(vid, &records);
            }
        }
        let flag = format!("traning_master_skill.{vnum}.read_count");
        let read_count = self
            .quests
            .as_ref()
            .and_then(|quests| quests.manager().quest_flag(pid, flag.as_bytes()))
            .unwrap_or(0);
        let mut percent = 65;
        if self
            .affect_events
            .get(&vid)
            .is_some_and(|event| Self::has_book_affect(&event.affects, AFFECT_SKILL_BOOK_BONUS))
        {
            percent = 0;
            let _gone = self.remove_pc_affect(vid, AFFECT_SKILL_BOOK_BONUS);
        }
        if number(&mut self.dice, 1, 100) <= percent {
            self.book_read_tail(vid, mover, vnum, last_level);
            return (true, None);
        }
        let need_bookcount = i32::from(last_level.saturating_sub(20));
        if read_count < need_bookcount {
            if let Some(quests) = self.quests.as_mut() {
                quests
                    .manager_mut()
                    .set_quest_flag(pid, flag.as_bytes(), read_count + 1);
            }
            let talking: &[u8] = match number(&mut self.dice, 1, 3) {
                1 => b"[LS;961]",
                2 => b"[LS;962]",
                _ => b"[LS;963]",
            };
            self.tell(vid, mover, CHAT_TYPE_TALKING, talking, &[]);
            let left = [Arg::Int(i64::from(need_bookcount - read_count))];
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;995;%d]", &left);
            return (true, None);
        }
        if let Some(quests) = self.quests.as_mut() {
            quests.manager_mut().set_quest_flag(pid, flag.as_bytes(), 0);
        }
        let (job, group) = match self.bodies.get(&vid) {
            Some(body) => (body.card.job, body.card.skill_group),
            None => return (false, None),
        };
        let climbed = self.skill_level_up(vid, job, group, vnum, SkillUpMethod::Book);
        self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;964]", &[]);
        (true, climbed)
    }
    /// The `UseItemEx` book arms (`G/char_item.cpp:3282-3331` for `ITEM_SKILLBOOK`,
    /// `:4407-4790` for the `ITEM_USE` books): the item's skill and chance, its gates,
    /// then `LearnSkillByBook`. No polymorph is ported (`m_dwPolymorphRace` is always 0),
    /// so every `IsPolymorphed` gate passes. `values` are the proto's six `alValues`
    /// (`GetValue`), `socket0` the item's socket 0 (only 50300 reads it).
    ///
    /// The first answer is whether a book was read: the caller consumes one and stamps
    /// `next_read[vnum] = now + number(64800, 108000)` (`SKILLBOOK_DELAY_MIN/MAX`,
    /// `C/length.h:73-74`). The second is the climbed slot, for the store.
    /// `None` (with `false`) means no book arm matched: the caller runs the world step.
    /// The skill book's gate half: the skill and chance its arm would read, or
    /// `None` when the arm refuses. The ladder, language, summon, passive,
    /// craft, mining and horse arms below (see `read_skill_book`).
    /// The leadership and combo book arms (`:4407-4479`), which refuse from
    /// inside their ladder: 50301 reads values 0-1 (its poly gate answers 521,
    /// its siblings 1041 — both pass with no polymorph), combo climbs at 30
    /// then 50, twice. `None` means the item is no basic arm.
    fn book_basic_arms(
        &self,
        vid: Vid,
        mover: Mover,
        item_vnum: u32,
        values: [i32; 6],
    ) -> Option<(u32, u8, bool)> {
        match item_vnum {
            50301..=50303 => {
                let level = self.pc_skill_level(vid.raw(), SKILL_LEADERSHIP);
                if i32::from(level) < values[0] {
                    self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;274]", &[]);
                    return None;
                }
                if i32::from(level) >= values[1] {
                    self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;284]", &[]);
                    return None;
                }
                Some((SKILL_LEADERSHIP, 0, false))
            }
            50304..=50306 => {
                let level = self.pc_skill_level(vid.raw(), SKILL_COMBO);
                let pc_level = self.pc_level(vid);
                if level == 0 && pc_level < 30 {
                    self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;295]", &[]);
                    return None;
                }
                if level == 1 && pc_level < 50 {
                    self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;305]", &[]);
                    return None;
                }
                if level >= 2 {
                    self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;316]", &[]);
                    return None;
                }
                Some((
                    SKILL_COMBO,
                    u8::try_from(values[0].clamp(0, 100)).unwrap_or(100),
                    false,
                ))
            }
            _ => None,
        }
    }

    fn book_arm_gates(
        &self,
        vid: Vid,
        mover: Mover,
        item_vnum: u32,
        values: [i32; 6],
    ) -> Option<(u32, u8, bool)> {
        if let Some(armed) = self.book_basic_arms(vid, mover, item_vnum, values) {
            return Some(armed);
        }
        if !(50311..=50313).contains(&item_vnum)
            && item_vnum != 50061
            && !(50314..=50316).contains(&item_vnum)
            && !(50323..=50326).contains(&item_vnum)
            && !(50335..=50340).contains(&item_vnum)
            && !(50902..=50904).contains(&item_vnum)
            && item_vnum != 50600
            && item_vnum != 50060
        {
            return None;
        }
        let pct = values[1].clamp(0, 100);
        let armed: Option<(u32, u8)> = match item_vnum {
            // `:4481-4509`: the language trio; the empire gate refuses a book of the
            // reader's own tongue (`skill - 126 + 1 == empire`).
            50311..=50313 => {
                let skill = values[0];
                if skill <= 0 {
                    return None;
                }
                let vnum = u32::try_from(skill).unwrap_or(0);
                let empire = self.bodies.get(&vid).map_or(0, |body| body.card.empire);
                if self.pc_skill_level(vid.raw(), vnum) >= 20
                    || vnum.wrapping_sub(SKILL_LANGUAGE1).wrapping_add(1) == u32::from(empire)
                {
                    self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;328]", &[]);
                    return None;
                }
                Some((vnum, u8::try_from(pct).unwrap_or(100)))
            }
            // `:4511-4539`: the summon book (50061: value 0 is 131).
            50061 => {
                let skill = values[0];
                if skill <= 0 {
                    return None;
                }
                let vnum = u32::try_from(skill).unwrap_or(0);
                if self.pc_skill_level(vid.raw(), vnum) >= 10 {
                    self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;948]", &[]);
                    return None;
                }
                Some((vnum, u8::try_from(pct).unwrap_or(100)))
            }
            // `:4541-4633`: the passive ladder (low, high, chance, PC level). 50323/50324
            // are `ITEM_GIFTBOX` on the owner's data, so they never reach this arm (230g).
            50314..=50316 | 50323..=50326 | 50335..=50340 => {
                let skill = match item_vnum {
                    50314..=50316 => SKILL_POLYMORPH,
                    50323..=50324 => SKILL_ADD_HP,
                    50325..=50326 => SKILL_RESIST_PENETRATE,
                    50335..=50336 => 164,
                    50337..=50338 => 165,
                    _ => 166,
                };
                if self.pc_level(vid) < values[3] {
                    self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;350]", &[]);
                    return None;
                }
                if self.pc_skill_level(vid.raw(), skill) >= 40 {
                    self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;948]", &[]);
                    return None;
                }
                if i32::from(self.pc_skill_level(vid.raw(), skill)) < values[0] {
                    self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;274]", &[]);
                    return None;
                }
                if i32::from(self.pc_skill_level(vid.raw(), skill)) >= values[1] {
                    self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;371]", &[]);
                    return None;
                }
                Some((skill, u8::try_from(values[2].clamp(0, 100)).unwrap_or(100)))
            }
            // `:4635-4671` (123) and `:4673-4701` (124): below 40 only.
            50902..=50904 => {
                if self.pc_skill_level(vid.raw(), SKILL_CREATE) >= 40 {
                    self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;948]", &[]);
                    return None;
                }
                Some((SKILL_CREATE, u8::try_from(pct).unwrap_or(100)))
            }
            50600 => {
                if self.pc_skill_level(vid.raw(), SKILL_MINING) >= 40 {
                    self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;948]", &[]);
                    return None;
                }
                Some((SKILL_MINING, u8::try_from(pct).unwrap_or(100)))
            }
            // `:4703-4762`: the horse book runs its own body (see `read_horse_book`).
            50060 => Some((0, u8::try_from(pct).unwrap_or(100))),
            _ => None,
        };
        let (vnum, prob) = armed?;
        Some((vnum, prob, item_vnum == 50060))
    }

    pub(super) fn read_skill_book(
        &mut self,
        vid: Vid,
        mover: Mover,
        item_vnum: u32,
        socket0: i32,
        values: [i32; 6],
    ) -> (bool, Option<(u8, u8)>) {
        let proto = match self.protos.get(item_vnum) {
            Some(proto) => proto.clone(),
            None => return (false, None),
        };
        if proto.item_type == gamedata::item_kind::ITEM_SKILLBOOK {
            // `:3282-3331`: 50300 reads the socket, every other book value 0.
            let skill = if item_vnum == 50300 {
                socket0
            } else {
                values[0]
            };
            if skill <= 0 {
                // Legacy destroys the book (`RemoveItem`) and refuses. The Rewrite keeps it:
                // silently destroying a player's item is a Divergence (230g).
                return (false, None);
            }
            let vnum = u32::try_from(skill).unwrap_or(0);
            let (read, climbed) = self.learn_skill_by_book(vid, mover, vnum, 0);
            if read {
                self.stamp_next_read(vid, vnum);
            }
            return (read, climbed);
        }
        if proto.item_type != gamedata::item_kind::ITEM_USE
            || proto.sub_type != gamedata::item_kind::USE_SPECIAL
        {
            return (false, None);
        }
        let Some((vnum, prob, horse)) = self.book_arm_gates(vid, mover, item_vnum, values) else {
            return (false, None);
        };
        if horse {
            return self.read_horse_book(vid, mover, i32::from(prob));
        }
        let (read, climbed) = self.learn_skill_by_book(vid, mover, vnum, prob);
        if read {
            self.stamp_next_read(vid, vnum);
        }
        (read, climbed)
    }

    /// `SetSkillNextReadTime(vnum, now + number(64800, 108000))` after a read the gates
    /// passed, and byte 76 with the new stamp.
    fn stamp_next_read(&mut self, vid: Vid, vnum: u32) {
        if vnum >= 255 {
            return;
        }
        let delay = number(&mut self.dice, SKILLBOOK_DELAY_MIN, SKILLBOOK_DELAY_MAX);
        let stamp = Self::global_time().saturating_add(u32::try_from(delay).unwrap_or(0));
        self.pc_skill_state(vid.raw()).next_read[vnum as usize] = stamp;
        self.send_skill_levels(vid);
    }

    /// The horse book's own body (`:4703-4762`): level 50, the read-time gate with the
    /// no-delay consume, the triple cap (`POINT_HORSE_SKILL` past 19, either riding trio
    /// past 59), the chance roll, `POINT_HORSE_SKILL + 1` with the read stamp on success,
    /// 382 on failure. `test_server` is always false here. Consumes always (the caller
    /// consumes on `true`); stamps only on success.
    fn read_horse_book(&mut self, vid: Vid, mover: Mover, pct: i32) -> (bool, Option<(u8, u8)>) {
        if self.pc_level(vid) < 50 {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;376]", &[]);
            return (false, None);
        }
        let now = Self::global_time();
        let next = self
            .pc_skills
            .get(&vid.raw())
            .map_or(0, |state| state.next_read[SKILL_HORSE as usize]);
        if now < next {
            if self.affect_events.get(&vid).is_some_and(|event| {
                Self::has_book_affect(&event.affects, AFFECT_SKILL_NO_BOOK_DELAY)
            }) {
                let _gone = self.remove_pc_affect(vid, AFFECT_SKILL_NO_BOOK_DELAY);
                self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;377]", &[]);
            } else {
                self.skill_learn_wait_more_time_message(vid, mover, next - now);
                return (false, None);
            }
        }
        let horse = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .map_or(0, |points| points.get_point(point::POINT_HORSE_SKILL));
        let wild = i32::from(self.pc_skill_level(vid.raw(), SKILL_HORSE_WILDATTACK));
        let range = i32::from(self.pc_skill_level(vid.raw(), SKILL_HORSE_WILDATTACK_RANGE));
        let charge = i32::from(self.pc_skill_level(vid.raw(), SKILL_HORSE_CHARGE));
        let escape = i32::from(self.pc_skill_level(vid.raw(), SKILL_HORSE_ESCAPE));
        if horse >= 20 || wild + charge + escape >= 60 || range + charge + escape >= 60 {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;378]", &[]);
            return (false, None);
        }
        if number(&mut self.dice, 1, 100) <= pct {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;379]", &[]);
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;380]", &[]);
            self.change_pc_point(vid, point::POINT_HORSE_SKILL, 1, false);
            self.stamp_next_read(vid, SKILL_HORSE);
        } else {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;382]", &[]);
        }
        (true, None)
    }

    /// The PC level of the player under `vid` (`GetLevel`), 0 while it is gone.
    fn pc_level(&self, vid: Vid) -> i32 {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .map_or(0, |points| i32::from(points.level()))
    }

    /// The shared read tail (`:633-644`): a changed level talks 954/955 and
    /// infos 964, else 952/953. The `CharLog` lines are the operator log's, not
    /// the wire's.
    fn book_read_tail(&self, vid: Vid, mover: Mover, vnum: u32, last_level: u8) {
        if last_level == self.pc_skill_level(vid.raw(), vnum) {
            self.tell(vid, mover, CHAT_TYPE_TALKING, b"[LS;952]", &[]);
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;953]", &[]);
        } else {
            self.tell(vid, mover, CHAT_TYPE_TALKING, b"[LS;954]", &[]);
            self.tell(vid, mover, CHAT_TYPE_TALKING, b"[LS;955]", &[]);
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;964]", &[]);
        }
    }
    /// `SkillLearnWaitMoreTimeMessage(ms)` (`:3264-3300`): the TALKING lines for
    /// the seconds a read still waits (3m/5m/10m/30m/1h/2h/3h/6h/12h/18h ladder;
    /// 978-979 share 3-6h, 981-982 6-12h, 983-984 12-18h, 985-986 past it).
    fn skill_learn_wait_more_time_message(&self, vid: Vid, mover: Mover, wait: u32) {
        let lines: &[&[u8]] = if wait < 3 * 60 {
            &[b"[LS;970]"]
        } else if wait < 5 * 60 {
            &[b"[LS;971]"]
        } else if wait < 10 * 60 {
            &[b"[LS;972]"]
        } else if wait < 30 * 60 {
            &[b"[LS;973]", b"[LS;974]"]
        } else if wait < 3600 {
            &[b"[LS;975]"]
        } else if wait < 2 * 3600 {
            &[b"[LS;976]"]
        } else if wait < 3 * 3600 {
            &[b"[LS;977]"]
        } else if wait < 6 * 3600 {
            &[b"[LS;978]", b"[LS;979]"]
        } else if wait < 12 * 3600 {
            &[b"[LS;981]", b"[LS;982]"]
        } else if wait < 18 * 3600 {
            &[b"[LS;983]", b"[LS;984]"]
        } else {
            &[b"[LS;985]", b"[LS;986]"]
        };
        for line in lines {
            self.tell(vid, mover, CHAT_TYPE_TALKING, line, &[]);
        }
    }
    /// Whether the affects hold `kind` with no apply (`FindAffect(kind)`,
    /// `G/char_affect.cpp:129-141`: the default `APPLY_NONE` arm matches any
    /// row of the kind).
    fn has_book_affect(affects: &[super::affect::Affect], kind: u32) -> bool {
        affects.iter().any(|affect| affect.kind == kind)
    }
    /// `get_global_time()`: the wall clock in seconds, as the quest bridge
    /// reads it (the uptime gap waits — see `global_time` in `main.rs`).
    fn global_time() -> u32 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| {
                u32::try_from(since.as_secs()).unwrap_or(u32::MAX)
            })
    }

    /// `ComputePoints`' `SKILL_ADD_HP` arm (`G/char.cpp:2956-2966`) after a 141
    /// up: the row's point poly at `k = power / 100` refreshes the passive
    /// max-HP bonus, recomputes the points, and sends the records. (142 reads
    /// at hit time, not from the points; 164-166 are unhosted.)
    fn refresh_passive_hp(&mut self, vid: Vid) {
        let (job, group) = match self.bodies.get(&vid) {
            Some(body) => (body.card.job, body.card.skill_group),
            None => return,
        };
        let Some(row) = self.pc_skill_tables.get(SKILL_ADD_HP).cloned() else {
            return;
        };
        let level = self.pc_skill_level(vid.raw(), SKILL_ADD_HP);
        let power = self.pc_skill_tables.power(job, group, level, false);
        let bonus = f64_to_i32(
            row.point
                .eval(&BTreeMap::from([("k", f64::from(power) / 100.0)])),
        );
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return;
        };
        let passive = world::character::PassiveBonuses {
            max_hp: bonus,
            ..world::character::PassiveBonuses::default()
        };
        points.set_passive_bonuses(passive);
        let records = points.compute_points();
        self.send_point_records(vid, &records);
    }

    /// `SkillLevelPacket` (`G/char_skill.cpp:170-181`): byte 76 with all 255 slots
    /// — the stored level, the ladder's master type, and the stored read time.
    fn send_skill_levels(&self, vid: Vid) {
        use protocol::gc_nested::{GcSkill, GcSkillLevelNew};
        let state = self.pc_skills.get(&vid.raw());
        let mut skills = crate::loading_phase::skill_levels();
        for (slot, entry) in skills.iter_mut().enumerate() {
            let (level, next_read) =
                state.map_or((0, 0), |state| (state.levels[slot], state.next_read[slot]));
            *entry = GcSkill {
                master_type: master_of(level),
                level,
                next_read: i32::from_ne_bytes(next_read.to_ne_bytes()),
            };
        }
        let frame = GcSkillLevelNew {
            header: GcSkillLevelNew::header(),
            skills,
        };
        let mut out = Vec::new();
        frame.encode_into(&mut out);
        let _sent = self.write_to_client(vid, out);
    }

    /// `k`: `1.0 * GetSkillPower(vnum, level) * bMaxLevel / 100`
    /// (`G/char_skill.cpp:2523`), as `f64`.
    fn pc_skill_k(&self, job: u8, group: u8, vnum: u32, level: u8) -> f64 {
        let power = self.pc_skill_tables.power(job, group, level, false);
        let max = self
            .pc_skill_tables
            .get(vnum)
            .map_or(0, |skill| skill.record.max_level);
        f64::from(power) * f64::from(max) / 100.0
    }

    /// `CSkillProto::IsChargeSkill` (`G/skill.h:244-247`): the TANHWAN pair —
    /// the warrior's 5 and the horse's 138. Only 5 is hosted (type-5 rows never
    /// load), so 138's arms are coded and dead.
    const fn is_charge_skill(vnum: u32) -> bool {
        vnum == SKILL_TANHWAN || vnum == SKILL_HORSE_CHARGE
    }

    /// Whether the player under `vid` holds the TANHWAN dash: the charge
    /// skill's affect with flag 32 (bit 31 of word 0).
    fn has_tanhwan_dash(&self, vid: Vid) -> bool {
        self.affect_flags_of(vid)[0] & (1 << (AFF_TANHWAN_DASH - 1)) != 0
    }

    /// `CInputMain::UseSkill` + `CHARACTER::UseSkill`'s entry half
    /// (`G/input_main.cpp:2173-2178`, `G/char_skill.cpp:2896-3112`): the
    /// `CanUseSkill`, stun, row and level gates, the revive-invisible removal,
    /// the SP charge, and the cooldown stamp.
    ///
    /// ATTACK rows stop here (their `CG_ATTACK` lands the damage); non-ATTACK
    /// rows return true with nothing further (their buffs wait). A refused
    /// cast sends nothing, as `CInputMain::UseSkill` ignores the answer.
    pub(super) fn use_skill(&mut self, vid: Vid, skill_vnum: u32, target_vid: u32) -> bool {
        let (job, group) = match self.bodies.get(&vid) {
            Some(body) => (body.card.job, body.card.skill_group),
            None => return false,
        };
        if !gamedata::pc_skill_data::can_use(job, group, false, skill_vnum) {
            return false;
        }
        // `g_bSkillDisable`, the observer mode, `CanMove`, the polymorph: only
        // the stun arm of `CanMove` is ported (no shop, riding, polymorph,
        // or observer mode).
        if self.affect_flags_of(vid)[0] & AFF_STUN_WORD != 0 {
            return false;
        }
        if self.pc_skill_tables.get(skill_vnum).is_none() {
            return false;
        }
        // The charge arm (`:2979-2995`): with the DASH held, or a victim that is
        // not the caster, the cast strikes at once. Without the DASH the caster
        // first dashes (the recursive self-cast, which charges and stamps);
        // then the main VID stamps, the skill computes eagerly on the victim,
        // the DASH goes, and the cast ends with no skill-time stamp.
        if Self::is_charge_skill(skill_vnum)
            && (self.has_tanhwan_dash(vid) || (target_vid != 0 && target_vid != vid.raw()))
        {
            if target_vid == 0 {
                return false;
            }
            if !self.has_tanhwan_dash(vid) && !self.use_skill(vid, skill_vnum, vid.raw()) {
                return false;
            }
            self.pc_skill_state(vid.raw())
                .uses
                .entry(skill_vnum)
                .or_default()
                .main_vid = target_vid;
            self.compute_skill(vid, skill_vnum, target_vid);
            let _dashed = self.remove_pc_affect(vid, skill_vnum);
            return true;
        }
        // The combo toggle (`:2996-3006`): the index flips between 0 and the
        // level, the client is told, and the cast ends before the level gate
        // (a level-0 combo still chats `combo 0`), the charge and the stamp.
        // The hack check the motion carries is anti-cheat and never ported.
        if skill_vnum == SKILL_COMBO {
            let level = self.pc_skill_level(vid.raw(), SKILL_COMBO);
            let state = self.pc_skill_state(vid.raw());
            if state.combo_index != 0 {
                state.combo_index = 0;
            } else {
                state.combo_index = level;
            }
            let index = self.pc_skills[&vid.raw()].combo_index;
            if let Some(mover) = self.own_mover(vid) {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_COMMAND,
                    format!("combo {index}").as_bytes(),
                    &[],
                );
            }
            return true;
        }
        let level = self.pc_skill_level(vid.raw(), skill_vnum);
        if level == 0 {
            return false;
        }
        // The rod/pick refusal (`:2971-2973`): no mining is ported, so no
        // equipped rod or pick can refuse.
        let (toggle, affect_flag) = {
            let skill = self
                .pc_skill_tables
                .get(skill_vnum)
                .expect("the row was read above");
            (
                skill.record.flag & SKILL_FLAG_TOGGLE != 0,
                skill.record.affect_flag,
            )
        };
        // The TOGGLE-off arm (`:3006-3009`): an active toggle is removed and the
        // cast ends here, before the revive removal, the charge and the stamp.
        if toggle && (affect_flag != 0 || skill_vnum == SKILL_MUYEONG) {
            let removed = self.remove_pc_affect(vid, skill_vnum);
            if removed {
                return true;
            }
        }
        self.remove_revive_invisible(vid);
        self.cast_skill(vid, job, group, skill_vnum, target_vid, level)
    }

    /// `UseSkill`'s charge-and-stamp half (`:3021-3130`): the SP charge (with
    /// 64's pre-charge cooling refusal and the grandmaster cost), the cooldown
    /// stamp, and the dispatch tail (non-ATTACK rows compute here).
    fn cast_skill(
        &mut self,
        vid: Vid,
        job: u8,
        group: u8,
        skill_vnum: u32,
        target_vid: u32,
        level: u8,
    ) -> bool {
        let k = self.pc_skill_k(job, group, skill_vnum, level);
        let skill = self
            .pc_skill_tables
            .get(skill_vnum)
            .expect("the row was read above");
        let cool_secs = f64_to_i32(skill.cooldown.eval(&BTreeMap::from([("k", k)])));
        let hit_count = if skill.record.max_hit == 0 {
            -1
        } else {
            skill.record.max_hit
        };
        let hp_cost = skill.record.flag & SKILL_FLAG_USE_HP_AS_COST != 0;
        let sp_cost = skill.sp_cost.clone();
        let grand_add = skill.grand_add_sp.clone();
        let is_attack = skill.record.flag & SKILL_FLAG_ATTACK != 0;
        // No owner PC row sets `USE_HP_AS_COST`, so the HP arm is refused
        // rather than charged (its rows never reach here — but the flag test
        // keeps the shape for them).
        if hp_cost {
            return false;
        }
        let casting = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .map_or(0, |points| points.get_point(point::POINT_CASTING_SPEED));
        // `SKILL_TERROR`'s cooling check (`:3032-3037`): 64 alone refuses before
        // the charge; every other row's cooling refusal comes from the stamp
        // after it.
        if skill_vnum == SKILL_TERROR {
            let cooling = self
                .pc_skills
                .get(&vid.raw())
                .and_then(|state| state.uses.get(&skill_vnum))
                .is_some_and(|stamped| stamped.cooling(self.last_pulse));
            if cooling {
                return false;
            }
        }
        // The grandmaster cost (`:3049-3053`, `:3068-3071`): GRAND+ with the
        // packet's grand bit costs the grand-add poly instead.
        let grand_cost = master_of(level) >= SKILL_GRAND_MASTER
            && self.pc_skill_used_grand(vid.raw(), skill_vnum);
        let records = {
            let Some(points) = self.pc_points_mut(vid.raw()) else {
                return false;
            };
            let vars = BTreeMap::from([
                ("k", k),
                ("maxhp", f64::from(points.max_hp())),
                ("maxv", f64::from(points.max_sp())),
                ("v", f64::from(points.sp())),
            ]);
            let needed = if grand_cost {
                f64_to_i32(grand_add.eval(&vars))
            } else {
                f64_to_i32(sp_cost.eval(&vars))
            };
            if points.sp() < needed {
                return false;
            }
            match points.point_change(point::POINT_SP, -needed, false, false) {
                Ok(records) => records.clone(),
                Err(_) => return false,
            }
        };
        self.send_point_records(vid, &records);
        let cool_millis = calculate_duration(casting, cool_secs.saturating_mul(1000));
        let cool = (cool_millis > 0).then(|| ms_to_pulses(cool_millis));
        let now = self.last_pulse;
        let stamped = self
            .pc_skill_state(vid.raw())
            .uses
            .entry(skill_vnum)
            .or_default()
            .stamp(now, target_vid, cool, hit_count, true);
        if !stamped {
            return false;
        }
        self.pc_skill_state(vid.raw()).last_skill = Some(now);
        // The dispatch tail (`:3119-3130`): SELFONLY computes on the caster, PARTY
        // fans to the near members (`ComputeSkillParty`, `:2430-2440`), else a
        // non-ATTACK row computes on the target; ATTACK rows wait for their
        // `CG_ATTACK` — except BYEURAK 107, MUYEONG 78 and the charge rows,
        // which compute eagerly (107/78 are refused at load, so their eager
        // arms are unreachable; 5's first cast dashes below).
        let flag = self
            .pc_skill_tables
            .get(skill_vnum)
            .map_or(0, |skill| skill.record.flag);
        if flag & SKILL_FLAG_SELFONLY != 0 {
            self.compute_skill_affect(vid, skill_vnum, vid.raw(), k, level);
        } else if flag & SKILL_FLAG_PARTY != 0 {
            self.compute_skill_party(vid, skill_vnum, k, level);
        } else if !is_attack {
            self.compute_skill_affect(vid, skill_vnum, target_vid, k, level);
        } else if Self::is_charge_skill(skill_vnum) {
            // The charge's eager self-compute (`:3130` with `:2625-2628`): a
            // second cast never reaches here (the `use_skill` arm intercepts
            // it), so the victim is the caster and the damage arm is skipped —
            // the first cast only dashes (`:2832-2856`).
            self.compute_charge_self(vid, skill_vnum, k, level);
        }
        true
    }
    /// `CHARACTER::RemoveAffect(dwType)` (`G/char_affect.cpp:864-884`): every affect
    /// of `kind` is uncomputed and its remove packet sent; true when any was.
    /// (The chat-block arm is chat's, not skills'.)
    pub(super) fn remove_pc_affect(&mut self, vid: Vid, kind: u32) -> bool {
        let removed = match self.affect_events.get_mut(&vid) {
            Some(event) => super::affect::take_affects_of_kind(&mut event.affects, kind),
            None => Vec::new(),
        };
        if removed.is_empty() {
            return false;
        }
        for affect in &removed {
            self.change_pc_point(vid, usize::from(affect.apply_on), -affect.value, false);
            let frame = super::affect::affect_remove_frame(affect);
            let _retired = self.write_to_client(vid, frame);
        }
        let flags_after = self.affect_flags_of(vid);
        self.send_affect_update(vid, flags_after);
        true
    }
    /// `CHARACTER::RemoveBadAffect` (`G/char_affect.cpp:951-960`): the poison
    /// (with its event, `RemovePoison`), the fire, the stun, the slow, and 80 go
    /// — in that order. No player fire event is ported, so the fire's cancel is
    /// vacuous.
    fn remove_bad_affect(&mut self, vid: Vid) {
        use crate::loading_phase::{AFFECT_SLOW, AFFECT_STUN};
        self.remove_pc_poison(vid);
        let _fire = self.remove_pc_affect(vid, AFFECT_FIRE);
        let _stun = self.remove_pc_affect(vid, AFFECT_STUN);
        let _slow = self.remove_pc_affect(vid, AFFECT_SLOW);
        let _tusok = self.remove_pc_affect(vid, SKILL_TUSOK);
    }

    /// `CHARACTER::OnMove(true)` (`G/char.cpp:6148-6179`): the last move and
    /// attack times stamp, revive-invisible, if held, goes, and the Eunhyung
    /// flag arm fires: 34 goes and the affected level stamps, else the level
    /// clears. (The hide itself waits: monsters still see the cloaked.) Mining
    /// is unported, so its cancel is vacuous.
    fn on_move_attack(&mut self, vid: Vid) {
        let now = self.clock.now();
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.last_move = now;
            body.last_attack = Some(now);
        }
        self.remove_revive_invisible(vid);
        // Flag 19 is bit 18 of word 0.
        if self.affect_flags_of(vid)[0] & (1 << 18) != 0 {
            let _cloaked = self.remove_pc_affect(vid, SKILL_EUNHYUNG);
            let (job, group) = match self.bodies.get(&vid) {
                Some(body) => (body.card.job, body.card.skill_group),
                None => return,
            };
            let level = self.pc_skill_level(vid.raw(), SKILL_EUNHYUNG);
            let power = self.pc_skill_tables.power(job, group, level, false);
            self.pc_skill_state(vid.raw()).affected_eunhyung = u32::try_from(power).unwrap_or(0);
        } else {
            self.pc_skill_state(vid.raw()).affected_eunhyung = 0;
        }
    }

    /// `CHARACTER::IsGoodAffect` (`G/char_affect.cpp:919-947`): the potion kinds
    /// (200 201 204 205 206 207 224) and the buff vnums (3 4 19 34 49 63 64 65 79
    /// 94 95 110 111 96).
    fn is_good_affect(vnum: u32) -> bool {
        matches!(
            vnum,
            200 | 201
                | 204
                | 205
                | 206
                | 207
                | 224
                | 3
                | 4
                | 19
                | 34
                | 49
                | 63
                | 64
                | 65
                | 79
                | 94
                | 95
                | 96
                | 110
                | 111
        )
    }

    /// The stamp's grand bit for `vnum` (false while unstamped).
    fn pc_skill_used_grand(&self, vid: u32, vnum: u32) -> bool {
        self.pc_skills
            .get(&vid)
            .and_then(|state| state.uses.get(&vnum))
            .is_some_and(|stamped| stamped.grand)
    }

    /// `CInputMain::Attack` with `bType > 0` on a non-player (`G/input_main.cpp:1928-2007`)
    /// and its `CHARACTER::Attack(bType)` (`G/char_battle.cpp:284-326`): `CanUseSkill`, the
    /// shoot-list refusal, `CheckSkillHitCount`, the skill-term gate, and the
    /// `ComputeSkill` damage arm on the 230d melee rows (`FuncSplashDamage` on the one
    /// victim, `DAMAGE_TYPE_MELEE`). A refused attack sends nothing, as
    /// `CInputMain::Attack` ignores `Attack`'s answer, and steals no gold.
    pub(super) fn skill_attack(&mut self, vid: Vid, skill_vnum: u8, target_vid: u32) -> bool {
        let (job, group) = match self.bodies.get(&vid) {
            Some(body) => (body.card.job, body.card.skill_group),
            None => return false,
        };
        let skill_vnum = u32::from(skill_vnum);
        if !gamedata::pc_skill_data::can_use(job, group, false, skill_vnum) {
            return false;
        }
        if SHOOT_LIST.contains(&skill_vnum) {
            return false;
        }
        let row = match self.pc_skill_tables.get(skill_vnum) {
            Some(row)
                if row.is_castable_melee()
                    || row.is_castable_splash_melee()
                    || row.is_castable_magic() =>
            {
                row.clone()
            }
            _ => return false,
        };
        let Some(place) = self.place_of(vid.raw()) else {
            return false;
        };
        let Some(spot) = self.spot_of(vid.raw()) else {
            return false;
        };
        let npc_alive = self
            .npc_on(target_vid, place)
            .is_some_and(|npc| is_struck(npc) && !npc.dead);
        if !npc_alive {
            return false;
        }
        let npc_snapshot = self
            .npc_by_vid(target_vid)
            .expect("the victim stood above")
            .clone();
        // `battle_is_attackable` (`G/battle.cpp:165`): beyond it the attack is `BATTLE_NONE`
        // and sends nothing. There is no melee reach here: `ComputeSkill`'s target-range
        // arm (`+ 50`) below is the only distance gate.
        if !self.is_attackable(vid, (place.1, spot.x, spot.y), &npc_snapshot) {
            return false;
        }
        // `CheckSkillHitCount` (`G/char_skill.cpp:4184-4254`): an unused skill refuses;
        // YONGKWON 62/HWAYEOMPOK 77/DAEJINGAK 18/PAERYONG 93 refuse the attack record;
        // then each victim's count is capped (3 for SAMYEON=1/CHARYUN=33; the 230d rows
        // keep the default 1 — the YEONSA-7/HORSE_WILDATTACK_RANGE-5/HORSE_ESCAPE-10
        // arms have no hosted caster and stay out).
        if matches!(skill_vnum, 62 | 77 | 18 | 93) {
            return false;
        }
        let max_hits = u32::from(match skill_vnum {
            1 | 33 => 3_u8,
            _ => 1,
        });
        let seen = self
            .pc_skills
            .get(&vid.raw())
            .and_then(|state| state.uses.get(&skill_vnum))
            .filter(|info| info.used)
            .and_then(|info| info.hits_per_victim.get(&target_vid).copied())
            .unwrap_or(0);
        if seen >= max_hits {
            return false;
        }
        let count = self
            .pc_skill_state(vid.raw())
            .uses
            .entry(skill_vnum)
            .or_default();
        if !count.used {
            return false;
        }
        count
            .hits_per_victim
            .insert(target_vid, seen.saturating_add(1));
        // `Attack(bType)`'s skill-term gate (`G/char_battle.cpp:297-304`): past 1500 ms
        // after `m_dwLastSkillTime` the cast is refused. `use_skill` stamps the pulse,
        // so the `>` fires past 37 Pulses.
        let now = self.last_pulse;
        let term = self
            .pc_skills
            .get(&vid.raw())
            .and_then(|state| state.last_skill)
            .is_some_and(|last| now.saturating_sub(last) > SKILL_TERM_PULSES);
        if term {
            return false;
        }
        self.compute_skill_melee(vid, &row, target_vid);
        // The `ComputeSkill` `OnMove(true)` for a non-NORMAL attr
        // (`G/char_skill.cpp:2576-2577`): every 230d row is MELEE-attr.
        let now_ms = self.clock.now();
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.fighting = true;
            body.last_move = now_ms;
            body.last_attack = Some(now_ms);
        }
        // `SetPosition(POS_FIGHTING)` and `SetRotationToXY` (`G/char_battle.cpp:219-223`).
        let (nx, ny) = self
            .npc_by_vid(target_vid)
            .map(|npc| (npc.x, npc.y))
            .expect("the victim stood above");
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.rotation = rotation_to_xy(nx.saturating_sub(spot.x), ny.saturating_sub(spot.y));
        }
        true
    }

    /// `ComputeSkill`'s damage arm (`G/char_skill.cpp:2443-2732`) on the 230d rows: the
    /// riding and polymorph gates, the row, the horse-type arms, the SELFONLY victim
    /// (no 230d row sets it), the target-range arm (`DISTANCE_SQRT >= range + 50`
    /// refuses), the level, and the functor on the one victim — no SPLASH walk
    /// (no 230d row sets SPLASH). Then `Damage(pc, iDam, MELEE)`: the MELEE pipe, the
    /// damage map with MELEE aggro, the stun at 0, the kill of a stunned victim, and
    /// the victim's `BeginFight`.
    fn compute_skill_melee(
        &mut self,
        vid: Vid,
        row: &gamedata::pc_skill_data::PcSkill,
        target_vid: u32,
    ) {
        let spot = self.spot_of(vid.raw()).expect("the caster stood above");
        let Some((nx, ny)) = self.skill_range_spot(row, target_vid, spot) else {
            return;
        };
        let level = self.pc_skill_level(vid.raw(), row.record.vnum);
        if level == 0 {
            return;
        }
        // No riding and no polymorph is ported, so both gates pass; no hosted cast is a
        // horse row. `AFF_PABEOP` waits for the buffs.
        let Some(body) = self.bodies.get(&vid) else {
            return;
        };
        let (job, group) = (body.card.job, body.card.skill_group);
        let k = self.pc_skill_k(job, group, row.record.vnum, level);
        // The functor's per-victim draws (`G/char_skill.cpp:1412-1446`): PENETRATE's and
        // IGNORE_TARGET_RATING's `kPointPoly2` rolls, then `atk` and `ar`. No 230d row
        // sets IGNORE_TARGET_RATING; only vnum 61 sets PENETRATE, and its point2 poly
        // (`1+k*9`) reads a plain level-1 `k`, so it can draw.
        let point2_vars = BTreeMap::from([("k", k)]);
        let point2 = f64_to_i32(row.point2.eval(&point2_vars));
        let flag = row.record.flag;
        let ignore_defence =
            flag & SKILL_FLAG_PENETRATE != 0 && number(&mut self.dice, 1, 100) <= point2;
        let ignore_rating =
            flag & SKILL_FLAG_IGNORE_TARGET_RATING != 0 && number(&mut self.dice, 1, 100) <= point2;
        let points_snapshot = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points().cloned())
            .expect("the caster stood above");
        let victim_snapshot = self
            .npc_by_vid(target_vid)
            .expect("the victim stood above")
            .clone();
        let ar = attack_rating(AttackRatingInput {
            attacker_dexterity: points_snapshot.get_point(point::POINT_DX),
            attacker_level: i32::from(points_snapshot.level()),
            victim_dexterity: victim_snapshot.fight.point(point::POINT_DX),
            ignore_target_rating: ignore_rating,
        });
        let weapon = self.wielded_weapon(vid);
        let atk = calc_melee_skill_atk(
            &mut self.dice,
            &points_snapshot,
            weapon.as_ref(),
            &victim_snapshot,
        );
        let vars = skill_vars(&points_snapshot, &victim_snapshot, k, atk, ar);
        let mut amount = -f64_to_i32(row.point.eval(&vars));
        // No hosted cast rises past NORMAL (`SetSkillLevel` maps level < 20 there), so
        // the grandmaster `kMasterBonusPoly` arm never fires.
        amount = battle_damage_floor(&mut self.dice, amount);
        // The non-main splash adjust: a no-SPLASH row hits only the main victim
        // (`m_x`/`m_y` are the victim's spot), so no adjust multiplies.
        let damage = skill_melee_attr(
            amount,
            weapon.as_ref(),
            victim_snapshot.fight.point(point::POINT_DEF_GRADE),
            &victim_snapshot.fight,
            ignore_defence,
        );
        // `HitOnce` (`:1393-1400`): unused refuses (unreachable — the hit-count gate
        // above already demanded `used`), an expired stamp refuses (MUYEONG excepted —
        // never a 230d row), `-1` is infinite, else the count falls by one.
        let now = self.last_pulse;
        let once = self
            .pc_skill_state(vid.raw())
            .uses
            .entry(row.record.vnum)
            .or_default()
            .hit_once(now, row.record.vnum);
        if !once {
            return;
        }
        let (struck, events) = self.skill_damage_melee(vid, target_vid, damage, weapon.as_ref());
        self.show_strike(vid, target_vid, events);
        match struck {
            Struck::Landed { damage } => {
                self.set_last_attacked(target_vid);
                self.after_damage_typed(vid, target_vid, damage, AggroDamage::Melee);
            }
            Struck::Finished => {
                self.set_last_attacked(target_vid);
                if let Some(body) = self.bodies.get_mut(&vid) {
                    body.fighting = false;
                }
                self.dead_npc(target_vid, Some(vid));
            }
            Struck::Whiffed | Struck::Refused | Struck::AlreadyDead | Struck::StunnedByGuard => {}
        }
        // `iRet != BATTLE_NONE`: `SetSyncOwner(this)`, then the victim's `BeginFight`.
        let victim_pos = SyncPositionVictim {
            vid: Vid::new(target_vid),
            kind: SyncPositionVictimKind::Monster,
            x: nx,
            y: ny,
        };
        let now_ms = self.clock.elapsed();
        let _owner = self.set_npc_sync_owner(vid, (spot.x, spot.y), &victim_pos, now_ms);
        if self
            .npc_by_vid(target_vid)
            .is_some_and(super::mob_battle::fights_back)
            && self.can_begin_fight(target_vid)
        {
            self.begin_fight(target_vid, vid);
        }
    }

    /// `CHARACTER::ComputeSkill`'s damage arm (`G/char_skill.cpp:2443-2732`) for the 230e
    /// rows: the row, the SELFONLY victim, the target-range arm, the level, and the
    /// functor — the SPLASH walk or the one victim — with the RANGE/MAGIC attr arms.
    /// `ComputeSkill` returns void, so this answers nothing; the functor's own gates
    /// refuse quietly per victim.
    fn compute_skill(&mut self, vid: Vid, vnum: u32, target_vid: u32) {
        let row = match self.pc_skill_tables.get(vnum) {
            Some(row) => row.clone(),
            None => return,
        };
        let flag = row.record.flag;
        // No riding and no polymorph is ported, so both gates pass; no hosted cast is a
        // horse row. `AFF_PABEOP` waits for the buffs.
        let Some(body) = self.bodies.get(&vid) else {
            return;
        };
        let (job, group) = (body.card.job, body.card.skill_group);
        let level = self.pc_skill_level(vid.raw(), vnum);
        if level == 0 {
            return;
        }
        let k = self.pc_skill_k(job, group, vnum, level);
        // SELFONLY (`:2490-2496`): the caster is the victim (no hosted caller, kept for
        // the shape — a player victim waits for PvP, so the walk centres on the caster
        // and the per-victim gates refuse the caster itself).
        let centre = if flag & SKILL_FLAG_SELFONLY != 0 {
            vid.raw()
        } else {
            target_vid
        };
        let Some(spot) = self.spot_of(vid.raw()) else {
            return;
        };
        let Some((nx, ny)) = self.skill_range_spot(&row, centre, spot) else {
            return;
        };
        let victims = if flag & SKILL_FLAG_SPLASH != 0 {
            self.splash_walk(centre, nx, ny, row.record.splash_range)
        } else {
            vec![centre]
        };
        let main = self
            .pc_skills
            .get(&vid.raw())
            .and_then(|state| state.uses.get(&vnum))
            .map_or(target_vid, |info| {
                if info.main_vid == 0 {
                    target_vid
                } else {
                    info.main_vid
                }
            });
        for victim in victims {
            self.skill_functor(vid, &row, victim, k, main, (nx, ny));
        }
        // `iRet != BATTLE_NONE`: `SetSyncOwner(this)`, then the main victim's
        // `BeginFight` (`:2714-2726` — the functor already began each victim's).
        let victim_pos = SyncPositionVictim {
            vid: Vid::new(centre),
            kind: SyncPositionVictimKind::Monster,
            x: nx,
            y: ny,
        };
        let now_ms = self.clock.elapsed();
        let _owner = self.set_npc_sync_owner(vid, (spot.x, spot.y), &victim_pos, now_ms);
    }
    /// `CHARACTER::ComputeSkill`'s affect arm (`G/char_skill.cpp:2443-2457`,
    /// `:2516-2560`, `:2585-2590`, `:2593-2602`, `:2707-2757`) for the 230f buff
    /// rows (3 4 19 34 49 63 64 65 79) and the heal 109: the SELFONLY victim,
    /// the target-range arm, the level, the PABEOP-good refusal, `atk`/`maxv`/
    /// the point vars, the MAGIC-attr `OnMove(true)`, the grandmaster amount
    /// swap, the cure roll, and the three point arms. 109's empty duration
    /// routes to the caster's `PointChange(HP)` — the instant heal. A refused
    /// compute sends nothing.
    /// The point2 arm of `compute_skill_affect` (`:2765-2783`, `:2839-2856`):
    /// the old vnum goes first, then the point2 row affect-adds with override
    /// off when its duration is positive, else the point changes now. A charge
    /// self-cast carries the DASH flag instead of the row's (`:2844-2847`).
    /// The point2 arm's inputs: the point, its amount, the duration vars and the
    /// party buffer — one tuple so the arm keeps seven params.
    fn apply_point2(
        &mut self,
        vid: Vid,
        row: &gamedata::pc_skill_data::PcSkill,
        args: (usize, i32, &BTreeMap<&str, f64>, i32),
        charge: bool,
    ) {
        let (on2, amount2, dur_vars, buffer) = args;
        let _old = self.remove_pc_affect(vid, row.record.vnum);
        let dur2 = f64_to_i32(row.duration2.eval(dur_vars)).saturating_add(buffer);
        if dur2 > 0 {
            let flag = if charge {
                AFF_TANHWAN_DASH
            } else {
                row.record.affect_flag2
            };
            self.add_pc_affect(
                vid,
                super::affect::Affect::buff(row.record.vnum, on2, amount2, flag, dur2, 0),
                false,
            );
        } else {
            self.change_pc_point(vid, on2, amount2, false);
        }
    }

    fn compute_skill_affect(&mut self, vid: Vid, vnum: u32, target_vid: u32, k: f64, level: u8) {
        let row = match self.pc_skill_tables.get(vnum) {
            Some(row) => row.clone(),
            None => return,
        };
        // No riding and no polymorph is ported, so both gates pass; no hosted
        // buff is a horse row.
        let centre = if row.record.flag & SKILL_FLAG_SELFONLY != 0 {
            vid.raw()
        } else {
            target_vid
        };
        let Some(spot) = self.spot_of(vid.raw()) else {
            return;
        };
        if self.skill_range_spot(&row, centre, spot).is_none() {
            return;
        }
        self.compute_skill_on(vid, Vid::new(centre), &row, k, level);
    }

    /// The charge's eager self-compute (`:3130` with `:2625-2628`, `:2736-2860`
    /// narrowed): the damage arm is skipped for a charge victim that is the
    /// caster, and the non-damage body runs — which for 5's empty duration
    /// skips the point change and lands the DASH through `apply_point2`'s
    /// charge flag. The target-range gate is the caster's own spot, which
    /// always passes; `compute_skill_on` keeps the shape.
    fn compute_charge_self(&mut self, vid: Vid, vnum: u32, k: f64, level: u8) {
        let row = match self.pc_skill_tables.get(vnum) {
            Some(row) => row.clone(),
            None => return,
        };
        self.compute_skill_on(vid, vid, &row, k, level);
    }

    /// `ComputeSkillParty(vnum, victim, 0)` (`:2430-2440`): the functor over the
    /// near members, or the caster alone with no party or no near member. The
    /// functor's attacker is the victim the call passes — the dispatch passes the
    /// caster (`:2408`), so the fan calls each near member with the caster as the
    /// attacker and the member as the victim. Dead today: no hosted PC row is
    /// PARTY-flagged (vnum 265 is unhosted), so no cast routes here; the fan's
    /// walk is pinned by `party.rs` tests calling it directly.
    fn compute_skill_party(&mut self, vid: Vid, vnum: u32, k: f64, level: u8) {
        let row = match self.pc_skill_tables.get(vnum) {
            Some(row) => row.clone(),
            None => return,
        };
        let pid = self
            .characters
            .find_by_vid(vid)
            .ok()
            .map(world::character::Character::player_id);
        if let Some(pid) = pid {
            if let Some(vids) = self.near_party_vids(pid) {
                for victim in vids {
                    self.compute_skill_on(vid, victim, &row, k, level);
                }
                return;
            }
        }
        self.compute_skill_on(vid, vid, &row, k, level);
    }

    /// `ComputeSkill(vnum, victim, 0)`'s per-victim body (`:2443-2890`), after the
    /// SELFONLY victim rewrite and the target-range gate: the level-zero refusal,
    /// the victim's PABEOP-good refusal, the caster-vars snapshot (with the
    /// victim's maxima and move-speed limit), and the amount/cure/point arms on
    /// the victim.
    /// `compute_skill_on`'s points snapshots: the caster's, and the victim's when
    /// one stands. `None` when the caster is gone.
    fn compute_vars_points(
        &self,
        vid: Vid,
        victim: Vid,
    ) -> Option<(world::character::Points, Option<world::character::Points>)> {
        let points_snapshot = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points().cloned())?;
        let victim_points = self
            .characters
            .find_by_vid(victim)
            .ok()
            .and_then(|character| character.points().cloned());
        Some((points_snapshot, victim_points))
    }

    /// `compute_skill_on`'s attack rating: the caster's dexterity and level
    /// against the victim's dexterity (0 with no victim).
    fn compute_attack_rating(
        points_snapshot: &world::character::Points,
        victim_points: Option<&world::character::Points>,
    ) -> f32 {
        let victim_dex = victim_points.map_or(0, |points| points.get_point(point::POINT_DX));
        attack_rating(AttackRatingInput {
            attacker_dexterity: points_snapshot.get_point(point::POINT_DX),
            attacker_level: i32::from(points_snapshot.level()),
            victim_dexterity: victim_dex,
            ignore_target_rating: false,
        })
    }

    fn compute_skill_on(
        &mut self,
        vid: Vid,
        victim: Vid,
        row: &gamedata::pc_skill_data::PcSkill,
        k: f64,
        level: u8,
    ) {
        if level == 0 {
            return;
        }
        // A non-player victim waits: this build's buff set computes on players only
        // (no `FuncSplashAffect` heals a mob here). The victim's own PABEOP flag
        // refuses its compute (`:2516-2519`: flag 33 is bit 0 of word 1).
        if self.npc_by_vid(victim.raw()).is_some() {
            return;
        }
        if self.affect_flags_of(victim)[1] & 1 != 0 && Self::is_good_affect(row.record.vnum) {
            return;
        }
        let Some((points_snapshot, victim_points)) = self.compute_vars_points(vid, victim) else {
            return;
        };
        let ar = Self::compute_attack_rating(&points_snapshot, victim_points.as_ref());
        // `atk` is 0 on every hosted arm: no 230f row sets a `USE_*_DAMAGE` flag
        // (`:2538-2555` — melee/magic/arrow `SetPointVar` all skipped) and 109's
        // `mwep` reads 0 with no weapon vars set. `ek` is the Eunhyung level
        // over 100: the 230f set can hold 34, but no attack since a cast sets
        // the level (EUNHYUNG hide waits), so 0.
        let atk = 0;
        let ek = 0.0;
        let mut vars = buff_vars(&points_snapshot, k, atk, ar, ek);
        if let Some(victim_points) = &victim_points {
            vars.insert(
                "maxhp",
                f64::from(victim_points.get_point(point::POINT_MAX_HP)),
            );
            vars.insert(
                "maxsp",
                f64::from(victim_points.get_point(point::POINT_MAX_SP)),
            );
        }
        // `maxv` is the victim's move-speed limit, set only for a move-speed
        // point (`:2557-2560`).
        if super::mob_skill::point_on(&row.record.point_on) == point::POINT_MOV_SPEED {
            vars.insert("maxv", f64::from(self.move_speed_of(victim.raw())));
        }
        let charge = Self::is_charge_skill(row.record.vnum);
        // The MAGIC-attr arm (`:2577-2578`): 79 runs `OnMove(true)` — the last
        // move and attack times stamp, and revive-invisible, if held, goes.
        // Mining is unported, so its cancel is vacuous; no Eunhyung level is
        // held, so the else arm clears nothing.
        if row.record.skill_attr_type == SKILL_ATTR_MAGIC {
            self.on_move_attack(vid);
        }
        let mut amount = f64_to_i32(row.point.eval(&vars));
        let amount2 = f64_to_i32(row.point2.eval(&vars));
        // The grandmaster swap (`:2593-2602`): GRAND+ with the stamp's grand
        // bit replaces the amount with the master bonus (the commented add is
        // dead).
        if master_of(level) >= SKILL_GRAND_MASTER
            && self.pc_skill_used_grand(vid.raw(), row.record.vnum)
        {
            amount = f64_to_i32(row.master_bonus.eval(&vars));
        }
        // The cure roll (`:2609-2615`): 109 rolls `number(1, 100) <= amount2`
        // and cures its victim on success.
        if row.record.flag & SKILL_FLAG_REMOVE_BAD_AFFECT != 0
            && number(&mut self.dice, 1, 100) <= amount2
        {
            self.remove_bad_affect(victim);
        }
        let dur_vars = BTreeMap::from([("k", k)]);
        let buffer = points_snapshot.get_point(point::POINT_PARTY_BUFFER_BONUS);
        let mut added = false;
        // The point arms (`:2761-2858`): a positive duration affect-adds on the
        // victim (with the duration-SP-cost vars holding `k` only), else the
        // victim's point changes now. With a point2, the old vnum goes first,
        // the point2 row adds with override off, then the point row adds with
        // override off (and 19 below GRAND carries `CHEONGEUN_WITH_FALL`);
        // without, the point row adds with `!added`. The buff set's durations
        // are all positive; 109's empty one routes to the victim's instant heal.
        let on2 = super::mob_skill::point_on(&row.record.point_on2);
        let dur = f64_to_i32(row.duration.eval(&dur_vars)).saturating_add(buffer);
        let on = super::mob_skill::point_on(&row.record.point_on);
        if dur > 0 {
            let sp_cost = f64_to_i32(row.duration_sp_cost.eval(&dur_vars));
            if on2 == point::POINT_NONE {
                self.add_pc_affect(
                    victim,
                    super::affect::Affect::buff(
                        row.record.vnum,
                        on,
                        amount,
                        row.record.affect_flag,
                        dur,
                        sp_cost,
                    ),
                    !added,
                );
            } else {
                let flag = if row.record.vnum == SKILL_CHUNKEON
                    && !(master_of(level) >= SKILL_GRAND_MASTER
                        && self.pc_skill_used_grand(vid.raw(), row.record.vnum))
                {
                    // 19's `CHEONGEUN_WITH_FALL` below GRAND (`:2792-2794`):
                    // flag 34 is bit 1 of word 1.
                    AFF_CHEONGEUN_WITH_FALL
                } else {
                    row.record.affect_flag
                };
                self.apply_point2(victim, row, (on2, amount2, &dur_vars, buffer), charge);
                self.add_pc_affect(
                    victim,
                    super::affect::Affect::buff(row.record.vnum, on, amount, flag, dur, sp_cost),
                    false,
                );
            }
            added = true;
        } else {
            // The charge guard (`:2832-2834`): a charge self-cast skips the
            // point change (5's empty duration lands here and only dashes).
            if !charge {
                self.change_pc_point(victim, on, amount, false);
            }
            if on2 != point::POINT_NONE {
                self.apply_point2(victim, row, (on2, amount2, &dur_vars, buffer), charge);
            }
        }
        // The point3 arm (`:2861-2890`): GRAND+ only, flag 0, `!added`. Every
        // 230f row carries an empty point3, so the arm never fires here.
        let _ = added;
    }

    /// The SPLASH walk's victim list: a no-sectree centre walks nothing; else every
    /// NPC of the sectrees around it within `DISTANCE_APPROX ≤ range`, the centre
    /// first (`ForEachAround` visits the centre's own sectree first, and the functor
    /// held the centre's spot — a Divergence pins the order, legacy's walk order is
    /// pointer order).
    fn splash_walk(&self, centre: u32, nx: i32, ny: i32, range: u32) -> Vec<u32> {
        // A SELFONLY centre is the caster: the walk starts empty (the caster refuses
        // its own functor's `is_struck` gate, so it is not walked as a victim).
        let mut victims = if self.npc_by_vid(centre).is_some() {
            vec![centre]
        } else {
            Vec::new()
        };
        let centre_key = if victims.is_empty() {
            super::view::EntityKey::Character(centre)
        } else {
            super::view::EntityKey::Npc(centre)
        };
        let Some(place) = self
            .npc_of
            .get(&centre)
            .map(|&(channel, map, _)| (channel, map))
            .or_else(|| self.place_of(centre))
        else {
            return victims;
        };
        let Some(index) = self.maps.get(&place) else {
            return victims;
        };
        for key in index.around_of(centre_key) {
            let super::view::EntityKey::Npc(other) = key else {
                continue;
            };
            if other == centre {
                continue;
            };
            let Some(npc) = self.npc_by_vid(other) else {
                continue;
            };
            if crate::sync_position::distance_approx(npc.x - nx, npc.y - ny)
                <= i32::try_from(range).unwrap_or(i32::MAX)
            {
                victims.push(other);
            }
        }
        victims
    }

    /// One `FuncSplashDamage` run (`G/char_skill.cpp:1356-1991`): the rim and
    /// attackable gates, `HitOnce`, the per-victim draws, `atk`/`ar`, the poly, the
    /// floor, the non-main adjust, the attr arm, `Damage` with the typed aggro, the
    /// stun at 0, the kill of a stunned victim, the victim's `BeginFight`, and — only
    /// when `Damage` answers false on a live, unstunned victim — the
    /// `STUN`/`POISON`/`FIRE_CONT` arms and `CRUSH`.
    #[allow(clippy::too_many_lines)]
    fn skill_functor(
        &mut self,
        vid: Vid,
        row: &gamedata::pc_skill_data::PcSkill,
        victim: u32,
        k: f64,
        main: u32,
        centre: (i32, i32),
    ) {
        let flag = row.record.flag;
        let Some(place) = self.place_of(vid.raw()) else {
            return;
        };
        let Some(spot) = self.spot_of(vid.raw()) else {
            return;
        };
        let Some(npc_snapshot) = self.npc_on(victim, place).cloned() else {
            return;
        };
        // The rim arm (`:1369-1378`): `DISTANCE_APPROX > iSplashRange` refuses. A
        // no-SPLASH row walks only the main victim at the centre, so the rim passes.
        if crate::sync_position::distance_approx(
            npc_snapshot.x - centre.0,
            npc_snapshot.y - centre.1,
        ) > i32::try_from(row.record.splash_range).unwrap_or(i32::MAX)
            && victim != main
        {
            return;
        }
        if !self.is_attackable(vid, (place.1, spot.x, spot.y), &npc_snapshot) {
            return;
        }
        // `HitOnce` (`:1393-1400`): unused refuses, an expired stamp refuses
        // (MUYEONG excepted — never a 230e row), `-1` is infinite, else the count
        // falls by one.
        let now = self.last_pulse;
        let once = self
            .pc_skill_state(vid.raw())
            .uses
            .entry(row.record.vnum)
            .or_default()
            .hit_once(now, row.record.vnum);
        if !once {
            return;
        }
        // The per-victim draws (`:1412-1446`): PENETRATE's and IGNORE_TARGET_RATING's
        // `kPointPoly2` rolls, then `atk` and `ar`.
        let point2_vars = BTreeMap::from([("k", k)]);
        let point2 = f64_to_i32(row.point2.eval(&point2_vars));
        let ignore_defence =
            flag & SKILL_FLAG_PENETRATE != 0 && number(&mut self.dice, 1, 100) <= point2;
        let ignore_rating =
            flag & SKILL_FLAG_IGNORE_TARGET_RATING != 0 && number(&mut self.dice, 1, 100) <= point2;
        let points_snapshot = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points().cloned())
            .expect("the caster stood above");
        let victim_snapshot = self
            .npc_by_vid(victim)
            .expect("the victim stood above")
            .clone();
        let ar = attack_rating(AttackRatingInput {
            attacker_dexterity: points_snapshot.get_point(point::POINT_DX),
            attacker_level: i32::from(points_snapshot.level()),
            victim_dexterity: victim_snapshot.fight.point(point::POINT_DX),
            ignore_target_rating: ignore_rating,
        });
        let weapon = self.wielded_weapon(vid);
        let atk = if flag & SKILL_FLAG_USE_ARROW_DAMAGE != 0 {
            // `USE_ARROW_DAMAGE` (`:1429-1437`): bow+arrow or 0 (hosted: vnum 47).
            // `USE_MAGIC_DAMAGE`'s `CalcMagicDamage` has no hosted PC row — not ported.
            match self.arrow_and_bow(vid) {
                Some((bow, arrow)) => {
                    let distance = f32_to_i32(distance_sqrt(
                        victim_snapshot.x - spot.x,
                        victim_snapshot.y - spot.y,
                    ));
                    calc_arrow_skill_atk(
                        &mut self.dice,
                        &points_snapshot,
                        &bow,
                        &arrow,
                        &victim_snapshot,
                        distance,
                    )
                }
                None => 0,
            }
        } else {
            calc_melee_skill_atk(
                &mut self.dice,
                &points_snapshot,
                weapon.as_ref(),
                &victim_snapshot,
            )
        };
        let mut vars = skill_vars(&points_snapshot, &victim_snapshot, k, atk, ar);
        // `SetPolyVarForAttack` (`:1324-1354`): hosted polys name `mwep` only
        // (81/91/92/93); `wep`/`mtk` unset-read-0.
        let mwep = weapon.as_ref().map_or(0, |weapon| {
            number(&mut self.dice, weapon.values[1], weapon.values[2])
                .wrapping_add(weapon.values[5])
        });
        vars.insert("mwep", f64::from(mwep));
        let mut amount = -f64_to_i32(row.point.eval(&vars));
        // No hosted cast rises past NORMAL (`SetSkillLevel` maps level < 20 there), so
        // the grandmaster `kMasterBonusPoly` arm never fires. AMSEOP/GUNGSIN (31/32)
        // are refused — not ported.
        amount = battle_damage_floor(&mut self.dice, amount);
        // The non-main splash adjust (`:1526-1531`): a main victim keeps the full
        // amount; any other takes `* kSplashAroundDamageAdjustPoly` (`k`-only var).
        if victim != main {
            let adjust = row.splash_adjust.eval(&BTreeMap::from([("k", k)]));
            amount = f64_to_i32(f64::from(amount) * adjust);
        }
        // The attr arms (`:1549-1600`): MELEE/RANGE/MAGIC with their `dt`. NORMAL
        // never reaches here (no hosted row is NORMAL-attr); the NPC
        // WIND/ELEC/FIRE arms and `COMPUTE_MAGIC_DAMAGE` have no hosted row.
        // (The HELP/ANTI arms read level 0 on every helper/anti vnum — unhosted —
        // so they multiply nothing and stay out.)
        let attr = row.record.skill_attr_type;
        let (damage, kind, aggro) = if attr == SKILL_ATTR_RANGE {
            (
                skill_range_attr(amount, &victim_snapshot.fight),
                SkillDamageKind::Range,
                AggroDamage::Range,
            )
        } else if attr == SKILL_ATTR_MAGIC {
            (
                skill_magic_attr(amount, &points_snapshot, &victim_snapshot),
                SkillDamageKind::Magic,
                AggroDamage::Magic,
            )
        } else {
            let damage = skill_melee_attr(
                amount,
                weapon.as_ref(),
                victim_snapshot.fight.point(point::POINT_DEF_GRADE),
                &victim_snapshot.fight,
                ignore_defence,
            );
            // A SPLASH MELEE row still lands through the MELEE pipe.
            let (struck, events) = self.skill_damage_melee(vid, victim, damage, weapon.as_ref());
            self.show_strike(vid, victim, events);
            return self.after_skill_damage(vid, row, victim, struck, k, AggroDamage::Melee);
        };
        let (struck, _stolen, events) = self.skill_damage_typed(vid, victim, damage, kind);
        self.show_strike(vid, victim, events);
        self.after_skill_damage(vid, row, victim, struck, k, aggro);
    }

    /// The functor's tail past `Damage`: the damage map with the typed aggro, the stun
    /// at 0, the kill of a stunned victim, the victim's `BeginFight`, and — only when
    /// `Damage` answers false on a live, unstunned victim — the
    /// `STUN`/`POISON`/`FIRE_CONT` arms (`KNOCKBACK`/`REMOVE_GOOD_AFFECT`/`SLOW`
    /// have no hosted row) and `CRUSH`.
    fn after_skill_damage(
        &mut self,
        vid: Vid,
        row: &gamedata::pc_skill_data::PcSkill,
        victim: u32,
        struck: Struck,
        k: f64,
        aggro: AggroDamage,
    ) {
        let vnum = row.record.vnum;
        let main = self
            .pc_skills
            .get(&vid.raw())
            .and_then(|state| state.uses.get(&vnum))
            .map_or(victim, |info| {
                if info.main_vid == 0 {
                    victim
                } else {
                    info.main_vid
                }
            });
        match struck {
            Struck::Landed { damage } => {
                self.set_last_attacked(victim);
                self.after_damage_typed(vid, victim, damage, aggro);
            }
            Struck::Finished => {
                self.set_last_attacked(victim);
                if let Some(body) = self.bodies.get_mut(&vid) {
                    body.fighting = false;
                }
                self.dead_npc(victim, Some(vid));
                return;
            }
            Struck::Whiffed | Struck::Refused | Struck::AlreadyDead | Struck::StunnedByGuard => {
                return;
            }
        }
        // `iRet != BATTLE_NONE`: the victim's `BeginFight` (`:1615-1616`).
        if self
            .npc_by_vid(victim)
            .is_some_and(super::mob_battle::fights_back)
            && self.can_begin_fight(victim)
        {
            self.begin_fight(victim, vid);
        }
        // `!Damage && !IsStun` (`:1791`): the post-damage arms. `Damage` answered
        // false (a Landed hit), so only the stun gate remains.
        if self.npc_by_vid(victim).is_some_and(|npc| npc.stunned) {
            return;
        }
        self.skill_affect_arms(vid, row, victim, k);
        self.skill_crush(vid, row, victim, main);
    }

    /// The functor's `STUN`/`POISON`/`FIRE_CONT` arms
    /// (`G/char_skill.cpp:1858-1897`): the `kPointPoly2` pct, the `kDurationPoly2`
    /// duration, the sung-ma halve, the party buffer bonus; `STUN` via
    /// `SkillAttackAffect`, `FIRE_CONT` via the dur2-draw then `AttackedByFire`,
    /// `POISON` via `AttackedByPoison`. `SLOW`'s only row (80) is refused —
    /// documented, not ported.
    fn skill_affect_arms(
        &mut self,
        vid: Vid,
        row: &gamedata::pc_skill_data::PcSkill,
        victim: u32,
        k: f64,
    ) {
        let flag = row.record.flag;
        if flag & (SKILL_FLAG_STUN | SKILL_FLAG_POISON | SKILL_FLAG_FIRE_CONT) == 0 {
            return;
        }
        let points_snapshot = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points().cloned())
            .expect("the caster stood above");
        let mut pct = f64_to_i32(row.point2.eval(&BTreeMap::from([("k", k)])));
        // The sung-ma halve (`__CONQUEROR_LEVEL__` is defined): the map's immune will
        // past the attacker's halves the pct.
        if i32::from(points_snapshot.will().immune)
            > points_snapshot.get_point(point::POINT_SUNGMA_IMMUNE)
        {
            pct /= 2;
        }
        let buffer = points_snapshot.get_point(point::POINT_PARTY_BUFFER_BONUS);
        if flag & SKILL_FLAG_STUN != 0 {
            // `SkillAttackAffect` (`G/battle.h:64-94`): `number(1, 1000) <= pct`,
            // past the flag and `IMMUNE_STUN`.
            let words = self.npc_affect_words(victim);
            let npc = self.npc_by_vid(victim).expect("the victim stood above");
            let immune = npc.immune_flag & IMMUNE_STUN != 0;
            if pct != 0
                && words[0] & AFF_STUN_WORD == 0
                && !immune
                && number(&mut self.dice, 1, 1000) <= pct
            {
                let duration = f64_to_i32(row.duration2.eval(&BTreeMap::from([("k", k)])))
                    .wrapping_add(buffer);
                self.add_npc_affect(
                    victim,
                    super::affect::Affect {
                        kind: crate::loading_phase::AFFECT_STUN,
                        apply_on: u8::try_from(point::POINT_NONE).unwrap_or(u8::MAX),
                        value: 0,
                        flag: crate::loading_phase::AFF_STUN,
                        remaining: duration,
                        sp_cost: 0,
                    },
                );
            }
        } else if flag & SKILL_FLAG_FIRE_CONT != 0 {
            // The dur2-draw (`:1880-1894`): `number(1, 100) <= iDur` then
            // `AttackedByFire(attacker, iPct, 5)`.
            let duration = f64_to_i32(row.duration2.eval(&BTreeMap::from([("k", k)])));
            let duration = if buffer != 0 {
                duration.wrapping_add(buffer / 2)
            } else {
                duration
            };
            if number(&mut self.dice, 1, 100) <= duration {
                self.fire_npc(vid, victim, pct);
            }
        } else if flag & SKILL_FLAG_POISON != 0 && number(&mut self.dice, 1, 100) <= pct {
            self.poison_npc(vid, victim);
        }
    }

    /// `AttackedByFire` on the non-player under `victim` (`G/char_resist.cpp:161-176`):
    /// nothing while a fire event runs, else the `AFFECT_FIRE` affect lands
    /// (`count*3+1` ticks) and the fire event with 5 ticks is due 1 Pulse on.
    fn fire_npc(&mut self, attacker: Vid, victim: u32, amount: i32) {
        if self
            .npc_combat
            .get(&victim)
            .is_some_and(|combat| combat.fire_due.is_some())
        {
            return;
        }
        self.add_npc_affect(
            victim,
            super::affect::Affect {
                kind: AFFECT_FIRE,
                apply_on: u8::try_from(point::POINT_NONE).unwrap_or(u8::MAX),
                value: 0,
                flag: AFF_FIRE,
                remaining: 5 * 3 + 1,
                sp_cost: 0,
            },
        );
        let when = self.last_pulse.saturating_add(1);
        let combat = self.npc_combat.entry(victim).or_default();
        combat.fire_attacker = Some(attacker.raw());
        combat.fire_due = Some((when, 5, amount));
        self.regens.due.insert((
            when,
            super::regen::Due::Fire {
                vid: victim,
                count: 5,
                amount,
            },
        ));
    }

    /// The functor's CRUSH arm (`G/char_skill.cpp:1899-1961`): past a NOMOVE flag, the
    /// 200.0 slide from the caster through the victim, the `WALLS_FIX` loop stepping
    /// back 10 at a time off a blocked cell, then `Sync`, `Goto`,
    /// `CalculateMoveDuration` — and the main victim takes a 1000-pct 4 s stun
    /// while the others only take a `SyncPacket`.
    fn skill_crush(
        &mut self,
        vid: Vid,
        row: &gamedata::pc_skill_data::PcSkill,
        victim: u32,
        main: u32,
    ) {
        use super::mob_ai::delta_by_degree;
        let flag = row.record.flag;
        if flag & (SKILL_FLAG_CRUSH | SKILL_FLAG_CRUSH_LONG) == 0 {
            return;
        }
        let Some(npc) = self.npc_by_vid(victim) else {
            return;
        };
        if npc.ai_flags & AIFLAG_NOMOVE != 0 {
            return;
        }
        let Some(spot) = self.spot_of(vid.raw()) else {
            return;
        };
        let Some((_, map)) = self.place_of(vid.raw()) else {
            return;
        };
        let (vx, vy) = (npc.x, npc.y);
        let degree = rotation_to_xy(vx.saturating_sub(spot.x), vy.saturating_sub(spot.y));
        let mut length = CRUSH_SLIDE_PC;
        if flag & SKILL_FLAG_CRUSH_LONG != 0 {
            length *= 2.0;
        }
        let (mut fx, mut fy) = delta_by_degree(degree, length);
        let (mut tx, mut ty) = (
            vx.saturating_add(common::cfloat::f32_to_i32(fx)),
            vy.saturating_add(common::cfloat::f32_to_i32(fy)),
        );
        while !self.is_movable(map, tx, ty) && length > 0.0 {
            if length >= 10.0 {
                length -= 10.0;
            } else {
                length = 0.0;
            }
            (fx, fy) = delta_by_degree(degree, length);
            tx = vx.saturating_add(common::cfloat::f32_to_i32(fx));
            ty = vy.saturating_add(common::cfloat::f32_to_i32(fy));
        }
        // `Sync(tx, ty)` + `Goto(tx, ty)` + `CalculateMoveDuration`: the victim stands
        // at the slid point.
        self.npc_move(victim, tx, ty);
        if victim == main {
            // The main victim's 1000-pct 4 s stun (`:1952-1955`): the draw always
            // passes, so only the flag and `IMMUNE_STUN` gates remain.
            let words = self.npc_affect_words(victim);
            let npc = self.npc_by_vid(victim).expect("the victim stood above");
            if words[0] & AFF_STUN_WORD == 0 && npc.immune_flag & IMMUNE_STUN == 0 {
                self.add_npc_affect(
                    victim,
                    super::affect::Affect {
                        kind: row.record.vnum,
                        apply_on: u8::try_from(point::POINT_NONE).unwrap_or(u8::MAX),
                        value: 0,
                        flag: crate::loading_phase::AFF_STUN,
                        remaining: 4,
                        sp_cost: 0,
                    },
                );
            }
        } else {
            // `SyncPacket`: the one-element `GC_SYNC_POSITION` around the victim.
            let mut record = vec![
                protocol::gc_inventory::HEADER_GC_SYNC_POSITION.value(),
                0,
                0,
            ];
            record.extend_from_slice(
                &protocol::gc_position::GcSyncPositionElement::new(victim, tx, ty).encode(),
            );
            let size = u16::try_from(record.len()).unwrap_or(u16::MAX);
            record[1..3].copy_from_slice(&size.to_le_bytes());
            self.packet_around(super::view::EntityKey::Npc(victim), &record, None);
        }
    }

    /// `ComputeSkill`'s target-range arm (`G/char_skill.cpp:2505-2517`): the
    /// victim's spot, or `None` past `dwTargetRange + 50` (`0` reaches everywhere).
    fn skill_range_spot(
        &self,
        row: &gamedata::pc_skill_data::PcSkill,
        target_vid: u32,
        spot: super::view::Spot,
    ) -> Option<(i32, i32)> {
        // A SELFONLY centre is the caster, not a non-player: the walk starts at the
        // caster's own spot (no target-range measure — the caster is 0 away).
        let (nx, ny) = match self.npc_by_vid(target_vid) {
            Some(npc) => (npc.x, npc.y),
            None => match self.spot_of(target_vid) {
                Some(at) => (at.x, at.y),
                None => return None,
            },
        };
        // `pkSk->dwTargetRange && DISTANCE_SQRT(...) >= range + 50`.
        let ok = row.record.target_range == 0
            || distance_sqrt(nx - spot.x, ny - spot.y)
                < f64_to_f32(f64::from(row.record.target_range)) + 50.0;
        ok.then_some((nx, ny))
    }

    /// `Damage(pc, iDam, MELEE)` on the monster under `target_vid`: the MELEE
    /// pipe, split from [`GameState::compute_skill_melee`] so the borrow of the
    /// caster's points ends before the hit is shown.
    fn skill_damage_melee(
        &mut self,
        vid: Vid,
        target_vid: u32,
        damage: i32,
        weapon: Option<&gamedata::item_proto::ItemProto>,
    ) -> (Struck, Vec<world::character::HitEvent>) {
        let empire = self
            .bodies
            .get(&vid)
            .expect("the caster stood above")
            .card
            .empire;
        let map = self.place_of(vid.raw()).expect("the caster stood above").1;
        let user_rate = self.user_damage_rate();
        let &(channel, nmap, at) = self
            .npc_of
            .get(&target_vid)
            .expect("the victim stood above");
        let character = self
            .characters
            .find_by_vid_mut(vid)
            .expect("the caster stood above");
        let points_mut = character
            .items_and_points_mut()
            .1
            .expect("the caster stood above");
        let mut striker = Striker {
            points: points_mut,
            weapon,
            empire,
            map,
            user_rate,
        };
        let npcs = self
            .npcs
            .get_mut(&(channel, nmap))
            .expect("the victim stood above");
        let npc = Arc::make_mut(npcs)
            .npcs
            .get_mut(at)
            .expect("the victim stood above");
        damage_melee(&mut self.dice, &mut striker, npc, damage)
    }

    /// The typed skill pipes: RANGE or MAGIC through the one shared branch
    /// (`damage_range`/`damage_magic`), `NORMAL_RANGE` through the dodge-gated
    /// NORMAL body (`damage_normal_range`, which answers the stolen gold too).
    fn skill_damage_typed(
        &mut self,
        vid: Vid,
        target_vid: u32,
        damage: i32,
        kind: SkillDamageKind,
    ) -> (Struck, i32, Vec<world::character::HitEvent>) {
        // `battle_is_attackable` refused a victim on an `ATTR_BANPK` cell, and a standing
        // non-player has a sectree, so `Damage` never refuses it here (as in `strike`).
        let banned = false;
        let empire = self
            .bodies
            .get(&vid)
            .expect("the caster stood above")
            .card
            .empire;
        let map = self.place_of(vid.raw()).expect("the caster stood above").1;
        let user_rate = self.user_damage_rate();
        let &(channel, nmap, at) = self
            .npc_of
            .get(&target_vid)
            .expect("the victim stood above");
        let weapon = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| {
                Equipment::of(character.items(), &self.protos).wear(EWearPositions::Weapon as u16)
            })
            .map(|worn| worn.proto.clone());
        let character = self
            .characters
            .find_by_vid_mut(vid)
            .expect("the caster stood above");
        let points_mut = character
            .items_and_points_mut()
            .1
            .expect("the caster stood above");
        let mut striker = Striker {
            points: points_mut,
            weapon: weapon.as_ref(),
            empire,
            map,
            user_rate,
        };
        let npcs = self
            .npcs
            .get_mut(&(channel, nmap))
            .expect("the victim stood above");
        let npc = Arc::make_mut(npcs)
            .npcs
            .get_mut(at)
            .expect("the victim stood above");
        match kind {
            SkillDamageKind::Range => {
                let (struck, events) = damage_range(&mut self.dice, &mut striker, npc, damage);
                (struck, 0, events)
            }
            SkillDamageKind::Magic => {
                let (struck, events) = damage_magic(&mut self.dice, &mut striker, npc, damage);
                (struck, 0, events)
            }
            SkillDamageKind::NormalRange => {
                damage_normal_range(&mut self.dice, &mut striker, npc, damage, banned)
            }
        }
    }

    /// The proto of what the player under `vid` wears in `WEAR_WEAPON`, cloned so the
    /// skill pipe can hold it across `&mut self` draws.
    fn wielded_weapon(&self, vid: Vid) -> Option<gamedata::item_proto::ItemProto> {
        let character = self.characters.find_by_vid(vid).ok()?;
        Equipment::of(character.items(), &self.protos)
            .wear(EWearPositions::Weapon as u16)
            .map(|worn| worn.proto.clone())
    }
    /// `CInputMain::FlyTarget` + `CHARACTER::FlyTarget` (`G/input_main.cpp:2168-2172`,
    /// `G/char_battle.cpp:3342-3371`): `main` stores the single main VID, else the VID
    /// is appended to the extras; both relay a `GC_(ADD_)FLY_TARGETING` around but the
    /// shooter. An unknown VID relays with target 0 and the sent spot, storing nothing.
    /// No observer check: the input arm passes `ch` unguarded.
    pub(super) fn fly_target(&mut self, vid: Vid, main: bool, target: u32, x: i32, y: i32) {
        let (relay_vid, rx, ry) = if let Some(npc) = self.npc_by_vid(target) {
            (target, npc.x, npc.y)
        } else if let Some(spot) = self.spot_of(target) {
            (target, spot.x, spot.y)
        } else {
            let header = if main {
                protocol::gc::HEADER_GC_FLY_TARGETING
            } else {
                protocol::gc::HEADER_GC_ADD_FLY_TARGETING
            };
            let mut relay = Vec::new();
            protocol::gc_nested::GcFlyTargeting::new(header, vid.raw(), 0, x, y)
                .encode_into(&mut relay);
            let me = super::view::EntityKey::Character(vid.raw());
            self.packet_around(me, &relay, Some(me));
            return;
        };
        let state = self.pc_skill_state(vid.raw());
        if main {
            state.fly_main = relay_vid;
        } else {
            state.fly_extra.push(relay_vid);
        }
        let header = if main {
            protocol::gc::HEADER_GC_FLY_TARGETING
        } else {
            protocol::gc::HEADER_GC_ADD_FLY_TARGETING
        };
        let mut relay = Vec::new();
        protocol::gc_nested::GcFlyTargeting::new(header, vid.raw(), relay_vid, rx, ry)
            .encode_into(&mut relay);
        let me = super::view::EntityKey::Character(vid.raw());
        self.packet_around(me, &relay, Some(me));
    }

    /// `CInputMain::Attack`'s `CG_SHOOT` arm + `CHARACTER::Shoot` (`G/input_main.cpp:2000-2007`,
    /// `G/char_battle.cpp:3319-3340`): the stun gate, then `CFuncShoot` over the main VID
    /// first (cleared) and every extra (cleared after). Answers the stolen gold, as the
    /// plain bow arm lands through the `NORMAL_RANGE` pipe; `m_bSucceed` stays
    /// false and
    /// the input arm ignores the answer, so no other answer is kept.
    pub(super) fn shoot(&mut self, vid: Vid, btype: u8) -> crate::game_loop_messages::AttackAnswer {
        use crate::game_loop_messages::AttackAnswer;
        // `!CanMove()` (`:3323-3326`): only the stun arm is ported.
        if self.affect_flags_of(vid)[0] & AFF_STUN_WORD != 0 {
            return AttackAnswer::default();
        }
        let (main, extras) = {
            let state = self.pc_skill_state(vid.raw());
            let main = std::mem::replace(&mut state.fly_main, 0);
            let extras = std::mem::take(&mut state.fly_extra);
            (main, extras)
        };
        let mut stolen = None;
        if main != 0 {
            // Past type 1 the functor stamps the main VID (`:3041-3042`); the commented
            // `ResetHitCount` never runs.
            if btype > 1 {
                if let Some(info) = self
                    .pc_skill_state(vid.raw())
                    .uses
                    .get_mut(&u32::from(btype))
                {
                    info.main_vid = main;
                }
            }
            stolen = self.shoot_one(vid, btype, main).or(stolen);
        }
        for target in extras {
            if btype > 1 {
                if let Some(info) = self
                    .pc_skill_state(vid.raw())
                    .uses
                    .get_mut(&u32::from(btype))
                {
                    info.main_vid = target;
                }
            }
            stolen = self.shoot_one(vid, btype, target).or(stolen);
        }
        AttackAnswer { gold: stolen }
    }

    /// One `CFuncShoot` run (`G/char_battle.cpp:3035-3317`): the victim must exist and be
    /// attackable (the NPC shooter's 5000 range never fires for a player). Answers the
    /// stolen gold for the plain bow arm, `None` for the skill arms.
    fn shoot_one(&mut self, vid: Vid, btype: u8, target: u32) -> Option<i64> {
        let (Some(place), Some(spot)) = (self.place_of(vid.raw()), self.spot_of(vid.raw())) else {
            return None;
        };
        let npc_snapshot = self.npc_on(target, place)?.clone();
        if !self.is_attackable(vid, (place.1, spot.x, spot.y), &npc_snapshot) {
            return None;
        }
        match btype {
            0 => self.shoot_bow(vid, target),
            1 => None,
            vnum if u32::from(vnum) == SKILL_YEONSA
                || u32::from(vnum) == SKILL_KWANKYEOK
                || u32::from(vnum) == SKILL_HWAJO
                || u32::from(vnum) == SKILL_GIGUNG
                || u32::from(vnum) == SKILL_HORSE_WILDATTACK_RANGE =>
            {
                // The arrow skills (`:3149-3230`): `== iUseArrow` needs 1 arrow, then the
                // `OnMove`s, `BeginFight`, `ComputeSkill` and `UseArrow(1)`. Only vnum 47
                // is hosted; the others refuse as unknown rows in `compute_skill`.
                self.arrow_and_bow(vid)?;
                self.on_move_pair(vid, target);
                if self
                    .npc_by_vid(target)
                    .is_some_and(super::mob_battle::fights_back)
                    && self.can_begin_fight(target)
                {
                    self.begin_fight(target, vid);
                }
                self.compute_skill(vid, u32::from(vnum), target);
                self.use_arrow(vid, 1);
                None
            }
            vnum if matches!(u32::from(vnum), 76 | 80 | 81 | 106 | 20 | 35 | 91 | 33) => {
                // The MELEE/MAGIC skill arms (`:3232-3289`): `MARYUNG/TUSOK/MAHWAN/BIPABU/`
                // `NOEJEON/GEOMPUNG/SANGONG/PABEOB` — vnum 33 is `PABEOB` (`G/skill.h:94`).
                // The `OnMove`s, `BeginFight` and `ComputeSkill`, no arrow. The hosted
                // set is 20/35/81/91 (MAGIC 92/93 and SPLASH 5/18/62 cast through
                // `CG_ATTACK`; they share the pipe, so the shape stays).
                self.on_move_pair(vid, target);
                if self
                    .npc_by_vid(target)
                    .is_some_and(super::mob_battle::fights_back)
                    && self.can_begin_fight(target)
                {
                    self.begin_fight(target, vid);
                }
                self.compute_skill(vid, u32::from(vnum), target);
                None
            }
            vnum if u32::from(vnum) == SKILL_CHAIN => {
                // `CHAIN` (`:3282-3289`): the `OnMove`s, `BeginFight` and `ComputeSkill`;
                // the chain walk waits (the TODO comment).
                self.on_move_pair(vid, target);
                if self
                    .npc_by_vid(target)
                    .is_some_and(super::mob_battle::fights_back)
                    && self.can_begin_fight(target)
                {
                    self.begin_fight(target, vid);
                }
                self.compute_skill(vid, u32::from(vnum), target);
                None
            }
            vnum if u32::from(vnum) == SKILL_YONGBI => {
                // `OnMove(true)` only (`:3290-3295`): no hosted caller, kept for the shape.
                let now = self.clock.now();
                if let Some(body) = self.bodies.get_mut(&vid) {
                    body.last_move = now;
                    body.last_attack = Some(now);
                }
                None
            }
            _ => None,
        }
    }

    /// The plain bow arm (`G/char_battle.cpp:3073-3104`): assassin only, needs a
    /// bow + arrow, group 2 skips the 5-SP charge (others pay it when SP ≥ 5),
    /// then `CalcArrowDamage`, `UseArrow(1)`, `NormalAttackAffect`, `RESIST_BOW`,
    /// the `OnMove`s, `BeginFight`, and `Damage(NORMAL_RANGE)`. The speed-hack
    /// gate reads keys no configuration sets, so refused attacks never fire.
    /// Answers the stolen gold (`NORMAL_RANGE` steals gold).
    fn shoot_bow(&mut self, vid: Vid, target: u32) -> Option<i64> {
        let (job, group) = match self.bodies.get(&vid) {
            Some(body) => (body.card.job, body.card.skill_group),
            None => return None,
        };
        if job != JOB_ASSASSIN {
            return None;
        }
        let (bow, arrow) = self.arrow_and_bow(vid)?;
        if group != 0 && group != 2 {
            let points = self.pc_points_mut(vid.raw())?;
            if points.sp() < BOW_SHOT_SP {
                return None;
            }
            let records = points
                .point_change(point::POINT_SP, -BOW_SHOT_SP, false, false)
                .ok()?;
            self.send_point_records(vid, &records);
        }
        let spot = self.spot_of(vid.raw())?;
        let npc_snapshot = self.npc_by_vid(target)?.clone();
        let distance = f32_to_i32(distance_sqrt(
            npc_snapshot.x - spot.x,
            npc_snapshot.y - spot.y,
        ));
        let points_snapshot = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points().cloned())?;
        let mut damage = world::character::calc_arrow_damage(
            &mut self.dice,
            &points_snapshot,
            &bow,
            &arrow,
            &npc_snapshot,
            distance,
        );
        self.use_arrow(vid, 1);
        let landed =
            world::character::normal_attack_affect(&mut self.dice, &points_snapshot, &npc_snapshot);
        self.apply_npc_affect(vid, target, landed);
        damage = damage
            .wrapping_mul(100 - npc_snapshot.fight.point(point::POINT_RESIST_BOW))
            .wrapping_div(100);
        self.on_move_pair(vid, target);
        if self
            .npc_by_vid(target)
            .is_some_and(super::mob_battle::fights_back)
            && self.can_begin_fight(target)
        {
            self.begin_fight(target, vid);
        }
        let (struck, stolen, events) =
            self.skill_damage_typed(vid, target, damage, SkillDamageKind::NormalRange);
        self.show_strike(vid, target, events);
        match struck {
            Struck::Landed { damage } => {
                self.set_last_attacked(target);
                self.after_damage_typed(vid, target, damage, AggroDamage::NormalRange);
            }
            Struck::Finished => {
                self.set_last_attacked(target);
                if let Some(body) = self.bodies.get_mut(&vid) {
                    body.fighting = false;
                }
                self.dead_npc(target, Some(vid));
            }
            Struck::Whiffed | Struck::Refused | Struck::AlreadyDead | Struck::StunnedByGuard => {}
        }
        let victim_pos = SyncPositionVictim {
            vid: Vid::new(target),
            kind: SyncPositionVictimKind::Monster,
            x: npc_snapshot.x,
            y: npc_snapshot.y,
        };
        let now_ms = self.clock.elapsed();
        let _owner = self.set_npc_sync_owner(vid, (spot.x, spot.y), &victim_pos, now_ms);
        // The plain hit answers `None` with no steal (`fight.rs`'s `steal_gold` gate);
        // a bow shot with no steal answers the same silence.
        if stolen == 0 {
            None
        } else {
            Some(i64::from(stolen))
        }
    }

    /// The caster's `OnMove(true)` plus the victim's `OnMove()`: stamps the caster's move
    /// and attack times and clears the revive-invisible (`G/char.cpp:6148-6179`); the
    /// victim's holds nothing for a non-player.
    fn on_move_pair(&mut self, vid: Vid, _target: u32) {
        let now = self.clock.now();
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.last_move = now;
            body.last_attack = Some(now);
        }
        self.remove_revive_invisible(vid);
    }

    /// `GetArrowAndBow(vid, 1)`: the bow and arrow protos when `WEAR_WEAPON` holds an
    /// `ITEM_WEAPON`/`WEAPON_BOW` and `WEAR_ARROW` an `ITEM_WEAPON`/`WEAPON_ARROW`
    /// (`G/char_battle.cpp:2981-3001`). Answers `None` on 0, as `0 ==` refuses.
    fn arrow_and_bow(
        &self,
        vid: Vid,
    ) -> Option<(
        gamedata::item_proto::ItemProto,
        gamedata::item_proto::ItemProto,
    )> {
        let character = self.characters.find_by_vid(vid).ok()?;
        let worn = Equipment::of(character.items(), &self.protos);
        let bow = worn.wear(EWearPositions::Weapon as u16)?;
        if bow.proto.item_type != ITEM_WEAPON || bow.proto.sub_type != WEAPON_BOW {
            return None;
        }
        let arrow = worn.wear(EWearPositions::Arrow as u16)?;
        if arrow.proto.item_type != ITEM_WEAPON || arrow.proto.sub_type != WEAPON_ARROW {
            return None;
        }
        if arrow.item.count == 0 {
            return None;
        }
        Some((bow.proto.clone(), arrow.proto.clone()))
    }

    /// `UseArrow(n)` (`G/char_battle.cpp:3003-3022`): `count -= MIN(count, n)`; at 0 the
    /// first stack of the same vnum in the base inventory (`FindSpecifyItem`,
    /// `INVENTORY_MAX_NUM` = 180 — the custom band is not what the Rewrite ports) is
    /// equipped into the arrow slot; with none found the slot keeps a 0-count item worn
    /// (`SetCount(0)` — the legacy quirk, kept).
    fn use_arrow(&mut self, vid: Vid, n: u32) {
        use common::item_slots::{EWindows, INVENTORY_MAX_NUM};
        use protocol::item_pos::ItemPos;
        let arrow_pos = ItemPos::new(
            EWindows::Inventory as u8,
            INVENTORY_MAX_NUM + EWearPositions::Arrow as u16,
        );
        let Some((id, vnum, count)) = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(arrow_pos))
            .map(|item| (item.id, item.vnum, item.count))
        else {
            return;
        };
        let left = u32::from(count).saturating_sub(n.min(u32::from(count)));
        if left == 0 {
            // The quirk's 0-count write: `SetCount(0)` keeps the slot worn. The Rewrite's
            // `set_count` refuses 0, so the arrow stack is removed and re-stored with a
            // 0 count through `set`'s body insert (which validates the size, not the count).
            let held = self
                .characters
                .find_by_vid(vid)
                .ok()
                .and_then(|character| character.items().item_at(arrow_pos).cloned());
            let Some(mut held) = held else {
                return;
            };
            held.count = 0;
            let Ok(character) = self.characters.find_by_vid_mut(vid) else {
                return;
            };
            let probe = held.clone();
            if character.items_mut().remove(arrow_pos, &probe).is_err() {
                return;
            }
            if character.items_mut().set(arrow_pos, &held).is_err() {
                return;
            }
            // The quirk's `UpdatePacket` too: the client sees the 0-count stack.
            let update = held.gc_item_update();
            let _sent = self.write_to_client(vid, update.encode());
        } else {
            let Ok(character) = self.characters.find_by_vid_mut(vid) else {
                return;
            };
            if character.items_mut().set_count(id, left).is_err() {
                return;
            }
            // `SetCount`'s `UpdatePacket` (`G/item.cpp:296-348`): the new count.
            let update = self
                .characters
                .find_by_vid(vid)
                .ok()
                .and_then(|character| character.items().item_at(arrow_pos))
                .map(world::item::Item::gc_item_update);
            if let Some(update) = update {
                let _sent = self.write_to_client(vid, update.encode());
            }
            return;
        }
        self.restack_arrow(vid, arrow_pos, vnum);
    }

    /// `UseArrow`'s restack arm (`G/char_battle.cpp:3009-3017`): the first stack of the
    /// same vnum in cells `0..180`, equipped into the arrow slot. `EquipItem` swaps the
    /// worn stack out first (`:8512-8524`); the swap needs a free inventory cell, so the
    /// 0-count stack is removed and the client is told the arrow cell holds the new
    /// stack (`GC_ITEM_SET`, byte 21) and the old cell is empty (a cleared `GC_ITEM_SET`).
    fn restack_arrow(&mut self, vid: Vid, arrow_pos: protocol::item_pos::ItemPos, vnum: u32) {
        use common::item_slots::{EWindows, INVENTORY_MAX_NUM};
        use protocol::item_pos::ItemPos;
        let found = (0..INVENTORY_MAX_NUM).find_map(|cell| {
            let pos = ItemPos::new(EWindows::Inventory as u8, cell);
            self.characters
                .find_by_vid(vid)
                .ok()
                .and_then(|character| character.items().item_at(pos))
                .filter(|item| item.vnum == vnum)
                .map(|item| (pos, item.clone()))
        });
        let Some((from, item)) = found else {
            return;
        };
        let probe = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(arrow_pos).cloned());
        let Some(probe) = probe else {
            return;
        };
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let worn = item.clone();
        if character.items_mut().remove(arrow_pos, &probe).is_err() {
            return;
        }
        if character
            .items_mut()
            .move_item(from, arrow_pos, &worn)
            .is_err()
        {
            // The replacement did not fit: put the quirk's stack back and keep it worn.
            let _ = character.items_mut().set(arrow_pos, &probe);
            return;
        }
        let (equipped, from_cell) = (
            self.characters
                .find_by_vid(vid)
                .ok()
                .and_then(|character| character.items().item_at(arrow_pos).cloned()),
            from,
        );
        if let Some(equipped) = equipped {
            let set = equipped.gc_item_set(arrow_pos, 0);
            let _sent = self.write_to_client(vid, set.encode());
            let mut cleared = protocol::gc_item_window::GcItemDel::default_record();
            cleared.cell = from_cell;
            let _sent = self.write_to_client(vid, cleared.encode());
        }
    }
}

fn ms_to_pulses(ms: i32) -> u64 {
    u64::try_from(ms).unwrap_or(0).div_ceil(MS_PER_PULSE)
}
/// `SetSkillLevel`'s ladder (`G/char_skill.cpp:206-213`): 40+ is PERFECT (3),
/// 30+ GRAND (2), 20+ MASTER (1), else NORMAL (0).
#[must_use]
pub(crate) fn master_of(level: u8) -> u8 {
    match level {
        40.. => 3,
        30.. => 2,
        20.. => 1,
        _ => 0,
    }
}
/// The functor's variables for a no-SPLASH cast (`G/char_skill.cpp:1402-1446`,
/// `SetPolyVarForAttack`): `k`, the stat vars, `ar`, `atk`, the victim's maxima,
/// and `chain` 0 (no splash walk raises it). `def`/`odef` are the caster's; no
/// horse is ported, so `horse_level` is 0; `ek` is 0 (no Eunhyung affect is
/// ported). The weapon vars (`wep`/`mtk`/`mwep`) wait: no 230d point poly names
/// them, and an unset name reads 0 either way (see the `Poly` docs).
fn skill_vars(
    points: &Points,
    victim: &Npc,
    k: f64,
    atk: i32,
    ar: f32,
) -> BTreeMap<&'static str, f64> {
    BTreeMap::from([
        ("k", k),
        ("lv", f64::from(points.level())),
        ("iq", f64::from(points.get_point(point::POINT_IQ))),
        ("str", f64::from(points.get_point(point::POINT_ST))),
        ("dex", f64::from(points.get_point(point::POINT_DX))),
        ("con", f64::from(points.get_point(point::POINT_HT))),
        ("ar", f64::from(ar)),
        ("atk", f64::from(atk)),
        ("def", f64::from(points.get_point(point::POINT_DEF_GRADE))),
        (
            "odef",
            f64::from(
                points
                    .get_point(point::POINT_DEF_GRADE)
                    .wrapping_sub(points.get_point(point::POINT_DEF_GRADE_BONUS)),
            ),
        ),
        ("horse_level", 0.0),
        ("maxhp", f64::from(victim.max_hp)),
        // `GetMaxSP` of a monster is 0 (`ComputePoints`' else arm, `G/char.cpp:3016-3018`).
        ("maxsp", 0.0),
        ("chain", 0.0),
        ("ek", 0.0),
    ])
}
/// `ComputeSkill`'s point variables for a SELFONLY buff cast
/// (`G/char_skill.cpp:2538-2581`): `k`, the stat vars, `ar` (caster on caster),
/// `def`/`odef` (the caster's), and the victim maxima — the caster's own, since
/// the victim is the caster. `atk` is the caller's (0 on every 230f row: none
/// sets a `USE_*_DAMAGE` flag); `ek` is the caller's Eunhyung level over 100.
/// `chain` is 0 and the horse is 0 (no horse is ported); the weapon vars
/// (`wep`/`mtk`/`mwep`) wait as in [`skill_vars`] (109 names `mwep`, which
/// reads 0 unset); `maxv` is the *victim's* move-speed limit, set by the
/// caller only for a move-speed point (`:2557-2560`).
fn buff_vars(points: &Points, k: f64, atk: i32, ar: f32, ek: f64) -> BTreeMap<&'static str, f64> {
    BTreeMap::from([
        ("k", k),
        ("lv", f64::from(points.level())),
        ("iq", f64::from(points.get_point(point::POINT_IQ))),
        ("str", f64::from(points.get_point(point::POINT_ST))),
        ("dex", f64::from(points.get_point(point::POINT_DX))),
        ("con", f64::from(points.get_point(point::POINT_HT))),
        ("ar", f64::from(ar)),
        ("atk", f64::from(atk)),
        ("def", f64::from(points.get_point(point::POINT_DEF_GRADE))),
        (
            "odef",
            f64::from(
                points
                    .get_point(point::POINT_DEF_GRADE)
                    .wrapping_sub(points.get_point(point::POINT_DEF_GRADE_BONUS)),
            ),
        ),
        ("horse_level", 0.0),
        ("maxhp", f64::from(points.max_hp())),
        ("maxsp", f64::from(points.max_sp())),
        ("chain", 0.0),
        ("ek", ek),
    ])
}
/// `do_skillup`'s parse: `one_argument` + `str_to_number<DWORD>` — the first
/// argument's leading digits, wrapping mod 2^32 (32-bit `strtoul` saturates
/// at `ULONG_MAX`, then the `DWORD` cast wraps), a non-digit start reading 0.
/// An empty arg is `None`.
pub(super) fn skill_up_vnum(rest: &[u8]) -> Option<u32> {
    let (arg, _) = crate::command::one_argument(rest);
    if arg.is_empty() {
        return None;
    }
    let mut wide = 0_u64;
    for &byte in &arg {
        let Some(digit) = byte.checked_sub(b'0').filter(|digit| *digit < 10) else {
            break;
        };
        wide = wide.wrapping_mul(10).wrapping_add(u64::from(digit));
    }
    u32::try_from(wide.min(u64::from(u32::MAX))).ok()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use common::vid::Vid;
    use gamedata::mob_proto::BATTLE_TYPE_MELEE;
    use gamedata::pc_skill_data::PcSkills;
    use protocol::gc_chat::GcChat;
    use tokio::sync::mpsc::UnboundedReceiver;
    use world::npc::FIRST_NPC_VID;

    use super::super::fixtures::{
        a_fighter, enter, enter_seeing_with, open_cells, stand_field, TestClock, PLACE,
    };
    use super::*;
    use crate::game_loop::PulseProcessor;

    /// The monster every test casts on.
    const ME: u32 = FIRST_NPC_VID;
    /// Where it stands.
    const SPOT: (i32, i32) = (3200, 3200);
    /// 100 west: inside every 230d target range arm (`+ 50`).
    const NEAR: (i32, i32) = (3100, 3200);
    /// When the tests' first Pulse runs, in milliseconds.
    const T0: u32 = 1_000;
    /// The tests' first Pulse, which every due counts from.
    const P0: u64 = 1;

    fn dumps() -> Arc<PcSkills> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/sql/gamedata");
        let player = std::fs::read(root.join("player.sql")).unwrap();
        let common = std::fs::read(root.join("common.sql")).unwrap();
        Arc::new(PcSkills::from_dumps(&player, &common).unwrap())
    }

    /// A dog at [`SPOT`] and the player 1 at [`NEAR`], after Pulse [`P0`] at [`T0`];
    /// the inbox is the player's, drained. The caster is a group-1 warrior, so vnum 1
    /// passes `CanUseSkill`.
    fn a_cast() -> (TestClock, GameState, UnboundedReceiver<Vec<u8>>) {
        let clock = TestClock::default();
        clock.set(T0);
        let npc = a_fighter(ME, 101, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000);
        let mut world = stand_field(&clock, vec![npc], open_cells(), true).with_pc_skills(dumps());
        let mut inbox = enter(&mut world, 1, NEAR, 12_345);
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.skill_group = 1;
        world.process_pulse(P0);
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        (clock, world, inbox)
    }

    fn sp_of(world: &GameState, vid: u32) -> i32 {
        world
            .characters
            .find_by_vid(Vid::new(vid))
            .unwrap()
            .points()
            .unwrap()
            .sp()
    }

    /// `CG_USE_SKILL` charges the SP (`40+100*k`, level-1 `k = 5/100`) and stamps the
    /// cooldown (`12 s → 300 Pulses`); a second cast inside the window is refused
    /// (the SP the second cast would charge stays).
    #[test]
    fn a_cast_charges_the_sp_and_stamps_the_cooldown() {
        let (_clock, mut world, mut inbox) = a_cast();
        world.set_pc_skill_level(1, 1, 1);
        let before = sp_of(&world, 1);
        assert!(world.use_skill(Vid::new(1), 1, ME));
        assert_eq!(sp_of(&world, 1), before - 45);
        let stamp = world.pc_skills.get(&1).unwrap().uses.get(&1).unwrap();
        assert!(stamp.used);
        assert_eq!(stamp.next_usable, Some(P0 + 300));
        assert_eq!(stamp.hit_count, 5);
        // The SP point change went out.
        assert!(!std::iter::from_fn(|| inbox.try_recv().ok())
            .collect::<Vec<_>>()
            .is_empty());
        // A second cast inside the 12 s window refuses — but the SP was already
        // charged before the stamp (`G/char_skill.cpp:3066-3112` charges, then
        // `UseSkill` stamps).
        assert!(!world.use_skill(Vid::new(1), 1, ME));
        assert_eq!(sp_of(&world, 1), before - 90);
    }

    /// The combo toggle chats and stamps no cooldown: level-1 122 flips the
    /// index to 1 with `combo 1`, a second cast flips it back with `combo 0`,
    /// and no SP is charged and no stamp is held.
    #[test]
    fn the_combo_toggle_chats_and_stamps_nothing() {
        let (_clock, mut world, mut inbox) = a_cast();
        world.set_pc_skill_level(1, SKILL_COMBO, 1);
        let before = sp_of(&world, 1);
        assert!(world.use_skill(Vid::new(1), SKILL_COMBO, 0));
        assert_eq!(world.pc_skills.get(&1).unwrap().combo_index, 1);
        assert_eq!(sp_of(&world, 1), before);
        assert!(!world
            .pc_skills
            .get(&1)
            .unwrap()
            .uses
            .contains_key(&SKILL_COMBO));
        let frames: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        assert_eq!(texts(&frames), [b"combo 1".to_vec()]);
        assert!(world.use_skill(Vid::new(1), SKILL_COMBO, 0));
        assert_eq!(world.pc_skills.get(&1).unwrap().combo_index, 0);
        assert_eq!(sp_of(&world, 1), before);
        let frames: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        assert_eq!(texts(&frames), [b"combo 0".to_vec()]);
    }

    /// A level-0 combo still toggles: the arm sits before the level gate, so the
    /// cast chats `combo 0` and answers true.
    #[test]
    fn a_level_zero_combo_still_chats() {
        let (_clock, mut world, mut inbox) = a_cast();
        assert!(world.use_skill(Vid::new(1), SKILL_COMBO, 0));
        assert_eq!(world.pc_skills.get(&1).unwrap().combo_index, 0);
        let frames: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        assert_eq!(texts(&frames), [b"combo 0".to_vec()]);
    }

    /// The charge's first cast dashes: level-1 5 on the caster charges the SP,
    /// stamps the cooldown, holds the DASH (kind 5, `MOV_SPEED +150`, flag 32),
    /// and neither changes the caster's HP nor touches the dog.
    #[test]
    fn a_first_charge_cast_dashes() {
        let (_clock, mut world, mut inbox) = a_cast();
        world.set_pc_skill_level(1, SKILL_TANHWAN, 1);
        let before = sp_of(&world, 1);
        let hp_before = world
            .characters
            .find_by_vid(Vid::new(1))
            .unwrap()
            .points()
            .unwrap()
            .hp();
        let dog_before = world.npc_by_vid(ME).unwrap().hp;
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        assert!(world.use_skill(Vid::new(1), SKILL_TANHWAN, Vid::new(1).raw()));
        // `60+120*k`, level-1 `k = 5/100`.
        assert_eq!(sp_of(&world, 1), before - 66);
        let stamp = world
            .pc_skills
            .get(&1)
            .unwrap()
            .uses
            .get(&SKILL_TANHWAN)
            .unwrap();
        assert!(stamp.used);
        assert!(world.has_tanhwan_dash(Vid::new(1)));
        let dashed = world
            .affect_events
            .get(&Vid::new(1))
            .unwrap()
            .affects
            .iter()
            .find(|affect| affect.kind == SKILL_TANHWAN)
            .expect("the dash");
        assert_eq!(dashed.flag, AFF_TANHWAN_DASH);
        assert_eq!(
            dashed.apply_on,
            u8::try_from(point::POINT_MOV_SPEED).unwrap()
        );
        assert_eq!(dashed.value, 150);
        let hp_after = world
            .characters
            .find_by_vid(Vid::new(1))
            .unwrap()
            .points()
            .unwrap()
            .hp();
        assert_eq!(hp_after, hp_before);
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, dog_before);
    }

    /// The charge's second cast strikes: with the DASH held, casting 5 on the
    /// dog computes eagerly (the dog loses HP), clears the DASH, charges no
    /// further SP, and stamps no skill time (the term gate still reads the
    /// first cast's stamp).
    #[test]
    fn a_second_charge_cast_strikes_and_clears_the_dash() {
        let (_clock, mut world, mut inbox) = a_cast();
        world.set_pc_skill_level(1, SKILL_TANHWAN, 1);
        assert!(world.use_skill(Vid::new(1), SKILL_TANHWAN, Vid::new(1).raw()));
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        let sp_dashed = sp_of(&world, 1);
        let dog_before = world.npc_by_vid(ME).unwrap().hp;
        let last_skill = world.pc_skills.get(&1).unwrap().last_skill;
        assert!(world.use_skill(Vid::new(1), SKILL_TANHWAN, ME));
        assert!(world.npc_by_vid(ME).unwrap().hp < dog_before);
        assert!(!world.has_tanhwan_dash(Vid::new(1)));
        assert_eq!(sp_of(&world, 1), sp_dashed);
        assert_eq!(world.pc_skills.get(&1).unwrap().last_skill, last_skill);
        let main = world
            .pc_skills
            .get(&1)
            .unwrap()
            .uses
            .get(&SKILL_TANHWAN)
            .unwrap()
            .main_vid;
        assert_eq!(main, ME);
    }

    /// A second cast without the DASH recurses: the self-cast dashes (one SP
    /// charge), then the eager compute strikes and the DASH is gone at the end.
    #[test]
    fn a_dashless_second_cast_recurses_then_strikes() {
        let (_clock, mut world, mut inbox) = a_cast();
        world.set_pc_skill_level(1, SKILL_TANHWAN, 1);
        let before = sp_of(&world, 1);
        let dog_before = world.npc_by_vid(ME).unwrap().hp;
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        assert!(world.use_skill(Vid::new(1), SKILL_TANHWAN, ME));
        assert!(world.npc_by_vid(ME).unwrap().hp < dog_before);
        assert!(!world.has_tanhwan_dash(Vid::new(1)));
        assert_eq!(sp_of(&world, 1), before - 66);
    }

    /// An unlearned skill refuses: level 0 never reaches the SP charge.
    #[test]
    fn an_unlearned_skill_refuses() {
        let (_clock, mut world, mut inbox) = a_cast();
        let before = sp_of(&world, 1);
        assert!(!world.use_skill(Vid::new(1), 1, ME));
        assert_eq!(sp_of(&world, 1), before);
        assert!(inbox.try_recv().is_err());
    }

    /// A `CG_ATTACK` of a shoot-listed vnum is refused even after a cast stamps it:
    /// vnum 20 (`GEOMPUNG`) needs `CG_SHOOT`.
    #[test]
    fn a_shoot_listed_vnum_refuses_through_cg_attack() {
        let (_clock, mut world, mut inbox) = a_cast();
        // Vnum 20 sits in the warrior's group-2 list.
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.skill_group = 2;
        world.set_pc_skill_level(1, 20, 1);
        assert!(world.use_skill(Vid::new(1), 20, ME));
        let hp = world.npc_by_vid(ME).unwrap().hp;
        assert!(!world.skill_attack(Vid::new(1), 20, ME));
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, hp);
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        assert!(inbox.try_recv().is_err());
    }

    /// A vnum-1 cast lands MELEE damage on the dog: the hit points fall, the damage
    /// map holds a MELEE row, gold is not stolen (NORMAL-only), and the dog begins
    /// the fight back.
    #[test]
    fn a_skill_hit_lands_melee_damage_and_the_dog_fights_back() {
        let (_clock, mut world, _inbox) = a_cast();
        world.set_pc_skill_level(1, 1, 1);
        world.dice = world::character::Pcg32::new(7, 0);
        let before = world.npc_by_vid(ME).unwrap().hp;
        assert!(world.use_skill(Vid::new(1), 1, ME));
        assert!(world.skill_attack(Vid::new(1), 1, ME));
        let after = world.npc_by_vid(ME).unwrap().hp;
        assert!(after < before, "the dog took damage: {before} -> {after}");
        let row = world.npc_combat.get(&ME).unwrap().damage.get(&1).unwrap();
        assert_eq!(row.total, before - after);
        assert!(row.aggro > 0);
        assert!(world.minds.get(ME).is_some());
    }

    /// Past 37 Pulses after the cast, the skill-term gate refuses the `CG_ATTACK`:
    /// the dog is whole.
    #[test]
    fn a_late_cg_attack_hits_the_skill_term_gate() {
        let (_clock, mut world, _inbox) = a_cast();
        world.set_pc_skill_level(1, 1, 1);
        assert!(world.use_skill(Vid::new(1), 1, ME));
        for pulse in P0 + 1..=P0 + 38 {
            world.process_pulse(pulse);
        }
        let hp = world.npc_by_vid(ME).unwrap().hp;
        assert!(!world.skill_attack(Vid::new(1), 1, ME));
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, hp);
    }
    /// `CG_FLY_TARGETING` stores the main VID and relays it to everyone around but
    /// the shooter; `CG_ADD_FLY_TARGETING` appends. The shooter's own inbox stays
    /// silent. An unknown VID relays target 0 and stores nothing.
    #[test]
    fn fly_targets_store_and_relay_skipping_the_shooter() {
        let (_clock, mut world, mut inbox) = a_cast();
        let mut beta = enter(&mut world, 2, NEAR, 12_345);
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        world.fly_target(Vid::new(1), true, ME, 0, 0);
        assert_eq!(world.pc_skills.get(&1).unwrap().fly_main, ME);
        let relay = beta.try_recv().expect("beta hears the relay");
        assert_eq!(relay[0], protocol::gc::HEADER_GC_FLY_TARGETING);
        assert_eq!(&relay[1..5], &1u32.to_le_bytes());
        assert_eq!(&relay[5..9], &ME.to_le_bytes());
        assert_eq!(&relay[9..13], &SPOT.0.to_le_bytes());
        assert!(inbox.try_recv().is_err(), "the shooter hears nothing");
        world.fly_target(Vid::new(1), false, ME, 0, 0);
        assert_eq!(world.pc_skills.get(&1).unwrap().fly_extra, vec![ME]);
        let add = beta.try_recv().expect("beta hears the add");
        assert_eq!(add[0], protocol::gc::HEADER_GC_ADD_FLY_TARGETING);
        world.fly_target(Vid::new(1), true, 999, 100, 200);
        assert_eq!(world.pc_skills.get(&1).unwrap().fly_main, ME);
        let unknown = beta.try_recv().expect("beta hears the unknown");
        assert_eq!(&unknown[5..9], &0u32.to_le_bytes());
        assert_eq!(&unknown[9..13], &100i32.to_le_bytes());
    }

    /// A plain bow shot (assassin, bow + arrow, group 0 so no SP charge) lands
    /// `NORMAL_RANGE` damage on the dog, consumes one arrow, and answers no gold.
    /// A warrior, and an assassin with no bow, refuse with the dog whole.
    #[test]
    fn the_plain_bow_shot_lands_and_the_refusals_hold() {
        use common::item_slots::{EWindows, INVENTORY_MAX_NUM};
        use gamedata::item_kind::WEAPON_ARROW;
        use protocol::item_pos::ItemPos;
        use world::item::Item;

        let clock = TestClock::default();
        clock.set(T0);
        let npc = a_fighter(ME, 101, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000);
        let mut world = stand_field(&clock, vec![npc], open_cells(), true).with_pc_skills(dumps());
        let bow_vnum = world
            .protos
            .rows()
            .iter()
            .find(|proto| {
                proto.item_type == gamedata::item_kind::ITEM_WEAPON
                    && proto.sub_type == gamedata::item_kind::WEAPON_BOW
            })
            .unwrap()
            .vnum;
        let arrow_vnum = world
            .protos
            .rows()
            .iter()
            .find(|proto| {
                proto.item_type == gamedata::item_kind::ITEM_WEAPON
                    && proto.sub_type == WEAPON_ARROW
            })
            .unwrap()
            .vnum;
        let mut bow = Item::new(11, bow_vnum);
        bow.set_size(1).unwrap();
        let mut arrow = Item::new(12, arrow_vnum);
        arrow.set_size(1).unwrap();
        arrow.count = 10;
        let weapon_cell = INVENTORY_MAX_NUM + EWearPositions::Weapon as u16;
        let arrow_cell = INVENTORY_MAX_NUM + EWearPositions::Arrow as u16;
        let items = [
            (ItemPos::new(EWindows::Inventory as u8, weapon_cell), bow),
            (ItemPos::new(EWindows::Inventory as u8, arrow_cell), arrow),
        ];
        let (mut inbox, _shown) = enter_seeing_with(&mut world, PLACE, 1, NEAR, 12_345, &items);
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.job = JOB_ASSASSIN;
        world.process_pulse(P0);
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        world.dice = world::character::Pcg32::new(7, 0);
        world.fly_target(Vid::new(1), true, ME, 0, 0);
        let before = world.npc_by_vid(ME).unwrap().hp;
        let answer = world.shoot(Vid::new(1), 0);
        let after = world.npc_by_vid(ME).unwrap().hp;
        assert!(
            after < before,
            "the dog took the arrow: {before} -> {after}"
        );
        assert_eq!(answer.gold, None);
        let worn = world
            .characters
            .find_by_vid(Vid::new(1))
            .unwrap()
            .items()
            .item_at(ItemPos::new(EWindows::Inventory as u8, arrow_cell))
            .unwrap();
        assert_eq!(worn.count, 9);
        assert_eq!(world.pc_skills.get(&1).unwrap().fly_main, 0);
        // A warrior refuses with the same bow.
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.job = 0;
        world.fly_target(Vid::new(1), true, ME, 0, 0);
        let hp = world.npc_by_vid(ME).unwrap().hp;
        world.shoot(Vid::new(1), 0);
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, hp);
        // An assassin with no bow refuses too.
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.job = JOB_ASSASSIN;
        let held = world
            .characters
            .find_by_vid(Vid::new(1))
            .unwrap()
            .items()
            .item_at(ItemPos::new(EWindows::Inventory as u8, weapon_cell))
            .unwrap()
            .clone();
        world
            .characters
            .find_by_vid_mut(Vid::new(1))
            .unwrap()
            .items_mut()
            .remove(ItemPos::new(EWindows::Inventory as u8, weapon_cell), &held)
            .unwrap();
        world.fly_target(Vid::new(1), true, ME, 0, 0);
        world.shoot(Vid::new(1), 0);
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, hp);
    }
    /// Vnum 47 (`KWANKYEOK`) needs its arrow: with bow + arrow the `CG_SHOOT` lands
    /// RANGE damage through the shoot arm; without the arrow it refuses. Vnum 20
    /// (`GEOMPUNG`) needs none: its shoot arm lands MELEE-pipe damage unarmed.
    #[test]
    fn an_arrow_skill_needs_its_arrow_a_melee_skill_needs_none() {
        use common::item_slots::{EWindows, INVENTORY_MAX_NUM};
        use gamedata::item_kind::WEAPON_ARROW;
        use protocol::item_pos::ItemPos;
        use world::item::Item;

        let clock = TestClock::default();
        clock.set(T0);
        let npc = a_fighter(ME, 101, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000);
        let mut world = stand_field(&clock, vec![npc], open_cells(), true).with_pc_skills(dumps());
        let bow_vnum = world
            .protos
            .rows()
            .iter()
            .find(|proto| {
                proto.item_type == gamedata::item_kind::ITEM_WEAPON
                    && proto.sub_type == gamedata::item_kind::WEAPON_BOW
            })
            .unwrap()
            .vnum;
        let arrow_vnum = world
            .protos
            .rows()
            .iter()
            .find(|proto| {
                proto.item_type == gamedata::item_kind::ITEM_WEAPON
                    && proto.sub_type == WEAPON_ARROW
            })
            .unwrap()
            .vnum;
        let mut bow = Item::new(11, bow_vnum);
        bow.set_size(1).unwrap();
        let mut arrow = Item::new(12, arrow_vnum);
        arrow.set_size(1).unwrap();
        arrow.count = 10;
        let weapon_cell = INVENTORY_MAX_NUM + EWearPositions::Weapon as u16;
        let arrow_cell = INVENTORY_MAX_NUM + EWearPositions::Arrow as u16;
        let items = [
            (ItemPos::new(EWindows::Inventory as u8, weapon_cell), bow),
            (ItemPos::new(EWindows::Inventory as u8, arrow_cell), arrow),
        ];
        let (mut inbox, _shown) = enter_seeing_with(&mut world, PLACE, 1, NEAR, 12_345, &items);
        // Assassin group 2: vnum 47 sits in its list.
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.job = JOB_ASSASSIN;
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.skill_group = 2;
        world.process_pulse(P0);
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        world.set_pc_skill_level(1, 47, 1);
        world.dice = world::character::Pcg32::new(7, 0);
        assert!(world.use_skill(Vid::new(1), 47, ME));
        world.fly_target(Vid::new(1), true, ME, 0, 0);
        let before = world.npc_by_vid(ME).unwrap().hp;
        world.shoot(Vid::new(1), 47);
        let after = world.npc_by_vid(ME).unwrap().hp;
        assert!(
            after < before,
            "47 lands with its arrow: {before} -> {after}"
        );
        let worn = world
            .characters
            .find_by_vid(Vid::new(1))
            .unwrap()
            .items()
            .item_at(ItemPos::new(EWindows::Inventory as u8, arrow_cell))
            .unwrap();
        assert_eq!(worn.count, 9);
        let row = world.npc_combat.get(&ME).unwrap().damage.get(&1).unwrap();
        assert!(row.aggro > 0);
        // Without the arrow the skill refuses (the cooldown is spent, so re-stamp).
        let held = world
            .characters
            .find_by_vid(Vid::new(1))
            .unwrap()
            .items()
            .item_at(ItemPos::new(EWindows::Inventory as u8, arrow_cell))
            .unwrap()
            .clone();
        world
            .characters
            .find_by_vid_mut(Vid::new(1))
            .unwrap()
            .items_mut()
            .remove(ItemPos::new(EWindows::Inventory as u8, arrow_cell), &held)
            .unwrap();
        for pulse in P0 + 1..=P0 + 300 {
            world.process_pulse(pulse);
        }
        assert!(world.use_skill(Vid::new(1), 47, ME));
        world.fly_target(Vid::new(1), true, ME, 0, 0);
        let hp = world.npc_by_vid(ME).unwrap().hp;
        world.shoot(Vid::new(1), 47);
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, hp);
    }

    /// Vnum 20 (`GEOMPUNG`) is a SPLASH MELEE row: one `CG_SHOOT` hits the centred
    /// dog and a neighbour 100 east (splash 200), and skips a third 500 east.
    #[test]
    fn a_splash_cast_hits_two_dogs_and_skips_a_far_third() {
        let clock = TestClock::default();
        clock.set(T0);
        let centre = a_fighter(ME, 101, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000);
        let near = a_fighter(
            ME + 1,
            101,
            BATTLE_TYPE_MELEE,
            (175, 2000),
            0,
            (SPOT.0 + 100, SPOT.1),
            10_000,
        );
        let far = a_fighter(
            ME + 2,
            101,
            BATTLE_TYPE_MELEE,
            (175, 2000),
            0,
            (SPOT.0 + 500, SPOT.1),
            10_000,
        );
        let mut world = stand_field(&clock, vec![centre, near, far], open_cells(), true)
            .with_pc_skills(dumps());
        let mut inbox = enter(&mut world, 1, NEAR, 12_345);
        // Warrior group 2: vnum 20 sits in its list.
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.skill_group = 2;
        world.process_pulse(P0);
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        world.set_pc_skill_level(1, 20, 1);
        world.dice = world::character::Pcg32::new(7, 0);
        assert!(world.use_skill(Vid::new(1), 20, ME));
        world.fly_target(Vid::new(1), true, ME, 0, 0);
        world.shoot(Vid::new(1), 20);
        assert!(
            world.npc_by_vid(ME).unwrap().hp < 10_000,
            "the centred dog took the splash"
        );
        assert!(
            world.npc_by_vid(ME + 1).unwrap().hp < 10_000,
            "the near dog took the splash"
        );
        assert_eq!(
            world.npc_by_vid(ME + 2).unwrap().hp,
            10_000,
            "the far dog stands whole"
        );
    }

    /// Vnum 62 (`YONGKWON`) is SELFONLY + SPLASH: the walk centres on the caster, so
    /// a dog 100 east of the player (300 west of the named victim) takes the hit
    /// while a dog past the 500 rim stands whole.
    #[test]
    fn a_selfonly_walk_centres_on_the_caster() {
        let clock = TestClock::default();
        clock.set(T0);
        // The named victim, 100 east of the caster's spot; the walk ignores it.
        let named = a_fighter(ME, 101, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000);
        // 100 east of the caster (NEAR): inside the 500 rim around the caster.
        let close = a_fighter(
            ME + 1,
            101,
            BATTLE_TYPE_MELEE,
            (175, 2000),
            0,
            (NEAR.0 + 100, NEAR.1),
            10_000,
        );
        // 600 east of the caster: past the rim.
        let far = a_fighter(
            ME + 2,
            101,
            BATTLE_TYPE_MELEE,
            (175, 2000),
            0,
            (NEAR.0 + 600, NEAR.1),
            10_000,
        );
        let mut world = stand_field(&clock, vec![named, close, far], open_cells(), true)
            .with_pc_skills(dumps());
        let mut inbox = enter(&mut world, 1, NEAR, 12_345);
        // Surah group 1: vnum 62 sits in its list.
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.job = 2;
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.skill_group = 1;
        world.process_pulse(P0);
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        world.set_pc_skill_level(1, 62, 1);
        world.dice = world::character::Pcg32::new(7, 0);
        assert!(world.use_skill(Vid::new(1), 62, ME));
        // Vnum 62 refuses `CG_ATTACK` (`CheckSkillHitCount`), so the walk runs through
        // `CG_SHOOT`'s `compute_skill` (the dispatch arm is the chain shape — vnum 62
        // has no shoot arm, so this drives `compute_skill` directly).
        world.compute_skill(Vid::new(1), 62, ME);
        assert!(
            world.npc_by_vid(ME + 1).unwrap().hp < 10_000,
            "the dog by the caster took the splash"
        );
        assert_eq!(
            world.npc_by_vid(ME + 2).unwrap().hp,
            10_000,
            "the far dog stands whole"
        );
    }

    /// Vnum 20 hosts CRUSH: the centred dog slides 200 east (away from the caster
    /// standing 100 west).
    #[test]
    fn crush_slides_the_victim() {
        let clock = TestClock::default();
        clock.set(T0);
        let centre = a_fighter(ME, 101, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000);
        let mut world =
            stand_field(&clock, vec![centre], open_cells(), true).with_pc_skills(dumps());
        let mut inbox = enter(&mut world, 1, NEAR, 12_345);
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.skill_group = 2;
        world.process_pulse(P0);
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        world.set_pc_skill_level(1, 20, 1);
        world.dice = world::character::Pcg32::new(7, 0);
        assert!(world.use_skill(Vid::new(1), 20, ME));
        world.fly_target(Vid::new(1), true, ME, 0, 0);
        world.shoot(Vid::new(1), 20);
        let slid = world.npc_by_vid(ME).unwrap();
        assert!(
            slid.x > SPOT.0,
            "the dog slid east: {} -> {}",
            SPOT.0,
            slid.x
        );
        assert_eq!(slid.y, SPOT.1);
    }

    /// Vnum 92 (`YONGBI`) runs `OnMove(true)` only: the shoot stamps the caster's
    /// move time and the dog stands whole.
    #[test]
    fn yongbi_stamps_the_move_and_hits_nothing() {
        let (_clock, mut world, _inbox) = a_cast();
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.job = 3;
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.skill_group = 1;
        world.set_pc_skill_level(1, 92, 1);
        assert!(world.use_skill(Vid::new(1), 92, ME));
        world.fly_target(Vid::new(1), true, ME, 0, 0);
        let hp = world.npc_by_vid(ME).unwrap().hp;
        world.shoot(Vid::new(1), 92);
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, hp);
        assert!(world
            .bodies
            .get(&Vid::new(1))
            .unwrap()
            .last_attack
            .is_some());
    }

    /// Vnum 92 (`SEONGSU`) is a MAGIC-attr no-SPLASH row: one `CG_ATTACK` lands
    /// through the MAGIC pipe (the `RESIST_MAGIC` `#else` arm), with MAGIC aggro.
    #[test]
    fn a_magic_cast_lands_through_the_magic_pipe() {
        let (_clock, mut world, _inbox) = a_cast();
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.job = 3;
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.skill_group = 1;
        world.set_pc_skill_level(1, 92, 1);
        world.dice = world::character::Pcg32::new(7, 0);
        assert!(world.use_skill(Vid::new(1), 92, ME));
        let before = world.npc_by_vid(ME).unwrap().hp;
        assert!(world.skill_attack(Vid::new(1), 92, ME));
        let after = world.npc_by_vid(ME).unwrap().hp;
        assert!(after < before, "92 lands MAGIC: {before} -> {after}");
        let row = world.npc_combat.get(&ME).unwrap().damage.get(&1).unwrap();
        assert!(row.aggro > 0);
    }
    /// `/skillup 1` at group 1 with a skill point climbs to 1, spends the
    /// point, and sends byte 76 with slot 1 learned; a second up climbs to 2.
    #[test]
    fn a_skill_up_climbs_spends_and_sends_byte_76() {
        use protocol::gc::HEADER_GC_SKILL_LEVEL_NEW;
        let (_clock, mut world, mut inbox) = a_cast();
        world.change_pc_point(Vid::new(1), point::POINT_SKILL, 2, false);
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        assert_eq!(world.skill_up(Vid::new(1), b"1"), Some((1, 1)));
        assert_eq!(world.pc_skill_level(1, 1), 1);
        let spent = world
            .characters
            .find_by_vid(Vid::new(1))
            .unwrap()
            .points()
            .unwrap()
            .get_point(point::POINT_SKILL);
        assert_eq!(spent, 1);
        let frames: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        assert!(frames
            .iter()
            .any(|frame| frame[0] == HEADER_GC_SKILL_LEVEL_NEW));
        let burst = frames
            .iter()
            .find(|frame| frame[0] == HEADER_GC_SKILL_LEVEL_NEW)
            .unwrap();
        // Slot 1: master 0, level 1 (6 bytes a slot, 1 header byte).
        assert_eq!(burst[1 + 6], 0);
        assert_eq!(burst[1 + 7], 1);
        assert_eq!(world.skill_up(Vid::new(1), b"1"), Some((1, 2)));
        assert_eq!(world.pc_skill_level(1, 1), 2);
    }

    /// The refusals hold: group 0, no point, a wrong-job vnum, an empty arg,
    /// and a non-digit arg all refuse silently with nothing spent and nothing
    /// sent.
    #[test]
    fn a_skill_up_refuses_silently() {
        let (_clock, mut world, mut inbox) = a_cast();
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.skill_group = 0;
        world.change_pc_point(Vid::new(1), point::POINT_SKILL, 1, false);
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        assert_eq!(world.skill_up(Vid::new(1), b"1"), None);
        assert_eq!(world.pc_skill_level(1, 1), 0);
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.skill_group = 1;
        // No point left: the grant above was never spent, so take it back.
        world.change_pc_point(Vid::new(1), point::POINT_SKILL, -1, false);
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        assert_eq!(world.skill_up(Vid::new(1), b"1"), None);
        // Wrong job: 31 is the assassin's.
        world.change_pc_point(Vid::new(1), point::POINT_SKILL, 1, false);
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        assert_eq!(world.skill_up(Vid::new(1), b"31"), None);
        assert_eq!(world.skill_up(Vid::new(1), b""), None);
        assert_eq!(world.skill_up(Vid::new(1), b"abc"), None);
        let left = world
            .characters
            .find_by_vid(Vid::new(1))
            .unwrap()
            .points()
            .unwrap()
            .get_point(point::POINT_SKILL);
        assert_eq!(left, 1);
        assert!(inbox.try_recv().is_err());
    }

    /// `/skillup 3x` ups 3 (`strtoul` stops at `x`); `/skillup 999999999999`
    /// wraps mod 2^32 and refuses as an unknown row.
    #[test]
    fn a_skill_up_parses_like_strtoul() {
        let (_clock, mut world, mut inbox) = a_cast();
        world.change_pc_point(Vid::new(1), point::POINT_SKILL, 2, false);
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        assert_eq!(world.skill_up(Vid::new(1), b"3x"), Some((3, 1)));
        assert_eq!(world.skill_up(Vid::new(1), b"999999999999"), None);
        // Only the first up spent.
        let left = world
            .characters
            .find_by_vid(Vid::new(1))
            .unwrap()
            .points()
            .unwrap()
            .get_point(point::POINT_SKILL);
        assert_eq!(left, 1);
    }

    /// `master_of` climbs the ladder: 0-19 NORMAL, 20-29 MASTER, 30-39 GRAND,
    /// 40 PERFECT.
    #[test]
    fn the_master_ladder_has_four_rungs() {
        assert_eq!(master_of(0), SKILL_NORMAL);
        assert_eq!(master_of(19), SKILL_NORMAL);
        assert_eq!(master_of(20), SKILL_MASTER);
        assert_eq!(master_of(29), SKILL_MASTER);
        assert_eq!(master_of(30), SKILL_GRAND_MASTER);
        assert_eq!(master_of(39), SKILL_GRAND_MASTER);
        assert_eq!(master_of(40), SKILL_PERFECT_MASTER);
    }
    /// Casting 3 lands the buff: `ATT_SPEED` up 2 (`50*0.05`), the JEONGWIHON
    /// flag (bit 14) set, a second affect for the move-speed point; casting
    /// 3 again toggles off with no SP charged and the flag cleared.
    #[test]
    fn a_buff_lands_and_toggles_off_free() {
        let (_clock, mut world, mut inbox) = a_cast();
        world.set_pc_skill_level(1, 3, 1);
        let att_before = world
            .characters
            .find_by_vid(Vid::new(1))
            .unwrap()
            .points()
            .unwrap()
            .get_point(point::POINT_ATT_SPEED);
        let sp_before = sp_of(&world, 1);
        assert!(world.use_skill(Vid::new(1), 3, ME));
        let att_after = world
            .characters
            .find_by_vid(Vid::new(1))
            .unwrap()
            .points()
            .unwrap()
            .get_point(point::POINT_ATT_SPEED);
        assert_eq!(att_after, att_before + 2);
        assert_eq!(world.affect_flags_of(Vid::new(1))[0] & (1 << 14), 1 << 14);
        assert_eq!(
            world.affect_events.get(&Vid::new(1)).unwrap().affects.len(),
            2
        );
        assert!(sp_of(&world, 1) < sp_before);
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        let sp_charged = sp_of(&world, 1);
        assert!(world.use_skill(Vid::new(1), 3, ME));
        assert_eq!(sp_of(&world, 1), sp_charged);
        assert_eq!(world.affect_flags_of(Vid::new(1))[0] & (1 << 14), 0);
        let att_off = world
            .characters
            .find_by_vid(Vid::new(1))
            .unwrap()
            .points()
            .unwrap()
            .get_point(point::POINT_ATT_SPEED);
        assert_eq!(att_off, att_before);
    }

    /// Casting 4 lands the exact amount: `(100+6+10*3)*0.05 = 6` `ATT_GRADE`
    /// with the `GEOMGYEONG` flag (bit 15) and one affect row.
    #[test]
    fn a_buff_lands_its_poly_amount() {
        let (_clock, mut world, _inbox) = a_cast();
        world.set_pc_skill_level(1, 4, 1);
        let att_before = world
            .characters
            .find_by_vid(Vid::new(1))
            .unwrap()
            .points()
            .unwrap()
            .get_point(point::POINT_ATT_GRADE);
        assert!(world.use_skill(Vid::new(1), 4, ME));
        let att_after = world
            .characters
            .find_by_vid(Vid::new(1))
            .unwrap()
            .points()
            .unwrap()
            .get_point(point::POINT_ATT_GRADE);
        assert_eq!(att_after, att_before + 6);
        assert_eq!(world.affect_flags_of(Vid::new(1))[0] & (1 << 15), 1 << 15);
        assert_eq!(
            world.affect_events.get(&Vid::new(1)).unwrap().affects.len(),
            1
        );
    }

    /// The cooling asymmetry: 64's re-cast refuses before the charge (SP
    /// untouched); 62's re-cast refuses after it (SP charged again).
    #[test]
    fn terror_refuses_before_the_charge_others_after() {
        let (_clock, mut world, _inbox) = a_cast();
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.job = 2;
        world.set_pc_skill_level(1, 64, 1);
        assert!(world.use_skill(Vid::new(1), 64, ME));
        let sp_charged = sp_of(&world, 1);
        assert!(!world.use_skill(Vid::new(1), 64, ME));
        assert_eq!(sp_of(&world, 1), sp_charged);

        let (_clock, mut world, _inbox) = a_cast();
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.job = 2;
        world.set_pc_skill_level(1, 62, 1);
        assert!(world.use_skill(Vid::new(1), 62, ME));
        let sp_charged = sp_of(&world, 1);
        assert!(!world.use_skill(Vid::new(1), 62, ME));
        assert!(sp_of(&world, 1) < sp_charged);
    }

    /// 109 heals its target: a wounded player gains `200+7*10+(30*3+600)*k`
    /// with TYPE7 `k = 5/100`, i.e. 304, capped at its maximum.
    #[test]
    fn the_heal_lands_on_its_player_target() {
        let (_clock, mut world, _inbox) = a_cast();
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.job = 3;
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.skill_group = 2;
        world.set_pc_skill_level(1, 109, 1);
        let max_hp = world
            .characters
            .find_by_vid(Vid::new(1))
            .unwrap()
            .points()
            .unwrap()
            .max_hp();
        assert!(max_hp > 304, "fixture maximum holds the heal: {max_hp}");
        world.change_pc_point(Vid::new(1), point::POINT_HP, -500, false);
        let before = world
            .characters
            .find_by_vid(Vid::new(1))
            .unwrap()
            .points()
            .unwrap()
            .hp();
        // 109 is not SELFONLY: its victim is the target, so a player target takes
        // the heal (a monster victim waits — see `compute_skill_on`).
        assert!(world.use_skill(Vid::new(1), 109, 1));
        let after = world
            .characters
            .find_by_vid(Vid::new(1))
            .unwrap()
            .points()
            .unwrap()
            .hp();
        assert_eq!(after - before, 304);
    }
    /// 109 heals a second player, not the caster: the wounded Beta gains the
    /// 304, and Alpha's own pool stays (the victim-maxima come from Beta).
    #[test]
    fn the_heal_lands_on_a_second_player() {
        let (_clock, mut world, _inbox) = a_cast();
        let _beta = enter(&mut world, 2, NEAR, 12_345);
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.job = 3;
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.skill_group = 2;
        world.set_pc_skill_level(1, 109, 1);
        world.change_pc_point(Vid::new(2), point::POINT_HP, -500, false);
        let beta_before = world
            .characters
            .find_by_vid(Vid::new(2))
            .unwrap()
            .points()
            .unwrap()
            .hp();
        let alpha_before = world
            .characters
            .find_by_vid(Vid::new(1))
            .unwrap()
            .points()
            .unwrap()
            .hp();
        assert!(world.use_skill(Vid::new(1), 109, 2));
        let beta_after = world
            .characters
            .find_by_vid(Vid::new(2))
            .unwrap()
            .points()
            .unwrap()
            .hp();
        let alpha_after = world
            .characters
            .find_by_vid(Vid::new(1))
            .unwrap()
            .points()
            .unwrap()
            .hp();
        assert_eq!(beta_after - beta_before, 304);
        assert_eq!(alpha_after, alpha_before);
    }

    /// The grandmaster cost lands: 49 at level 30 (a group-2 assassin) charges
    /// the grand-add poly `30+40*0.82 = 62`, and the stamp's grand bit sets.
    #[test]
    fn the_grandmaster_cast_charges_the_grand_add() {
        let (_clock, mut world, _inbox) = a_cast();
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.job = 1;
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.skill_group = 2;
        world.set_pc_skill_level(1, 49, 30);
        let before = sp_of(&world, 1);
        assert!(world.use_skill(Vid::new(1), 49, ME));
        assert_eq!(before - sp_of(&world, 1), 62);
        let stamp = world.pc_skills.get(&1).unwrap().uses.get(&49).unwrap();
        assert!(stamp.grand);
    }
    /// A book and the exp for it in the player's hands: `vnum` at cell 0, `exp` toward
    /// the next level, the `a_cast` field otherwise. The mover is the player's own.
    fn a_reader(vnum: u32, exp: i64) -> (GameState, UnboundedReceiver<Vec<u8>>) {
        let clock = TestClock::default();
        clock.set(T0);
        let npc = a_fighter(ME, 101, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000);
        let mut book = world::item::Item::new(11, vnum);
        book.set_size(1).expect("a positive footprint");
        let cell =
            protocol::item_pos::ItemPos::new(common::item_slots::EWindows::Inventory as u8, 0);
        let mut world = stand_field(&clock, vec![npc], open_cells(), true).with_pc_skills(dumps());
        let (tx, mut inbox) = tokio::sync::mpsc::unbounded_channel();
        world
            .enter_world_with_items(
                Vid::new(1),
                1,
                "P1",
                &[(cell, book)],
                crate::client_registry::ClientOutbox::new(tx),
            )
            .expect("the player enters");
        world
            .characters
            .find_by_vid_mut(Vid::new(1))
            .expect("the player is in the world")
            .set_points(Some(crate::game_state::fixtures::points_at(
                10, 12_345, exp,
            )));
        let card = crate::loading_phase::PcCard {
            name: String::from("P1"),
            job: 0,
            empire: 1,
            level: 10,
            conqueror_level: 0,
            language: 0,
            pk_mode: crate::loading_phase::PK_MODE_PROTECT,
            skill_group: 0,
            alignment: 0,
        };
        world.place_body(
            Vid::new(1),
            crate::game_loop_messages::EnterPlace {
                channel: crate::game_state::fixtures::PLACE.0,
                map: crate::game_state::fixtures::PLACE.1,
                x: NEAR.0,
                y: NEAR.1,
                z: 0,
            },
            card,
        );
        world.process_pulse(P0);
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        (world, inbox)
    }

    /// The chat texts of `frames`, decoded: the `[LS;...]` keys survive because the
    /// fixture locale holds no table (`find` answers the key itself).
    fn texts(frames: &[Vec<u8>]) -> Vec<Vec<u8>> {
        frames
            .iter()
            .map(|frame| GcChat::decode(frame).expect("a chat frame").text)
            .collect()
    }

    /// The leadership ladder reads through `use_item`: 50301 (values 0/20, pct 0) trains
    /// 121 from 0 to 1, the book is gone, and the store half carries slot, level and stamp.
    /// Seed 14's first `number(1, 100)` is 97, past the 65 % train roll (`14 -> [97, ...]`).
    #[test]
    fn a_book_read_climbs_through_use_item_and_consumes() {
        let (mut world, _inbox) = a_reader(50301, 25_000);
        world.dice = world::character::Pcg32::new(14, 0);
        let mover = world.own_mover(Vid::new(1)).expect("the body stands");
        let cell =
            protocol::item_pos::ItemPos::new(common::item_slots::EWindows::Inventory as u8, 0);
        let used = world
            .use_item(Vid::new(1), cell, mover)
            .expect("the read runs");
        assert_eq!(used.kind, world::character::MoveKind::Used);
        assert_eq!(world.pc_skill_level(1, SKILL_LEADERSHIP), 1);
        assert!(
            world
                .characters
                .find_by_vid(Vid::new(1))
                .unwrap()
                .items()
                .item_at(cell)
                .is_none(),
            "the book is consumed"
        );
        let stamp = world.pc_skills.get(&1).unwrap().next_read[SKILL_LEADERSHIP as usize];
        assert_eq!(
            used.climbed,
            Some((121, 1, i32::from_ne_bytes(stamp.to_ne_bytes())))
        );
        assert_eq!(
            world
                .characters
                .find_by_vid(Vid::new(1))
                .unwrap()
                .points()
                .unwrap()
                .exp(),
            5_000,
            "the training spent the 20k"
        );
    }

    /// Combo's level gate refuses with 295 and keeps the book: combo 0 below 30.
    #[test]
    fn a_gated_book_refuses_and_keeps_the_book() {
        let (mut world, mut inbox) = a_reader(50304, 25_000);
        let mover = world.own_mover(Vid::new(1)).expect("the body stands");
        let cell =
            protocol::item_pos::ItemPos::new(common::item_slots::EWindows::Inventory as u8, 0);
        let used = world
            .use_item(Vid::new(1), cell, mover)
            .expect("the step answers");
        assert!(used.changes.is_empty(), "nothing was consumed");
        assert!(
            world
                .characters
                .find_by_vid(Vid::new(1))
                .unwrap()
                .items()
                .item_at(cell)
                .is_some(),
            "the book stays"
        );
        let frames: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        assert_eq!(
            texts(&frames),
            [b"[LS;295]".to_vec()],
            "the combo gate talks 295"
        );
    }

    /// The language empire gate refuses with 328: 50311 teaches 126, and the reader is
    /// empire 1 (`126 - 126 + 1 == 1`).
    #[test]
    fn a_book_of_the_reader_s_own_tongue_refuses() {
        let (mut world, mut inbox) = a_reader(50311, 25_000);
        let mover = world.own_mover(Vid::new(1)).expect("the body stands");
        let cell =
            protocol::item_pos::ItemPos::new(common::item_slots::EWindows::Inventory as u8, 0);
        let used = world
            .use_item(Vid::new(1), cell, mover)
            .expect("the step answers");
        assert!(used.changes.is_empty(), "nothing was consumed");
        let frames: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        assert_eq!(
            texts(&frames),
            [b"[LS;328]".to_vec()],
            "the empire gate talks 328"
        );
    }

    /// The horse book raises `POINT_HORSE_SKILL`: a level-50 reader, 50060 at 50%,
    /// seed 7 rolls 50.
    #[test]
    fn a_horse_book_raises_the_horse_point() {
        let (mut world, _inbox) = a_reader(50060, 25_000);
        world
            .characters
            .find_by_vid_mut(Vid::new(1))
            .unwrap()
            .set_points(Some(crate::game_state::fixtures::points_at(
                50, 12_345, 25_000,
            )));
        world.dice = world::character::Pcg32::new(7, 0);
        let mover = world.own_mover(Vid::new(1)).expect("the body stands");
        let cell =
            protocol::item_pos::ItemPos::new(common::item_slots::EWindows::Inventory as u8, 0);
        let used = world
            .use_item(Vid::new(1), cell, mover)
            .expect("the read runs");
        assert_eq!(used.kind, world::character::MoveKind::Used);
        assert_eq!(
            world
                .characters
                .find_by_vid(Vid::new(1))
                .unwrap()
                .points()
                .unwrap()
                .get_point(common::point_slot::POINT_HORSE_SKILL),
            1
        );
        assert!(used.climbed.is_none(), "the horse arm climbs no slot");
        assert!(
            world.pc_skills.get(&1).unwrap().next_read[SKILL_HORSE as usize] > 0,
            "success stamps the read time"
        );
    }

    /// A stamped read refuses with the wait ladder and keeps the book: the second read
    /// of 50301 lands 18-30 h out, past the ladder's last rung (985/986).
    #[test]
    fn a_stamped_read_refuses_with_the_wait_ladder() {
        let (mut world, mut inbox) = a_reader(50301, 50_000);
        world.dice = world::character::Pcg32::new(14, 0);
        let mover = world.own_mover(Vid::new(1)).expect("the body stands");
        let cell =
            protocol::item_pos::ItemPos::new(common::item_slots::EWindows::Inventory as u8, 0);
        let _ = world
            .use_item(Vid::new(1), cell, mover)
            .expect("the first read runs");
        let mut book = world::item::Item::new(12, 50301);
        book.set_size(1).expect("a positive footprint");
        world
            .characters
            .find_by_vid_mut(Vid::new(1))
            .unwrap()
            .items_mut()
            .set(cell, &book)
            .expect("a second book is held");
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        let used = world
            .use_item(Vid::new(1), cell, mover)
            .expect("the step answers");
        assert!(used.changes.is_empty(), "nothing was consumed");
        let frames: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        assert_eq!(
            texts(&frames),
            [b"[LS;985]".to_vec(), b"[LS;986]".to_vec()],
            "the wait ladder's last rung talks"
        );
    }

    /// Build a [`Game`] session for the reader at 1 and run one GRAND read of `vnum`
    /// at count `total`: whether it climbed, the records it queued, and the climbs.
    /// The live borrows split like `run_quests` does.
    fn grand_read(
        world: &mut GameState,
        vnum: u32,
        total: i32,
    ) -> (bool, super::super::quests::QuestStepOutcome) {
        use quest::manager::Game;
        let mover = world.own_mover(Vid::new(1)).expect("the body stands");
        let reader = Vid::new(1);
        let card = world
            .bodies
            .get(&reader)
            .map_or((0, 0), |body| (body.card.job, body.card.skill_group));
        let pc_level = world
            .characters
            .find_by_vid(reader)
            .ok()
            .and_then(|character| character.points())
            .map_or(0, world::character::Points::level);
        let GameState {
            characters,
            locale,
            dice,
            pc_skills,
            affect_events,
            pc_skill_tables,
            ..
        } = world;
        let points = characters
            .find_by_vid_mut(reader)
            .ok()
            .and_then(|character| character.items_and_points_mut().1);
        let mut session = super::super::quests::Session {
            level: pc_level,
            npcs: None,
            protos: &gamedata::mob_proto::MobProtos::default(),
            names: &gamedata::mob_locale_names::MobNamesByLanguage::default(),
            locale,
            mover,
            tables: pc_skill_tables,
            dice,
            reader,
            card,
            pc_level,
            slots: pc_skills.entry(reader.raw()).or_default(),
            affects: affect_events.get_mut(&reader),
            points,
            climbs: Vec::new(),
            records: Vec::new(),
        };
        let climbed = session.learn_grand_master_skill(vnum, total);
        let super::super::quests::Session {
            records, climbs, ..
        } = session;
        (climbed, (records, climbs))
    }

    /// The GRAND read climbs past its max: vnum 1 at 30, the count already at 6 (max 5
    /// at index 0), so the roll cannot fail; the tail is 954/955/956, the climb is
    /// queued for the store, and the stamp is 8-12 h out.
    #[test]
    fn a_grand_read_past_its_max_always_climbs() {
        let (mut world, mut inbox) = a_reader(50301, 25_000);
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.skill_group = 1;
        world.set_pc_skill_level(1, 1, 30);
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        let (climbed, (records, climbs)) = grand_read(&mut world, 1, 6);
        assert!(climbed);
        assert_eq!(world.pc_skill_level(1, 1), 31);
        assert_eq!(climbs.len(), 1, "the climb is queued for the store");
        assert_eq!((climbs[0].0, climbs[0].1), (1, 31));
        // Byte 4 only: the climb and the stamp each send the skill list (76) first.
        let chats: Vec<Vec<u8>> = records
            .into_iter()
            .filter(|frame| frame.first() == Some(&4))
            .collect();
        assert_eq!(
            texts(&chats),
            [
                b"[LS;954]".to_vec(),
                b"[LS;955]".to_vec(),
                b"[LS;956]".to_vec()
            ],
            "the climb tail talks"
        );
        let stamp = world.pc_skills.get(&1).unwrap().next_read[1];
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| {
                u32::try_from(since.as_secs()).unwrap_or(u32::MAX)
            });
        assert!(
            (28800..=43200).contains(&stamp.saturating_sub(now)),
            "the stamp is 8-12 h out"
        );
    }

    /// A type-0 GRAND read refuses with 949: 121 is typed 0.
    #[test]
    fn a_typeless_grand_read_refuses() {
        let (mut world, mut inbox) = a_reader(50301, 25_000);
        world.set_pc_skill_level(1, SKILL_LEADERSHIP, 30);
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        let (climbed, (records, climbs)) = grand_read(&mut world, SKILL_LEADERSHIP, 1);
        assert!(!climbed);
        assert!(climbs.is_empty(), "no climb, no store");
        assert_eq!(texts(&records), [b"[LS;949]".to_vec()]);
    }
}
