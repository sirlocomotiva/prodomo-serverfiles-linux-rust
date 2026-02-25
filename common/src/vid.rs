//! Virtual ID (VID) system for network entity identification.
//!
//! VID wraps a u32 identifier used to uniquely identify game entities
//! (players, mobs, items) across the network. Ported from C++ `vid.h`.

use std::fmt;

/// Virtual ID for network entity identification.
///
/// In the C++ codebase, VID is a class with `m_id` and `m_crc` fields (both DWORD).
/// For the Rust port, we use a simple newtype around u32 since the CRC is primarily
/// used for validation in the original C++ and can be handled separately if needed.
///
/// The C++ VID has `operator DWORD()` which returns `m_id`, so the wire format
/// is effectively just the u32 id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Vid(pub u32);

impl Vid {
    /// Invalid/null VID constant.
    pub const NULL: Vid = Vid(0);

    /// Create a new VID with the given id.
    #[inline]
    pub const fn new(id: u32) -> Self {
        Vid(id)
    }

    /// Returns true if this VID is null (zero).
    #[inline]
    pub const fn is_null(self) -> bool {
        self.0 == 0
    }

    /// Get the raw u32 value.
    #[inline]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl Default for Vid {
    #[inline]
    fn default() -> Self {
        Vid::NULL
    }
}

impl fmt::Display for Vid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VID({})", self.0)
    }
}

impl From<u32> for Vid {
    #[inline]
    fn from(id: u32) -> Self {
        Vid(id)
    }
}

impl From<Vid> for u32 {
    #[inline]
    fn from(vid: Vid) -> Self {
        vid.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn vid_size_matches_cpp_dword() {
        // C++ VID class has two DWORDs (m_id, m_crc) = 8 bytes,
        // but our simplified Vid is just the id (transparent u32) = 4 bytes.
        // The C++ operator DWORD() returns m_id, so wire protocol uses 4 bytes.
        assert_eq!(size_of::<Vid>(), 4);
        assert_eq!(size_of::<Vid>(), size_of::<u32>());
    }

    #[test]
    fn vid_null() {
        assert!(Vid::NULL.is_null());
        assert!(!Vid::new(1).is_null());
    }

    #[test]
    fn vid_conversions() {
        let vid = Vid::from(42u32);
        assert_eq!(u32::from(vid), 42);
        assert_eq!(vid.raw(), 42);
    }

    #[test]
    fn vid_equality() {
        assert_eq!(Vid::new(100), Vid::new(100));
        assert_ne!(Vid::new(100), Vid::new(200));
    }
}
