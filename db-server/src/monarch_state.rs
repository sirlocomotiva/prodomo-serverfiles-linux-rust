//! Immutable, transport-free boot state for Monarch values.
//!
//! The legacy DB server declares a fixed `TMonarchInfo`, an insertion-ordered
//! `std::vector<MonarchCandidacy>`, and a `std::map<DWORD,
//! MonarchElectionInfo*>` in `server/server/db/Monarch.h:11-67`. This module
//! intentionally models only the values that can be projected into the active
//! boot tail. It reuses [`protocol::db_boot::BootMonarchInfo`] and
//! [`protocol::db_boot::BootMonarchCandidacy`], so the existing 304-byte and
//! 68-byte x86 wire shapes remain the only record layouts in this slice.
//!
//! The active startup path calls only `CMonarch::LoadMonarch`
//! (`ClientManagerBoot.cpp:1153-1157`). The visible candidate and election
//! statements are mutation SQL, not audited acquisition queries. In addition,
//! the legacy candidate and election records leave their date fields
//! uninitialized. Therefore this module has no election model, no date
//! generation, no mutation API, and no implicit empty state. The incumbent and
//! any candidate bytes must be supplied explicitly by a future caller that has
//! a separate source boundary. This module performs no SQL, transport,
//! authentication, cache, persistence, or gameplay operation.

use std::error::Error;
use std::fmt;

use protocol::db_boot::{
    BootMonarchCandidacy, BootMonarchCandidacySection, BootMonarchInfo, MONARCH_CANDIDACY_WIRE_SIZE,
};

/// Maximum candidate count representable by the legacy boot `WORD` count.
pub const MAX_MONARCH_CANDIDATES: usize = u16::MAX as usize;

const MONARCH_CANDIDACY_RECORD_SIZE: u16 = 68;
const _: () = assert!(MONARCH_CANDIDACY_RECORD_SIZE as usize == MONARCH_CANDIDACY_WIRE_SIZE);

/// A checked failure while constructing or projecting a Monarch boot state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonarchBootStateError {
    /// The candidate vector cannot be represented by the boot `WORD` count.
    CandidateCountOverflow {
        /// Candidate count after the attempted operation.
        count: usize,
        /// Maximum count representable by the boot count.
        maximum: usize,
    },
    /// A candidate PID already exists in the supplied vector.
    DuplicateCandidate {
        /// Repeated candidate PID.
        pid: u32,
    },
    /// The fixed candidate record width could not be represented as a `u16`.
    RecordSizeOverflow {
        /// Computed record width.
        size: usize,
    },
    /// A fallible reserve for validation or a boot projection failed.
    AllocationFailed,
}

impl fmt::Display for MonarchBootStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CandidateCountOverflow { count, maximum } => write!(
                formatter,
                "Monarch state has {count} candidates; maximum is {maximum}"
            ),
            Self::DuplicateCandidate { pid } => {
                write!(formatter, "Monarch candidate PID {pid} already exists")
            }
            Self::RecordSizeOverflow { size } => {
                write!(
                    formatter,
                    "Monarch candidacy record size {size} does not fit in u16"
                )
            }
            Self::AllocationFailed => formatter.write_str("Monarch state allocation failed"),
        }
    }
}

impl Error for MonarchBootStateError {}

/// The immutable, boot-visible portion of an explicitly resolved Monarch state.
///
/// The candidate vector retains the order supplied by the caller. No method
/// mutates either the incumbent or candidates, and this type is not a database
/// cache or a claim that a candidate query exists. Elections are intentionally
/// absent because the active source does not acquire them for startup and the
/// boot tail does not carry them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonarchBootState {
    info: BootMonarchInfo,
    candidates: Box<[BootMonarchCandidacy]>,
    candidate_count: u16,
}

/// Values that a boot sender needs from one explicit Monarch boot state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonarchBootProjection {
    /// The fixed incumbent record.
    pub info: BootMonarchInfo,
    /// The candidate section in supplied order.
    pub candidacy: BootMonarchCandidacySection,
}

