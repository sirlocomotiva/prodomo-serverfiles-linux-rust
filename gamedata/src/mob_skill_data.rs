//! The mob skill tables: the hosted `skill_proto` rows, the mob power row, and the
//! splash data the `.msa` clips carry (ledger 229u).
//!
//! Legacy boots skills from MySQL (`CClientManager::InitializeSkillTable`,
//! `DB/ClientManagerBoot.cpp:654-740`): the query selects thirty columns with `setFlag+0`
//! and `eSkillType+0` as numbers, and `CSkillManager::Initialize` (`G/skill.cpp:94-230`)
//! refuses a row whose point names are unknown. The Rewrite reads the same rows from the
//! owner's `player.sql` dump and the mob power row from `common.sql`'s `locale` table.
//!
//! Only the rows the owner's skill mobs name are kept (20, 256, 258, 263, 264, 265, 266;
//! vnum 270 has no row and is refused at cast, as legacy's NULL `CSkillProto` is). Each
//! kept row carries its compiled polys ([`common::skill_poly::Poly`]); a poly outside the
//! hosted subset refuses the row with its vnum, mirroring legacy's `sys_err` + skip.
//!
//! The splash table comes from the monster `.msa` clips (`CMotion::
//! LoadMobSkillFromFile`, `G/motion.cpp:412-518`): per `SPECIAL` clip, the `MotionDuration`
//! and every `MotionEventType 4` event's `StartingTime` and sphere `Position` y. The
//! timing is `100 + (DWORD)(t*1000)`, the distance `-(DWORD)(y)`; see
//! [`crate::mob_skill_data::splash_of`] for the provisional negative-y reading (no i686 probe on this machine).

use std::collections::BTreeMap;
use std::fmt;

use common::skill_poly::{Poly, PolyError};

use crate::records::SkillTableRecord;
use crate::sql_dump::{read_table, SqlDumpError, SqlValue};

/// The skill vnums the owner's skill mobs name that have a `skill_proto` row.
pub const HOSTED_SKILL_VNUMS: [u32; 7] = [20, 256, 258, 263, 264, 265, 266];

/// `SKILL_MAX_LEVEL` (`C/length.h:75`): the power table's clamp.
pub const SKILL_MAX_LEVEL: usize = 40;

/// `SKILL_FLAG_*` bits (`G/skill.h:9-28`), as the schema's `setFlag` member order
/// (`legacy/sql/schema/player.sql:1345`) numbers them for the `+0` decode: MySQL
/// `set+0` sets bit `i` for the `i`-th member. This matches the legacy enum bit for
/// bit EXCEPT `PARTY` (schema 29th → bit 28; enum bit 27 — see
/// [`SKILL_FLAG_PARTY`]).
pub const SKILL_FLAG_ORDER: [&str; 29] = [
    "ATTACK",
    "USE_MELEE_DAMAGE",
    "COMPUTE_ATTGRADE",
    "SELFONLY",
    "USE_MAGIC_DAMAGE",
    "USE_HP_AS_COST",
    "COMPUTE_MAGIC_DAMAGE",
    "SPLASH",
    "GIVE_PENALTY",
    "USE_ARROW_DAMAGE",
    "PENETRATE",
    "IGNORE_TARGET_RATING",
    "ATTACK_SLOW",
    "ATTACK_STUN",
    "HP_ABSORB",
    "SP_ABSORB",
    "ATTACK_FIRE_CONT",
    "REMOVE_BAD_AFFECT",
    "REMOVE_GOOD_AFFECT",
    "CRUSH",
    "ATTACK_POISON",
    "TOGGLE",
    "DISABLE_BY_POINT_UP",
    "CRUSH_LONG",
    "ATTACK_WIND",
    "ATTACK_ELEC",
    "ATTACK_FIRE",
    "ATTACK_BLEEDING",
    "PARTY",
];

