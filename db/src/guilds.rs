//! The guild tables: legacy's `guild`, `guild_grade`, `guild_member` and `guild_comment`
//! (`legacy/sql/schema/player.sql:139-211`, migrations `0011_guild.sql` and
//! `0012_guild_comment.sql`), plus the mark and symbol rows legacy kept in files
//! (`mark/mark_index`, the five TGA files, `guild_symbol.tga`; migration
//! `0013_guild_mark.sql`).
//!
//! The world loads every guild at boot (`CGuildManager::Initialize`,
//! `G/guild_manager.cpp:212-243`) and serves creation and membership from memory, so the
//! store owns the rows and the world owns the judgements. A creation is one transaction —
//! the guild row, the 15 grade rows and the master's member row — because legacy's
//! constructor writes them together (`CGuild::CGuild`, `G/guild.cpp:59-127`).

use sqlx::Row;

use crate::accounts::AccountError;
use crate::store::Store;

/// One guild row, as [`load_all`] reads it: the columns `LoadGuildData` reads
/// (`G/guild.cpp:602-640`), without the unported dungeon triple.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGuild {
    /// The guild id.
    pub id: u32,
    /// `TGuildData::name`.
    pub name: String,
    /// `TGuildData::master_pid`.
    pub master_pid: u32,
    /// `TGuildData::power` (the legacy `sp` column).
    pub sp: i32,
    /// `TGuildData::level`.
    pub level: u8,
    /// `TGuildData::exp`.
    pub exp: u32,
    /// `TGuildData::skill_point`.
    pub skill_point: u8,
    /// `TGuildData::abySkill`: the 12 skill bytes.
    pub skills: Vec<u8>,
    /// `TGuildData::ladder_point`.
    pub ladder_point: i32,
    /// `TGuildData::win`.
    pub win: i32,
    /// `TGuildData::draw`.
    pub draw: i32,
    /// `TGuildData::loss`.
    pub loss: i32,
    /// `TGuildData::gold`.
    pub gold: i32,
}

/// One grade row, as [`load_all`] reads it: the columns `LoadGuildGradeData` reads
/// (`G/guild.cpp:576-600`). Grades outside 1-15 are dropped, as legacy drops them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGrade {
    /// The guild id.
    pub guild_id: u32,
    /// The grade, 1 to 15.
    pub grade: u8,
    /// `TGuildGrade::grade_name`.
    pub name: String,
    /// `TGuildGrade::auth_flag`.
    pub auth: u8,
}

/// One member row, as [`load_all`] reads it, with the name, job and level the load's join
/// reads from the player row (`G/guild.cpp:673-675`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMember {
    /// `TGuildMember::pid`.
    pub pid: u32,
    /// The guild id.
    pub guild_id: u32,
    /// `TGuildMember::grade`.
    pub grade: u8,
    /// `TGuildMember::is_general`.
    pub is_general: bool,
    /// `TGuildMember::offer_exp`.
    pub offer: u32,
    /// The player row's level.
    pub level: u8,
    /// The player row's job.
    pub job: u8,
    /// The player row's name.
    pub name: String,
}

/// Every guild: the rows, the grades and the members.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoredGuilds {
    /// The guild rows, in id order.
    pub guilds: Vec<StoredGuild>,
    /// The grade rows, by guild id, in grade order.
    pub grades: Vec<StoredGrade>,
    /// The member rows, by guild id, in pid order.
    pub members: Vec<StoredMember>,
}

/// One comment row, as [`board`] reads it: the columns `RefreshCommentForce` reads
/// (`G/guild.cpp:1104`), in the order it reads them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredComment {
    /// The row id.
    pub id: u32,
    /// The poster's name.
    pub name: String,
    /// The content.
    pub content: String,
}