impl MonarchBootState {
    /// Construct a checked state from complete caller-owned values.
    ///
    /// The incumbent is always explicit; this API intentionally has no
    /// `empty()` or zero-initializing constructor. A caller may explicitly
    /// provide an empty candidate vector, but must not turn a database
    /// `EmptyResult` into an incumbent value. Elections are not accepted because
    /// the active source has no audited startup acquisition or boot projection
    /// for them.
    ///
    /// Candidate order is preserved. Candidate PIDs must be unique, and the
    /// count must fit the boot `WORD` count. The method does not query,
    /// mutate, or persist any source.
    ///
    /// # Errors
    ///
    /// Returns [`MonarchBootStateError::CandidateCountOverflow`] for a count above
    /// `u16::MAX`, [`MonarchBootStateError::DuplicateCandidate`] for repeated
    /// candidate PIDs, or [`MonarchBootStateError::AllocationFailed`] if the
    /// temporary PID validation vector cannot be reserved.
    pub fn try_new(
        info: BootMonarchInfo,
        candidates: Vec<BootMonarchCandidacy>,
    ) -> Result<Self, MonarchBootStateError> {
        if candidates.len() > MAX_MONARCH_CANDIDATES {
            return Err(MonarchBootStateError::CandidateCountOverflow {
                count: candidates.len(),
                maximum: MAX_MONARCH_CANDIDATES,
            });
        }

        let mut candidate_pids = Vec::new();
        candidate_pids
            .try_reserve_exact(candidates.len())
            .map_err(|_| MonarchBootStateError::AllocationFailed)?;
        candidate_pids.extend(candidates.iter().map(|candidate| candidate.pid));
        candidate_pids.sort_unstable();
        for pair in candidate_pids.windows(2) {
            if pair[0] == pair[1] {
                return Err(MonarchBootStateError::DuplicateCandidate { pid: pair[0] });
            }
        }

        let candidate_count = u16::try_from(candidates.len()).map_err(|_| {
            MonarchBootStateError::CandidateCountOverflow {
                count: candidates.len(),
                maximum: MAX_MONARCH_CANDIDATES,
            }
        })?;

        Ok(Self {
            info,
            candidates: candidates.into_boxed_slice(),
            candidate_count,
        })
    }

    /// Return the explicitly supplied incumbent record.
    #[must_use]
    pub const fn info(&self) -> BootMonarchInfo {
        self.info
    }

    /// Borrow candidates in the caller-supplied order.
    #[must_use]
    pub fn candidates(&self) -> &[BootMonarchCandidacy] {
        &self.candidates
    }

    /// Find a candidate by PID.
    #[must_use]
    pub fn candidate(&self, pid: u32) -> Option<&BootMonarchCandidacy> {
        self.candidates
            .iter()
            .find(|candidate| candidate.pid == pid)
    }

    /// Return the current candidate count.
    #[must_use]
    pub fn candidate_count(&self) -> usize {
        self.candidates.len()
    }

    /// Project the incumbent and candidate values for a boot sender.
    ///
    /// The candidate vector is copied into the existing protocol section type.
    /// No election or other hidden state is synthesized. The fixed 68-byte
    /// record width and the candidate count are checked before projection.
    ///
    /// # Errors
    ///
    /// Returns [`MonarchBootStateError::CandidateCountOverflow`] if an internally
    /// inconsistent state exceeds the `u16` count, [`MonarchBootStateError::
    /// RecordSizeOverflow`] if the fixed 68-byte width cannot be represented,
    /// or [`MonarchBootStateError::AllocationFailed`] if the projection vector
    /// cannot reserve its checked length.
    pub fn project_for_boot(&self) -> Result<MonarchBootProjection, MonarchBootStateError> {
        let record_size = u16::try_from(MONARCH_CANDIDACY_WIRE_SIZE).map_err(|_| {
            MonarchBootStateError::RecordSizeOverflow {
                size: MONARCH_CANDIDACY_WIRE_SIZE,
            }
        })?;
        let count = u16::try_from(self.candidates.len()).map_err(|_| {
            MonarchBootStateError::CandidateCountOverflow {
                count: self.candidates.len(),
                maximum: MAX_MONARCH_CANDIDATES,
            }
        })?;
        let mut candidates = Vec::new();
        candidates
            .try_reserve_exact(self.candidates.len())
            .map_err(|_| MonarchBootStateError::AllocationFailed)?;
        candidates.extend_from_slice(&self.candidates);
        Ok(MonarchBootProjection {
            info: self.info,
            candidacy: BootMonarchCandidacySection {
                record_size,
                count,
                candidates,
            },
        })
    }