/// `SKILL_FLAG_ATTACK`.
pub const SKILL_FLAG_ATTACK: u32 = 1 << 0;
/// `SKILL_FLAG_USE_MELEE_DAMAGE`.
pub const SKILL_FLAG_USE_MELEE_DAMAGE: u32 = 1 << 1;
/// `SKILL_FLAG_SELFONLY`.
pub const SKILL_FLAG_SELFONLY: u32 = 1 << 3;
/// `SKILL_FLAG_USE_MAGIC_DAMAGE`.
pub const SKILL_FLAG_USE_MAGIC_DAMAGE: u32 = 1 << 4;
/// `SKILL_FLAG_SPLASH`.
pub const SKILL_FLAG_SPLASH: u32 = 1 << 7;
/// `SKILL_FLAG_PENETRATE`: the functor's per-victim ignore-defence roll (no hosted row
/// sets it, but the roll is coded and dead).
pub const SKILL_FLAG_PENETRATE: u32 = 1 << 10;
/// `SKILL_FLAG_IGNORE_TARGET_RATING`: the functor's per-victim ignore-rating roll (no
/// hosted row sets it, but the roll is coded and dead).
pub const SKILL_FLAG_IGNORE_TARGET_RATING: u32 = 1 << 11;
/// `SKILL_FLAG_CRUSH`.
pub const SKILL_FLAG_CRUSH: u32 = 1 << 19;
/// `SKILL_FLAG_PARTY` (`G/skill.h:36`): bit 27 — but the schema's `setFlag` member
/// order puts `PARTY` 29th, so MySQL `setFlag+0` sets bit 28 instead (the schema
/// gained `ATTACK_BLEEDING`, which the 2008-era enum lacks). Legacy decodes the
/// row to bit 28 and then tests bit 27, so the test always fails: vnum 265 always
/// self-heals, even with a party (a Defect; the owner's 265 row sets only `PARTY`).
/// The Rewrite decodes the same bit 28 and tests the same bit 27.
pub const SKILL_FLAG_PARTY: u32 = 1 << 27;

/// `SKILL_ATTR_TYPE_MELEE` (`C/length.h:753-756`): 2, the MySQL enum index of `MELEE`.
pub const SKILL_ATTR_MELEE: u8 = 2;
/// `SKILL_ATTR_TYPE_NORMAL`: 1.
pub const SKILL_ATTR_NORMAL: u8 = 1;

/// `HEALING_SKILL_VNUM` (`C/prodomodefines.h:41`).
pub const HEALING_SKILL_VNUM: u32 = 265;

/// One hosted skill row, with its polys compiled.
#[derive(Debug, Clone)]
pub struct MobSkill {
    /// The `skill_proto` row.
    pub record: SkillTableRecord,
    /// `kPointPoly`.
    pub point: Poly,
    /// `kCooldownPoly`.
    pub cooldown: Poly,
    /// `kPointPoly2`.
    pub point2: Poly,
    /// `kDurationPoly2`.
    pub duration2: Poly,
}

/// The mob skill tables.
#[derive(Debug, Clone, Default)]
pub struct MobSkills {
    by_vnum: BTreeMap<u32, MobSkill>,
    /// `SKILL_POWER_BY_LEVEL_TYPE0`, the mob row (`G/config.cpp:688-727`).
    power: Vec<i32>,
}

