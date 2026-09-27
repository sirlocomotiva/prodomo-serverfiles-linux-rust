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

    /// The character IDs by slot, with 0 for an empty slot.
    ///
    /// This is the shape the character-select reducer judges against, and it matches legacy's
    /// `TAccountTable::players`, whose unused entries keep `dwID == 0` (`G/input_login.cpp:265-306`
    ///). A slot past the end of the array is `Ignore` in the reducer, which is where the
    /// Rewrite's bound check lives.
    #[must_use]
    pub fn slot_ids(&self) -> [u32; PLAYER_SLOTS] {
        let mut slots = [0; PLAYER_SLOTS];
        for player in &self.players {
            if let Some(slot) = slots.get_mut(usize::from(player.slot)) {
                *slot = player.id;
            }
        }
        slots
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

/// Write a character's stored position, which is what a Warp commits.
///
/// Legacy does this in `CHARACTER::Save` (`G/char.cpp:1551-1565`), which prefers a pending
/// `m_posWarp` over the live position and then writes the row through the retired DB process.
/// ADR-0003 makes every Transfer commit in the one transaction that performs it, and this is
/// that transaction: the caller's Warp and this write are the same event, so a character that
/// is refused a map is moved in the same statement that moves it.
///
/// Returns `false` when the account holds no such character, so a caller cannot report a Warp
/// that did not reach a row.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for a player ID above `i32::MAX`. The column is
/// `integer` and a `u32` above `i32::MAX` cannot name a row, so this is reported rather than
/// converted: `as i32` would wrap to a negative ID that could match a real character. Returns
/// [`AccountError::Database`] otherwise. Every value is a bound parameter; no SQL is formatted.
pub async fn save_position(
    store: &Store,
    account: AccountId,
    player: u32,
    x: i32,
    y: i32,
) -> Result<bool, AccountError> {
    let player_id = i32::try_from(player).map_err(|_| AccountError::NoSuchPlayer(player))?;
    let updated = sqlx::query("UPDATE player SET x = $3, y = $4 WHERE account_id = $1 AND id = $2")
        .bind(account.to_column()?)
        .bind(player_id)
        .bind(x)
        .bind(y)
        .execute(store.pool())
        .await?;
    Ok(updated.rows_affected() == 1)
}

/// The mutable columns of one character row, which is what a save writes.
///
/// Legacy builds the same set in `CHARACTER::CreatePlayerProto` (`G/char.cpp:1478-1654`) and
/// sends it whole in `CHARACTER::SaveReal` (`G/char.cpp:1656-1683`) through
/// `HEADER_GD_PLAYER_SAVE`. Only the columns a live session can change are here: the select
/// slot, the Name, the race, the parts, and the creation stamp are fixed at create time, and
/// the account's empire is the account row's, not this one's.
///
/// A save writes the whole row rather than the columns that changed, because that is what
/// legacy does: `CreatePlayerProto` clears the table with `memset` and fills every field, so a
/// write is a full replacement and a missing column is a value legacy would have preserved
/// from its own read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerSave {
    /// `tab.level`.
    pub level: u8,
    /// `tab.exp`.
    pub exp: i64,
    /// `tab.conqueror_level`, which legacy only has under `__CONQUEROR_LEVEL__`.
    pub conqueror_level: u8,
    /// The conqueror experience, the conqueror counterpart of `tab.exp`.
    pub conqueror_exp: i64,
    /// `tab.st`.
    pub st: u8,
    /// `tab.ht`.
    pub ht: u8,
    /// `tab.dx`.
    pub dx: u8,
    /// `tab.iq`.
    pub iq: u8,
    /// `POINT_HP` through `GetRealPoint`, which is `tab.hp` after `tab.st`/`tab.ht` are added.
    pub hp: i32,
    /// `POINT_SP`.
    pub sp: i32,
    /// `POINT_STAMINA`.
    pub stamina: i32,
    /// `tab.gold`, carried in `__ENABLE_GAYA_SYSTEM__` as `tab.gaya` as well.
    pub gold: i64,
    /// `tab.voice`, which is `POINT_VOICE`.
    pub voice: u8,
    /// `tab.part_base`, a `BYTE` from `m_pointsInstant.bBasePart`.
    pub part_base: u8,
    /// `tab.part_main`.
    pub main_part: u16,
    /// `tab.part_hair`.
    pub hair_part: u16,
    /// `tab.part_sash`, which `__SASH_SYSTEM__` adds.
    pub sash_part: u16,
    /// The saved x, in world units.
    pub x: i32,
    /// The saved y, in world units.
    pub y: i32,
    /// `tab.skill_group`.
    pub skill_group: u8,
    /// Minutes played, `POINT_PLAYTIME` accumulated in `CreatePlayerProto`.
    pub playtime_minutes: i32,
}