/// Load every guild with its grades and members, as `CGuildManager::Initialize` loads them
/// (`G/guild_manager.cpp:212-243`) through `CGuild::Load` (`G/guild.cpp:642-675`).
///
/// # Errors
///
/// Returns [`AccountError::Database`], including for a stored value outside its range,
/// which the migration's checks make unreachable.
pub async fn load_all(store: &Store) -> Result<StoredGuilds, AccountError> {
    let rows = sqlx::query(
        "SELECT id, name, master_pid, sp, level, exp, skill_point, skills, ladder_point, win, \
         draw, loss, gold FROM guild ORDER BY id",
    )
    .fetch_all(store.pool())
    .await?;
    let mut guilds = Vec::with_capacity(rows.len());
    for row in &rows {
        let id: i32 = row.try_get("id")?;
        let master_pid: i32 = row.try_get("master_pid")?;
        let level: i16 = row.try_get("level")?;
        let exp: i32 = row.try_get("exp")?;
        let skill_point: i16 = row.try_get("skill_point")?;
        let skills: Vec<u8> = row.try_get("skills")?;
        guilds.push(StoredGuild {
            id: u32::try_from(id).map_err(|_| AccountError::NoSuchPlayer(0))?,
            name: row.try_get("name")?,
            master_pid: u32::try_from(master_pid).map_err(|_| AccountError::NoSuchPlayer(0))?,
            sp: row.try_get("sp")?,
            level: u8::try_from(level).map_err(|_| AccountError::NoSuchPlayer(0))?,
            exp: u32::try_from(exp).map_err(|_| AccountError::NoSuchPlayer(0))?,
            skill_point: u8::try_from(skill_point).map_err(|_| AccountError::NoSuchPlayer(0))?,
            skills,
            ladder_point: row.try_get("ladder_point")?,
            win: row.try_get("win")?,
            draw: row.try_get("draw")?,
            loss: row.try_get("loss")?,
            gold: row.try_get("gold")?,
        });
    }
    let rows =
        sqlx::query("SELECT guild_id, grade, name, auth FROM guild_grade ORDER BY guild_id, grade")
            .fetch_all(store.pool())
            .await?;
    let mut grades = Vec::with_capacity(rows.len());
    for row in &rows {
        let guild_id: i32 = row.try_get("guild_id")?;
        let grade: i16 = row.try_get("grade")?;
        let auth: i16 = row.try_get("auth")?;
        grades.push(StoredGrade {
            guild_id: u32::try_from(guild_id).map_err(|_| AccountError::NoSuchPlayer(0))?,
            grade: u8::try_from(grade).map_err(|_| AccountError::NoSuchPlayer(0))?,
            name: row.try_get("name")?,
            auth: u8::try_from(auth).map_err(|_| AccountError::NoSuchPlayer(0))?,
        });
    }
    let rows = sqlx::query(
        "SELECT m.pid, m.guild_id, m.grade, m.is_general, m.offer, p.level, p.job, p.name \
         FROM guild_member m JOIN player p ON p.id = m.pid ORDER BY m.guild_id, m.pid",
    )
    .fetch_all(store.pool())
    .await?;
    let mut members = Vec::with_capacity(rows.len());
    for row in &rows {
        let pid: i32 = row.try_get("pid")?;
        let guild_id: i32 = row.try_get("guild_id")?;
        let grade: i16 = row.try_get("grade")?;
        let offer: i32 = row.try_get("offer")?;
        let level: i16 = row.try_get("level")?;
        let job: i16 = row.try_get("job")?;
        members.push(StoredMember {
            pid: u32::try_from(pid).map_err(|_| AccountError::NoSuchPlayer(0))?,
            guild_id: u32::try_from(guild_id).map_err(|_| AccountError::NoSuchPlayer(0))?,
            grade: u8::try_from(grade).map_err(|_| AccountError::NoSuchPlayer(0))?,
            is_general: row.try_get("is_general")?,
            offer: u32::try_from(offer).map_err(|_| AccountError::NoSuchPlayer(0))?,
            level: u8::try_from(level).map_err(|_| AccountError::NoSuchPlayer(0))?,
            job: u8::try_from(job).map_err(|_| AccountError::NoSuchPlayer(0))?,
            name: row.try_get("name")?,
        });
    }
    Ok(StoredGuilds {
        guilds,
        grades,
        members,
    })
}

/// Whether a guild named `name` exists, comparing case-insensitively as legacy's `latin1`
/// `name = '...'` does (`CGuildManager::CreateGuild`, `G/guild_manager.cpp:95-113`).
///
/// # Errors
///
/// Returns [`AccountError::Database`].
pub async fn named(store: &Store, name: &str) -> Result<bool, AccountError> {
    let row = sqlx::query("SELECT COUNT(*) AS count FROM guild WHERE lower(name) = lower($1)")
        .bind(name)
        .fetch_one(store.pool())
        .await?;
    let count: i64 = row.try_get("count")?;
    Ok(count > 0)
}

