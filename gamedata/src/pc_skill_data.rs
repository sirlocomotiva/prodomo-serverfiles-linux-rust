//! The PC skill tables: the hosted `skill_proto` rows, the eight power rows, and the
//! `SkillList`/`CanUseSkill` gates (ledger 230d).
//!
//! Legacy boots skills from MySQL (`CClientManager::InitializeSkillTable`,
//! `DB/ClientManagerBoot.cpp:654-740`) and the power rows from the `locale`
//! table (`G/config.cpp:643-727`). The Rewrite reads the same rows from the
//! owner's `player.sql` and `common.sql` dumps, reusing the `setFlag+0` /
//! `eSkillType+0` decoders of [`crate::mob_skill_data`].
//!
//! Only the PC rows are kept: `bType` 1-4 with a vnum at most 111 (the four
//! job groups of `SkillList`, `G/char_skill.cpp:4108-4115`). Each kept row
//! carries its compiled polys ([`common::skill_poly::Poly`]); a row whose
//! point, SP-cost, cooldown, master-bonus or splash-adjust poly falls outside
//! the hosted subset is refused with its vnum, mirroring legacy's `sys_err` +
//! skip (`G/skill.cpp`). On the owner's data the 13 `number(`/`floor(` rows
//! (31 32 48 50 51 66 76 77 78 80 106 107 108) are refused, so 35 rows load.
//!
//! The power table is `SKILL_POWER_BY_LEVEL` plus `SKILL_POWER_BY_LEVEL_TYPEn`
//! for `n` in 0-8 (`JOB_MAX_NUM * 2`); a missing `TYPEn` row reads the base row
//! (`G/config.cpp:694-698`). `power` is `GetSkillPowerByLevelFromType`
//! (`G/skill_power.cpp:32-43`): row 0 for a mob, `TYPE[(job*2)+(group-1)]`
//! for a player, 0 past job 3 or group 0, the level clamped to 0-40.

use std::collections::BTreeMap;
use std::fmt;

use common::skill_poly::{Poly, PolyError};

use crate::mob_skill_data::{
    SKILL_ATTR_MELEE, SKILL_FLAG_ATTACK, SKILL_FLAG_USE_MELEE_DAMAGE, SKILL_MAX_LEVEL,
};
use crate::records::SkillTableRecord;
use crate::sql_dump::{read_table, SqlDumpError, SqlValue};

/// The PC skill types: `bType` 1-4 (`CSkillProto::dwType`, `G/skill.h:199`).
const PC_TYPE_MIN: u32 = 1;
/// The last PC skill vnum: the shaman's (`SkillList[3][1][5]`, `G/char_skill.cpp:4113`).
const PC_VNUM_MAX: u32 = 111;
/// The type-0 rows `/skillup` can reach (`do_skillup`'s switch plus the always
/// list, `G/cmd_general.cpp:939-971`, `G/char_skill.cpp:4169-4174`): leadership,
/// combo, mining, the languages, polymorph, the horse rows and the two passives.
/// 123 (`SKILL_CREATE`) is dead (`ClientManagerPlayer.cpp:804` zeroes it).
const TYPE_ZERO_KEPT: [u32; 11] = [121, 122, 124, 126, 127, 128, 129, 130, 131, 141, 142];

/// `SKILL_COUNT` (`G/char_skill.cpp:4107`): six skills per job and group.
pub const SKILL_COUNT: usize = 6;

/// `SkillList` (`G/char_skill.cpp:4108-4115`): the six vnums per job (0-3) and
/// group (0-1).
pub const SKILL_LIST: [[[u32; SKILL_COUNT]; 2]; 4] = [
    [[1, 2, 3, 4, 5, 6], [16, 17, 18, 19, 20, 21]],
    [[31, 32, 33, 34, 35, 36], [46, 47, 48, 49, 50, 51]],
    [[61, 62, 63, 64, 65, 66], [76, 77, 78, 79, 80, 81]],
    [[91, 92, 93, 94, 95, 96], [106, 107, 108, 109, 110, 111]],
];
/// The always-usable vnums of `CanUseSkill` (`G/char_skill.cpp:4169-4174`):
/// 121 122 124 126-131, 151-159.
pub const ALWAYS_USABLE: [u32; 18] = [
    121, 122, 124, 126, 127, 128, 129, 130, 131, 151, 152, 153, 154, 155, 156, 157, 158, 159,
];

/// The riding skills of `CanUseSkill` (`G/char_skill.cpp:4160-4167`): 137-140.
pub const RIDING_USABLE: [u32; 4] = [137, 138, 139, 140];