impl MobSkills {
    /// Read the hosted rows from the `player.sql` dump and the power row from the
    /// `common.sql` dump.
    ///
    /// # Errors
    ///
    /// Returns [`MobSkillError`] for a dump that does not parse, a missing power row,
    /// or a hosted row whose flags or polys fall outside the hosted subset.
    pub fn from_dumps(player: &[u8], common: &[u8]) -> Result<Self, MobSkillError> {
        let table = read_table(player, "skill_proto").map_err(MobSkillError::Dump)?;
        let mut by_vnum = BTreeMap::new();
        for row in &table.rows {
            let vnum = number_at(&table, row, "dwVnum")?;
            if !HOSTED_SKILL_VNUMS.contains(&vnum) {
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
                Poly::parse(&text(poly)).map_err(|error| MobSkillError::Poly { vnum, what, error })
            };
            by_vnum.insert(
                vnum,
                MobSkill {
                    record,
                    point: compile(&row_poly(&table, row, "szPointPoly")?, "point")?,
                    cooldown: compile(&row_poly(&table, row, "szCooldownPoly")?, "cooldown")?,
                    point2: compile(&row_poly(&table, row, "szPointPoly2")?, "point2")?,
                    duration2: compile(&row_poly(&table, row, "szDurationPoly2")?, "duration2")?,
                },
            );
        }
        let power = power_from_dump(common)?;
        Ok(Self { by_vnum, power })
    }

    /// The hosted row, or `None` for an unknown vnum (270).
    #[must_use]
    pub fn get(&self, vnum: u32) -> Option<&MobSkill> {
        self.by_vnum.get(&vnum)
    }

    /// `GetSkillPowerByLevelFromType` row 0 (`G/skill_power.cpp:32-37`): `power[level]`
    /// clamped to 0-40.
    #[must_use]
    pub fn power(&self, level: u8) -> i32 {
        let index = usize::from(level).min(SKILL_MAX_LEVEL);
        self.power.get(index).copied().unwrap_or(0)
    }
}