/// The linked guild of each lobby slot's character, by slot: the id and name legacy's
/// `SendLoginSuccessPacket` reads off `GetLinkedGuild` (`G/desc.cpp:892-904`). An empty or
/// unguilded slot answers `None`.
///
/// # Errors
///
/// Returns [`AccountError::Database`], including for a stored id or name outside its range,
/// which the migration's checks make unreachable.
pub async fn lobby_guilds(
    store: &Store,
    lobby: &crate::players::Lobby,
) -> Result<[Option<(u32, String)>; crate::players::PLAYER_SLOTS], AccountError> {
    let mut linked: [Option<(u32, String)>; crate::players::PLAYER_SLOTS] = Default::default();
    for player in &lobby.players {
        let index = usize::from(player.slot);
        if index >= linked.len() {
            continue;
        }
        let id = i32::try_from(player.id).map_err(|_| AccountError::NoSuchPlayer(player.id))?;
        let row = sqlx::query(
            "SELECT g.id, g.name FROM guild_member m JOIN guild g ON g.id = m.guild_id \
             WHERE m.pid = $1",
        )
        .bind(id)
        .fetch_optional(store.pool())
        .await?;
        if let Some(row) = row {
            let guild_id: i32 = row.try_get("id")?;
            linked[index] = Some((
                u32::try_from(guild_id).map_err(|_| AccountError::NoSuchPlayer(0))?,
                row.try_get("name")?,
            ));
        }
    }
    Ok(linked)
}