/// The `CG_ATTACK` shoot list (`G/input_main.cpp:1940-1963`): a `CG_ATTACK` of
/// one is refused, only `CG_SHOOT` casts it. `GEOMPUNG SANGONG YEONSA
/// KWANKYEOK HWAJO GIGUNG PABEOB MARYUNG TUSOK MAHWAN BIPABU NOEJEON CHAIN
/// HORSE_WILDATTACK_RANGE`. `GIGUNG` is 50 (`G/skill.h:84`: 49 is
/// `GYEONGGONG`, not a shoot arm); vnum 51 hosts nothing shoot-listed.
pub const SHOOT_LIST: [u32; 14] = [20, 35, 46, 47, 48, 50, 66, 76, 80, 81, 91, 106, 108, 140];

/// `CanUseSkill` (`G/char_skill.cpp:4135-4183`): the vnum is 0-refused, then
/// readable from the job/group `SkillList` past group 0, the riding arms while
/// riding, or the always list. No riding or mount is ported, so callers pass
/// `riding = false`.
#[must_use]
pub const fn can_use(job: u8, group: u8, riding: bool, vnum: u32) -> bool {
    if vnum == 0 {
        return false;
    }
    if group > 0 && job < 4 && group < 3 {
        let row = SKILL_LIST[job as usize][(group - 1) as usize];
        let mut slot = 0;
        while slot < SKILL_COUNT {
            if row[slot] == vnum {
                return true;
            }
            slot += 1;
        }
    }
    if riding {
        let mut slot = 0;
        while slot < RIDING_USABLE.len() {
            if RIDING_USABLE[slot] == vnum {
                return true;
            }
            slot += 1;
        }
    }
    let mut slot = 0;
    while slot < ALWAYS_USABLE.len() {
        if ALWAYS_USABLE[slot] == vnum {
            return true;
        }
        slot += 1;
    }
    false
}

/// One hosted PC skill row, with its polys compiled.
#[derive(Debug, Clone)]
pub struct PcSkill {
    /// The `skill_proto` row.
    pub record: SkillTableRecord,
    /// `kPointPoly`.
    pub point: Poly,
    /// `kPointPoly2`: only the functor's PENETRATE/ignore-rating draws read it
    /// on the 230d rows.
    pub point2: Poly,
    /// `kSPCostPoly`.
    pub sp_cost: Poly,
    /// `kCooldownPoly`.
    pub cooldown: Poly,
    /// `kMasterBonusPoly`.
    pub master_bonus: Poly,
    /// `kSplashAroundDamageAdjustPoly`.
    pub splash_adjust: Poly,
    /// `kDurationPoly2`: the functor's `STUN`/`FIRE_CONT` arms read it (230e).
    pub duration2: Poly,
    /// `kDurationPoly`: the buff arm reads it (230f).
    pub duration: Poly,
    /// `kDurationSPCostPoly`: the buff arm's per-tick SP cost (230f).
    pub duration_sp_cost: Poly,
    /// `kGrandMasterAddSPCostPoly`: the grandmaster SP cost (230f).
    pub grand_add_sp: Poly,
}

impl PcSkill {
    /// Whether the row casts through the 230d melee pipe: `ATTACK` +
    /// `USE_MELEE_DAMAGE`, MELEE-attr, no SPLASH, and off the shoot list.
    #[must_use]
    pub fn is_castable_melee(&self) -> bool {
        use crate::mob_skill_data::SKILL_FLAG_SPLASH;
        const WANTED: u32 = SKILL_FLAG_ATTACK | SKILL_FLAG_USE_MELEE_DAMAGE;
        self.record.flag & WANTED == WANTED
            && self.record.flag & SKILL_FLAG_SPLASH == 0
            && self.record.skill_attr_type == SKILL_ATTR_MELEE
            && !SHOOT_LIST.contains(&self.record.vnum)
    }

    /// Whether the row casts through the 230e SPLASH MELEE pipe: `ATTACK` +
    /// `USE_MELEE_DAMAGE`, MELEE-attr, SPLASH, and off the shoot list (hosted:
    /// 5/18/62).
    #[must_use]
    pub fn is_castable_splash_melee(&self) -> bool {
        use crate::mob_skill_data::SKILL_FLAG_SPLASH;
        const WANTED: u32 = SKILL_FLAG_ATTACK | SKILL_FLAG_USE_MELEE_DAMAGE;
        self.record.flag & WANTED == WANTED
            && self.record.flag & SKILL_FLAG_SPLASH != 0
            && self.record.skill_attr_type == SKILL_ATTR_MELEE
            && !SHOOT_LIST.contains(&self.record.vnum)
    }

