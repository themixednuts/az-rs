//! Shared concrete-fragment reconciliation for Bevy replication state.

use std::{any::Any, error::Error, fmt};

use gridmate::hub::{Fragment, SequenceNumber};

pub fn merge_fragment<C>(
    previous: &C,
    incoming: &mut C,
    sequence: SequenceNumber,
) -> Result<C, FragmentMergeError>
where
    C: Fragment + 'static,
{
    if !sequence.is_valid() {
        return Err(FragmentMergeError::InvalidSequence);
    }
    let merged = previous
        .merge_and_update_sequence(incoming, sequence, false)
        .ok_or(FragmentMergeError::UnsupportedMerge)?;
    let merged: Box<dyn Any> = merged;
    merged
        .downcast::<C>()
        .map(|fragment| *fragment)
        .map_err(|_| FragmentMergeError::ConcreteTypeMismatch)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FragmentMergeError {
    InvalidSequence,
    NonIncreasingSequence {
        current: SequenceNumber,
        incoming: SequenceNumber,
    },
    UnsupportedMerge,
    ConcreteTypeMismatch,
}

impl fmt::Display for FragmentMergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSequence => f.write_str("fragment merge sequence is invalid"),
            Self::NonIncreasingSequence { current, incoming } => write!(
                f,
                "fragment publication sequence {incoming:?} does not follow {current:?}"
            ),
            Self::UnsupportedMerge => {
                f.write_str("fragment type does not support state reconciliation")
            }
            Self::ConcreteTypeMismatch => {
                f.write_str("fragment merge returned a different concrete type")
            }
        }
    }
}

impl Error for FragmentMergeError {}
