use super::{
    HistoryCheckpoint, HistoryError, MerkleHistory, largest_power_of_two_less_than, node_hash,
    tree_hash,
};

/// Logarithmic proof that one checkpoint is an exact prefix of another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryConsistencyProof {
    old_size: u64,
    new_size: u64,
    siblings: Vec<[u8; 32]>,
}

impl HistoryConsistencyProof {
    /// Returns the earlier checkpoint size.
    #[must_use]
    pub const fn old_size(&self) -> u64 {
        self.old_size
    }

    /// Returns the later checkpoint size.
    #[must_use]
    pub const fn new_size(&self) -> u64 {
        self.new_size
    }

    /// Returns the bounded number of hashes in the proof.
    #[must_use]
    pub const fn sibling_count(&self) -> usize {
        self.siblings.len()
    }
}

impl MerkleHistory {
    /// Proves that an earlier checkpoint is an exact prefix of a later one.
    ///
    /// # Errors
    ///
    /// Returns a closed error for a shrinking range or an uncommitted size.
    pub fn consistency_proof(
        &self,
        old_size: u64,
        new_size: u64,
    ) -> Result<HistoryConsistencyProof, HistoryError> {
        if old_size > new_size {
            return Err(HistoryError::InvalidConsistencyRange { old_size, new_size });
        }
        self.checkpoint_at(new_size)?;
        let available = self.checkpoint().tree_size();
        let old = usize::try_from(old_size).map_err(|_| HistoryError::TreeSizeUnavailable {
            requested: old_size,
            available,
        })?;
        let new = usize::try_from(new_size).map_err(|_| HistoryError::TreeSizeUnavailable {
            requested: new_size,
            available,
        })?;
        let mut siblings = Vec::with_capacity(64);
        if old != 0 && old != new {
            build_consistency_path(&self.leaves[..new], old, true, &mut siblings);
        }
        Ok(HistoryConsistencyProof {
            old_size,
            new_size,
            siblings,
        })
    }
}

/// Verifies that `old` is an exact prefix of `new` under one proof.
#[must_use]
pub fn verify_history_consistency(
    old: HistoryCheckpoint,
    new: HistoryCheckpoint,
    proof: &HistoryConsistencyProof,
) -> bool {
    if proof.old_size != old.tree_size()
        || proof.new_size != new.tree_size()
        || proof.old_size > proof.new_size
    {
        return false;
    }
    if proof.old_size == 0 {
        return proof.siblings.is_empty() && old.root().0 == tree_hash(&[]);
    }
    if proof.old_size == proof.new_size {
        return proof.siblings.is_empty() && old.root() == new.root();
    }

    let mut old_node = proof.old_size - 1;
    let mut new_node = proof.new_size - 1;
    while old_node & 1 == 1 {
        old_node >>= 1;
        new_node >>= 1;
    }

    let (mut old_root, mut new_root, remaining) = if old_node == 0 {
        (old.root().0, old.root().0, proof.siblings.as_slice())
    } else {
        let Some((first, remaining)) = proof.siblings.split_first() else {
            return false;
        };
        (*first, *first, remaining)
    };

    for sibling in remaining {
        if new_node == 0 {
            return false;
        }
        if old_node & 1 == 1 || old_node == new_node {
            old_root = node_hash(*sibling, old_root);
            new_root = node_hash(*sibling, new_root);
            while old_node != 0 && old_node & 1 == 0 {
                old_node >>= 1;
                new_node >>= 1;
            }
        } else {
            new_root = node_hash(new_root, *sibling);
        }
        old_node >>= 1;
        new_node >>= 1;
    }

    old_root == old.root().0 && new_root == new.root().0 && new_node == 0
}

fn build_consistency_path(
    leaves: &[super::ReceiptCommitment],
    old_size: usize,
    include_old_root: bool,
    siblings: &mut Vec<[u8; 32]>,
) {
    if old_size == leaves.len() {
        if !include_old_root {
            siblings.push(tree_hash(leaves));
        }
        return;
    }
    let split = largest_power_of_two_less_than(leaves.len());
    if old_size <= split {
        build_consistency_path(&leaves[..split], old_size, include_old_root, siblings);
        siblings.push(tree_hash(&leaves[split..]));
    } else {
        build_consistency_path(&leaves[split..], old_size - split, false, siblings);
        siblings.push(tree_hash(&leaves[..split]));
    }
}