/// Create a guild: the row, the 15 grade rows and the master's member row, in one
/// transaction (`CGuild::CGuild`, `G/guild.cpp:59-127`). `grades` holds the 15
/// `(name, auth)` pairs in grade order; `master_pid` is the leader's `player.id`.
///
/// Legacy inserts `sp` 1000, level 1, exp 0, skill point 0 and zeroed skills; the master
/// row is grade 1. The id is the answer, as `m_data.guild_id = uiInsertID` is legacy's.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for a master id above `i32::MAX`, and
/// [`CreateGuildError::GuildNameTaken`] for a name a concurrent creation took first (the
/// `guild_name_lower` index refuses it, and the descriptor then answers `[LS;473]`);
/// anything else is [`AccountError::Database`].
pub async fn create_guild(
    store: &Store,
    name: &str,
    master_pid: u32,
    grades: &[(&str, u8); 15],
) -> Result<u32, CreateGuildError> {
    let master = i32::try_from(master_pid)
        .map_err(|_| CreateGuildError::Account(AccountError::NoSuchPlayer(master_pid)))?;
    let mut transaction = store.pool().begin().await.map_err(CreateGuildError::from)?;
    let row = sqlx::query("INSERT INTO guild (name, master_pid) VALUES ($1, $2) RETURNING id")
        .bind(name)
        .bind(master)
        .fetch_one(&mut *transaction)
        .await
        .map_err(|error| classify_create(error, name))?;
    let id: i32 = row.try_get("id").map_err(CreateGuildError::from)?;
    for (index, (grade_name, auth)) in grades.iter().enumerate() {
        let grade = i16::try_from(index + 1)
            .map_err(|_| CreateGuildError::Account(AccountError::NoSuchPlayer(0)))?;
        sqlx::query(
            "INSERT INTO guild_grade (guild_id, grade, name, auth) VALUES ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(grade)
        .bind(*grade_name)
        .bind(i16::from(*auth))
        .execute(&mut *transaction)
        .await
        .map_err(CreateGuildError::from)?;
    }
    sqlx::query("INSERT INTO guild_member (pid, guild_id, grade) VALUES ($1, $2, 1)")
        .bind(master)
        .bind(id)
        .execute(&mut *transaction)
        .await
        .map_err(CreateGuildError::from)?;
    transaction.commit().await.map_err(CreateGuildError::from)?;
    u32::try_from(id).map_err(|_| CreateGuildError::Account(AccountError::NoSuchPlayer(master_pid)))
}

/// Add a member row: `pid` joins `guild_id` at `grade` (`RequestAddMember`'s insert,
/// `G/guild.cpp:144-163`, via `HEADER_GD_GUILD_ADD_MEMBER`). The invitee's job, level
/// and name stay on the player row, which [`load_all`]'s join reads.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for an id above `i32::MAX`, and
/// [`AccountError::Database`] otherwise — including the PID primary key's refusal of a
/// concurrent join, which the caller treats as `RequestAddMember`'s quiet duplicate
/// return (`:151-154`).
pub async fn add_member(
    store: &Store,
    guild_id: u32,
    pid: u32,
    grade: u8,
) -> Result<(), AccountError> {
    let guild = i32::try_from(guild_id).map_err(|_| AccountError::NoSuchPlayer(guild_id))?;
    let member = i32::try_from(pid).map_err(|_| AccountError::NoSuchPlayer(pid))?;
    sqlx::query("INSERT INTO guild_member (pid, guild_id, grade) VALUES ($1, $2, $3)")
        .bind(member)
        .bind(guild)
        .bind(i16::from(grade))
        .execute(store.pool())
        .await?;
    Ok(())
}

/// Delete a member row: `pid` leaves its guild (`RequestRemoveMember`'s delete,
/// `G/guild.cpp:195-213`, via `HEADER_GD_GUILD_REMOVE_MEMBER`). Answers whether a row
/// was there.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for a pid above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn remove_member(store: &Store, pid: u32) -> Result<bool, AccountError> {
    let member = i32::try_from(pid).map_err(|_| AccountError::NoSuchPlayer(pid))?;
    let done = sqlx::query("DELETE FROM guild_member WHERE pid = $1")
        .bind(member)
        .execute(store.pool())
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Rename a grade: `ChangeGradeName`'s update (`G/guild.cpp:851-855`). Answers whether
/// a row was there.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for an id above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn set_grade_name(
    store: &Store,
    guild_id: u32,
    grade: u8,
    name: &str,
) -> Result<bool, AccountError> {
    let id = i32::try_from(guild_id).map_err(|_| AccountError::NoSuchPlayer(guild_id))?;
    let done = sqlx::query("UPDATE guild_grade SET name = $1 WHERE guild_id = $2 AND grade = $3")
        .bind(name)
        .bind(id)
        .bind(i16::from(grade))
        .execute(store.pool())
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Re-auth a grade: `ChangeGradeAuth`'s update (`G/guild.cpp:881-885`). Answers
/// whether a row was there.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for an id above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn set_grade_auth(
    store: &Store,
    guild_id: u32,
    grade: u8,
    auth: u8,
) -> Result<bool, AccountError> {
    let id = i32::try_from(guild_id).map_err(|_| AccountError::NoSuchPlayer(guild_id))?;
    let done = sqlx::query("UPDATE guild_grade SET auth = $1 WHERE guild_id = $2 AND grade = $3")
        .bind(i16::from(auth))
        .bind(id)
        .bind(i16::from(grade))
        .execute(store.pool())
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Re-grade a member: the grade half of `SaveMember` (`G/guild.cpp:1660-1670`).
/// Answers whether a row was there.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for an id above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn set_member_grade(store: &Store, pid: u32, grade: u8) -> Result<bool, AccountError> {
    let member = i32::try_from(pid).map_err(|_| AccountError::NoSuchPlayer(pid))?;
    let done = sqlx::query("UPDATE guild_member SET grade = $1 WHERE pid = $2")
        .bind(i16::from(grade))
        .bind(member)
        .execute(store.pool())
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Re-flag a member's general: the `is_general` half of `SaveMember`
/// (`G/guild.cpp:1660-1670`). Answers whether a row was there.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for an id above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn set_member_general(
    store: &Store,
    pid: u32,
    is_general: bool,
) -> Result<bool, AccountError> {
    let member = i32::try_from(pid).map_err(|_| AccountError::NoSuchPlayer(pid))?;
    let done = sqlx::query("UPDATE guild_member SET is_general = $1 WHERE pid = $2")
        .bind(is_general)
        .bind(member)
        .execute(store.pool())
        .await?;
    Ok(done.rows_affected() > 0)
}

/// One judged experience offer, the whole row the descriptor stores (`OfferExp`'s
/// `HEADER_GD_GUILD_EXP_UPDATE` plus `SaveLevel`, `SaveMember` and the speaker's
/// `PointChange(POINT_EXP, -offer)`).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuildOffer {
    /// The guild id.
    pub guild_id: u32,
    /// The speaker's PID, whose `offer_exp` grows and whose `POINT_EXP` fell.
    pub pid: u32,
    /// The floored debit the speaker's `POINT_EXP` fell by.
    pub floored: u32,
    /// The levelled delta.
    pub offer: u32,
    /// The guild's judged level.
    pub level: u8,
    /// The guild's judged exp.
    pub exp: u32,
    /// The guild's judged ladder.
    pub ladder: i32,
}

