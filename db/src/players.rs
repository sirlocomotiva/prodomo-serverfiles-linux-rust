//! Characters, as the Channel login and the select screen read them.
//!
//! Every value reaches SQL as a bound parameter.

use sqlx::Row;

use crate::accounts::{AccountError, AccountId};
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
