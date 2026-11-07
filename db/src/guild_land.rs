//! The guild land owners: legacy's `land.guild_id` (`legacy/sql/schema/player.sql:323-334`,
//! migration `0029_guild_land.sql`).
//!
//! Legacy's DB server reads every land at boot (`D/ClientManagerBoot.cpp:919-983`) and each
//! game process keeps them in `CManager::m_map_pkLand` (`G/building.cpp`); `CLand::SetOwner`
//! writes the owner through `HEADER_GD_UPDATE_LAND` (`:605-623`). The Rewrite reads the
//! land rows from the owner's dump at boot (Game data) and stores only the owner, which is
//! the only column that changes at runtime. A row exists only while a guild owns the land.

use sqlx::Row;

use crate::accounts::AccountError;
use crate::store::Store;

/// Every owned land: `(land_id, guild_id)` in land-id order.
pub type StoredLandOwners = Vec<(u32, u32)>;

/// Load every owned land, as `InitializeLandTable` loads them
/// (`D/ClientManagerBoot.cpp:937-941`).
///
/// # Errors
///
/// Returns [`AccountError::Database`], including for a stored value outside its range,
/// which the migration's checks make unreachable.
pub async fn load_all(store: &Store) -> Result<StoredLandOwners, AccountError> {
    let rows = sqlx::query("SELECT land_id, guild_id FROM guild_land ORDER BY land_id")
        .fetch_all(store.pool())
        .await?;
    let mut owners = Vec::with_capacity(rows.len());
    for row in &rows {
        let land: i32 = row.try_get("land_id")?;
        let guild: i32 = row.try_get("guild_id")?;
        let corrupt = || AccountError::Corrupt(format!("guild_land row {land}"));
        owners.push((
            u32::try_from(land).map_err(|_| corrupt())?,
            u32::try_from(guild).map_err(|_| corrupt())?,
        ));
    }
    Ok(owners)
}

/// Own `land` for `guild`: `CLand::SetOwner`'s store half
/// (`G/building.cpp:605-623`).
///
/// # Errors
///
/// Returns [`AccountError::Database`].
pub async fn set_owner(store: &Store, land: u32, guild: u32) -> Result<(), AccountError> {
    let land = i64::from(land);
    let guild = i64::from(guild);
    sqlx::query(
        "INSERT INTO guild_land (land_id, guild_id) VALUES ($1, $2) ON CONFLICT (land_id) DO \
         UPDATE SET guild_id = EXCLUDED.guild_id",
    )
    .bind(land)
    .bind(guild)
    .execute(store.pool())
    .await?;
    Ok(())
}

/// One stored building object, as `TObject` holds it (`C/building.h:45-60`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StoredObject {
    /// `dwID`.
    pub id: u32,
    /// `dwLandID`.
    pub land_id: u32,
    /// `dwVnum`.
    pub vnum: u32,
    /// `lMapIndex`.
    pub map: i32,
    /// `x`, `y` absolute.
    pub x: i32,
    /// `y` absolute.
    pub y: i32,
    /// `xRot`, `yRot`, `zRot`.
    pub rot: (f32, f32, f32),
    /// `lLife`.
    pub life: i32,
}

/// Every stored building object, in id order.
pub type StoredObjects = Vec<StoredObject>;

/// The land half of the guild install: the owned lands and the stored objects,
/// bundled so the install command stays under the argument lint.
#[derive(Debug, Default)]
pub struct StoredLandInstall {
    /// Every owned land, `(land_id, guild_id)` in land-id order.
    pub owners: StoredLandOwners,
    /// Every stored building object, in id order.
    pub objects: StoredObjects,
}

/// Load every building object, as `InitializeObjectTable` loads them
/// (`D/ClientManagerBoot.cpp`).
///
/// # Errors
///
/// Returns [`AccountError::Database`], including for a stored value outside its range,
/// which the migration's checks make unreachable.
pub async fn load_objects(store: &Store) -> Result<StoredObjects, AccountError> {
    let rows = sqlx::query(
        "SELECT id, land_id, vnum, map_index, x, y, x_rot, y_rot, z_rot, life FROM          building_object ORDER BY id",
    )
    .fetch_all(store.pool())
    .await?;
    let mut objects = Vec::with_capacity(rows.len());
    for row in &rows {
        let id: i32 = row.try_get("id")?;
        let land: i32 = row.try_get("land_id")?;
        let vnum: i32 = row.try_get("vnum")?;
        let corrupt = || AccountError::Corrupt(format!("building_object row {id}"));
        objects.push(StoredObject {
            id: u32::try_from(id).map_err(|_| corrupt())?,
            land_id: u32::try_from(land).map_err(|_| corrupt())?,
            vnum: u32::try_from(vnum).map_err(|_| corrupt())?,
            map: row.try_get("map_index")?,
            x: row.try_get("x")?,
            y: row.try_get("y")?,
            rot: (
                row.try_get("x_rot")?,
                row.try_get("y_rot")?,
                row.try_get("z_rot")?,
            ),
            life: row.try_get("life")?,
        });
    }
    Ok(objects)
}

/// Rebuild one object: `CObject::Reconstruct`'s store halves
/// (`G/building.cpp:80-89`): the delete row goes, then the create row lands, in one
/// transaction.
///
/// # Errors
///
/// Returns [`AccountError::Database`].
#[allow(clippy::too_many_arguments)]
pub async fn reconstruct(
    store: &Store,
    delete_id: u32,
    land_id: u32,
    vnum: u32,
    map: i32,
    x: i32,
    y: i32,
    rot: (f32, f32, f32),
    life: i32,
) -> Result<(), AccountError> {
    let mut tx = store.pool().begin().await?;
    sqlx::query("DELETE FROM building_object WHERE id = $1")
        .bind(i64::from(delete_id))
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO building_object (id, land_id, vnum, map_index, x, y, x_rot, y_rot, z_rot,          life) VALUES ((SELECT COALESCE(MAX(id), 0) + 1 FROM building_object), $1, $2, $3, $4,          $5, $6, $7, $8, $9)",
    )
    .bind(i64::from(land_id))
    .bind(i64::from(vnum))
    .bind(map)
    .bind(x)
    .bind(y)
    .bind(rot.0)
    .bind(rot.1)
    .bind(rot.2)
    .bind(life)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}
