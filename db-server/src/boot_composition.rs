//! Pure, profile-bound registration of already-loaded boot sections.
//!
//! This module is the handoff between source-specific table loaders and
//! [`crate::boot_snapshot::BootSnapshot`]. It does not execute SQL, choose a
//! feature profile, resolve request-specific GM/admin data, authorize a peer,
//! mutate a cache, or create a missing section. A successful loader result
//! may be an empty section; a loader error is returned unchanged and leaves
//! its slot empty.

use std::error::Error;
use std::fmt;

use protocol::db_boot::{BootFeatureProfile, BootSection, BootSectionKind, BANWORD_WIRE_SIZE};
use protocol::db_records::{
    EVENT_TABLE_WIRE_SIZE, ITEM_ATTR_RECORD_WIRE_SIZE, ITEM_TABLE_RECORD_WIRE_SIZE,
    LAND_RECORD_WIRE_SIZE, MARKET_ITEM_PRICE_WIRE_SIZE, MOB_TABLE_RECORD_WIRE_SIZE,
    OBJECT_PROTO_RECORD_WIRE_SIZE, OBJECT_RECORD_WIRE_SIZE, REFINE_TABLE_WIRE_SIZE,
    SHOP_TABLE_RECORD_WIRE_SIZE, SKILL_TABLE_RECORD_WIRE_SIZE,
};

/// A failure in the pure section registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootCompositionError {
    /// The registry or ordered output could not reserve its bounded storage.
    AllocationFailed,
    /// A section kind is not selected by the explicit profile.
    SectionDisabled {
        /// Unavailable section kind.
        kind: BootSectionKind,
    },
    /// A section kind was inserted more than once.
    DuplicateSection {
        /// Repeated section kind.
        kind: BootSectionKind,
    },
    /// A loader declared one kind but returned another.
    KindMismatch {
        /// Kind declared by the loader call.
        expected: BootSectionKind,
        /// Kind carried by the returned section.
        actual: BootSectionKind,
    },
    /// A section declared the invalid zero record width.
    InvalidRecordSize {
        /// Section whose width is invalid.
        kind: BootSectionKind,
        /// Supplied zero width.
        record_size: u16,
    },
    /// A source-fixed section used a width other than the verified legacy
    /// record width.
    InvalidKnownRecordSize {
        /// Source-fixed section kind.
        kind: BootSectionKind,
        /// Verified packed width.
        expected: usize,
        /// Supplied width.
        actual: u16,
    },
    /// The declared width/count product cannot be represented by `usize`.
    SizeOverflow {
        /// Section whose metadata caused the arithmetic failure.
        kind: BootSectionKind,
    },
    /// The section's data length disagrees with its declared width/count.
    InvalidSectionLength {
        /// Section whose metadata is inconsistent.
        kind: BootSectionKind,
        /// Required data length.
        expected: usize,
        /// Supplied data length.
        actual: usize,
    },
    /// The registry has no slot for a profile-selected section.
    MissingSection {
        /// Required section kind.
        kind: BootSectionKind,
        /// Its zero-based profile position.
        index: usize,
    },
}

impl fmt::Display for BootCompositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AllocationFailed => {
                formatter.write_str("boot section registry allocation failed")
            }
            Self::SectionDisabled { kind } => {
                write!(formatter, "boot profile disables the {kind} section")
            }
            Self::DuplicateSection { kind } => {
                write!(formatter, "boot section {kind} was inserted more than once")
            }
            Self::KindMismatch { expected, actual } => write!(
                formatter,
                "boot loader declared {expected} but returned section {actual}"
            ),
            Self::InvalidRecordSize { kind, record_size } => write!(
                formatter,
                "boot section {kind} has invalid record size {record_size}"
            ),
            Self::InvalidKnownRecordSize {
                kind,
                expected,
                actual,
            } => write!(
                formatter,
                "boot section {kind} has record size {actual}; expected {expected}"
            ),
            Self::SizeOverflow { kind } => {
                write!(formatter, "boot section {kind} size arithmetic overflowed")
            }
            Self::InvalidSectionLength {
                kind,
                expected,
                actual,
            } => write!(
                formatter,
                "boot section {kind} has {actual} data bytes; expected {expected}"
            ),
            Self::MissingSection { kind, index } => write!(
                formatter,
                "boot profile section {index} ({kind}) has not been loaded"
            ),
        }
    }
}

impl Error for BootCompositionError {}