    /// Whether the row casts through the 230e RANGE pipe: `ATTACK` +
    /// `USE_ARROW_DAMAGE`, RANGE-attr, shoot-listed (hosted: 47 only —
    /// 46/48/50 are refused; 51 is not on the shoot list).
    #[must_use]
    pub fn is_castable_range(&self) -> bool {
        use crate::mob_skill_data::{SKILL_ATTR_RANGE, SKILL_FLAG_USE_ARROW_DAMAGE};
        const WANTED: u32 = SKILL_FLAG_ATTACK | SKILL_FLAG_USE_ARROW_DAMAGE;
        self.record.flag & WANTED == WANTED
            && self.record.skill_attr_type == SKILL_ATTR_RANGE
            && SHOOT_LIST.contains(&self.record.vnum)
    }

    /// Whether the row casts through the 230e MAGIC pipe: `ATTACK`,
    /// MAGIC-attr (hosted: 35/81/91/92/93 — 76/77/78/80/106/107/108 refused).
    #[must_use]
    pub fn is_castable_magic(&self) -> bool {
        use crate::mob_skill_data::SKILL_ATTR_MAGIC;
        self.record.flag & SKILL_FLAG_ATTACK != 0 && self.record.skill_attr_type == SKILL_ATTR_MAGIC
    }

    /// Whether `CG_ATTACK` refuses the row and only `CG_SHOOT` casts it: the
    /// shoot list (`G/input_main.cpp:1940-1963`), hosted: 20/35/47/81/91.
    #[must_use]
    pub fn is_shoot_skill(&self) -> bool {
        SHOOT_LIST.contains(&self.record.vnum)
    }
}
/// The PC skill tables.
#[derive(Debug, Clone, Default)]
pub struct PcSkills {
    by_vnum: BTreeMap<u32, PcSkill>,
    /// The eight `TYPEn` rows with the base fallback applied (`G/config.cpp:694-698`).
    power_types: [Vec<i32>; 8],
    /// The refused rows' vnums, in dump order (the `number(`/`floor(` rows).
    refused: Vec<u32>,
}

impl PcSkills {
    /// Read the PC rows from the `player.sql` dump and the power rows from the
    /// `common.sql` dump. Rows outside the hosted subset are refused with their
    /// vnum (see [`PcSkills::refused`]); the load itself stays green.
    ///
    /// # Errors
    ///
    /// Returns [`PcSkillError`] for a dump that does not parse, a missing base
    /// power row, or a kept row whose flags fall outside the hosted subset.
    pub fn from_dumps(player: &[u8], common: &[u8]) -> Result<Self, PcSkillError> {
        let table = read_table(player, "skill_proto").map_err(PcSkillError::Dump)?;
        let mut by_vnum = BTreeMap::new();
        let mut refused = Vec::new();
        for row in &table.rows {
            let vnum = number_at(&table, row, "dwVnum")?;
            let skill_type = number_at(&table, row, "bType")?;
            let typed = (PC_TYPE_MIN..=4).contains(&skill_type) && vnum <= PC_VNUM_MAX;
            let sub = skill_type == 0 && TYPE_ZERO_KEPT.contains(&vnum);
            if !typed && !sub {
                continue;
            }
            let record = decode_row(&table, row, vnum)?;
            let text = |bytes: &[u8]| {
                let end = bytes
                    .iter()
                    .position(|byte| *byte == 0)
                    .unwrap_or(bytes.len());
                String::from_utf8_lossy(&bytes[..end]).into_owned()
            };
            let compile = |poly: &[u8], what: &'static str| {
                Poly::parse(&text(poly)).map_err(|error| PcSkillError::Poly { vnum, what, error })
            };
            let compiled = (|| {
                Ok::<_, PcSkillError>(PcSkill {
                    record,
                    point: compile(&row_poly(&table, row, "szPointPoly")?, "point")?,
                    point2: compile(&row_poly(&table, row, "szPointPoly2")?, "point2")?,
                    sp_cost: compile(&row_poly(&table, row, "szSPCostPoly")?, "sp_cost")?,
                    cooldown: compile(&row_poly(&table, row, "szCooldownPoly")?, "cooldown")?,
                    master_bonus: compile(
                        &row_poly(&table, row, "szMasterBonusPoly")?,
                        "master_bonus",
                    )?,
                    splash_adjust: compile(
                        &row_poly(&table, row, "szSplashAroundDamageAdjustPoly")?,
                        "splash_adjust",
                    )?,
                    duration2: compile(&row_poly(&table, row, "szDurationPoly2")?, "duration2")?,
                    duration: compile(&row_poly(&table, row, "szDurationPoly")?, "duration")?,
                    duration_sp_cost: compile(
                        &row_poly(&table, row, "szDurationSPCostPoly")?,
                        "duration_sp_cost",
                    )?,
                    grand_add_sp: compile(
                        &row_poly(&table, row, "szGrandMasterAddSPCostPoly")?,
                        "grand_add_sp",
                    )?,
                })
            })();
            match compiled {
                Ok(skill) => {
                    by_vnum.insert(vnum, skill);
                }
                Err(PcSkillError::Poly { .. }) => refused.push(vnum),
                Err(other) => return Err(other),
            }
        }
        let power_types = power_from_dump(common)?;
        Ok(Self {
            by_vnum,
            power_types,
            refused,
        })
    }

    /// The hosted row, or `None` for an unknown or refused vnum.
    #[must_use]
    pub fn get(&self, vnum: u32) -> Option<&PcSkill> {
        self.by_vnum.get(&vnum)
    }

    /// The refused rows' vnums, in dump order.
    #[must_use]
    pub fn refused(&self) -> &[u32] {
        &self.refused
    }

    /// `GetSkillPowerByLevelFromType` (`G/skill_power.cpp:32-43`): row 0 for a
    /// mob, `TYPE[(job*2)+(group-1)]` for a player, 0 past job 3 or group 0,
    /// the level clamped to 0-40.
    #[must_use]
    pub fn power(&self, job: u8, group: u8, level: u8, mob: bool) -> i32 {
        let index = usize::from(level).min(SKILL_MAX_LEVEL);
        if mob {
            return self.power_types[0].get(index).copied().unwrap_or(0);
        }
        if job >= 4 || group == 0 || group > 2 {
            return 0;
        }
        let row = usize::from(job) * 2 + usize::from(group - 1);
        self.power_types[row].get(index).copied().unwrap_or(0)
    }
}

