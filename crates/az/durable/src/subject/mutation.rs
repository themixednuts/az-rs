use arrayvec::ArrayVec;

use crate::{
    CanonicalBytes, CodecError, CodecId, CodecVersion, DurableCodec, DurableSubject,
    journal::JournalClause, outbox::OutboxClause,
};

/// Maximum number of state, journal, outbox, and deadline clauses in one atom.
pub const MAX_MUTATION_CLAUSES: usize = 32;

/// Bounded sealed plan for one atomic subject mutation.
pub struct SubjectMutation<T: DurableSubject> {
    pub(crate) replacement: Option<CanonicalBytes>,
    pub(crate) codec_id: CodecId,
    pub(crate) codec_version: CodecVersion,
    pub(crate) journal: ArrayVec<JournalClause, MAX_MUTATION_CLAUSES>,
    pub(crate) outbox: ArrayVec<OutboxClause, MAX_MUTATION_CLAUSES>,
    marker: std::marker::PhantomData<fn() -> T>,
}

impl<T: DurableSubject> SubjectMutation<T> {
    /// Makes no state replacement.
    #[must_use]
    pub const fn unchanged() -> Self {
        Self {
            replacement: None,
            codec_id: T::State::CODEC_ID,
            codec_version: T::State::CURRENT_VERSION,
            journal: ArrayVec::new_const(),
            outbox: ArrayVec::new_const(),
            marker: std::marker::PhantomData,
        }
    }

    /// Replaces state after validating its canonical encoding and bound.
    ///
    /// # Errors
    ///
    /// Returns a codec/build failure before any storage effect.
    pub fn replace(next: &T::State) -> Result<Self, MutationBuildError> {
        let replacement = next.encode_canonical()?;
        Ok(Self {
            replacement: Some(replacement),
            codec_id: T::State::CODEC_ID,
            codec_version: T::State::CURRENT_VERSION,
            journal: ArrayVec::new_const(),
            outbox: ArrayVec::new_const(),
            marker: std::marker::PhantomData,
        })
    }

    /// Returns the number of atomic mutation clauses.
    #[must_use]
    pub fn clause_count(&self) -> usize {
        usize::from(self.replacement.is_some()) + self.journal.len() + self.outbox.len()
    }

    /// Returns the validated canonical payload size.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        self.replacement.as_ref().map_or(0, CanonicalBytes::len)
            + self
                .journal
                .iter()
                .map(|record| record.bytes.len())
                .sum::<usize>()
            + self
                .outbox
                .iter()
                .map(|effect| effect.bytes.len())
                .sum::<usize>()
    }

    /// Returns the sealed replacement bytes, if this mutation replaces state.
    #[must_use]
    pub const fn replacement(&self) -> Option<&CanonicalBytes> {
        self.replacement.as_ref()
    }

    /// Returns the codec identity and version bound when the mutation was sealed.
    #[must_use]
    pub const fn codec(&self) -> (CodecId, CodecVersion) {
        (self.codec_id, self.codec_version)
    }

    pub(crate) fn push_journal(&mut self, clause: JournalClause) -> Result<(), MutationBuildError> {
        if self.clause_count() == MAX_MUTATION_CLAUSES {
            return Err(MutationBuildError::TooManyClauses {
                maximum: MAX_MUTATION_CLAUSES,
            });
        }
        let encoded_len = self.encoded_len().checked_add(clause.bytes.len()).ok_or(
            MutationBuildError::EncodedSizeExceeded {
                actual: usize::MAX,
                maximum: crate::MAX_CANONICAL_VALUE_BYTES,
            },
        )?;
        if encoded_len > crate::MAX_CANONICAL_VALUE_BYTES {
            return Err(MutationBuildError::EncodedSizeExceeded {
                actual: encoded_len,
                maximum: crate::MAX_CANONICAL_VALUE_BYTES,
            });
        }
        self.journal.push(clause);
        Ok(())
    }

    pub(crate) fn push_outbox(&mut self, clause: OutboxClause) -> Result<(), MutationBuildError> {
        if self.clause_count() == MAX_MUTATION_CLAUSES {
            return Err(MutationBuildError::TooManyClauses {
                maximum: MAX_MUTATION_CLAUSES,
            });
        }
        let encoded_len = self.encoded_len().checked_add(clause.bytes.len()).ok_or(
            MutationBuildError::EncodedSizeExceeded {
                actual: usize::MAX,
                maximum: crate::MAX_CANONICAL_VALUE_BYTES,
            },
        )?;
        if encoded_len > crate::MAX_CANONICAL_VALUE_BYTES {
            return Err(MutationBuildError::EncodedSizeExceeded {
                actual: encoded_len,
                maximum: crate::MAX_CANONICAL_VALUE_BYTES,
            });
        }
        self.outbox.push(clause);
        Ok(())
    }
}

/// Failure to construct a bounded subject mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MutationBuildError {
    /// Atomic mutation exceeded its fixed clause bound.
    #[error("durable mutation exceeds {maximum} clauses")]
    TooManyClauses {
        /// Accepted clause count.
        maximum: usize,
    },
    /// Combined canonical clause payload exceeded its fixed byte bound.
    #[error("durable mutation has {actual} encoded bytes; maximum is {maximum}")]
    EncodedSizeExceeded {
        /// Observed canonical byte count.
        actual: usize,
        /// Accepted canonical byte count.
        maximum: usize,
    },
    /// Canonical state encoding failed.
    #[error(transparent)]
    Codec(#[from] CodecError),
}