/// Add an offer to a member's `offer_exp` and the guild's row, in one transaction,
/// and hold the speaker's fallen `POINT_EXP` on the player row. Answers whether the
/// guild row was there.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for an id above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn add_offer(store: &Store, offer: GuildOffer) -> Result<bool, AccountError> {
    let id =
        i32::try_from(offer.guild_id).map_err(|_| AccountError::NoSuchPlayer(offer.guild_id))?;
    let member = i32::try_from(offer.pid).map_err(|_| AccountError::NoSuchPlayer(offer.pid))?;
    let step = i64::from(offer.offer);
    let mut deal = store.pool().begin().await?;
    let done =
        sqlx::query("UPDATE guild SET exp = $1, level = $2, ladder_point = $3 WHERE id = $4")
            .bind(i64::from(offer.exp))
            .bind(i16::from(offer.level))
            .bind(offer.ladder)
            .bind(id)
            .execute(&mut *deal)
            .await?;
    sqlx::query("UPDATE guild_member SET offer = offer + $1 WHERE pid = $2")
        .bind(step)
        .bind(member)
        .execute(&mut *deal)
        .await?;
    sqlx::query("UPDATE player SET exp = exp - $2 WHERE id = $1")
        .bind(member)
        .bind(i64::from(offer.floored))
        .execute(&mut *deal)
        .await?;
    deal.commit().await?;
    Ok(done.rows_affected() > 0)
}

/// Add faith to a guild's `power` (`sp` column): `ChargeSP`'s `SendDBSkillUpdate`
/// (`G/guild.cpp:1826`), which persists only the power — the skills, the skill point
/// and the level stay as they are — with the speaker's `PointChange(POINT_GOLD, -gold)`
/// held by the player row (`:1822`). Answers whether a row was there.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for an id above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn add_power(
    store: &Store,
    guild_id: u32,
    pid: u32,
    debit: u64,
    power: i32,
) -> Result<bool, AccountError> {
    let id = i32::try_from(guild_id).map_err(|_| AccountError::NoSuchPlayer(guild_id))?;
    let member = i32::try_from(pid).map_err(|_| AccountError::NoSuchPlayer(pid))?;
    let gold = i64::try_from(debit).map_err(|_| AccountError::NoSuchPlayer(pid))?;
    let mut deal = store.pool().begin().await?;
    let done = sqlx::query("UPDATE guild SET sp = $1 WHERE id = $2")
        .bind(power)
        .bind(id)
        .execute(&mut *deal)
        .await?;
    sqlx::query("UPDATE player SET gold = gold - $2 WHERE id = $1")
        .bind(member)
        .bind(gold)
        .execute(&mut *deal)
        .await?;
    deal.commit().await?;
    Ok(done.rows_affected() > 0)
}

/// Spend one skill point into a skill: `SkillLevelUp`'s `SendDBSkillUpdate` (whole-row
/// write, `G/guild.cpp:1262`), which persists first and then changes the in-memory
/// skill and deducts the point (`:1262-1264`). The judge hands over the post-commit
/// bytes and point, so the store writes exactly what the commit stores. Answers
/// whether a row was there.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for an id above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn set_skills(
    store: &Store,
    guild_id: u32,
    skills: &[u8; 12],
    skill_point: u8,
) -> Result<bool, AccountError> {
    let id = i32::try_from(guild_id).map_err(|_| AccountError::NoSuchPlayer(guild_id))?;
    let done = sqlx::query("UPDATE guild SET skills = $1, skill_point = $2 WHERE id = $3")
        .bind(skills.as_slice())
        .bind(i16::from(skill_point))
        .bind(id)
        .execute(store.pool())
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Debit the cast's SP: `UseSkill`'s `SetSP` (`G/guild.cpp:1353`), which persists only
/// the power (as the charge's does). Answers whether a row was there.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for an id above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn spend_power(store: &Store, guild_id: u32, power: i32) -> Result<bool, AccountError> {
    let id = i32::try_from(guild_id).map_err(|_| AccountError::NoSuchPlayer(guild_id))?;
    let done = sqlx::query("UPDATE guild SET sp = $1 WHERE id = $2")
        .bind(power)
        .bind(id)
        .execute(store.pool())
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Change a guild's master: `ChangeMaster`'s three updates (`db/GuildManager.cpp:1175-1192`)
/// in one transaction — the guild row's master, the new master's grade 1, the old
/// master's grade 15. Answers whether the guild row was there.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for an id above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn set_master(
    store: &Store,
    guild_id: u32,
    from: u32,
    to: u32,
) -> Result<bool, AccountError> {
    let id = i32::try_from(guild_id).map_err(|_| AccountError::NoSuchPlayer(guild_id))?;
    let old = i32::try_from(from).map_err(|_| AccountError::NoSuchPlayer(from))?;
    let new = i32::try_from(to).map_err(|_| AccountError::NoSuchPlayer(to))?;
    let mut deal = store.pool().begin().await?;
    let done = sqlx::query("UPDATE guild SET master_pid = $1 WHERE id = $2")
        .bind(new)
        .bind(id)
        .execute(&mut *deal)
        .await?;
    sqlx::query("UPDATE guild_member SET grade = 1 WHERE pid = $1")
        .bind(new)
        .execute(&mut *deal)
        .await?;
    sqlx::query("UPDATE guild_member SET grade = 15 WHERE pid = $1")
        .bind(old)
        .execute(&mut *deal)
        .await?;
    deal.commit().await?;
    Ok(done.rows_affected() > 0)
}