/// What went wrong reading the PC skill tables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PcSkillError {
    /// The dump did not parse.
    Dump(SqlDumpError),
    /// A needed column is missing.
    NoColumn {
        /// The row's vnum (0 for a power row).
        vnum: u32,
        /// The missing column.
        column: String,
    },
    /// A numeric column is not a number.
    NotANumber {
        /// The column.
        column: &'static str,
        /// The cell text.
        text: String,
    },
    /// A `setFlag` token outside the schema order.
    UnknownFlag {
        /// The row's vnum.
        vnum: u32,
        /// The token.
        token: String,
    },
    /// An `eSkillType` outside NORMAL/MELEE/RANGE/MAGIC.
    UnknownAttr {
        /// The row's vnum.
        vnum: u32,
        /// The token.
        token: String,
    },
    /// A kept row's poly is outside the hosted subset.
    Poly {
        /// The row's vnum.
        vnum: u32,
        /// Which poly.
        what: &'static str,
        /// The parse error.
        error: PolyError,
    },
    /// A missing base power row.
    NoPowerRow,
    /// A non-numeric power entry.
    NotAPower(String),
    /// A power row too short to index 0-40.
    ShortPower(usize),
}

impl fmt::Display for PcSkillError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dump(error) => write!(f, "dump: {error}"),
            Self::NoColumn { vnum, column } => write!(f, "skill {vnum}: no {column} column"),
            Self::NotANumber { column, text } => {
                write!(f, "{column}: not a number: {text:?}")
            }
            Self::UnknownFlag { vnum, token } => {
                write!(f, "skill {vnum}: unknown flag {token:?}")
            }
            Self::UnknownAttr { vnum, token } => {
                write!(f, "skill {vnum}: unknown attr {token:?}")
            }
            Self::Poly { vnum, what, error } => {
                write!(f, "skill {vnum}: {what} poly: {error:?}")
            }
            Self::NoPowerRow => write!(f, "no SKILL_POWER_BY_LEVEL row"),
            Self::NotAPower(entry) => write!(f, "power entry not a number: {entry:?}"),
            Self::ShortPower(len) => write!(f, "power row has {len} entries, need 41+"),
        }
    }
}

impl std::error::Error for PcSkillError {}