/// Write every mutable column of one character row.
///
/// This is the Rewrite's `CHARACTER::SaveReal`, reached on the save event and at disconnect.
/// The account and the player are both named, so a descriptor cannot write a character of
/// another account by holding a stale claim.
///
/// Returns `false` when the account holds no such character. Legacy cannot report this: its
/// write goes to the DB process, which drops a row that does not exist without telling the game
/// process, so the rewrite reports it instead of reporting a save that reached nothing.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for a player ID above `i32::MAX`, for the reason
/// [`save_position`] gives, and [`AccountError::Database`] otherwise. Every value is a bound
/// parameter; no SQL is formatted.
pub async fn save_character(
    store: &Store,
    account: AccountId,
    player: u32,
    save: &PlayerSave,
) -> Result<bool, AccountError> {
    let player_id = i32::try_from(player).map_err(|_| AccountError::NoSuchPlayer(player))?;
    let updated = sqlx::query(
        "UPDATE player SET level = $3, exp = $4, conqueror_level = $5, conqueror_exp = $6, \
         st = $7, ht = $8, dx = $9, iq = $10, hp = $11, sp = $12, stamina = $13, gold = $14, \
         voice = $15, part_base = $16, part_main = $17, part_hair = $18, part_sash = $19, \
         x = $20, y = $21, skill_group = $22, playtime_minutes = $23 \
         WHERE account_id = $1 AND id = $2",
    )
    .bind(account.to_column()?)
    .bind(player_id)
    .bind(i16::from(save.level))
    .bind(save.exp)
    .bind(i16::from(save.conqueror_level))
    .bind(save.conqueror_exp)
    .bind(i16::from(save.st))
    .bind(i16::from(save.ht))
    .bind(i16::from(save.dx))
    .bind(i16::from(save.iq))
    .bind(save.hp)
    .bind(save.sp)
    .bind(save.stamina)
    .bind(save.gold)
    .bind(i16::from(save.voice))
    .bind(i16::from(save.part_base))
    .bind(i32::from(save.main_part))
    .bind(i32::from(save.hair_part))
    .bind(i32::from(save.sash_part))
    .bind(save.x)
    .bind(save.y)
    .bind(i16::from(save.skill_group))
    .bind(save.playtime_minutes)
    .execute(store.pool())
    .await?;
    Ok(updated.rows_affected() == 1)
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

/// One character as the loading phase and the game phase read it.
///
/// This is the part of legacy `TPlayerTable` (`common/tables.h`) the loading burst and the
/// enter-game burst read, plus the account's empire. Legacy loads the whole table with
/// `QUERY_PLAYER_LOAD` (`D/ClientManagerPlayer.cpp:275-400`); the Rewrite reads only the
/// columns a shipped record needs, and a later system adds its own in a new migration.
///
/// # Values the Rewrite computes, not stores
///
/// The next-level cost, the maximum hit points, spell points and stamina are derived from
/// `common::levels` at the point of use, exactly as `CHARACTER::Init` does. Storing a
/// derived maximum would let a row written by hand disagree with the level and the race.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Character {
    /// The character ID, the 32-bit value the client sees.
    pub id: u32,
    /// The slot it was selected from, 0 to 3.
    pub slot: u8,
    /// The Name: 2 to 24 ASCII letters and digits.
    pub name: String,
    /// The account's empire.
    pub empire: u8,
    /// The race.
    pub job: u8,
    /// The level.
    pub level: u8,
    /// Experience banked towards the next level.
    pub exp: i64,
    /// The conqueror level (`__CONQUEROR_LEVEL__` is on).
    pub conqueror_level: u8,
    /// Strength.
    pub st: u8,
    /// Vitality.
    pub ht: u8,
    /// Dexterity.
    pub dx: u8,
    /// Intelligence.
    pub iq: u8,
    /// Current hit points.
    pub hp: i32,
    /// Current spell points.
    pub sp: i32,
    /// Current stamina.
    pub stamina: i32,
    /// Gold carried. Legacy is unsigned 64-bit; the column holds the low 63 bits of that
    /// range and the check refuses anything wider.
    pub gold: i64,
    /// Conqueror experience banked towards the next conqueror level.
    pub conqueror_exp: i64,
    /// `POINT_VOICE`, which legacy leaves uninitialised in the points record.
    pub voice: u8,
    /// The body shape, a `BYTE` (`part_base`).
    pub part_base: u8,
    /// The armour part.
    pub main_part: u16,
    /// The hair part.
    pub hair_part: u16,
    /// The sash part (`__SASH_SYSTEM__` is on).
    pub sash_part: u16,
    /// The last saved x, in world units.
    pub x: i32,
    /// The last saved y, in world units.
    pub y: i32,
    /// The skill group.
    pub skill_group: u8,
    /// Whole minutes played (`POINT_PLAYTIME`, sent as a `DWORD` and stored as `tab.playtime`).
    pub playtime_minutes: i32,
    /// Whether the player must choose a new Name before playing.
    pub change_name: bool,
}

