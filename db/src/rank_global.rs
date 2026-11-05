//! The global rank's rows: legacy's `ENABLE_GLOBAL_RANK` tables (`0026_rank_global.sql`).
//!
//! Legacy's DB process loaded the counters and the prizes at boot
//! (`CClientManager::RankGlobal_load`, `D/ClientManager.cpp:5249-5304`), wrote the counters
//! every 20 minutes and at shutdown (`RankGlobal_save`, `:5225-5247`), and at a season's end
//! wrote the winners and emptied the counters (`RankGlobal_end_season`, `:5382-5455`). The game
//! read a winner's prizes when its quest asked (`pc_global_rank_get_reward`,
//! `G/questlua_pc.cpp:3680-3721`).
//!
//! Here [`load`] reads everything once at boot, [`save_row`] writes one character's counters
//! with the character, [`end_season`] writes a season's end in one transaction, and [`claim`]
//! writes a claimed prize's items and its `done` flag in one transaction (ADR-0003).

use sqlx::Row;

use crate::accounts::AccountError;
use crate::items::{ItemError, RowChange, TransferSide};
use crate::store::Store;

/// How many counters a character has: `stRankGlobal_player::count` (`common/tables.h:1859`).
pub const COUNTERS: usize = 9;

/// One character's counters: a `rank_global` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRank {
    /// `pid`.
    pub pid: u32,
    /// The Name it had when it first scored.
    pub name: Vec<u8>,
    /// `lv`, its level when it last scored.
    pub level: i32,
    /// Its empire when it last scored.
    pub empire: u8,
    /// `type0` .. `type8`.
    pub counts: [i32; COUNTERS],
}

/// One prize: a `rank_global_reward` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredReward {
    /// The counter, 0 to 8.
    pub kind: u8,
    /// The place, from 1.
    pub pos: i32,
    /// The item's vnum.
    pub vnum: u32,
    /// How many.
    pub count: u16,
}

/// One prize a season's end awarded: a `rank_global_winner` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredWinner {
    /// The winner's `pid`.
    pub pid: u32,
    /// Its Name.
    pub name: Vec<u8>,
    /// Its place, 1 to 10.
    pub position: i32,
    /// The counter.
    pub kind: u8,
    /// The prize's vnum.
    pub vnum: u32,
    /// How many.
    pub count: u16,
}

/// What boot reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RankBoot {
    /// `RankGlobalSeason`, the season the client is shown.
    pub season: i32,
    /// The generation a row is written under.
    pub generation: i64,
    /// The counters, by `pid`.
    pub rows: Vec<StoredRank>,
    /// The prizes, in the order they were listed.
    pub rewards: Vec<StoredReward>,
    /// The prizes awarded and not yet claimed, as (`pid`, vnum, count), in award order.
    pub unclaimed: Vec<(u32, u32, u16)>,
}

/// Read the state, the counters, the prizes and the unclaimed awards.
///
/// The counters come in `pid` order, the order legacy's `SELECT` met them in its primary key
/// (`D/ClientManager.cpp:5281`).
///
/// # Errors
///
/// [`AccountError::Database`] when the store cannot answer or a row breaks a bound.
pub async fn load(store: &Store) -> Result<RankBoot, AccountError> {
    let state = sqlx::query("SELECT season, generation FROM rank_global_state")
        .fetch_one(store.pool())
        .await?;
    let generation: i64 = state.try_get("generation")?;
    let rows = sqlx::query(
        "SELECT player_id, name, level, empire, counts FROM rank_global \
         WHERE generation = $1 ORDER BY player_id",
    )
    .bind(generation)
    .fetch_all(store.pool())
    .await?;
    let rows = rows
        .iter()
        .map(|row| {
            let counts: Vec<i32> = row.try_get("counts")?;
            Ok(StoredRank {
                pid: unsigned(row.try_get::<i32, _>("player_id")?)?,
                name: row.try_get::<String, _>("name")?.into_bytes(),
                level: row.try_get("level")?,
                empire: narrow(row.try_get::<i16, _>("empire")?)?,
                counts: counts
                    .try_into()
                    .map_err(|_| decode("a rank row without nine counters"))?,
            })
        })
        .collect::<Result<Vec<_>, AccountError>>()?;
    let rewards = sqlx::query("SELECT type, pos, vnum, count FROM rank_global_reward ORDER BY id")
        .fetch_all(store.pool())
        .await?;
    let rewards = rewards
        .iter()
        .map(|row| {
            Ok(StoredReward {
                kind: narrow(row.try_get::<i16, _>("type")?)?,
                pos: row.try_get("pos")?,
                vnum: unsigned(row.try_get::<i32, _>("vnum")?)?,
                count: narrow(row.try_get::<i32, _>("count")?)?,
            })
        })
        .collect::<Result<Vec<_>, AccountError>>()?;
    let unclaimed = sqlx::query(
        "SELECT player_id, vnum, count FROM rank_global_winner WHERE NOT done ORDER BY id",
    )
    .fetch_all(store.pool())
    .await?;
    let unclaimed = unclaimed
        .iter()
        .map(|row| {
            Ok((
                unsigned(row.try_get::<i32, _>("player_id")?)?,
                unsigned(row.try_get::<i32, _>("vnum")?)?,
                narrow(row.try_get::<i32, _>("count")?)?,
            ))
        })
        .collect::<Result<Vec<_>, AccountError>>()?;
    Ok(RankBoot {
        season: state.try_get("season")?,
        generation,
        rows,
        rewards,
        unclaimed,
    })
}