/// Decode one dump row into the record shape. The flag/attr decoders live in
/// `mob_skill_data`; this module reuses them through the shared record.
fn decode_row(
    table: &crate::sql_dump::SqlTable,
    row: &[SqlValue],
    vnum: u32,
) -> Result<SkillTableRecord, PcSkillError> {
    let bytes_of = |column: &'static str| -> Result<Vec<u8>, PcSkillError> {
        let Some(index) = table.column(column) else {
            return Err(PcSkillError::NoColumn {
                vnum,
                column: column.to_string(),
            });
        };
        match &row[index] {
            SqlValue::Text(text) | SqlValue::Bare(text) => Ok(text.clone()),
            SqlValue::Null => Ok(Vec::new()),
        }
    };
    let mut record = SkillTableRecord {
        vnum,
        ..SkillTableRecord::default()
    };
    copy_into(&mut record.name, &bytes_of("szName")?);
    record.skill_type = u8::try_from(number_at(table, row, "bType")?).unwrap_or(u8::MAX);
    record.max_level = u8::try_from(number_at(table, row, "bMaxLevel")?).unwrap_or(u8::MAX);
    record.level_limit = u8::try_from(number_at(table, row, "bLevelLimit")?).unwrap_or(u8::MAX);
    record.splash_range = number_at(table, row, "dwSplashRange")?;
    record.target_range = number_at(table, row, "dwTargetRange")?;
    copy_into(&mut record.point_poly, &bytes_of("szPointPoly")?);
    copy_into(&mut record.sp_cost_poly, &bytes_of("szSPCostPoly")?);
    copy_into(&mut record.cooldown_poly, &bytes_of("szCooldownPoly")?);
    copy_into(
        &mut record.master_bonus_poly,
        &bytes_of("szMasterBonusPoly")?,
    );
    copy_into(
        &mut record.splash_around_damage_adjust_poly,
        &bytes_of("szSplashAroundDamageAdjustPoly")?,
    );
    record.flag = flag_mask(&bytes_of("setFlag")?, vnum)?;
    record.skill_attr_type = attr_index(&bytes_of("eSkillType")?, vnum)?;
    record.max_hit = i32::try_from(number_at(table, row, "iMaxHit")?).unwrap_or(i32::MAX);
    record.pre_skill_vnum = number_at(table, row, "prerequisiteSkillVnum")?;
    record.pre_skill_level =
        u8::try_from(number_at(table, row, "prerequisiteSkillLevel")?).unwrap_or(u8::MAX);
    record.affect_flag = affect_index(&bytes_of("setAffectFlag")?, vnum)?;
    record.affect_flag2 = affect_index(&bytes_of("setAffectFlag2")?, vnum)?;
    copy_into(&mut record.point_on, &bytes_of("szPointOn")?);
    copy_into(&mut record.point_on2, &bytes_of("szPointOn2")?);
    copy_into(&mut record.point_on3, &bytes_of("szPointOn3")?);
    Ok(record)
}

fn copy_into<const N: usize>(field: &mut [u8; N], bytes: &[u8]) {
    let end = bytes.len().min(N.saturating_sub(1));
    field[..end].copy_from_slice(&bytes[..end]);
}

fn number_at(
    table: &crate::sql_dump::SqlTable,
    row: &[SqlValue],
    column: &'static str,
) -> Result<u32, PcSkillError> {
    let Some(index) = table.column(column) else {
        return Err(PcSkillError::NoColumn {
            vnum: 0,
            column: column.to_string(),
        });
    };
    match &row[index] {
        SqlValue::Bare(text) | SqlValue::Text(text) => {
            let text = String::from_utf8_lossy(text);
            text.trim()
                .parse::<u32>()
                .map_err(|_| PcSkillError::NotANumber {
                    column,
                    text: text.into_owned(),
                })
        }
        SqlValue::Null => Ok(0),
    }
}

fn row_poly(
    table: &crate::sql_dump::SqlTable,
    row: &[SqlValue],
    column: &'static str,
) -> Result<Vec<u8>, PcSkillError> {
    let Some(index) = table.column(column) else {
        return Err(PcSkillError::NoColumn {
            vnum: 0,
            column: column.to_string(),
        });
    };
    match &row[index] {
        SqlValue::Bare(text) | SqlValue::Text(text) => Ok(text.clone()),
        SqlValue::Null => Ok(Vec::new()),
    }
}

/// The `setFlag+0` mask: MySQL `set+0` sets bit `i` for the `i`-th schema
/// member (see [`crate::mob_skill_data::SKILL_FLAG_ORDER`]).
fn flag_mask(field: &[u8], vnum: u32) -> Result<u32, PcSkillError> {
    use crate::mob_skill_data::SKILL_FLAG_ORDER;
    let field = String::from_utf8_lossy(field);
    let field = field.trim();
    if field.is_empty() || field == "NULL" {
        return Ok(0);
    }
    let mut mask = 0u32;
    for token in field.split(',') {
        let token = token.trim();
        let Some(bit) = SKILL_FLAG_ORDER.iter().position(|known| *known == token) else {
            return Err(PcSkillError::UnknownFlag {
                vnum,
                token: token.to_string(),
            });
        };
        mask |= 1 << bit;
    }
    Ok(mask)
}