/// Set a guild's ladder point: `ChangeLadderPoint`'s persisted delta
/// (`G/guild_war.cpp:753-758`). Answers whether a row was there.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for an id above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn set_ladder(store: &Store, guild_id: u32, ladder: i32) -> Result<bool, AccountError> {
    let id = i32::try_from(guild_id).map_err(|_| AccountError::NoSuchPlayer(guild_id))?;
    let done = sqlx::query("UPDATE guild SET ladder_point = $1 WHERE id = $2")
        .bind(ladder)
        .bind(id)
        .execute(store.pool())
        .await?;
    Ok(done.rows_affected() > 0)
}
/// Set a guild's ladder and war tallies: `ProcessWinLose`/`ProcessDraw`'s row write
/// (`db/GuildManager.cpp:407-431`) collapsed with `ChangeLadderPoint`'s (`:433-460`).
/// Answers whether a row was there.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for an id above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn set_war_tallies(
    store: &Store,
    guild_id: u32,
    ladder: i32,
    win: i32,
    draw: i32,
    loss: i32,
) -> Result<bool, AccountError> {
    let id = i32::try_from(guild_id).map_err(|_| AccountError::NoSuchPlayer(guild_id))?;
    let done = sqlx::query(
        "UPDATE guild SET ladder_point = $1, win = $2, draw = $3, loss = $4 WHERE id = $5",
    )
    .bind(ladder)
    .bind(win)
    .bind(draw)
    .bind(loss)
    .bind(id)
    .execute(store.pool())
    .await?;
    Ok(done.rows_affected() > 0)
}

/// Grant free experience: `give_exp0`'s loop answer (`G/guild.cpp:1582-1662`) — the guild
/// row's exp, level and ladder, with no debit and no member row. Answers whether the
/// guild row was there.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for an id above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn grant_exp(
    store: &Store,
    guild_id: u32,
    exp: u32,
    level: u8,
    ladder: i32,
) -> Result<bool, AccountError> {
    let id = i32::try_from(guild_id).map_err(|_| AccountError::NoSuchPlayer(guild_id))?;
    let done =
        sqlx::query("UPDATE guild SET exp = $1, level = $2, ladder_point = $3 WHERE id = $4")
            .bind(i64::from(exp))
            .bind(i16::from(level))
            .bind(ladder)
            .bind(id)
            .execute(store.pool())
            .await?;
    Ok(done.rows_affected() > 0)
}

/// Post a comment: `AddComment`'s insert (`G/guild.cpp:1066-1068`). The notice flag is
/// the content's first byte being `!`. Answers the new row's id.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for an id above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn post_comment(
    store: &Store,
    guild_id: u32,
    name: &str,
    content: &str,
) -> Result<u32, AccountError> {
    let id = i32::try_from(guild_id).map_err(|_| AccountError::NoSuchPlayer(guild_id))?;
    let notice = content.as_bytes().first() == Some(&b'!');
    let row = sqlx::query(
        "INSERT INTO guild_comment (guild_id, name, notice, content) VALUES ($1, $2, $3, $4)          RETURNING id",
    )
    .bind(id)
    .bind(name)
    .bind(notice)
    .bind(content)
    .fetch_one(store.pool())
    .await?;
    let posted: i32 = row.try_get("id")?;
    u32::try_from(posted).map_err(|_| AccountError::NoSuchPlayer(guild_id))
}

