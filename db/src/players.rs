//! Characters, as the Channel login and the select screen read them.
//!
//! Every value reaches SQL as a bound parameter.

use sqlx::Row;

use crate::accounts::{AccountError, AccountId, Name};
use crate::store::Store;

/// The character slots an account has (`PLAYER_PER_ACCOUNT`).
pub const PLAYER_SLOTS: usize = 4;

/// One character as the select screen shows it (legacy `TSimplePlayer` less its server
/// location, which the game fills in).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobbyPlayer {
    /// The slot, 0 to 3.
    pub slot: u8,
    /// The character ID.
    pub id: u32,
    /// The Name: 2 to 24 ASCII letters and digits.
    pub name: String,
    /// The race.
    pub job: u8,
    /// The level.
    pub level: u8,
    /// Minutes played.
    pub play_minutes: u32,
    /// Strength.
    pub st: u8,
    /// Vitality.
    pub ht: u8,
    /// Dexterity.
    pub dx: u8,
    /// Intelligence.
    pub iq: u8,
    /// The conqueror level.
    pub conqueror_level: u8,
    /// Sungma strength.
    pub sungma_str: u8,
    /// Sungma vitality.
    pub sungma_hp: u8,
    /// Sungma movement.
    pub sungma_move: u8,
    /// Sungma immunity.
    pub sungma_immune: u8,
    /// The armour part.
    pub main_part: u16,
    /// The hair part.
    pub hair_part: u16,
    /// The sash part.
    pub sash_part: u16,
    /// The last saved x.
    pub x: i32,
    /// The last saved y.
    pub y: i32,
    /// The skill group.
    pub skill_group: u8,
    /// Whether the player must choose a new Name first.
    pub change_name: bool,
}

/// What the Channel login sends to the select screen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lobby {
    /// The account's empire, or 0 before its first character.
    pub empire: u8,
    /// The characters, in slot order.
    pub players: Vec<LobbyPlayer>,
}

impl Lobby {
    /// The character in a slot.
    #[must_use]
    pub fn in_slot(&self, slot: u8) -> Option<&LobbyPlayer> {
        self.players.iter().find(|player| player.slot == slot)
    }
}

/// Read an account's empire and characters.
///
/// Legacy reads `player_index` and then each character row (`D/ClientManagerLogin.cpp:82-149`
/// and `:470-530`), and inserts a `player_index` row when the account has none. Here the empire
/// is a column of the account, so nothing is written.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchAccountId`], [`AccountError::Corrupt`] for a value outside
/// the range the schema allows, or [`AccountError::Database`].
pub async fn lobby(store: &Store, account: AccountId) -> Result<Lobby, AccountError> {
    let id = account.to_column()?;
    let mut transaction = store.pool().begin().await?;
    let empire: i16 = sqlx::query_scalar("SELECT empire FROM account WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(AccountError::NoSuchAccountId(account))?;
    let rows = sqlx::query(
        "SELECT slot, id, name, job, level, playtime_minutes, st, ht, dx, iq, conqueror_level, \
         sungma_str, sungma_hp, sungma_move, sungma_immune, part_main, part_hair, part_sash, \
         x, y, skill_group, change_name FROM player WHERE account_id = $1 ORDER BY slot",
    )
    .bind(id)
    .fetch_all(&mut *transaction)
    .await?;
    transaction.commit().await?;
    let players = rows
        .iter()
        .map(|row| {
            Ok(LobbyPlayer {
                slot: narrow(row.try_get::<i16, _>("slot")?, "slot")?,
                id: narrow(row.try_get::<i32, _>("id")?, "id")?,
                name: row.try_get("name")?,
                job: narrow(row.try_get::<i16, _>("job")?, "job")?,
                level: narrow(row.try_get::<i16, _>("level")?, "level")?,
                play_minutes: narrow(row.try_get::<i32, _>("playtime_minutes")?, "playtime")?,
                st: narrow(row.try_get::<i16, _>("st")?, "st")?,
                ht: narrow(row.try_get::<i16, _>("ht")?, "ht")?,
                dx: narrow(row.try_get::<i16, _>("dx")?, "dx")?,
                iq: narrow(row.try_get::<i16, _>("iq")?, "iq")?,
                conqueror_level: narrow(row.try_get::<i16, _>("conqueror_level")?, "conqueror")?,
                sungma_str: narrow(row.try_get::<i16, _>("sungma_str")?, "sungma_str")?,
                sungma_hp: narrow(row.try_get::<i16, _>("sungma_hp")?, "sungma_hp")?,
                sungma_move: narrow(row.try_get::<i16, _>("sungma_move")?, "sungma_move")?,
                sungma_immune: narrow(row.try_get::<i16, _>("sungma_immune")?, "sungma_immune")?,
                main_part: narrow(row.try_get::<i32, _>("part_main")?, "part_main")?,
                hair_part: narrow(row.try_get::<i32, _>("part_hair")?, "part_hair")?,
                sash_part: narrow(row.try_get::<i32, _>("part_sash")?, "part_sash")?,
                x: row.try_get("x")?,
                y: row.try_get("y")?,
                skill_group: narrow(row.try_get::<i16, _>("skill_group")?, "skill_group")?,
                change_name: row.try_get("change_name")?,
            })
        })
        .collect::<Result<Vec<_>, AccountError>>()?;
    Ok(Lobby {
        empire: narrow(empire, "empire")?,
        players,
    })
}

/// Narrow a column to the width the client is sent, or report the row as corrupt.
fn narrow<From, To>(value: From, column: &str) -> Result<To, AccountError>
where
    From: Copy + std::fmt::Display,
    To: TryFrom<From>,
{
    To::try_from(value)
        .map_err(|_| AccountError::Corrupt(format!("player column {column} holds {value}")))
}

/// A character to create, as legacy `NewPlayerTable2` fills it (`input_login.cpp:403-449`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPlayer {
    /// The slot, 0 to 3.
    pub slot: u8,
    /// The Name.
    pub name: Name,
    /// The race, 0 to 7.
    pub job: u8,
    /// Strength.
    pub st: u8,
    /// Vitality.
    pub ht: u8,
    /// Dexterity.
    pub dx: u8,
    /// Intelligence.
    pub iq: u8,
    /// Hit points.
    pub hp: i32,
    /// Spell points.
    pub sp: i32,
    /// Stamina.
    pub stamina: i32,
    /// The body shape, stored as the base, armour, and hair parts.
    pub part_base: u8,
    /// The start x.
    pub x: i32,
    /// The start y.
    pub y: i32,
}