/// Read one character of an account for the loading phase.
///
/// The account is named as well as the character, so a descriptor that holds a login can
/// never load a character of another account by guessing an ID.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] when the account holds no such character, or
/// [`AccountError::Database`].
pub async fn load_character(
    store: &Store,
    account: AccountId,
    player: u32,
) -> Result<Character, AccountError> {
    let player_id = i32::try_from(player).map_err(|_| AccountError::NoSuchPlayer(player))?;
    let row = sqlx::query(
        "SELECT p.slot, p.id, p.name, p.job, p.level, p.exp, p.conqueror_level, p.conqueror_exp, p.st, p.ht, \
         p.dx, p.iq, p.hp, p.sp, p.stamina, p.gold, p.voice, p.part_base, p.part_main, \
         p.part_hair, p.part_sash, p.x, p.y, p.skill_group, p.playtime_minutes, p.change_name, a.empire \
         FROM player AS p JOIN account AS a ON a.id = p.account_id \
         WHERE p.account_id = $1 AND p.id = $2",
    )
    .bind(account.to_column()?)
    .bind(player_id)
    .fetch_optional(store.pool())
    .await?
    .ok_or(AccountError::NoSuchPlayer(player))?;
    Ok(Character {
        id: narrow(row.try_get::<i32, _>("id")?, "id")?,
        slot: narrow(row.try_get::<i16, _>("slot")?, "slot")?,
        name: row.try_get("name")?,
        empire: narrow(row.try_get::<i16, _>("empire")?, "empire")?,
        job: narrow(row.try_get::<i16, _>("job")?, "job")?,
        level: narrow(row.try_get::<i16, _>("level")?, "level")?,
        exp: row.try_get("exp")?,
        conqueror_level: narrow(row.try_get::<i16, _>("conqueror_level")?, "conqueror")?,
        conqueror_exp: row.try_get("conqueror_exp")?,
        st: narrow(row.try_get::<i16, _>("st")?, "st")?,
        ht: narrow(row.try_get::<i16, _>("ht")?, "ht")?,
        dx: narrow(row.try_get::<i16, _>("dx")?, "dx")?,
        iq: narrow(row.try_get::<i16, _>("iq")?, "iq")?,
        hp: row.try_get("hp")?,
        sp: row.try_get("sp")?,
        stamina: row.try_get("stamina")?,
        gold: row.try_get("gold")?,
        voice: narrow(row.try_get::<i16, _>("voice")?, "voice")?,
        part_base: narrow(row.try_get::<i16, _>("part_base")?, "part_base")?,
        main_part: narrow(row.try_get::<i32, _>("part_main")?, "part_main")?,
        hair_part: narrow(row.try_get::<i32, _>("part_hair")?, "part_hair")?,
        sash_part: narrow(row.try_get::<i32, _>("part_sash")?, "part_sash")?,
        x: row.try_get("x")?,
        y: row.try_get("y")?,
        skill_group: narrow(row.try_get::<i16, _>("skill_group")?, "skill_group")?,
        playtime_minutes: row.try_get("playtime_minutes")?,
        change_name: row.try_get("change_name")?,
    })
}