/// Read a guild's board: `RefreshCommentForce`'s select (`G/guild.cpp:1104`) — `notice
/// DESC, id DESC`, taking `limit`. Legacy holds the last comment's packet separately
/// (`:1133`), but the bytes are the same.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for an id above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn board(
    store: &Store,
    guild_id: u32,
    limit: i64,
) -> Result<Vec<StoredComment>, AccountError> {
    let id = i32::try_from(guild_id).map_err(|_| AccountError::NoSuchPlayer(guild_id))?;
    let rows = sqlx::query(
        "SELECT id, name, content FROM guild_comment WHERE guild_id = $1 ORDER BY notice DESC,          id DESC LIMIT $2",
    )
    .bind(id)
    .bind(limit)
    .fetch_all(store.pool())
    .await?;
    let mut comments = Vec::with_capacity(rows.len());
    for row in &rows {
        let posted: i32 = row.try_get("id")?;
        comments.push(StoredComment {
            id: u32::try_from(posted).map_err(|_| AccountError::NoSuchPlayer(guild_id))?,
            name: row.try_get("name")?,
            content: row.try_get("content")?,
        });
    }
    Ok(comments)
}

/// Delete a comment: `DeleteComment`'s delete (`G/guild.cpp:1082-1086`). A leader names
/// the id; anyone else names the id and their own name. Answers whether a row was
/// there.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for an id above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn delete_comment(
    store: &Store,
    guild_id: u32,
    comment_id: u32,
    leader: bool,
    name: &str,
) -> Result<bool, AccountError> {
    let id = i32::try_from(guild_id).map_err(|_| AccountError::NoSuchPlayer(guild_id))?;
    let comment = i32::try_from(comment_id).map_err(|_| AccountError::NoSuchPlayer(comment_id))?;
    let done = if leader {
        sqlx::query("DELETE FROM guild_comment WHERE id = $1 AND guild_id = $2")
            .bind(comment)
            .bind(id)
            .execute(store.pool())
            .await?
    } else {
        sqlx::query("DELETE FROM guild_comment WHERE id = $1 AND guild_id = $2 AND name = $3")
            .bind(comment)
            .bind(id)
            .bind(name)
            .execute(store.pool())
            .await?
    };
    Ok(done.rows_affected() > 0)
}

/// Why a guild creation's store half failed.
#[derive(Debug)]
pub enum CreateGuildError {
    /// A concurrent creation took the name first: the `guild_name_lower` index refused
    /// the insert (`23505`). The descriptor answers `[LS;473]`, as the `SELECT COUNT(*)`
    /// arm does (`G/guild_manager.cpp:96-109`).
    GuildNameTaken {
        /// The name that lost the race.
        name: String,
    },
    /// Any other account-store failure.
    Account(AccountError),
}

impl std::fmt::Display for CreateGuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GuildNameTaken { name } => write!(f, "a guild named {name} already exists"),
            Self::Account(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for CreateGuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::GuildNameTaken { .. } => None,
            Self::Account(error) => Some(error),
        }
    }
}

impl From<AccountError> for CreateGuildError {
    fn from(error: AccountError) -> Self {
        Self::Account(error)
    }
}

impl From<sqlx::Error> for CreateGuildError {
    fn from(error: sqlx::Error) -> Self {
        Self::Account(AccountError::Database(error))
    }
}

/// Turn the guild-row insert's unique violation into the taken name.
fn classify_create(error: sqlx::Error, name: &str) -> CreateGuildError {
    if let sqlx::Error::Database(database) = &error {
        if database.code().as_deref() == Some("23505")
            && database.constraint() == Some("guild_name_lower")
        {
            return CreateGuildError::GuildNameTaken {
                name: name.to_owned(),
            };
        }
    }
    CreateGuildError::Account(AccountError::Database(error))
}

/// One mark row, as [`load_marks`] reads it: the id `__AllocMarkID` handed out and
/// the 768 image bytes the TGA files held (`G/MarkManager.cpp:57-73`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMark {
    /// The guild id.
    pub guild_id: u32,
    /// The mark id.
    pub mark_id: u32,
    /// The 768 image bytes (`16 * 12 * 4`).
    pub image: Vec<u8>,
}

/// One symbol row, as [`load_symbols`] reads it: the bytes `LoadSymbol` reads
/// (`G/MarkManager.cpp:322-350`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSymbol {
    /// The guild id.
    pub guild_id: u32,
    /// The uploaded symbol bytes.
    pub data: Vec<u8>,
}