/// A loader failure or a registry failure while inserting one section.
#[derive(Debug)]
pub enum BootSectionInsertError<E> {
    /// The source-specific loader failed. No section was installed.
    Loader {
        /// Section slot requested by the caller.
        kind: BootSectionKind,
        /// Original source error.
        source: E,
    },
    /// The profile or section metadata rejected the insertion.
    Composition(BootCompositionError),
}

impl<E: fmt::Display> fmt::Display for BootSectionInsertError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Loader { kind, source } => {
                write!(formatter, "boot {kind} section loader failed: {source}")
            }
            Self::Composition(source) => source.fmt(formatter),
        }
    }
}

impl<E: Error + 'static> Error for BootSectionInsertError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Loader { source, .. } => Some(source),
            Self::Composition(source) => Some(source),
        }
    }
}

/// A profile-bound collection of sections acquired by independent loaders.
///
/// The profile is fixed at construction. Slots are indexed only by
/// `BootFeatureProfile::section_kinds()`, so a successful `into_ordered`
/// returns the exact wire order and never a map or loader order. Missing and
/// failed sections remain distinguishable until the caller chooses to finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedBootSections {
    profile: BootFeatureProfile,
    slots: Vec<Option<BootSection>>,
}

impl LoadedBootSections {
    /// Create an empty registry for an explicit profile.
    ///
    /// # Errors
    ///
    /// Returns [`BootCompositionError::AllocationFailed`] if the profile-sized
    /// slot vector cannot be reserved.
    pub fn try_new(profile: BootFeatureProfile) -> Result<Self, BootCompositionError> {
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(profile.section_kinds().len())
            .map_err(|_| BootCompositionError::AllocationFailed)?;
        slots.resize_with(profile.section_kinds().len(), || None);
        Ok(Self { profile, slots })
    }

    /// Return the explicit profile bound to this registry.
    #[must_use]
    pub const fn profile(&self) -> BootFeatureProfile {
        self.profile
    }

    /// Return the number of profile-selected slots.
    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// Return whether every profile-selected slot has a validated section.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.slots.iter().all(Option::is_some)
    }

    /// Borrow a loaded section by its profile kind.
    #[must_use]
    pub fn section(&self, kind: BootSectionKind) -> Option<&BootSection> {
        self.index_for_kind(kind)
            .and_then(|index| self.slots[index].as_ref())
    }

    /// Insert a successful section and validate it before changing the slot.
    ///
    /// # Errors
    ///
    /// Returns [`BootCompositionError::SectionDisabled`] for a kind outside
    /// the profile, `DuplicateSection` for a repeated kind, and the typed
    /// width/length error for malformed source metadata. No slot is changed on
    /// error.
    pub fn insert(&mut self, section: BootSection) -> Result<(), BootCompositionError> {
        let kind = section.kind;
        let index = self
            .index_for_kind(kind)
            .ok_or(BootCompositionError::SectionDisabled { kind })?;
        if self.slots[index].is_some() {
            return Err(BootCompositionError::DuplicateSection { kind });
        }
        validate_section(&section)?;
        self.slots[index] = Some(section);
        Ok(())
    }

    /// Insert a loader result while preserving the empty-versus-error boundary.
    ///
    /// The declared kind is checked against the profile before the result is
    /// consumed. `Ok(section)` is accepted only when its carried kind agrees
    /// with the declaration. `Err(source)` is returned as
    /// [`BootSectionInsertError::Loader`] and never creates a zero-row
    /// fallback or otherwise changes the registry.
    ///
    /// # Errors
    ///
    /// Returns [`BootSectionInsertError::Loader`] for a source failure or
    /// [`BootSectionInsertError::Composition`] for a disabled kind, wrong
    /// returned kind, duplicate, or malformed section.
    pub fn insert_result<E>(
        &mut self,
        declared_kind: BootSectionKind,
        result: Result<BootSection, E>,
    ) -> Result<(), BootSectionInsertError<E>> {
        if self.index_for_kind(declared_kind).is_none() {
            return Err(BootSectionInsertError::Composition(
                BootCompositionError::SectionDisabled {
                    kind: declared_kind,
                },
            ));
        }
        let section = result.map_err(|source| BootSectionInsertError::Loader {
            kind: declared_kind,
            source,
        })?;
        if section.kind != declared_kind {
            return Err(BootSectionInsertError::Composition(
                BootCompositionError::KindMismatch {
                    expected: declared_kind,
                    actual: section.kind,
                },
            ));
        }
        self.insert(section)
            .map_err(BootSectionInsertError::Composition)
    }

    /// Materialize sections in the exact order selected by the bound profile.
    ///
    /// # Errors
    ///
    /// Returns [`BootCompositionError::MissingSection`] for the first absent
    /// profile slot or [`BootCompositionError::AllocationFailed`] if the
    /// bounded ordered vector cannot be reserved. The registry is consumed;
    /// no partial ordered result is returned.
    pub fn into_ordered(self) -> Result<Vec<BootSection>, BootCompositionError> {
        let mut ordered = Vec::new();
        ordered
            .try_reserve_exact(self.slots.len())
            .map_err(|_| BootCompositionError::AllocationFailed)?;
        for (index, slot) in self.slots.into_iter().enumerate() {
            let Some(section) = slot else {
                return Err(BootCompositionError::MissingSection {
                    kind: self.profile.section_kinds()[index],
                    index,
                });
            };
            ordered.push(section);
        }
        Ok(ordered)
    }

    fn index_for_kind(&self, kind: BootSectionKind) -> Option<usize> {
        self.profile
            .section_kinds()
            .iter()
            .position(|candidate| *candidate == kind)
    }
}