/// The `eSkillType+0` index: MySQL enum order NORMAL/MELEE/RANGE/MAGIC from 1.
fn attr_index(field: &[u8], vnum: u32) -> Result<u8, PcSkillError> {
    let field = String::from_utf8_lossy(field);
    match field.trim() {
        "NORMAL" => Ok(1),
        "MELEE" => Ok(2),
        "RANGE" => Ok(3),
        "MAGIC" => Ok(4),
        token => Err(PcSkillError::UnknownAttr {
            vnum,
            token: token.to_string(),
        }),
    }
}
/// The `setAffectFlag+0` index: MySQL `enum+0` numbers the schema members from
/// 1, so `YMIR` is 1 — exactly the legacy `AFF_*` numbers (`G/affect.h:208+`:
/// `AFF_NONE` 0, `AFF_YMIR` 1, ...). An empty member reads 0 (no flag).
fn affect_index(field: &[u8], vnum: u32) -> Result<u32, PcSkillError> {
    const ORDER: [&str; 45] = [
        "YMIR",
        "INVISIBILITY",
        "SPAWN",
        "POISON",
        "SLOW",
        "STUN",
        "DUNGEON_READY",
        "DUNGEON_UNIQUE",
        "BUILDING_CONSTRUCTION_SMALL",
        "BUILDING_CONSTRUCTION_LARGE",
        "BUILDING_UPGRADE",
        "MOV_SPEED_POTION",
        "ATT_SPEED_POTION",
        "FISH_MIND",
        "JEONGWIHON",
        "GEOMGYEONG",
        "CHEONGEUN",
        "GYEONGGONG",
        "EUNHYUNG",
        "GWIGUM",
        "TERROR",
        "JUMAGAP",
        "HOSIN",
        "BOHO",
        "KWAESOK",
        "MANASHIELD",
        "MUYEONG",
        "REVIVE_INVISIBLE",
        "FIRE",
        "GICHEON",
        "JEUNGRYEOK",
        "TANHWAN_DASH",
        "PABEOP",
        "CHEONGEUN_WITH_FALL",
        "POLYMORPH",
        "WAR_FLAG1",
        "WAR_FLAG2",
        "WAR_FLAG3",
        "CHINA_FIREWORK",
        "HAIR",
        "GERMANY",
        "RAMADAN_RING",
        "BLEEDING",
        "RED_POSSESSION",
        "BLUE_POSSESSION",
    ];
    let field = String::from_utf8_lossy(field);
    let field = field.trim();
    if field.is_empty() || field == "NULL" {
        return Ok(0);
    }
    let Some(index) = ORDER.iter().position(|known| *known == field) else {
        return Err(PcSkillError::UnknownAttr {
            vnum,
            token: field.to_string(),
        });
    };
    Ok(u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1))
}