/// Load every mark row, in guild-id order, as `LoadMarkIndex` reads the index file
/// (`G/MarkManager.cpp:57-73`). The world rebuilds its images from these at boot.
///
/// # Errors
///
/// Returns [`AccountError::Database`], including for a stored value outside its range,
/// which the migration's checks make unreachable.
pub async fn load_marks(store: &Store) -> Result<Vec<StoredMark>, AccountError> {
    let rows = sqlx::query("SELECT guild_id, mark_id, image FROM guild_mark ORDER BY guild_id")
        .fetch_all(store.pool())
        .await?;
    let mut marks = Vec::with_capacity(rows.len());
    for row in rows {
        let guild: i32 = row.try_get("guild_id")?;
        let mark: i32 = row.try_get("mark_id")?;
        let image: Vec<u8> = row.try_get("image")?;
        marks.push(StoredMark {
            guild_id: u32::try_from(guild).map_err(|_| AccountError::NoSuchPlayer(0))?,
            mark_id: u32::try_from(mark).map_err(|_| AccountError::NoSuchPlayer(0))?,
            image,
        });
    }
    Ok(marks)
}

/// Load every symbol row, in guild-id order, as `LoadSymbol` reads the symbol file
/// (`G/MarkManager.cpp:322-350`).
///
/// # Errors
///
/// Returns [`AccountError::Database`], including for a stored value outside its range,
/// which the migration's checks make unreachable.
pub async fn load_symbols(store: &Store) -> Result<Vec<StoredSymbol>, AccountError> {
    let rows = sqlx::query("SELECT guild_id, data FROM guild_symbol ORDER BY guild_id")
        .fetch_all(store.pool())
        .await?;
    let mut symbols = Vec::with_capacity(rows.len());
    for row in rows {
        let guild: i32 = row.try_get("guild_id")?;
        let data: Vec<u8> = row.try_get("data")?;
        symbols.push(StoredSymbol {
            guild_id: u32::try_from(guild).map_err(|_| AccountError::NoSuchPlayer(0))?,
            data,
        });
    }
    Ok(symbols)
}

/// Store a mark row: the id and the 768 image bytes (`SaveMark`'s index and TGA
/// writes, `G/MarkManager.cpp:234-237`, in one upsert). A re-upload keeps the id the
/// world judged and replaces the image.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for an id above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn save_mark(
    store: &Store,
    guild_id: u32,
    mark_id: u32,
    image: &[u8],
) -> Result<(), AccountError> {
    let guild = i32::try_from(guild_id).map_err(|_| AccountError::NoSuchPlayer(guild_id))?;
    let mark = i32::try_from(mark_id).map_err(|_| AccountError::NoSuchPlayer(mark_id))?;
    sqlx::query(
        "INSERT INTO guild_mark (guild_id, mark_id, image) VALUES ($1, $2, $3)          ON CONFLICT (guild_id) DO UPDATE SET mark_id = $2, image = $3",
    )
    .bind(guild)
    .bind(mark)
    .bind(image)
    .execute(store.pool())
    .await?;
    Ok(())
}

/// Delete a mark row (`DeleteMark`'s index write, `G/MarkManager.cpp:259`). Answers
/// whether a row was there, as the world's no-op on an unknown guild does.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for an id above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn delete_mark(store: &Store, guild_id: u32) -> Result<bool, AccountError> {
    let guild = i32::try_from(guild_id).map_err(|_| AccountError::NoSuchPlayer(guild_id))?;
    let done = sqlx::query("DELETE FROM guild_mark WHERE guild_id = $1")
        .bind(guild)
        .execute(store.pool())
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Store a symbol row (`UploadSymbol` + `SaveSymbol`, `G/input_login.cpp:1025-1026`,
/// in one upsert). A re-upload replaces the bytes.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for an id above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn save_symbol(store: &Store, guild_id: u32, data: &[u8]) -> Result<(), AccountError> {
    let guild = i32::try_from(guild_id).map_err(|_| AccountError::NoSuchPlayer(guild_id))?;
    sqlx::query(
        "INSERT INTO guild_symbol (guild_id, data) VALUES ($1, $2)          ON CONFLICT (guild_id) DO UPDATE SET data = $2",
    )
    .bind(guild)
    .bind(data)
    .execute(store.pool())
    .await?;
    Ok(())
}