    /// Consume this validated state and project it without cloning candidates.
    ///
    /// Construction has already checked the `u16` count and fixed record
    /// width, so this infallible consuming projection cannot introduce a new
    /// overflow or allocation failure.
    #[must_use]
    pub fn into_projection(self) -> MonarchBootProjection {
        let record_size = MONARCH_CANDIDACY_RECORD_SIZE;
        let count = self.candidate_count;
        MonarchBootProjection {
            info: self.info,
            candidacy: BootMonarchCandidacySection {
                record_size,
                count,
                candidates: self.candidates.into_vec(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::db_boot::MONARCH_CANDIDACY_WIRE_SIZE;

    fn candidate(pid: u32, name: &[u8], date: &[u8]) -> BootMonarchCandidacy {
        let mut name_bytes = [0; 32];
        let mut date_bytes = [0; 32];
        assert!(name.len() <= 32);
        assert!(date.len() <= 32);
        name_bytes[..name.len()].copy_from_slice(name);
        date_bytes[..date.len()].copy_from_slice(date);
        BootMonarchCandidacy {
            pid,
            name: name_bytes,
            date: date_bytes,
        }
    }

    fn info() -> BootMonarchInfo {
        BootMonarchInfo {
            pid: [11, 22, 33, 44],
            money: [1, 2, 3, 4],
            name: [[b'i'; 32]; 4],
            date: [[b'd'; 32]; 4],
        }
    }

    #[test]
    fn explicit_state_preserves_candidate_order() {
        let first = candidate(30, b"first", b"2025-01-01");
        let second = candidate(10, b"second", b"2025-01-02");
        let state = MonarchBootState::try_new(info(), vec![first, second]).unwrap();

        assert_eq!(state.info(), info());
        assert_eq!(state.candidates(), &[first, second]);
        assert_eq!(state.candidate_count(), 2);
        assert_eq!(state.candidate(10).unwrap().name[..6], *b"second");
    }

    #[test]
    fn explicit_empty_candidate_vector_is_not_an_empty_incumbent() {
        let state = MonarchBootState::try_new(info(), vec![]).unwrap();
        let projection = state.project_for_boot().unwrap();

        assert_eq!(projection.info, info());
        assert_eq!(
            projection.candidacy.record_size as usize,
            MONARCH_CANDIDACY_WIRE_SIZE
        );
        assert_eq!(projection.candidacy.count, 0);
        assert!(projection.candidacy.candidates.is_empty());
    }

    #[test]
    fn consuming_projection_moves_candidates_without_cloning() {
        let candidate = candidate(99, b"name", b"date");
        let state = MonarchBootState::try_new(info(), vec![candidate]).unwrap();
        let projection = state.into_projection();

        assert_eq!(projection.candidacy.record_size, 68);
        assert_eq!(projection.candidacy.count, 1);
        assert_eq!(projection.candidacy.candidates, vec![candidate]);
    }

    #[test]
    fn duplicate_candidate_pid_is_rejected() {
        assert_eq!(
            MonarchBootState::try_new(
                info(),
                vec![
                    candidate(7, b"first", b"date"),
                    candidate(7, b"other", b"other")
                ],
            ),
            Err(MonarchBootStateError::DuplicateCandidate { pid: 7 })
        );
    }

    #[test]
    fn candidate_count_is_bounded_by_wire_word() {
        let too_many = vec![candidate(0, b"", b""); MAX_MONARCH_CANDIDATES + 1];
        assert_eq!(
            MonarchBootState::try_new(info(), too_many),
            Err(MonarchBootStateError::CandidateCountOverflow {
                count: MAX_MONARCH_CANDIDATES + 1,
                maximum: MAX_MONARCH_CANDIDATES,
            })
        );
    }

    #[test]
    fn projection_preserves_complete_candidate_values() {
        let candidate = candidate(99, b"name", b"date");
        let state = MonarchBootState::try_new(info(), vec![candidate]).unwrap();
        let projection = state.project_for_boot().unwrap();

        assert_eq!(projection.candidacy.count, 1);
        assert_eq!(projection.candidacy.candidates[0], candidate);
        assert_eq!(projection.info, state.info());
    }
}