/// Decode one dump row into the record shape, with `setFlag`/`eSkillType` converted from
/// their names to the `+0` numbers the legacy query selected.
fn decode_row(
    table: &crate::sql_dump::SqlTable,
    row: &[SqlValue],
    vnum: u32,
) -> Result<SkillTableRecord, MobSkillError> {
    let bytes_of = |column: &'static str| -> Result<Vec<u8>, MobSkillError> {
        let Some(index) = table.column(column) else {
            return Err(MobSkillError::NoColumn {
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
    record.splash_range = number_at(table, row, "dwSplashRange")?;
    copy_into(&mut record.point_on, &bytes_of("szPointOn")?);
    copy_into(&mut record.point_poly, &bytes_of("szPointPoly")?);
    copy_into(&mut record.cooldown_poly, &bytes_of("szCooldownPoly")?);
    copy_into(&mut record.point_on2, &bytes_of("szPointOn2")?);
    copy_into(&mut record.point_poly2, &bytes_of("szPointPoly2")?);
    copy_into(&mut record.duration_poly2, &bytes_of("szDurationPoly2")?);
    record.flag = flag_mask(&bytes_of("setFlag")?, vnum)?;
    record.skill_attr_type = attr_index(&bytes_of("eSkillType")?, vnum)?;
    record.max_hit = i32::try_from(number_at(table, row, "iMaxHit")?).unwrap_or(i32::MAX);
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
) -> Result<u32, MobSkillError> {
    let Some(index) = table.column(column) else {
        return Err(MobSkillError::NoColumn {
            vnum: 0,
            column: column.to_string(),
        });
    };
    match &row[index] {
        SqlValue::Bare(text) | SqlValue::Text(text) => {
            let text = String::from_utf8_lossy(text);
            text.trim()
                .parse::<u32>()
                .map_err(|_| MobSkillError::NotANumber {
                    column: column.to_string(),
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
) -> Result<Vec<u8>, MobSkillError> {
    let Some(index) = table.column(column) else {
        return Err(MobSkillError::NoColumn {
            vnum: 0,
            column: column.to_string(),
        });
    };
    match &row[index] {
        SqlValue::Text(text) | SqlValue::Bare(text) => Ok(text.clone()),
        SqlValue::Null => Ok(Vec::new()),
    }
}

/// The `setFlag+0` mask: bit `i` per member, in [`SKILL_FLAG_ORDER`].
fn flag_mask(field: &[u8], vnum: u32) -> Result<u32, MobSkillError> {
    if field.is_empty() {
        return Ok(0);
    }
    let field = String::from_utf8_lossy(field);
    let mut mask = 0u32;
    for token in field.split(',') {
        let token = token.trim();
        let Some(bit) = SKILL_FLAG_ORDER.iter().position(|known| *known == token) else {
            return Err(MobSkillError::UnknownFlag {
                vnum,
                token: token.to_string(),
            });
        };
        mask |= 1 << bit;
    }
    Ok(mask)
}

/// The `eSkillType+0` index: MySQL enum order NORMAL/MELEE/RANGE/MAGIC from 1.
fn attr_index(field: &[u8], vnum: u32) -> Result<u8, MobSkillError> {
    let field = String::from_utf8_lossy(field);
    match field.trim() {
        "NORMAL" => Ok(1),
        "MELEE" => Ok(2),
        "RANGE" => Ok(3),
        "MAGIC" => Ok(4),
        token => Err(MobSkillError::UnknownAttr {
            vnum,
            token: token.to_string(),
        }),
    }
}

/// The mob power row: `SKILL_POWER_BY_LEVEL_TYPE0`'s space-separated ints.
fn power_from_dump(common: &[u8]) -> Result<Vec<i32>, MobSkillError> {
    let table = read_table(common, "locale").map_err(MobSkillError::Dump)?;
    let (Some(key), Some(value)) = (table.column("mKey"), table.column("mValue")) else {
        return Err(MobSkillError::NoPowerRow);
    };
    for row in &table.rows {
        let is_key = match &row[key] {
            SqlValue::Text(text) | SqlValue::Bare(text) => {
                text.as_slice() == b"SKILL_POWER_BY_LEVEL_TYPE0"
            }
            SqlValue::Null => false,
        };
        if !is_key {
            continue;
        }
        let text = match &row[value] {
            SqlValue::Text(text) | SqlValue::Bare(text) => {
                String::from_utf8_lossy(text).into_owned()
            }
            SqlValue::Null => return Err(MobSkillError::NoPowerRow),
        };
        let power = text
            .split_ascii_whitespace()
            .map(|entry| {
                entry
                    .parse::<i32>()
                    .map_err(|_| MobSkillError::NotAPower(entry.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if power.len() <= SKILL_MAX_LEVEL {
            return Err(MobSkillError::ShortPower(power.len()));
        }
        return Ok(power);
    }
    Err(MobSkillError::NoPowerRow)
}

/// One splash entry of a skill clip: `TMobSplashAttackInfo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplashEntry {
    /// `dwTiming`: `100 + (DWORD)(StartingTime * 1000)`.
    pub timing_ms: u32,
    /// `dwHitDistance`: `-(DWORD)(y)`.
    pub hit_distance: u32,
}

/// The skill data of one `SPECIAL` clip.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipSkill {
    /// `MotionDuration`, in seconds.
    pub duration_s: f32,
    /// The type-4 events, in file order.
    pub splash: Vec<SplashEntry>,
}

/// Read the skill data of one `.msa` clip.
///
/// `timing = 100 + StartingTime*1000` truncated toward zero; a negative time refuses
/// the file (the C cast is UB there, and no owned clip has one). `distance =
/// -(DWORD)(y)` truncated toward zero then wrapped: PROVISIONAL for negative y (no
/// i686 probe on this machine; assumed small-positive, falsifiable in play).
///
/// # Errors
///
/// Returns [`MsaError`] for a clip without a duration, a malformed event, or a
/// negative starting time.
pub fn splash_of(clip: &str, file: &str) -> Result<ClipSkill, MsaError> {
    let duration_s = find_float(clip, "MotionDuration").ok_or_else(|| MsaError {
        file: file.to_string(),
        what: "no MotionDuration",
    })?;
    let mut splash = Vec::new();
    for event in clip.split("Group Event") {
        let Some(type_at) = event.find("MotionEventType") else {
            continue;
        };
        let after = &event[type_at + "MotionEventType".len()..];
        if event_number(after) != Some(4) {
            continue;
        }
        let Some(time) = find_float(event, "StartingTime") else {
            return Err(MsaError {
                file: file.to_string(),
                what: "type-4 event without StartingTime",
            });
        };
        if time < 0.0 {
            return Err(MsaError {
                file: file.to_string(),
                what: "negative StartingTime",
            });
        }
        let Some(sphere) = event
            .find("spheredata")
            .or_else(|| event.find("SphereData"))
        else {
            return Err(MsaError {
                file: file.to_string(),
                what: "type-4 event without spheredata",
            });
        };
        let after = &event[sphere..];
        let Some(pos) = after.find("Position") else {
            return Err(MsaError {
                file: file.to_string(),
                what: "type-4 event without Position",
            });
        };
        let numbers = event_numbers(&after[pos + "Position".len()..]);
        if numbers.len() < 3 {
            return Err(MsaError {
                file: file.to_string(),
                what: "short Position",
            });
        };
        let timing_ms = 100 + common::cfloat::f32_to_u32(time * 1000.0);
        let negated = common::cfloat::f32_to_i32(numbers[1]).wrapping_neg();
        let hit_distance = u32::from_ne_bytes(negated.to_ne_bytes());
        splash.push(SplashEntry {
            timing_ms,
            hit_distance,
        });
    }
    Ok(ClipSkill { duration_s, splash })
}

fn find_float(clip: &str, token: &str) -> Option<f32> {
    let at = clip.find(token)? + token.len();
    event_numbers(&clip[at..]).first().copied()
}

fn event_number(after: &str) -> Option<i32> {
    event_numbers(after)
        .first()
        .map(|value| common::cfloat::f32_to_i32(*value))
}

fn event_numbers(after: &str) -> Vec<f32> {
    let mut numbers = Vec::new();
    let mut current = String::new();
    let mut in_number = false;
    for byte in after.bytes() {
        if byte.is_ascii_digit() || byte == b'.' || byte == b'-' || byte == b'+' {
            current.push(byte as char);
            in_number = true;
        } else if in_number {
            if let Ok(value) = current.parse::<f32>() {
                numbers.push(value);
            }
            current.clear();
            in_number = false;
        }
    }
    if in_number {
        if let Ok(value) = current.parse::<f32>() {
            numbers.push(value);
        }
    }
    numbers
}

/// A clip that carries no skill data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MsaError {
    /// The clip file.
    pub file: String,
    /// What was wrong.
    pub what: &'static str,
}

impl fmt::Display for MsaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.file, self.what)
    }
}

impl std::error::Error for MsaError {}

/// A mob skill table that failed to load.
#[derive(Debug, Clone, PartialEq)]
pub enum MobSkillError {
    /// A dump that does not parse.
    Dump(SqlDumpError),
    /// A missing column.
    NoColumn {
        /// The row's vnum (0 when the vnum itself was unreadable).
        vnum: u32,
        /// The column.
        column: String,
    },
    /// A non-numeric number cell.
    NotANumber {
        /// The column.
        column: String,
        /// The cell text.
        text: String,
    },
    /// A `setFlag` token outside [`SKILL_FLAG_ORDER`].
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
    /// A poly outside the hosted subset.
    Poly {
        /// The row's vnum.
        vnum: u32,
        /// Which poly.
        what: &'static str,
        /// The parse error.
        error: PolyError,
    },
    /// A missing mob power row.
    NoPowerRow,
    /// A non-numeric power entry.
    NotAPower(String),
    /// A power row too short to index 0-40.
    ShortPower(usize),
}

impl fmt::Display for MobSkillError {
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
            Self::NoPowerRow => write!(f, "no SKILL_POWER_BY_LEVEL_TYPE0 row"),
            Self::NotAPower(entry) => write!(f, "power entry not a number: {entry:?}"),
            Self::ShortPower(len) => write!(f, "power row has {len} entries, need 41+"),
        }
    }
}

impl std::error::Error for MobSkillError {}

/// The motlist's `SPECIAL` fields, by skill slot (`G/motion.cpp:91-95`):
/// `SPECIAL`, `SPECIAL1`, `SPECIAL2`, `SPECIAL3`, `SPECIAL4`.
pub const SPECIAL_FIELDS: [&str; 5] = ["SPECIAL", "SPECIAL1", "SPECIAL2", "SPECIAL3", "SPECIAL4"];

/// One slot's clip data: the duration and splash of its `SPECIALn` clip.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SlotClip {
    /// The clip's `MotionDuration`, or `None` without a line or file.
    pub duration_s: Option<f32>,
    /// The clip's splash entries, in file order.
    pub splash: Vec<SplashEntry>,
}

/// The skill clips of every folder: `CMotionManager`'s `SPECIALn` table
/// (`G/motion.cpp:327-347`).
#[derive(Debug, Clone, Default)]
pub struct MobSkillClips {
    by_folder: BTreeMap<String, [SlotClip; 5]>,
    /// The `NORMAL_ATTACK` clip's `MotionDuration` per folder, or `None` without
    /// the line or the file (`GetMotionFileName`'s `MOTION_NORMAL_ATTACK` arm,
    /// `G/motion.cpp:91-95`, through `LoadMotion`). Parsed from the clip like a
    /// skill duration; a malformed one is skipped and recorded like a bad skill
    /// clip.
    attack: BTreeMap<String, Option<f32>>,
    /// Every clip that failed to parse (warned, empty — legacy's `sys_err` arm).
    pub skipped: Vec<String>,
}

impl MobSkillClips {
    /// Load the clips of the folders `protos` name: each folder's `NORMAL_ATTACK`
    /// duration, and, for one carrying skill slots, the motlist's `SPECIALn` file
    /// per slot with a vnum, parsed with [`splash_of`]. A missing line or file
    /// leaves the slot empty; a malformed clip is skipped and recorded (legacy
    /// logs `sys_err` and keeps the empty vector). The motlist line is
    /// whitespace-split into four cells; a `GENERAL NORMAL_ATTACK` line with
    /// fewer than four names no file, as legacy's `strtok` arm reads the fourth
    /// before it compares the second. The match is exactly `NORMAL_ATTACK`
    /// (legacy's `strcasecmp` against the field), so a `NORMAL_ATTACK1` line on
    /// its own is not the swing clip.
    pub fn load(monster_dir: &std::path::Path, protos: &crate::mob_proto::MobProtos) -> Self {
        let mut clips = Self::default();
        for proto in protos.rows() {
            let folder = String::from_utf8_lossy(&proto.table.folder)
                .trim_end_matches('\0')
                .to_string();
            if folder.is_empty() || clips.attack.contains_key(&folder) {
                continue;
            }
            let motlist = monster_dir.join(&folder).join("motlist.txt");
            let text =
                std::fs::read(&motlist).map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
            let mut slots: [SlotClip; 5] = Default::default();
            let mut attack: Option<f32> = None;
            if let Ok(text) = text {
                let lines: Vec<Vec<&str>> = text
                    .lines()
                    .map(|line| line.split_whitespace().collect())
                    .collect();
                let skill_folder = proto.table.skills.iter().any(|slot| slot.vnum != 0);
                if skill_folder {
                    for (slot, field) in SPECIAL_FIELDS.iter().enumerate() {
                        if proto.table.skills[slot].vnum == 0 {
                            continue;
                        }
                        let file = lines.iter().find_map(|cells| {
                            (cells.len() >= 3 && cells[1] == *field).then(|| cells[2])
                        });
                        let Some(file) = file else { continue };
                        let path = monster_dir.join(&folder).join(file);
                        match std::fs::read(&path)
                            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                        {
                            Ok(clip) => match splash_of(&clip, file) {
                                Ok(skill) => {
                                    slots[slot] = SlotClip {
                                        duration_s: Some(skill.duration_s),
                                        splash: skill.splash.clone(),
                                    };
                                }
                                Err(_) => {
                                    clips.skipped.push(path.display().to_string());
                                }
                            },
                            Err(_) => {
                                clips.skipped.push(path.display().to_string());
                            }
                        }
                    }
                    clips.by_folder.insert(folder.clone(), slots);
                }
                let file = lines.iter().find_map(|cells| {
                    (cells.len() >= 4
                        && cells[0].eq_ignore_ascii_case("GENERAL")
                        && cells[1].eq_ignore_ascii_case("NORMAL_ATTACK"))
                    .then(|| cells[2])
                });
                if let Some(file) = file {
                    let path = monster_dir.join(&folder).join(file);
                    match std::fs::read(&path)
                        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                    {
                        Ok(clip) => match find_float(&clip, "MotionDuration") {
                            Some(seconds) => {
                                attack = Some(seconds);
                            }
                            None => {
                                clips.skipped.push(path.display().to_string());
                            }
                        },
                        Err(_) => {
                            clips.skipped.push(path.display().to_string());
                        }
                    }
                }
            }
            clips.attack.insert(folder, attack);
        }
        clips
    }

    /// The folder's `NORMAL_ATTACK` duration, or `None` without the line, the
    /// file, or a parseable clip.
    #[must_use]
    pub fn attack_s(&self, folder: &str) -> Option<f32> {
        self.attack.get(folder).copied().flatten()
    }

    /// The folder's 5 slots, or all-empty for an unknown folder.
    #[must_use]
    pub fn slots(&self, folder: &str) -> &[SlotClip; 5] {
        static EMPTY: [SlotClip; 5] = [
            SlotClip {
                duration_s: None,
                splash: Vec::new(),
            },
            SlotClip {
                duration_s: None,
                splash: Vec::new(),
            },
            SlotClip {
                duration_s: None,
                splash: Vec::new(),
            },
            SlotClip {
                duration_s: None,
                splash: Vec::new(),
            },
            SlotClip {
                duration_s: None,
                splash: Vec::new(),
            },
        ];
        self.by_folder.get(folder).unwrap_or(&EMPTY)
    }
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
    fn hosted_rows_decode_from_owner_dump() {
        let skills = MobSkills::from_dumps(&dump("player.sql"), &dump("common.sql")).unwrap();
        for vnum in HOSTED_SKILL_VNUMS {
            assert!(skills.get(vnum).is_some(), "missing {vnum}");
        }
        assert!(skills.get(270).is_none());
        // 20: the PC-row-shaped mob crush: MELEE, ATTACK + USE_MELEE + SPLASH + CRUSH.
        let crush = skills.get(20).unwrap();
        assert_eq!(crush.record.skill_attr_type, SKILL_ATTR_MELEE);
        assert_eq!(
            crush.record.flag,
            SKILL_FLAG_ATTACK | SKILL_FLAG_USE_MELEE_DAMAGE | SKILL_FLAG_SPLASH | SKILL_FLAG_CRUSH
        );
        assert_eq!(crush.record.max_hit, 10);
        assert_eq!(crush.record.splash_range, 200);
        // 263: the slow: MELEE, no PARTY, no SELFONLY.
        let slow = skills.get(263).unwrap();
        assert_eq!(slow.record.skill_attr_type, SKILL_ATTR_MELEE);
        assert_eq!(slow.record.flag & SKILL_FLAG_PARTY, 0);
        assert_eq!(slow.record.splash_range, 6000);
        // 265: the party heal — whose PARTY test always fails (bit 28 vs 27).
        let heal = skills.get(265).unwrap();
        assert_eq!(heal.record.flag, 1 << 28);
        assert_eq!(heal.record.flag & SKILL_FLAG_PARTY, 0);
        // 266: the silent self splash: NORMAL, SELFONLY + SPLASH, no ATTACK.
        let silent = skills.get(266).unwrap();
        assert_eq!(silent.record.skill_attr_type, SKILL_ATTR_NORMAL);
        assert_eq!(silent.record.flag, SKILL_FLAG_SELFONLY | SKILL_FLAG_SPLASH);
    }

    #[test]
    fn power_row_clamps_like_skill_power() {
        let skills = MobSkills::from_dumps(&dump("player.sql"), &dump("common.sql")).unwrap();
        assert_eq!(skills.power(0), 0);
        assert_eq!(skills.power(1), 5);
        assert_eq!(skills.power(40), skills.power(41));
        assert_eq!(skills.power(u8::MAX), skills.power(40));
    }

    #[test]
    fn splash_entry_math_truncates_toward_zero() {
        let clip = "Group MotionData\nMotionDuration 1.500\nGroup Event\nMotionEventType 4\nStartingTime 0.250\nspheredata 0 0 0\nPosition 0.0 -150.0 0.0\n";
        let clip = splash_of(clip, "test.msa").unwrap();
        assert!((clip.duration_s - 1.5).abs() < 1e-6);
        assert_eq!(
            clip.splash,
            vec![SplashEntry {
                timing_ms: 350,
                hit_distance: 150,
            }]
        );
    }

    #[test]
    fn negative_time_refuses_the_clip() {
        let clip = "MotionDuration 1.0\nGroup Event\nMotionEventType 4\nStartingTime -0.1\nspheredata 0\nPosition 0 0 0\n";
        assert!(splash_of(clip, "neg.msa").is_err());
    }

    #[test]
    fn non_splash_events_are_skipped() {
        let clip = "MotionDuration 1.0\nGroup Event\nMotionEventType 1\nStartingTime 0.1\n";
        let clip = splash_of(clip, "none.msa").unwrap();
        assert!(clip.splash.is_empty());
    }

    #[test]
    fn clips_load_from_owner_folders() {
        use crate::mob_proto::MobProtos;
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata");
        let protos = MobProtos::load(&root.join("proto")).unwrap();
        let clips = MobSkillClips::load(&root.join("data/monster"), &protos);
        assert!(clips.skipped.is_empty(), "skipped: {:?}", clips.skipped);
        // The gorilla boss's slot 0: skill00.msa, 3 splash entries, the y=+150
        // wrap ((DWORD)(150.0) = 150, negated in u32).
        let slots = clips.slots("plechi_demonic_gorillaBoss");
        let duration = slots[0].duration_s.unwrap();
        assert!((duration - 3.933_334).abs() < 1e-5, "{duration}");
        assert_eq!(slots[0].splash.len(), 3);
        assert_eq!(slots[0].splash[0].hit_distance, 500);
        assert_eq!(slots[0].splash[1].hit_distance, 4_294_967_146);
        assert_eq!(slots[0].splash[0].timing_ms, 100 + 1063);
        // An unknown folder is all-empty.
        let empty = clips.slots("no_such_folder");
        assert!(empty.iter().all(|slot| slot.duration_s.is_none()));
    }

    #[test]
    fn attack_durations_load_from_owner_folders() {
        use crate::mob_proto::MobProtos;
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata");
        let protos = MobProtos::load(&root.join("proto")).unwrap();
        let clips = MobSkillClips::load(&root.join("data/monster"), &protos);
        assert!(clips.skipped.is_empty(), "skipped: {:?}", clips.skipped);
        // The stray dog's swing clip (`20.msa`), not its `NORMAL_ATTACK1`.
        let seconds = clips.attack_s("stray_dog").unwrap();
        assert!((seconds - 0.933_333).abs() < 1e-5, "{seconds}");
        // A skill folder keeps both: the gorilla boss's swing and slot 0.
        assert!(clips.attack_s("plechi_demonic_gorillaBoss").is_some());
        assert!(clips.slots("plechi_demonic_gorillaBoss")[0]
            .duration_s
            .is_some());
        // A folder without the line reads none (`boar`); an unknown one too.
        assert_eq!(clips.attack_s("boar"), None);
        assert_eq!(clips.attack_s("no_such_folder"), None);
    }
}
