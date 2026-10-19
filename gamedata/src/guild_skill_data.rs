//! The guild skill rows: the 151-162 `skill_proto` rows `/gskillup` climbs and
//! `GUILD_SUBHEADER_CG_USE_SKILL` casts (ledger 230r).
//!
//! Legacy reads every skill row from one table (`CSkillManager::Initialize`,
//! `G/skill.cpp`), and the guild arms read the same rows by vnum (`CGuild::
//! SkillLevelUp` and `CGuild::UseSkill` both call `CSkillManager::instance().
//! Get(dwVnum)`, `G/guild.cpp:1242,1287`). The PC tables refuse the guild range
//! (`gamedata::pc_skill_data` keeps `bType` 1-4 plus the always-usable type-0 rows,
//! and 151-162 are type 0 outside that list), so the guild rows load here, from the
//! same `player.sql` dump, with the same decoders.
//!
//! Each kept row carries its compiled polys ([`common::skill_poly::Poly`]): the
//! point pair, the SP cost, the duration pair, the duration SP cost, and the
//! cooldown. A row whose poly falls outside the hosted subset is refused with its
//! vnum, mirroring legacy's `sys_err` + skip; on the owner's data all 12 rows load
//! (the empty polys compile to 0, as the blank-source arm does).
//!
//! The rows the port reads but never casts through: 158's row exists (max 3) but the
//! teleport arm branches before any poly runs, and the war gate answers `[LS;937]`
//! before the buff loop while no war runs, so the point/duration polys of 152-157
//! wait for `sys.guild.war`. The cost and cooldown polys run on every cast.

use std::collections::BTreeMap;
use std::fmt;

use common::skill_poly::{Poly, PolyError};

use crate::records::SkillTableRecord;
use crate::sql_dump::{read_table, SqlDumpError, SqlValue};

/// `GUILD_SKILL_START` (`G/skill.h:163`): 151.
pub const GUILD_SKILL_START: u32 = 151;
/// `GUILD_SKILL_END` (`G/skill.h:173`): 162.
pub const GUILD_SKILL_END: u32 = 162;
/// `GUILD_SKILL_TELEPORT` (`G/skill.h:170`): 158.
pub const GUILD_SKILL_TELEPORT: u32 = 158;

/// One hosted guild skill row, with its polys compiled.
#[derive(Debug, Clone)]
pub struct GuildSkill {
    /// The `skill_proto` row.
    pub record: SkillTableRecord,
    /// `kPointPoly`.
    pub point: Poly,
    /// `kPointPoly2`.
    pub point2: Poly,
    /// `kSPCostPoly`.
    pub sp_cost: Poly,
    /// `kCooldownPoly`.
    pub cooldown: Poly,
    /// `kDurationPoly`.
    pub duration: Poly,
    /// `kDurationPoly2`.
    pub duration2: Poly,
    /// `kDurationSPCostPoly`.
    pub duration_sp_cost: Poly,
}

/// The hosted guild skill rows, by vnum.
#[derive(Debug, Clone, Default)]
pub struct GuildSkills {
    /// The rows that parsed, by vnum.
    by_vnum: BTreeMap<u32, GuildSkill>,
    /// The vnums refused for an unparsable poly, in dump order.
    refused: Vec<u32>,
}

/// What a guild skill load refuses.
#[derive(Debug)]
pub enum GuildSkillError {
    /// The dump does not parse.
    Dump(SqlDumpError),
    /// A `NoColumn` for a missing column, a `NotANumber` for a bad number, an
    /// `UnknownFlag` for a bad flag, an `UnknownAttr` for a bad attr, an
    /// `UnknownAffect` for a bad affect.
    Row {
        /// The vnum being decoded.
        vnum: u32,
        /// What refused.
        what: &'static str,
        /// The offending text.
        text: String,
    },
    /// A poly falls outside the hosted subset.
    Poly {
        /// The vnum being decoded.
        vnum: u32,
        /// Which poly.
        what: &'static str,
        /// The parse error.
        error: PolyError,
    },
}

impl fmt::Display for GuildSkillError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dump(error) => write!(f, "guild skills: {error}"),
            Self::Row { vnum, what, text } => {
                write!(f, "guild skill {vnum}: {what} {text:?}")
            }
            Self::Poly { vnum, what, error } => {
                write!(f, "guild skill {vnum}: {what} poly: {error:?}")
            }
        }
    }
}

impl std::error::Error for GuildSkillError {}

