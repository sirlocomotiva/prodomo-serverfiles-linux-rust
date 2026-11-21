//! Map-sector coordinate, topology, and membership behavior tests.

use common::EntityId;
use world::{
    map::{MapId, SectorAddress, WorldMap},
    sector::{
        CoordinateAxis, SectorCoord, SectorCoordinateError, MAX_WORLD_COORDINATE, SECTOR_SIZE,
    },
    spatial::{SpatialError, SpatialIndex},
};

#[test]
fn sector_coordinates_use_checked_legacy_boundaries_and_explicit_packing() {
    // Given: world positions at and around every legacy sector boundary.
    let positions = [0, 6_399, 6_400, 419_430_399];

    // When: each position is converted on both axes.
    let sectors = positions.map(|position| SectorCoord::from_world(position, position));
    let packed = SectorCoord::new(1, 2).packed_key();

    // Then: sectors are 6400 units, x occupies the low half, and invalid inputs are typed.
    assert_eq!(SECTOR_SIZE, 6_400);
    assert_eq!(MAX_WORLD_COORDINATE, 419_430_399);
    assert_eq!(
        sectors,
        [
            Ok(SectorCoord::new(0, 0)),
            Ok(SectorCoord::new(0, 0)),
            Ok(SectorCoord::new(1, 1)),
            Ok(SectorCoord::new(u16::MAX, u16::MAX))
        ]
    );
    assert_eq!(packed.raw(), 1 | (2 << 16));
    assert_eq!(
        SectorCoord::from_world(419_430_400, 0),
        Err(SectorCoordinateError::OutOfRange {
            axis: CoordinateAxis::X,
            value: 419_430_400,
        })
    );
    assert_eq!(
        SectorCoord::from_world(0, -1),
        Err(SectorCoordinateError::OutOfRange {
            axis: CoordinateAxis::Y,
            value: -1,
        })
    );
}

#[test]
fn map_lookup_uses_explicit_topology_and_clipped_neighborhoods() {
    // Given: two maps with the same corner coordinate and sparse explicit topology.
    let origin = SectorCoord::new(0, 0);
    let maximum = SectorCoord::new(u16::MAX, u16::MAX);
    let map = WorldMap::new(
        MapId::new(7),
        [
            origin,
            origin,
            SectorCoord::new(0, 1),
            SectorCoord::new(1, 0),
            SectorCoord::new(u16::MAX - 1, u16::MAX),
            maximum,
        ],
    );
    let other_map = WorldMap::new(MapId::new(8), [origin]);

    // When: positions and neighborhoods are looked up at both coordinate extremes.
    let origin_address = map.sector_at(0, 0);
    let absent_address = map.sector_at(6_400, 6_400);
    let origin_neighbors = map.neighbors(origin);
    let maximum_neighbors = map.neighbors(maximum);

    // Then: only configured sectors are returned and map identity prevents collisions.
    assert_eq!(map.id(), MapId::new(7));
    assert_eq!(map.sector_count(), 5);
    assert_eq!(
        origin_address,
        Ok(Some(SectorAddress::new(MapId::new(7), origin)))
    );
    assert_eq!(absent_address, Ok(None));
    assert_eq!(
        other_map.sector_at(0, 0),
        Ok(Some(SectorAddress::new(MapId::new(8), origin)))
    );
    assert_eq!(
        origin_neighbors,
        vec![
            SectorAddress::new(MapId::new(7), origin),
            SectorAddress::new(MapId::new(7), SectorCoord::new(0, 1)),
            SectorAddress::new(MapId::new(7), SectorCoord::new(1, 0)),
        ]
    );
    assert_eq!(
        maximum_neighbors,
        vec![
            SectorAddress::new(MapId::new(7), SectorCoord::new(u16::MAX - 1, u16::MAX),),
            SectorAddress::new(MapId::new(7), maximum),
        ]
    );
}