/// The power rows: `SKILL_POWER_BY_LEVEL` plus `TYPE0..7`, each a
/// space-separated int list; a missing `TYPEn` reads the base
/// (`G/config.cpp:694-698`).
fn power_from_dump(common: &[u8]) -> Result<[Vec<i32>; 8], PcSkillError> {
    let table = read_table(common, "locale").map_err(PcSkillError::Dump)?;
    let (Some(key), Some(value)) = (table.column("mKey"), table.column("mValue")) else {
        return Err(PcSkillError::NoPowerRow);
    };
    let mut rows: BTreeMap<String, Vec<i32>> = BTreeMap::new();
    for row in &table.rows {
        let is_key = match &row[key] {
            SqlValue::Text(text) | SqlValue::Bare(text) => {
                text.as_slice() == b"SKILL_POWER_BY_LEVEL"
                    || (text.as_slice().starts_with(b"SKILL_POWER_BY_LEVEL_TYPE")
                        && text.as_slice().len() == b"SKILL_POWER_BY_LEVEL_TYPE0".len())
            }
            SqlValue::Null => false,
        };
        if !is_key {
            continue;
        }
        let name = match &row[key] {
            SqlValue::Text(text) | SqlValue::Bare(text) => {
                String::from_utf8_lossy(text).into_owned()
            }
            SqlValue::Null => continue,
        };
        let text = match &row[value] {
            SqlValue::Text(text) | SqlValue::Bare(text) => {
                String::from_utf8_lossy(text).into_owned()
            }
            SqlValue::Null => return Err(PcSkillError::NoPowerRow),
        };
        let power = text
            .split_ascii_whitespace()
            .map(|entry| {
                entry
                    .parse::<i32>()
                    .map_err(|_| PcSkillError::NotAPower(entry.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if power.len() <= SKILL_MAX_LEVEL {
            return Err(PcSkillError::ShortPower(power.len()));
        }
        rows.insert(name, power);
    }
    let Some(base) = rows.remove("SKILL_POWER_BY_LEVEL") else {
        return Err(PcSkillError::NoPowerRow);
    };
    let mut types: [Vec<i32>; 8] = Default::default();
    for (slot, row) in types.iter_mut().enumerate() {
        let name = format!("SKILL_POWER_BY_LEVEL_TYPE{slot}");
        *row = rows.remove(&name).unwrap_or_else(|| base.clone());
    }
    Ok(types)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn dump(name: &str) -> Vec<u8> {
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../legacy/sql/gamedata")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn pc_rows_load_and_function_rows_are_refused() {
        let skills = PcSkills::from_dumps(&dump("player.sql"), &dump("common.sql")).unwrap();
        // 48 PC rows (type 1-4, vnum <= 111); the 14 `number(`/`floor(` rows
        // are refused, so 34 load.
        assert_eq!(
            skills.refused(),
            &[31, 32, 46, 48, 50, 51, 66, 76, 77, 78, 80, 106, 107, 108]
        );
        assert!(skills.get(31).is_none());
        assert!(skills.get(46).is_none());
        // Vnum 1: the bare melee hit: ATTACK + USE_MELEE_DAMAGE, MELEE, off the
        // shoot list — the 230d pipe casts it.
        let first = skills.get(1).unwrap();
        assert_eq!(
            first.record.flag,
            SKILL_FLAG_ATTACK | SKILL_FLAG_USE_MELEE_DAMAGE
        );
        assert_eq!(first.record.skill_attr_type, SKILL_ATTR_MELEE);
        assert_eq!(first.record.max_level, 1);
        assert!(first.is_castable_melee());
        // Vnum 20: SPLASH + CRUSH and shoot-listed — the splash/`CG_SHOOT` half.
        let crush = skills.get(20).unwrap();
        assert!(!crush.is_castable_melee());
        // Vnum 61: PENETRATE is still a castable melee row (its empty point2
        // poly reads 0, so the ignore-defence draw never fires).
        assert!(skills.get(61).unwrap().is_castable_melee());
        // Vnum 3: a SELFONLY buff — not a melee cast.
        assert!(!skills.get(3).unwrap().is_castable_melee());
        // Horse (137) and guild (151) rows are not PC rows.
        assert!(skills.get(137).is_none());
        assert!(skills.get(151).is_none());
    }
    #[test]
    fn type_zero_rows_and_buff_flags_load() {
        let skills = PcSkills::from_dumps(&dump("player.sql"), &dump("common.sql")).unwrap();
        // The 11 type-0 rows `/skillup` can reach.
        for vnum in [121, 122, 124, 126, 127, 128, 129, 130, 131, 141, 142] {
            assert!(skills.get(vnum).is_some(), "{vnum}");
        }
        // 123 is dead; 132-140 and 143-150 stay out.
        assert!(skills.get(123).is_none());
        assert!(skills.get(137).is_none());
        // The buff flags are the schema enum index: JEONGWIHON 15 ... MANASHIELD 26.
        assert_eq!(skills.get(3).unwrap().record.affect_flag, 15);
        assert_eq!(skills.get(4).unwrap().record.affect_flag, 16);
        assert_eq!(skills.get(19).unwrap().record.affect_flag, 17);
        assert_eq!(skills.get(34).unwrap().record.affect_flag, 19);
        assert_eq!(skills.get(49).unwrap().record.affect_flag, 18);
        assert_eq!(skills.get(63).unwrap().record.affect_flag, 20);
        assert_eq!(skills.get(64).unwrap().record.affect_flag, 21);
        assert_eq!(skills.get(65).unwrap().record.affect_flag, 22);
        assert_eq!(skills.get(79).unwrap().record.affect_flag, 26);
        // The buff second flags are empty: no second flag.
        assert_eq!(skills.get(19).unwrap().record.affect_flag2, 0);
    }
    #[test]
    fn splash_and_shoot_rows_sort_into_their_pipes() {
        let skills = PcSkills::from_dumps(&dump("player.sql"), &dump("common.sql")).unwrap();
        let cast = |vnum: u32| skills.get(vnum).unwrap();
        // SPLASH MELEE off the shoot list: 5/18/62 (5 and 18 are SELFONLY too).
        for vnum in [5, 18, 62] {
            assert!(cast(vnum).is_castable_splash_melee(), "{vnum}");
            assert!(!cast(vnum).is_castable_melee(), "{vnum}");
            assert!(!cast(vnum).is_shoot_skill(), "{vnum}");
        }
        // The RANGE pipe: 47 alone (46/48/50 are refused, 51 never listed).
        assert!(cast(47).is_castable_range());
        assert!(cast(47).is_shoot_skill());
        // The MAGIC pipe: 35/81/91 shoot-listed, 92/93 not.
        for vnum in [35, 81, 91, 92, 93] {
            assert!(cast(vnum).is_castable_magic(), "{vnum}");
        }
        for vnum in [20, 35, 47, 81, 91] {
            assert!(cast(vnum).is_shoot_skill(), "{vnum}");
        }
        assert!(!cast(92).is_shoot_skill());
        assert!(!cast(93).is_shoot_skill());
        // The 230d pipe still refuses the SPLASH and shoot rows.
        for vnum in [5, 18, 20, 35, 47, 62, 81, 91] {
            assert!(!cast(vnum).is_castable_melee(), "{vnum}");
        }
        // Vnum 33: a poisoned plain melee row, no SPLASH, off the list.
        assert!(cast(33).is_castable_melee());
    }

    #[test]
    fn power_rows_read_per_job_and_group() {
        let skills = PcSkills::from_dumps(&dump("player.sql"), &dump("common.sql")).unwrap();
        // Every owner TYPE row equals the base row; a warrior of group 1 reads
        // TYPE0, a shaman of group 2 TYPE7.
        assert_eq!(skills.power(0, 1, 1, false), 5);
        assert_eq!(skills.power(3, 2, 1, false), 5);
        assert_eq!(skills.power(0, 1, 40, false), skills.power(0, 1, 41, false));
        assert_eq!(
            skills.power(0, 1, u8::MAX, false),
            skills.power(0, 1, 40, false)
        );
        // A mob reads row 0 whatever the job and group are.
        assert_eq!(skills.power(9, 9, 1, true), 5);
        // Group 0, group 3 and job 4 read 0.
        assert_eq!(skills.power(0, 0, 20, false), 0);
        assert_eq!(skills.power(0, 3, 20, false), 0);
        assert_eq!(skills.power(4, 1, 20, false), 0);
    }

    #[test]
    fn missing_type_row_falls_back_to_base() {
        let base = "0 5 6 8 10 12 14 16 18 20 22 24 26 28 30 32 34 36 38 40 50 52 54 56 58 60 63 66 69 72 82 85 88 91 94 98 102 106 110 115 125";
        let common = format!(
            "INSERT INTO `locale` (`mKey`, `mValue`) VALUES ('SKILL_POWER_BY_LEVEL','{base}');\n\
             INSERT INTO `locale` (`mKey`, `mValue`) VALUES ('SKILL_POWER_BY_LEVEL_TYPE0','{base}');\n"
        );
        let player = dump("player.sql");
        let skills = PcSkills::from_dumps(&player, common.as_bytes()).unwrap();
        // TYPE3 is missing, so a group-2 warrior reads the base row.
        assert_eq!(skills.power(1, 2, 1, false), 5);
        assert_eq!(skills.power(0, 1, 1, false), 5);
    }

    #[test]
    fn can_use_pins_the_skill_list() {
        // Group 1 warrior: 1-6 usable, 16 not; group 2: 16-21.
        assert!(can_use(0, 1, false, 1));
        assert!(can_use(0, 1, false, 6));
        assert!(!can_use(0, 1, false, 16));
        assert!(can_use(0, 2, false, 16));
        assert!(can_use(0, 2, false, 21));
        assert!(!can_use(0, 2, false, 1));
        // Shaman group 2 ends at 111.
        assert!(can_use(3, 2, false, 111));
        assert!(!can_use(3, 2, false, 112));
        // Group 0 reaches only the always list.
        assert!(!can_use(0, 0, false, 1));
        assert!(can_use(0, 0, false, 121));
        assert!(can_use(1, 0, false, 159));
        // Vnum 0 is refused everywhere; riding opens 137-140.
        assert!(!can_use(0, 1, false, 0));
        assert!(!can_use(0, 1, false, 137));
        assert!(can_use(0, 1, true, 137));
        assert!(can_use(0, 1, true, 140));
        assert!(!can_use(0, 1, true, 141));
    }
}