/// Write one character's counters under `generation`. Answers whether the row was written:
/// a row of an older generation is not, since a season ended after its save was taken.
///
/// The state row is read `FOR SHARE`, so a season's end, which updates it first, waits for
/// this write and then deletes it, or this write waits for the end and then finds its
/// generation gone.
///
/// # Errors
///
/// [`AccountError::NoSuchPlayer`] for a `pid` above `i32::MAX`, and [`AccountError::Database`]
/// otherwise.
pub async fn save_row(
    store: &Store,
    generation: i64,
    row: &StoredRank,
) -> Result<bool, AccountError> {
    let pid = i32::try_from(row.pid).map_err(|_| AccountError::NoSuchPlayer(row.pid))?;
    let name = String::from_utf8(row.name.clone()).map_err(|_| decode("a rank Name"))?;
    let mut transaction = store.pool().begin().await?;
    let current: i64 = sqlx::query_scalar("SELECT generation FROM rank_global_state FOR SHARE")
        .fetch_one(&mut *transaction)
        .await?;
    if current != generation {
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO rank_global (player_id, generation, name, level, empire, counts) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         ON CONFLICT (player_id) DO UPDATE SET generation = EXCLUDED.generation, \
         level = EXCLUDED.level, empire = EXCLUDED.empire, counts = EXCLUDED.counts",
    )
    .bind(pid)
    .bind(generation)
    .bind(name)
    .bind(row.level)
    .bind(i16::from(row.empire))
    .bind(row.counts.to_vec())
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(true)
}

/// Write a season's end in one transaction: the next season and generation, every prize
/// awarded, and no counters (`RankGlobal_end_season`, `D/ClientManager.cpp:5382-5455`).
///
/// # Errors
///
/// [`AccountError::Database`] when the store refuses any part; nothing is written then.
pub async fn end_season(
    store: &Store,
    season: i32,
    generation: i64,
    winners: &[StoredWinner],
) -> Result<(), AccountError> {
    let mut transaction = store.pool().begin().await?;
    sqlx::query("UPDATE rank_global_state SET season = $1, generation = $2")
        .bind(season)
        .bind(generation)
        .execute(&mut *transaction)
        .await?;
    for winner in winners {
        let pid = i32::try_from(winner.pid).map_err(|_| AccountError::NoSuchPlayer(winner.pid))?;
        let name = String::from_utf8(winner.name.clone()).map_err(|_| decode("a winner Name"))?;
        let vnum = i32::try_from(winner.vnum).map_err(|_| decode("a prize vnum"))?;
        sqlx::query(
            "INSERT INTO rank_global_winner (player_id, name, position, type, vnum, count) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(pid)
        .bind(name)
        .bind(winner.position)
        .bind(i16::from(winner.kind))
        .bind(vnum)
        .bind(i32::from(winner.count))
        .execute(&mut *transaction)
        .await?;
    }
    sqlx::query("DELETE FROM rank_global")
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

/// Write a claim of `owner_id`'s prizes in one transaction: the items its quest gave, the gold
/// its quest moved when `gold` is `Some`, and every unclaimed prize of it marked `done`
/// (`G/questlua_pc.cpp:3690-3707`).
///
/// # Errors
///
/// As [`crate::items::apply_transfer`]; nothing is written then.
pub async fn claim(
    store: &Store,
    owner_id: u32,
    changes: &[RowChange],
    gold: Option<i64>,
) -> Result<(), ItemError> {
    let pid = i32::try_from(owner_id).map_err(|_| ItemError::NoSuchOwner(owner_id))?;
    let side = TransferSide {
        owner_id,
        changes,
        gold: gold.unwrap_or(0),
    };
    let mut transaction = store.pool().begin().await?;
    crate::items::write_sides(store, &mut transaction, &[side], gold.is_some()).await?;
    sqlx::query("UPDATE rank_global_winner SET done = true WHERE player_id = $1 AND NOT done")
        .bind(pid)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

/// A stored non-negative integer as the `u32` it stands for.
fn unsigned(value: i32) -> Result<u32, AccountError> {
    u32::try_from(value).map_err(|_| decode("a negative id"))
}

/// A stored integer into the narrower type its column bounds it to.
fn narrow<T: TryFrom<V>, V>(value: V) -> Result<T, AccountError> {
    T::try_from(value).map_err(|_| decode("a value outside its column's bound"))
}

/// A row the columns' bounds should have refused.
fn decode(what: &str) -> AccountError {
    AccountError::Database(sqlx::Error::Decode(what.to_owned().into()))
}