/// What [`create_player`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Created {
    /// The character exists with this ID.
    Player(u32),
    /// The slot holds a character or another character has the Name, regardless of case
    /// (legacy `HEADER_DG_PLAYER_CREATE_ALREADY`).
    Taken,
}

/// Create a character at level 1.
///
/// Legacy checks the slot and then the Name before inserting
/// (`D/ClientManagerPlayer.cpp:1062-1336`); here both are unique keys, so one insert decides
/// and two creates racing for a Name cannot both succeed.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchAccountId`], [`AccountError::Corrupt`], or
/// [`AccountError::Database`].
pub async fn create_player(
    store: &Store,
    account: AccountId,
    player: &NewPlayer,
) -> Result<Created, AccountError> {
    let id: Option<i32> = sqlx::query_scalar(
        "INSERT INTO player (account_id, slot, name, job, level, st, ht, dx, iq, hp, sp, \
         stamina, part_base, part_main, part_hair, x, y) \
         SELECT id, $2, $3, $4, 1, $5, $6, $7, $8, $9, $10, $11, $12, $12, $12, $13, $14 \
         FROM account WHERE id = $1 \
         ON CONFLICT DO NOTHING RETURNING id",
    )
    .bind(account.to_column()?)
    .bind(i16::from(player.slot))
    .bind(player.name.as_str())
    .bind(i16::from(player.job))
    .bind(i16::from(player.st))
    .bind(i16::from(player.ht))
    .bind(i16::from(player.dx))
    .bind(i16::from(player.iq))
    .bind(player.hp)
    .bind(player.sp)
    .bind(player.stamina)
    .bind(i16::from(player.part_base))
    .bind(player.x)
    .bind(player.y)
    .fetch_optional(store.pool())
    .await?;
    match id {
        Some(id) => Ok(Created::Player(narrow(id, "id")?)),
        None if account_exists(store, account).await? => Ok(Created::Taken),
        None => Err(AccountError::NoSuchAccountId(account)),
    }
}

async fn account_exists(store: &Store, account: AccountId) -> Result<bool, AccountError> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM account WHERE id = $1)")
            .bind(account.to_column()?)
            .fetch_one(store.pool())
            .await?,
    )
}

/// The character a delete names and the rules it must pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerDelete<'a> {
    /// The slot.
    pub slot: u8,
    /// The character the game believes is in the slot.
    pub player: u32,
    /// The code the client sent: legacy compares its first seven bytes with the account's
    /// delete code (`strncmp`, `D/ClientManagerPlayer.cpp:1362`).
    pub code: &'a [u8],
    /// A character at this level or above is kept (`PLAYER_DELETE_LEVEL_LIMIT`).
    pub level_limit: i32,
    /// A character below this level is kept (`PLAYER_DELETE_LEVEL_LIMIT_LOWER`).
    pub level_limit_lower: i32,
}

