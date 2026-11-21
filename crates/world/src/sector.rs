//! Legacy-compatible world coordinate conversion and sector keys.

use std::{error::Error, fmt};

/// Width and height of one sector in world units.
pub const SECTOR_SIZE: u64 = 6_400;

/// Largest world coordinate representable by a 16-bit sector coordinate.
pub const MAX_WORLD_COORDINATE: i64 = 419_430_399;

/// Axis associated with an invalid world coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinateAxis {
    /// Horizontal world axis.
    X,
    /// Vertical world axis.
    Y,
}

/// Failure converting a world position into a sector coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectorCoordinateError {
    /// A coordinate cannot be represented by the packed 16-bit sector format.
    OutOfRange {
        /// Axis containing the invalid value.
        axis: CoordinateAxis,
        /// Rejected world coordinate.
        value: i64,
    },
}

impl fmt::Display for SectorCoordinateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange { axis, value } => {
                write!(
                    formatter,
                    "{axis:?} coordinate {value} is outside the supported range"
                )
            }
        }
    }
}

impl Error for SectorCoordinateError {}

/// Two-dimensional coordinate in sector space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SectorCoord {
    x: u16,
    y: u16,
}

impl SectorCoord {
    /// Creates a sector coordinate from already-bounded axis values.
    #[must_use]
    pub const fn new(x: u16, y: u16) -> Self {
        Self { x, y }
    }

    /// Converts a world position into its containing sector.
    ///
    /// # Errors
    /// Returns [`SectorCoordinateError::OutOfRange`] when either axis is negative
    /// or exceeds [`MAX_WORLD_COORDINATE`].
    pub fn from_world(x: i64, y: i64) -> Result<Self, SectorCoordinateError> {
        Ok(Self::new(
            sector_axis(CoordinateAxis::X, x)?,
            sector_axis(CoordinateAxis::Y, y)?,
        ))
    }

    /// Returns the horizontal sector coordinate.
    #[must_use]
    pub const fn x(self) -> u16 {
        self.x
    }

    /// Returns the vertical sector coordinate.
    #[must_use]
    pub const fn y(self) -> u16 {
        self.y
    }

    /// Packs x into the low half and y into the high half of a 32-bit key.
    #[must_use]
    pub fn packed_key(self) -> SectorKey {
        SectorKey(u32::from(self.x) | (u32::from(self.y) << 16))
    }
}

/// Packed legacy sector key with x in bits 0-15 and y in bits 16-31.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SectorKey(u32);

impl SectorKey {
    /// Returns the packed integer representation.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

fn sector_axis(axis: CoordinateAxis, value: i64) -> Result<u16, SectorCoordinateError> {
    let error = SectorCoordinateError::OutOfRange { axis, value };
    if !(0..=MAX_WORLD_COORDINATE).contains(&value) {
        return Err(error);
    }

    let world = u64::try_from(value).map_err(|_| error)?;
    let sector = world.checked_div(SECTOR_SIZE).ok_or(error)?;
    u16::try_from(sector).map_err(|_| error)
}