#[cfg(test)]
mod tests {
    use super::{Lobby, LobbyPlayer, PLAYER_SLOTS};

    fn player(slot: u8, id: u32) -> LobbyPlayer {
        LobbyPlayer {
            slot,
            id,
            name: "Hero".to_string(),
            job: 1,
            level: 1,
            play_minutes: 0,
            st: 4,
            ht: 4,
            dx: 4,
            iq: 4,
            conqueror_level: 0,
            sungma_str: 0,
            sungma_hp: 0,
            sungma_move: 0,
            sungma_immune: 0,
            main_part: 0,
            hair_part: 0,
            sash_part: 0,
            x: 0,
            y: 0,
            skill_group: 1,
            change_name: false,
        }
    }

    fn lobby(players: Vec<LobbyPlayer>) -> Lobby {
        Lobby { empire: 1, players }
    }

    /// A full account keeps the ID in the character's own slot, and 0 in the rest. That is the
    /// shape `judge_select` reads, where 0 is what makes an empty slot a close.
    #[test]
    fn slot_ids_put_each_character_in_its_own_slot() {
        let table = lobby(vec![player(0, 11), player(2, 33)]).slot_ids();
        assert_eq!(table, [11, 0, 33, 0]);
    }

    /// A character created in the last slot is still found, so the array length is the
    /// `PLAYER_PER_ACCOUNT` bound rather than a coincidence.
    #[test]
    fn the_last_slot_is_reachable() {
        let last = u8::try_from(PLAYER_SLOTS - 1).expect("four slots fit in a u8");
        let table = lobby(vec![player(last, 44)]).slot_ids();
        assert_eq!(table, [0, 0, 0, 44]);
    }

    /// An account with no character is four zeros, which is `TAccountTable` after
    /// `PlayerDeleteSuccess` cleared the last entry (`G/input_db.cpp`).
    #[test]
    fn an_empty_account_is_all_zeros() {
        assert_eq!(lobby(Vec::new()).slot_ids(), [0; PLAYER_SLOTS]);
    }

    /// A slot outside the array is ignored rather than panicking. The store's `CHECK` keeps a
    /// slot in range, so a row cannot carry one; the guard is what lets a hand-edited row fail
    /// to be a crash.
    #[test]
    fn a_slot_past_the_array_is_ignored() {
        let table = lobby(vec![player(1, 22), player(9, 99)]).slot_ids();
        assert_eq!(table, [0, 22, 0, 0], "slot 9 has nowhere to go");
    }

    /// The array is exactly `PLAYER_PER_ACCOUNT` wide, which is the bound the select reducer
    /// range-checks against.
    #[test]
    fn the_array_is_as_wide_as_the_account() {
        let table = lobby(Vec::new()).slot_ids();
        assert_eq!(table.len(), PLAYER_SLOTS);
        assert_eq!(PLAYER_SLOTS, 4);
    }
}