/// The bytes of a delete code the client's code is compared with.
const DELETE_CODE_BYTES: usize = 7;

/// Delete a character, keeping a copy of its row in `player_deleted`. Returns `false`, and
/// changes nothing, when the code does not match, the level is outside the limits, or the slot
/// does not hold that character: legacy answers each of these with
/// `HEADER_DG_PLAYER_DELETE_FAILED` (`D/ClientManagerPlayer.cpp:1343-1470`).
///
/// # Errors
///
/// Returns [`AccountError::NoSuchAccountId`] or [`AccountError::Database`].
pub async fn delete_player(
    store: &Store,
    account: AccountId,
    delete: &PlayerDelete<'_>,
) -> Result<bool, AccountError> {
    let account_id = account.to_column()?;
    let Ok(player_id) = i32::try_from(delete.player) else {
        return Ok(false);
    };
    let mut transaction = store.pool().begin().await?;
    let code: String =
        sqlx::query_scalar("SELECT delete_code FROM account WHERE id = $1 FOR UPDATE")
            .bind(account_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(AccountError::NoSuchAccountId(account))?;
    if delete.code.get(..DELETE_CODE_BYTES) != Some(code.as_bytes()) {
        return Ok(false);
    }
    let archived = sqlx::query(
        "WITH gone AS (DELETE FROM player WHERE account_id = $1 AND slot = $2 AND id = $3 \
         AND level < $4 AND level >= $5 RETURNING *) \
         INSERT INTO player_deleted (player_id, account_id, name, player) \
         SELECT id, account_id, name, to_jsonb(gone) FROM gone",
    )
    .bind(account_id)
    .bind(i16::from(delete.slot))
    .bind(player_id)
    .bind(delete.level_limit)
    .bind(delete.level_limit_lower)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    transaction.commit().await?;
    Ok(archived == 1)
}

/// Choose the account's empire and move its characters to that empire's start. Returns
/// `false`, and changes nothing, when the account already has an empire and a character.
///
/// Legacy checks that on the game (`input_login.cpp:961-985`) and moves only the characters in
/// the first three slots (`D/ClientManager.cpp:1217-1281`, a Defect); here the check and the move
/// are one transaction and every slot moves.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchAccountId`] or [`AccountError::Database`].
pub async fn select_empire(
    store: &Store,
    account: AccountId,
    empire: u8,
    start: (i32, i32),
) -> Result<bool, AccountError> {
    let account_id = account.to_column()?;
    let mut transaction = store.pool().begin().await?;
    let current: i16 = sqlx::query_scalar("SELECT empire FROM account WHERE id = $1 FOR UPDATE")
        .bind(account_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(AccountError::NoSuchAccountId(account))?;
    let has_players: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM player WHERE account_id = $1)")
            .bind(account_id)
            .fetch_one(&mut *transaction)
            .await?;
    if current != 0 && has_players {
        return Ok(false);
    }
    sqlx::query("UPDATE account SET empire = $2 WHERE id = $1")
        .bind(account_id)
        .bind(i16::from(empire))
        .execute(&mut *transaction)
        .await?;
    sqlx::query("UPDATE player SET x = $2, y = $3 WHERE account_id = $1")
        .bind(account_id)
        .bind(start.0)
        .bind(start.1)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(true)
}

/// Give a character the new Name it was asked to choose, and clear the request. Returns `false`,
/// and changes nothing, when another character has the Name regardless of case (legacy
/// `HEADER_DG_PLAYER_CREATE_ALREADY`, `D/ClientManagerLogin.cpp:618-660`).
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] when the account holds no such character, or
/// [`AccountError::Database`].
pub async fn change_name(
    store: &Store,
    account: AccountId,
    player: u32,
    name: &Name,
) -> Result<bool, AccountError> {
    let player_id = i32::try_from(player).map_err(|_| AccountError::NoSuchPlayer(player))?;
    let renamed = sqlx::query(
        "UPDATE player SET name = $3, change_name = false WHERE id = $2 AND account_id = $1",
    )
    .bind(account.to_column()?)
    .bind(player_id)
    .bind(name.as_str())
    .execute(store.pool())
    .await;
    match renamed {
        Ok(done) if done.rows_affected() == 1 => Ok(true),
        Ok(_) => Err(AccountError::NoSuchPlayer(player)),
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => Ok(false),
        Err(error) => Err(error.into()),
    }
}
