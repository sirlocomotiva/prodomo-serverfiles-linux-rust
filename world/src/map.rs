//! Map identity and explicit sector topology.

use std::collections::BTreeSet;

use crate::sector::{SectorCoord, SectorCoordinateError};

/// Stable identity of a world map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MapId(u32);

impl MapId {
    /// Creates a map identity from its persistent numeric value.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the numeric map identity.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Map-qualified sector coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SectorAddress {
    map_id: MapId,
    sector: SectorCoord,
}

impl SectorAddress {
    /// Creates a map-qualified sector address.
    #[must_use]
    pub const fn new(map_id: MapId, sector: SectorCoord) -> Self {
        Self { map_id, sector }
    }

    /// Returns the map containing this sector.
    #[must_use]
    pub const fn map_id(self) -> MapId {
        self.map_id
    }

    /// Returns the coordinate within the map.
    #[must_use]
    pub const fn sector(self) -> SectorCoord {
        self.sector
    }
}

/// A map built from a fixed set of existing sectors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldMap {
    id: MapId,
    sectors: BTreeSet<SectorCoord>,
}

impl WorldMap {
    /// Builds a map from explicit sectors, discarding duplicate coordinates.
    pub fn new(id: MapId, sectors: impl IntoIterator<Item = SectorCoord>) -> Self {
        Self {
            id,
            sectors: sectors.into_iter().collect(),
        }
    }

    /// Returns the map identity.
    #[must_use]
    pub const fn id(&self) -> MapId {
        self.id
    }

    /// Returns the number of configured sectors.
    #[must_use]
    pub fn sector_count(&self) -> usize {
        self.sectors.len()
    }

    /// Reports whether a sector belongs to this map topology.
    #[must_use]
    pub fn contains_sector(&self, sector: SectorCoord) -> bool {
        self.sectors.contains(&sector)
    }

    /// Looks up the configured sector containing a world position.
    ///
    /// # Errors
    /// Returns [`SectorCoordinateError`] when either coordinate is outside the
    /// supported packed-sector range.
    pub fn sector_at(
        &self,
        x: i64,
        y: i64,
    ) -> Result<Option<SectorAddress>, SectorCoordinateError> {
        let sector = SectorCoord::from_world(x, y)?;
        Ok(self
            .contains_sector(sector)
            .then_some(SectorAddress::new(self.id, sector)))
    }

    /// Returns configured sectors in the clipped 3-by-3 area around a coordinate.
    #[must_use]
    pub fn neighbors(&self, center: SectorCoord) -> Vec<SectorAddress> {
        let mut neighbors = Vec::with_capacity(9);
        for x_delta in -1_i16..=1 {
            for y_delta in -1_i16..=1 {
                let Some(x) = center.x().checked_add_signed(x_delta) else {
                    continue;
                };
                let Some(y) = center.y().checked_add_signed(y_delta) else {
                    continue;
                };
                let sector = SectorCoord::new(x, y);
                if self.contains_sector(sector) {
                    neighbors.push(SectorAddress::new(self.id, sector));
                }
            }
        }
        neighbors
    }
}