#[test]
fn spatial_membership_is_idempotent_and_nearby_queries_are_ordered() {
    // Given: entities placed in one sector and its configured neighbor.
    let map_id = MapId::new(7);
    let origin = SectorAddress::new(map_id, SectorCoord::new(0, 0));
    let east = SectorAddress::new(map_id, SectorCoord::new(1, 0));
    let map = WorldMap::new(map_id, [origin.sector(), east.sector()]);
    let mut index = SpatialIndex::new([map]);
    let entity_10: EntityId = 10;
    let entity_20: EntityId = 20;
    let entity_30: EntityId = 30;

    // When: one entity is inserted twice and neighboring entities are inserted out of order.
    let first_location = index.move_entity(entity_20, origin);
    let repeated_location = index.move_entity(entity_20, origin);
    let east_location = index.move_entity(entity_30, east);
    let origin_location = index.move_entity(entity_10, origin);
    let origin_entities = index.entities_in(origin);
    let nearby_entities = index.nearby_entities(origin);

    // Then: membership is duplicate-free and snapshots use stable entity ordering.
    assert_eq!(first_location, Ok(None));
    assert_eq!(repeated_location, Ok(Some(origin)));
    assert_eq!(east_location, Ok(None));
    assert_eq!(origin_location, Ok(None));
    assert_eq!(origin_entities, Ok(vec![entity_10, entity_20]));
    assert_eq!(nearby_entities, Ok(vec![entity_10, entity_20, entity_30]));
}

#[test]
fn rejected_spatial_move_preserves_existing_membership() {
    // Given: an entity in a configured sector and an absent destination sector.
    let map_id = MapId::new(7);
    let origin = SectorAddress::new(map_id, SectorCoord::new(0, 0));
    let absent = SectorAddress::new(map_id, SectorCoord::new(9, 9));
    let map = WorldMap::new(map_id, [origin.sector()]);
    let mut index = SpatialIndex::new([map]);
    let entity: EntityId = 42;
    assert_eq!(index.move_entity(entity, origin), Ok(None));

    // When: movement targets a sector outside the configured topology.
    let result = index.move_entity(entity, absent);

    // Then: the move is rejected before the old membership is changed.
    assert_eq!(result, Err(SpatialError::UnknownSector { address: absent }));
    assert_eq!(index.location(entity), Some(origin));
    assert_eq!(index.entities_in(origin), Ok(vec![entity]));
}

#[test]
fn cross_sector_relocation_removes_stale_membership_and_preserves_atomic_failure() {
    // Given: an entity placed in the first of two configured sectors.
    let map_id = MapId::new(7);
    let origin = SectorAddress::new(map_id, SectorCoord::new(0, 0));
    let east = SectorAddress::new(map_id, SectorCoord::new(1, 0));
    let absent = SectorAddress::new(map_id, SectorCoord::new(9, 9));
    let map = WorldMap::new(map_id, [origin.sector(), east.sector()]);
    let mut index = SpatialIndex::new([map]);
    let entity: EntityId = 42;
    assert_eq!(index.move_entity(entity, origin), Ok(None));

    // When: the entity moves, a later move is rejected, and the entity is removed twice.
    let previous = index.move_entity(entity, east);
    let source_after_move = index.entities_in(origin);
    let destination_after_move = index.entities_in(east);
    let rejected_move = index.move_entity(entity, absent);
    let location_after_rejection = index.location(entity);
    let destination_after_rejection = index.entities_in(east);
    let first_removal = index.remove_entity(entity);
    let second_removal = index.remove_entity(entity);

    // Then: movement and removal update both indexes, while rejection is atomic.
    assert_eq!(previous, Ok(Some(origin)));
    assert_eq!(source_after_move, Ok(Vec::new()));
    assert_eq!(destination_after_move, Ok(vec![entity]));
    assert_eq!(
        rejected_move,
        Err(SpatialError::UnknownSector { address: absent })
    );
    assert_eq!(location_after_rejection, Some(east));
    assert_eq!(destination_after_rejection, Ok(vec![entity]));
    assert_eq!(first_removal, Some(east));
    assert_eq!(second_removal, None);
    assert_eq!(index.location(entity), None);
    assert_eq!(index.entities_in(east), Ok(Vec::new()));
}