impl GuildSkills {
    /// Read the 151-162 rows from the `player.sql` dump. Rows outside the range are
    /// skipped; a row whose poly refuses stays out of the map, as legacy's `sys_err`
    /// + skip leaves it uncastable.
    ///
    /// # Errors
    ///
    /// Returns [`GuildSkillError`] for a dump that does not parse or a kept row
    /// whose numbers, flag, attr or affect fall outside the hosted subset.
    pub fn from_dump(player: &[u8]) -> Result<Self, GuildSkillError> {
        let table = read_table(player, "skill_proto").map_err(GuildSkillError::Dump)?;
        let mut by_vnum = BTreeMap::new();
        let mut refused = Vec::new();
        for row in &table.rows {
            let vnum = number_at(&table, row, "dwVnum")?;
            if !(GUILD_SKILL_START..=GUILD_SKILL_END).contains(&vnum) {
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
                Poly::parse(&text(poly)).map_err(|error| GuildSkillError::Poly {
                    vnum,
                    what,
                    error,
                })
            };
            let compiled = (|| {
                Ok::<_, GuildSkillError>(GuildSkill {
                    record,
                    point: compile(&row_poly(&table, row, "szPointPoly")?, "point")?,
                    point2: compile(&row_poly(&table, row, "szPointPoly2")?, "point2")?,
                    sp_cost: compile(&row_poly(&table, row, "szSPCostPoly")?, "sp_cost")?,
                    cooldown: compile(&row_poly(&table, row, "szCooldownPoly")?, "cooldown")?,
                    duration: compile(&row_poly(&table, row, "szDurationPoly")?, "duration")?,
                    duration2: compile(&row_poly(&table, row, "szDurationPoly2")?, "duration2")?,
                    duration_sp_cost: compile(
                        &row_poly(&table, row, "szDurationSPCostPoly")?,
                        "duration_sp_cost",
                    )?,
                })
            })();
            match compiled {
                Ok(skill) => {
                    by_vnum.insert(vnum, skill);
                }
                Err(GuildSkillError::Poly { .. }) => refused.push(vnum),
                Err(other) => return Err(other),
            }
        }
        Ok(Self { by_vnum, refused })
    }

    /// The hosted row for `vnum`, if it parsed.
    #[must_use]
    pub fn get(&self, vnum: u32) -> Option<&GuildSkill> {
        self.by_vnum.get(&vnum)
    }

    /// The vnums refused for an unparsable poly, in dump order.
    #[must_use]
    pub fn refused(&self) -> &[u32] {
        &self.refused
    }

    /// The hosted vnums, in order.
    #[must_use]
    pub fn vnums(&self) -> Vec<u32> {
        self.by_vnum.keys().copied().collect()
    }
}

