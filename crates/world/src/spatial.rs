//! Deterministic entity membership across configured map sectors.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

use common::EntityId;

use crate::map::{MapId, SectorAddress, WorldMap};

/// Failure to address a configured map sector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpatialError {
    /// No map has the requested identity.
    UnknownMap {
        /// Missing map identity.
        map_id: MapId,
    },
    /// The map exists but does not contain the requested sector.
    UnknownSector {
        /// Missing map-qualified sector.
        address: SectorAddress,
    },
}

impl fmt::Display for SpatialError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownMap { map_id } => {
                write!(formatter, "map {} is not configured", map_id.raw())
            }
            Self::UnknownSector { address } => write!(
                formatter,
                "sector ({}, {}) is not configured for map {}",
                address.sector().x(),
                address.sector().y(),
                address.map_id().raw()
            ),
        }
    }
}

impl Error for SpatialError {}

/// Bidirectional entity membership index over fixed map topology.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpatialIndex {
    maps: BTreeMap<MapId, WorldMap>,
    locations: BTreeMap<EntityId, SectorAddress>,
    members: BTreeMap<SectorAddress, BTreeSet<EntityId>>,
}

impl SpatialIndex {
    /// Creates an empty membership index for the supplied maps.
    #[must_use]
    pub fn new(maps: impl IntoIterator<Item = WorldMap>) -> Self {
        Self {
            maps: maps.into_iter().map(|map| (map.id(), map)).collect(),
            locations: BTreeMap::new(),
            members: BTreeMap::new(),
        }
    }

    /// Moves an entity to a configured sector and returns its previous location.
    ///
    /// # Errors
    /// Returns [`SpatialError`] without changing membership when the destination
    /// map or sector is not configured.
    pub fn move_entity(
        &mut self,
        entity: EntityId,
        destination: SectorAddress,
    ) -> Result<Option<SectorAddress>, SpatialError> {
        self.map_for(destination)?;
        let previous = self.locations.get(&entity).copied();
        if previous == Some(destination) {
            return Ok(previous);
        }

        if let Some(address) = previous {
            let remove_empty_sector = self.members.get_mut(&address).is_some_and(|members| {
                members.remove(&entity);
                members.is_empty()
            });
            if remove_empty_sector {
                self.members.remove(&address);
            }
        }

        self.locations.insert(entity, destination);
        self.members.entry(destination).or_default().insert(entity);
        Ok(previous)
    }

    /// Removes an entity and returns its former location when present.
    pub fn remove_entity(&mut self, entity: EntityId) -> Option<SectorAddress> {
        let address = self.locations.remove(&entity)?;
        let remove_empty_sector = self.members.get_mut(&address).is_some_and(|members| {
            members.remove(&entity);
            members.is_empty()
        });
        if remove_empty_sector {
            self.members.remove(&address);
        }
        Some(address)
    }

    /// Returns the current location of an entity.
    #[must_use]
    pub fn location(&self, entity: EntityId) -> Option<SectorAddress> {
        self.locations.get(&entity).copied()
    }

    /// Returns an ordered snapshot of entities in one configured sector.
    ///
    /// # Errors
    /// Returns [`SpatialError`] when the map or sector is not configured.
    pub fn entities_in(&self, address: SectorAddress) -> Result<Vec<EntityId>, SpatialError> {
        self.map_for(address)?;
        Ok(self
            .members
            .get(&address)
            .map_or_else(Vec::new, |members| members.iter().copied().collect()))
    }

    /// Returns an ordered snapshot from configured sectors in a clipped 3-by-3 area.
    ///
    /// # Errors
    /// Returns [`SpatialError`] when the center map or sector is not configured.
    pub fn nearby_entities(&self, center: SectorAddress) -> Result<Vec<EntityId>, SpatialError> {
        let map = self.map_for(center)?;
        let mut nearby = BTreeSet::new();
        for address in map.neighbors(center.sector()) {
            if let Some(members) = self.members.get(&address) {
                nearby.extend(members.iter().copied());
            }
        }
        Ok(nearby.into_iter().collect())
    }

    fn map_for(&self, address: SectorAddress) -> Result<&WorldMap, SpatialError> {
        let map = self
            .maps
            .get(&address.map_id())
            .ok_or(SpatialError::UnknownMap {
                map_id: address.map_id(),
            })?;
        if !map.contains_sector(address.sector()) {
            return Err(SpatialError::UnknownSector { address });
        }
        Ok(map)
    }
}