fn validate_section(section: &BootSection) -> Result<(), BootCompositionError> {
    if section.record_size == 0 {
        return Err(BootCompositionError::InvalidRecordSize {
            kind: section.kind,
            record_size: section.record_size,
        });
    }
    if let Some(expected) = source_fixed_record_size(section.kind) {
        if usize::from(section.record_size) != expected {
            return Err(BootCompositionError::InvalidKnownRecordSize {
                kind: section.kind,
                expected,
                actual: section.record_size,
            });
        }
    }
    let expected = usize::from(section.record_size)
        .checked_mul(usize::from(section.count))
        .ok_or(BootCompositionError::SizeOverflow { kind: section.kind })?;
    if section.data.len() != expected {
        return Err(BootCompositionError::InvalidSectionLength {
            kind: section.kind,
            expected,
            actual: section.data.len(),
        });
    }
    Ok(())
}

/// Return a verified fixed width for a source-bound table section.
///
/// The source-fixed active x86 widths below are enforced when a section enters
/// [`LoadedBootSections`]. Mob and item rows are still opaque to this module;
/// their verified 255-byte and 204-byte wire widths are independent of SQL,
/// CSV, cache, and production-startup policy. Base and renewal shop records
/// share the independently verified 2,762-byte active x86 wire layout.
#[must_use]
pub const fn source_fixed_record_size(kind: BootSectionKind) -> Option<usize> {
    match kind {
        BootSectionKind::Banword => Some(BANWORD_WIRE_SIZE),
        BootSectionKind::Refine => Some(REFINE_TABLE_WIRE_SIZE),
        BootSectionKind::Event => Some(EVENT_TABLE_WIRE_SIZE),
        BootSectionKind::Land => Some(LAND_RECORD_WIRE_SIZE),
        BootSectionKind::PremiumMarketPrice => Some(MARKET_ITEM_PRICE_WIRE_SIZE),
        BootSectionKind::Skill => Some(SKILL_TABLE_RECORD_WIRE_SIZE),
        BootSectionKind::Mob => Some(MOB_TABLE_RECORD_WIRE_SIZE),
        BootSectionKind::Item => Some(ITEM_TABLE_RECORD_WIRE_SIZE),
        BootSectionKind::Shop | BootSectionKind::RenewalShop => Some(SHOP_TABLE_RECORD_WIRE_SIZE),
        BootSectionKind::ObjectProto => Some(OBJECT_PROTO_RECORD_WIRE_SIZE),
        BootSectionKind::ItemAttr | BootSectionKind::ItemRare => Some(ITEM_ATTR_RECORD_WIRE_SIZE),
        BootSectionKind::Object => Some(OBJECT_RECORD_WIRE_SIZE),
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use super::*;

    fn section(kind: BootSectionKind) -> BootSection {
        BootSection {
            kind,
            record_size: source_fixed_record_size(kind)
                .and_then(|size| u16::try_from(size).ok())
                .unwrap_or(1),
            count: 0,
            data: Vec::new(),
        }
    }

    fn complete(profile: BootFeatureProfile) -> LoadedBootSections {
        let mut loaded = LoadedBootSections::try_new(profile).unwrap();
        for kind in profile.section_kinds() {
            loaded.insert(section(*kind)).unwrap();
        }
        loaded
    }

    #[test]
    fn item_attribute_sections_require_the_active_71_byte_record_width() {
        let profile = BootFeatureProfile::minimal();
        let mut loaded = LoadedBootSections::try_new(profile).unwrap();
        let normal = BootSection {
            kind: BootSectionKind::ItemAttr,
            record_size: 71,
            count: 0,
            data: Vec::new(),
        };
        let rare = BootSection {
            kind: BootSectionKind::ItemRare,
            record_size: 71,
            count: 0,
            data: Vec::new(),
        };
        loaded.insert(normal.clone()).unwrap();
        loaded.insert(rare.clone()).unwrap();
        assert_eq!(loaded.section(BootSectionKind::ItemAttr), Some(&normal));
        assert_eq!(loaded.section(BootSectionKind::ItemRare), Some(&rare));

        let mut wrong = LoadedBootSections::try_new(profile).unwrap();
        let bad = BootSection {
            kind: BootSectionKind::ItemAttr,
            record_size: 70,
            count: 0,
            data: Vec::new(),
        };
        assert!(matches!(
            wrong.insert(bad),
            Err(BootCompositionError::InvalidKnownRecordSize {
                kind: BootSectionKind::ItemAttr,
                expected: ITEM_ATTR_RECORD_WIRE_SIZE,
                actual: 70,
            })
        ));
    }

    #[test]
    fn all_explicit_profiles_materialize_in_wire_order() {
        for mask in 0_u8..8 {
            let profile = BootFeatureProfile::new(mask & 1 != 0, mask & 2 != 0, mask & 4 != 0);
            let ordered = complete(profile).into_ordered().unwrap();
            assert_eq!(ordered.len(), profile.section_kinds().len());
            assert_eq!(
                ordered
                    .iter()
                    .map(|section| section.kind)
                    .collect::<Vec<_>>(),
                profile.section_kinds()
            );
        }
    }

    #[test]
    fn successful_empty_sections_are_not_replaced_or_dropped() {
        let profile = BootFeatureProfile::minimal();
        let mut loaded = LoadedBootSections::try_new(profile).unwrap();
        let refine = section(BootSectionKind::Refine);
        loaded
            .insert_result(BootSectionKind::Refine, Ok::<_, Infallible>(refine.clone()))
            .unwrap();
        assert_eq!(loaded.section(BootSectionKind::Refine), Some(&refine));
        assert!(!loaded.is_complete());
        assert_eq!(
            loaded
                .into_ordered()
                .expect_err("other sections are still missing"),
            BootCompositionError::MissingSection {
                kind: BootSectionKind::Mob,
                index: 0
            }
        );
    }

    #[test]
    fn loader_errors_do_not_install_fallback_sections() {
        let profile = BootFeatureProfile::minimal();
        let mut loaded = LoadedBootSections::try_new(profile).unwrap();
        let result = loaded.insert_result(BootSectionKind::Refine, Err("database unavailable"));
        assert!(matches!(
            result,
            Err(BootSectionInsertError::Loader {
                kind: BootSectionKind::Refine,
                source: "database unavailable"
            })
        ));
        assert_eq!(loaded.section(BootSectionKind::Refine), None);
        assert!(!loaded.is_complete());
    }

    #[test]
    fn disabled_duplicate_wrong_kind_and_missing_sections_are_rejected() {
        let profile = BootFeatureProfile::minimal();
        let mut loaded = LoadedBootSections::try_new(profile).unwrap();
        assert_eq!(
            loaded.insert(section(BootSectionKind::Event)),
            Err(BootCompositionError::SectionDisabled {
                kind: BootSectionKind::Event
            })
        );
        loaded.insert(section(BootSectionKind::Mob)).unwrap();
        assert_eq!(
            loaded.insert(section(BootSectionKind::Mob)),
            Err(BootCompositionError::DuplicateSection {
                kind: BootSectionKind::Mob
            })
        );
        assert!(matches!(
            loaded.insert_result(
                BootSectionKind::Mob,
                Ok::<_, Infallible>(section(BootSectionKind::Item)),
            ),
            Err(BootSectionInsertError::Composition(
                BootCompositionError::KindMismatch {
                    expected: BootSectionKind::Mob,
                    actual: BootSectionKind::Item
                }
            ))
        ));
        assert!(!loaded.is_complete());
    }

    #[test]
    fn source_fixed_widths_and_data_lengths_are_checked_before_insertion() {
        let profile = BootFeatureProfile::active();
        let mut loaded = LoadedBootSections::try_new(profile).unwrap();
        let mut wrong_width = section(BootSectionKind::Refine);
        wrong_width.record_size = 1;
        assert!(matches!(
            loaded.insert(wrong_width),
            Err(BootCompositionError::InvalidKnownRecordSize {
                kind: BootSectionKind::Refine,
                expected: 53,
                actual: 1
            })
        ));

        let mut wrong_length = section(BootSectionKind::Event);
        wrong_length.count = 1;
        assert!(matches!(
            loaded.insert(wrong_length),
            Err(BootCompositionError::InvalidSectionLength {
                kind: BootSectionKind::Event,
                expected: 85,
                actual: 0
            })
        ));
        assert!(!loaded.is_complete());
    }

    #[test]
    fn source_fixed_mob_and_item_widths_are_enforced() {
        assert_eq!(source_fixed_record_size(BootSectionKind::Mob), Some(255));
        assert_eq!(source_fixed_record_size(BootSectionKind::Item), Some(204));

        let profile = BootFeatureProfile::minimal();
        let mut loaded = LoadedBootSections::try_new(profile).unwrap();

        let mut wrong_mob_width = section(BootSectionKind::Mob);
        wrong_mob_width.record_size = 254;
        assert!(matches!(
            loaded.insert(wrong_mob_width),
            Err(BootCompositionError::InvalidKnownRecordSize {
                kind: BootSectionKind::Mob,
                expected: 255,
                actual: 254
            })
        ));

        let mut wrong_item_width = section(BootSectionKind::Item);
        wrong_item_width.record_size = 203;
        assert!(matches!(
            loaded.insert(wrong_item_width),
            Err(BootCompositionError::InvalidKnownRecordSize {
                kind: BootSectionKind::Item,
                expected: 204,
                actual: 203
            })
        ));

        let mut zero_width = section(BootSectionKind::Item);
        zero_width.record_size = 0;
        assert!(matches!(
            loaded.insert(zero_width),
            Err(BootCompositionError::InvalidRecordSize {
                kind: BootSectionKind::Item,
                record_size: 0
            })
        ));
    }

    #[test]
    fn land_is_a_source_fixed_36_byte_section_in_every_profile() {
        assert_eq!(source_fixed_record_size(BootSectionKind::Land), Some(36));
        for mask in 0_u8..8 {
            let profile = BootFeatureProfile::new(mask & 1 != 0, mask & 2 != 0, mask & 4 != 0);
            let loaded = complete(profile);
            let land = loaded.section(BootSectionKind::Land).unwrap();
            assert_eq!(land.record_size, 36);
            assert_eq!(land.data.len(), 0);
        }
    }

    #[test]
    fn object_proto_is_a_source_fixed_96_byte_section_in_every_profile() {
        assert_eq!(
            source_fixed_record_size(BootSectionKind::ObjectProto),
            Some(96)
        );
        for mask in 0_u8..8 {
            let profile = BootFeatureProfile::new(mask & 1 != 0, mask & 2 != 0, mask & 4 != 0);
            let loaded = complete(profile);
            let object_proto = loaded.section(BootSectionKind::ObjectProto).unwrap();
            assert_eq!(object_proto.record_size, 96);
            assert_eq!(object_proto.data.len(), 0);
        }
    }

    #[test]
    fn skill_is_a_source_fixed_1475_byte_section_in_every_profile() {
        assert_eq!(
            source_fixed_record_size(BootSectionKind::Skill),
            Some(1_475)
        );
        for mask in 0_u8..8 {
            let profile = BootFeatureProfile::new(mask & 1 != 0, mask & 2 != 0, mask & 4 != 0);
            let loaded = complete(profile);
            let skill = loaded.section(BootSectionKind::Skill).unwrap();
            assert_eq!(skill.record_size, 1_475);
            assert_eq!(skill.data.len(), 0);
        }
    }

    #[test]
    fn base_and_renewal_shops_are_source_fixed_2762_byte_sections() {
        assert_eq!(source_fixed_record_size(BootSectionKind::Shop), Some(2_762));
        assert_eq!(
            source_fixed_record_size(BootSectionKind::RenewalShop),
            Some(2_762)
        );
        for mask in 0_u8..8 {
            let profile = BootFeatureProfile::new(mask & 1 != 0, mask & 2 != 0, mask & 4 != 0);
            let loaded = complete(profile);
            let shop = loaded.section(BootSectionKind::Shop).unwrap();
            assert_eq!(shop.record_size, 2_762);
            assert_eq!(shop.data.len(), 0);
            if profile.renewal_shop_ex {
                let renewal = loaded.section(BootSectionKind::RenewalShop).unwrap();
                assert_eq!(renewal.record_size, 2_762);
                assert_eq!(renewal.data.len(), 0);
            }
        }
    }
}