fn number_at(
    table: &crate::sql_dump::SqlTable,
    row: &[SqlValue],
    column: &'static str,
) -> Result<u32, GuildSkillError> {
    let Some(index) = table.column(column) else {
        return Err(GuildSkillError::Row {
            vnum: 0,
            what: "no column",
            text: column.to_string(),
        });
    };
    match &row[index] {
        SqlValue::Bare(text) | SqlValue::Text(text) => {
            let text = String::from_utf8_lossy(text);
            text.trim()
                .parse::<u32>()
                .map_err(|_| GuildSkillError::Row {
                    vnum: 0,
                    what: "not a number",
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
) -> Result<Vec<u8>, GuildSkillError> {
    let Some(index) = table.column(column) else {
        return Err(GuildSkillError::Row {
            vnum: 0,
            what: "no column",
            text: column.to_string(),
        });
    };
    match &row[index] {
        SqlValue::Bare(text) | SqlValue::Text(text) => Ok(text.clone()),
        SqlValue::Null => Ok(Vec::new()),
    }
}

/// Decode one dump row into the record shape. The flag/attr decoders live in
/// `pc_skill_data`'s shape; the guild range only ever sets SELFONLY or nothing, but
/// the mask decodes the whole order so a future row cannot slip through half-read.
fn decode_row(
    table: &crate::sql_dump::SqlTable,
    row: &[SqlValue],
    vnum: u32,
) -> Result<SkillTableRecord, GuildSkillError> {
    use crate::mob_skill_data::SKILL_FLAG_ORDER;
    let bytes_of = |column: &'static str| -> Result<Vec<u8>, GuildSkillError> {
        let Some(index) = table.column(column) else {
            return Err(GuildSkillError::Row {
                vnum,
                what: "no column",
                text: column.to_string(),
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
    let copy_into = |field: &mut [u8], bytes: &[u8]| {
        let len = bytes.len().min(field.len());
        field[..len].copy_from_slice(&bytes[..len]);
    };
    copy_into(&mut record.name, &bytes_of("szName")?);
    record.skill_type = u8::try_from(number_at(table, row, "bType")?).unwrap_or(u8::MAX);
    record.max_level = u8::try_from(number_at(table, row, "bMaxLevel")?).unwrap_or(u8::MAX);
    record.level_limit = u8::try_from(number_at(table, row, "bLevelLimit")?).unwrap_or(u8::MAX);
    let flag_field = bytes_of("setFlag")?;
    let flag_field = String::from_utf8_lossy(&flag_field);
    let flag_field = flag_field.trim();
    let mut mask = 0u32;
    if !flag_field.is_empty() && flag_field != "NULL" {
        for token in flag_field.split(',') {
            let token = token.trim();
            let Some(bit) = SKILL_FLAG_ORDER.iter().position(|known| *known == token) else {
                return Err(GuildSkillError::Row {
                    vnum,
                    what: "unknown flag",
                    text: token.to_string(),
                });
            };
            mask |= 1 << bit;
        }
    }
    record.flag = mask;
    copy_into(&mut record.point_on, &bytes_of("szPointOn")?);
    copy_into(&mut record.point_on2, &bytes_of("szPointOn2")?);
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The owner's dump header for `skill_proto`, narrowed to the columns the guild
    /// rows read.
    const HEADER: &str = "(`dwVnum`, `szName`, `bType`, `bMaxLevel`, `bLevelLimit`, \
        `szPointOn`, `szPointPoly`, `szSPCostPoly`, `szDurationPoly`, \
        `szDurationSPCostPoly`, `szCooldownPoly`, `setFlag`, `szPointOn2`, \
        `szPointPoly2`, `szDurationPoly2`)";

    fn dump(rows: &[&str]) -> Vec<u8> {
        let mut out = format!("INSERT INTO `skill_proto` {HEADER} VALUES ");
        out.push_str(&rows.join(","));
        out.push(';');
        out.into_bytes()
    }

    #[test]
    fn the_guild_range_loads_and_the_rest_is_skipped() {
        let rows = dump(&[
            "(150,'x',0,1,0,'NONE','','','','','',NULL,'NONE','','')",
            "(151,'Ochi de Dragon',0,7,0,'NONE','','','','','',NULL,'NONE','','')",
            "(152,'Sangele Zeului Dragon',0,7,0,'MAX_HP','maxhp*0.2*k','150+150*k','300','','600',NULL,'NONE','','')",
            "(163,'x',0,1,0,'NONE','','','','','',NULL,'NONE','','')",
        ]);
        let skills = GuildSkills::from_dump(&rows).expect("the dump parses");
        assert_eq!(skills.vnums(), vec![151, 152]);
        assert!(skills.refused().is_empty());
        let row = skills.get(152).expect("152 loads");
        assert_eq!(row.record.max_level, 7);
        assert_eq!(row.record.flag, 0);
        assert_eq!(
            common::cfloat::f64_to_i32(
                row.sp_cost
                    .eval(&std::collections::BTreeMap::from([("k", 1.0 / 7.0)]))
            ),
            171
        );
    }

    #[test]
    fn selfonly_and_the_empty_cost_read() {
        let rows = dump(&[
            "(156,'Furia Zeului Dragon',0,7,0,'CRITICAL','50*k','150+150*k','180','','480','SELFONLY','NONE','','')",
            "(158,'PASSIVANONAME2',0,3,0,'NONE','','','','','',SELFONLY,'NONE','','')",
        ]);
        let skills = GuildSkills::from_dump(&rows).expect("the dump parses");
        let fury = skills.get(156).expect("156 loads");
        assert_eq!(fury.record.flag, crate::mob_skill_data::SKILL_FLAG_SELFONLY);
        let teleport = skills.get(158).expect("158 loads");
        assert_eq!(teleport.record.max_level, 3);
        assert_eq!(
            teleport
                .sp_cost
                .eval(&std::collections::BTreeMap::from([("k", 1.0)]))
                .to_bits(),
            0.0f64.to_bits()
        );
        assert_eq!(
            teleport
                .cooldown
                .eval(&std::collections::BTreeMap::from([("k", 1.0)]))
                .to_bits(),
            0.0f64.to_bits()
        );
    }

    #[test]
    fn a_bad_poly_refuses_its_row_and_a_bad_flag_fails_the_load() {
        let rows = dump(&[
            "(152,'x',0,7,0,'MAX_HP','maxhp*(','150+150*k','300','','600',NULL,'NONE','','')",
        ]);
        let skills = GuildSkills::from_dump(&rows).expect("the load stays green");
        assert!(skills.get(152).is_none());
        assert_eq!(skills.refused(), &[152]);
        let rows =
            dump(&["(152,'x',0,7,0,'MAX_HP','maxhp','150','300','','600','BOGUS','NONE','','')"]);
        assert!(GuildSkills::from_dump(&rows).is_err());
    }
}
