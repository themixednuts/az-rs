/// Domain separator for randomized receipt commitments.
pub const RECEIPT_COMMITMENT_V1_CONTEXT: &str = "azoth certified federation receipt commitment v1";

/// Domain separator for the RFC-9162-shaped Merkle construction.
pub const TRANSPARENCY_MERKLE_V1_CONTEXT: &str =
    "azoth certified federation transparency merkle v1";

mod consistency;
mod signed_checkpoint;
mod witness_monitor;

pub use consistency::{HistoryConsistencyProof, verify_history_consistency};
pub use signed_checkpoint::{
    HISTORY_CHECKPOINT_V1_CONTEXT, SignedHistoryCheckpoint, VerifiedHistoryCheckpoint,
    sign_history_checkpoint, verify_history_checkpoint,
};
pub use witness_monitor::{
    ForkEvidence, ForkReason, HistoryWitnessMonitor, MonitorCheckpointOutcome, MonitorError,
};

/// Digest of one canonical authority receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiptDigest([u8; 32]);

impl ReceiptDigest {
    /// Constructs a digest from canonical receipt bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Independent random nonce used to hide a receipt digest in public history.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CommitmentNonce([u8; 32]);

impl CommitmentNonce {
    /// Constructs a nonce from cryptographically random bytes supplied by the caller.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the nonce only to the private receipt-opening path.
    #[must_use]
    pub const fn expose_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for CommitmentNonce {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CommitmentNonce([REDACTED])")
    }
}

/// Randomized public commitment to one private authority receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiptCommitment([u8; 32]);

impl ReceiptCommitment {
    /// Commits to a canonical receipt digest under an independent nonce.
    #[must_use]
    pub fn new(nonce: CommitmentNonce, receipt: ReceiptDigest) -> Self {
        let mut hasher = blake3::Hasher::new_derive_key(RECEIPT_COMMITMENT_V1_CONTEXT);
        hasher.update(nonce.expose_bytes());
        hasher.update(receipt.as_bytes());
        Self(*hasher.finalize().as_bytes())
    }

    /// Returns the public commitment bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Verifies a private opening without exposing the receipt in public history.
    #[must_use]
    pub fn matches(self, nonce: CommitmentNonce, receipt: ReceiptDigest) -> bool {
        constant_time_equal(&self.0, &Self::new(nonce, receipt).0)
    }
}

/// Root of one exact-size transparency history checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HistoryRoot([u8; 32]);

impl HistoryRoot {
    /// Returns the root bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Immutable commitment to one exact prefix of transparency history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HistoryCheckpoint {
    tree_size: u64,
    root: HistoryRoot,
}

impl HistoryCheckpoint {
    /// Returns the number of committed leaves.
    #[must_use]
    pub const fn tree_size(self) -> u64 {
        self.tree_size
    }

    /// Returns the Merkle root for that exact prefix.
    #[must_use]
    pub const fn root(self) -> HistoryRoot {
        self.root
    }
}

/// Inclusion path for one leaf at one exact history size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptInclusionProof {
    leaf_index: u64,
    tree_size: u64,
    siblings: Vec<[u8; 32]>,
}

impl ReceiptInclusionProof {
    /// Returns the zero-based leaf index.
    #[must_use]
    pub const fn leaf_index(&self) -> u64 {
        self.leaf_index
    }

    /// Returns the exact checkpoint size this path proves against.
    #[must_use]
    pub const fn tree_size(&self) -> u64 {
        self.tree_size
    }

    /// Returns the bounded number of sibling hashes in the path.
    #[must_use]
    pub const fn sibling_count(&self) -> usize {
        self.siblings.len()
    }
}

/// Closed history construction and proof failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HistoryError {
    /// The requested leaf is not part of the requested tree prefix.
    #[error("leaf {leaf_index} is outside transparency tree size {tree_size}")]
    LeafOutsideTree {
        /// Zero-based requested leaf index.
        leaf_index: u64,
        /// Requested checkpoint size.
        tree_size: u64,
    },
    /// The requested prefix has not been committed by this history.
    #[error("transparency tree size {requested} is unavailable; current size is {available}")]
    TreeSizeUnavailable {
        /// Requested prefix size.
        requested: u64,
        /// Current committed size.
        available: u64,
    },
    /// A consistency proof cannot shrink a committed history.
    #[error("invalid transparency consistency range {old_size} -> {new_size}")]
    InvalidConsistencyRange {
        /// Earlier checkpoint size.
        old_size: u64,
        /// Proposed later checkpoint size.
        new_size: u64,
    },
    /// The platform cannot represent another leaf safely.
    #[error("transparency history exhausted its representable tree size")]
    TreeSizeExhausted,
}

/// Deterministic in-memory reference history used by adapter conformance tests.
#[derive(Debug, Clone, Default)]
pub struct MerkleHistory {
    leaves: Vec<ReceiptCommitment>,
}

impl MerkleHistory {
    /// Constructs an empty history.
    #[must_use]
    pub const fn new() -> Self {
        Self { leaves: Vec::new() }
    }

    /// Appends one public receipt commitment and returns its zero-based index.
    ///
    /// # Errors
    ///
    /// Returns [`HistoryError::TreeSizeExhausted`] when the platform length
    /// cannot be represented in the protocol's `u64` tree size.
    pub fn append(&mut self, commitment: ReceiptCommitment) -> Result<u64, HistoryError> {
        let index =
            u64::try_from(self.leaves.len()).map_err(|_| HistoryError::TreeSizeExhausted)?;
        self.leaves.push(commitment);
        Ok(index)
    }

    /// Returns the checkpoint for the full committed history.
    #[must_use]
    pub fn checkpoint(&self) -> HistoryCheckpoint {
        let tree_size = u64::try_from(self.leaves.len()).unwrap_or(u64::MAX);
        HistoryCheckpoint {
            tree_size,
            root: HistoryRoot(tree_hash(&self.leaves)),
        }
    }

    /// Returns the checkpoint for an already committed prefix.
    ///
    /// # Errors
    ///
    /// Returns [`HistoryError::TreeSizeUnavailable`] for a future or
    /// unrepresentable prefix.
    pub fn checkpoint_at(&self, tree_size: u64) -> Result<HistoryCheckpoint, HistoryError> {
        let available = u64::try_from(self.leaves.len()).unwrap_or(u64::MAX);
        let size = usize::try_from(tree_size).map_err(|_| HistoryError::TreeSizeUnavailable {
            requested: tree_size,
            available,
        })?;
        if size > self.leaves.len() {
            return Err(HistoryError::TreeSizeUnavailable {
                requested: tree_size,
                available,
            });
        }
        Ok(HistoryCheckpoint {
            tree_size,
            root: HistoryRoot(tree_hash(&self.leaves[..size])),
        })
    }

    /// Builds a bounded inclusion path for a committed leaf and prefix.
    ///
    /// # Errors
    ///
    /// Returns a closed error for a future prefix or leaf outside that prefix.
    pub fn inclusion_proof(
        &self,
        leaf_index: u64,
        tree_size: u64,
    ) -> Result<ReceiptInclusionProof, HistoryError> {
        let checkpoint = self.checkpoint_at(tree_size)?;
        if leaf_index >= checkpoint.tree_size {
            return Err(HistoryError::LeafOutsideTree {
                leaf_index,
                tree_size,
            });
        }
        let size = usize::try_from(tree_size).map_err(|_| HistoryError::TreeSizeUnavailable {
            requested: tree_size,
            available: self.checkpoint().tree_size,
        })?;
        let index = usize::try_from(leaf_index).map_err(|_| HistoryError::LeafOutsideTree {
            leaf_index,
            tree_size,
        })?;
        let mut siblings = Vec::with_capacity(64);
        build_inclusion_path(&self.leaves[..size], index, &mut siblings);
        Ok(ReceiptInclusionProof {
            leaf_index,
            tree_size,
            siblings,
        })
    }
}

/// Verifies a receipt commitment against one exact checkpoint.
#[must_use]
pub fn verify_receipt_inclusion(
    commitment: ReceiptCommitment,
    proof: &ReceiptInclusionProof,
    checkpoint: HistoryCheckpoint,
) -> bool {
    if proof.tree_size != checkpoint.tree_size || proof.leaf_index >= proof.tree_size {
        return false;
    }
    let Ok(size) = usize::try_from(proof.tree_size) else {
        return false;
    };
    let Ok(index) = usize::try_from(proof.leaf_index) else {
        return false;
    };
    let mut sibling_index = 0;
    let Some(root) = rebuild_root(
        leaf_hash(commitment),
        index,
        size,
        &proof.siblings,
        &mut sibling_index,
    ) else {
        return false;
    };
    sibling_index == proof.siblings.len() && root == checkpoint.root.0
}

fn build_inclusion_path(
    leaves: &[ReceiptCommitment],
    leaf_index: usize,
    siblings: &mut Vec<[u8; 32]>,
) {
    if leaves.len() == 1 {
        return;
    }
    let split = largest_power_of_two_less_than(leaves.len());
    if leaf_index < split {
        build_inclusion_path(&leaves[..split], leaf_index, siblings);
        siblings.push(tree_hash(&leaves[split..]));
    } else {
        build_inclusion_path(&leaves[split..], leaf_index - split, siblings);
        siblings.push(tree_hash(&leaves[..split]));
    }
}

fn rebuild_root(
    leaf: [u8; 32],
    leaf_index: usize,
    tree_size: usize,
    siblings: &[[u8; 32]],
    sibling_index: &mut usize,
) -> Option<[u8; 32]> {
    if tree_size == 1 {
        return Some(leaf);
    }
    let split = largest_power_of_two_less_than(tree_size);
    let child = if leaf_index < split {
        rebuild_root(leaf, leaf_index, split, siblings, sibling_index)?
    } else {
        rebuild_root(
            leaf,
            leaf_index - split,
            tree_size - split,
            siblings,
            sibling_index,
        )?
    };
    let sibling = *siblings.get(*sibling_index)?;
    *sibling_index += 1;
    Some(if leaf_index < split {
        node_hash(child, sibling)
    } else {
        node_hash(sibling, child)
    })
}

pub fn tree_hash(leaves: &[ReceiptCommitment]) -> [u8; 32] {
    match leaves {
        [] => empty_hash(),
        [leaf] => leaf_hash(*leaf),
        _ => {
            let split = largest_power_of_two_less_than(leaves.len());
            node_hash(tree_hash(&leaves[..split]), tree_hash(&leaves[split..]))
        }
    }
}

pub fn largest_power_of_two_less_than(value: usize) -> usize {
    debug_assert!(value > 1);
    let exponent = usize::BITS - (value - 1).leading_zeros() - 1;
    1_usize << exponent
}

fn empty_hash() -> [u8; 32] {
    domain_hash(2, &[])
}

fn leaf_hash(commitment: ReceiptCommitment) -> [u8; 32] {
    domain_hash(0, commitment.as_bytes())
}

pub fn node_hash(left: [u8; 32], right: [u8; 32]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_derive_key(TRANSPARENCY_MERKLE_V1_CONTEXT);
    hasher.update(&[1]);
    hasher.update(&left);
    hasher.update(&right);
    *hasher.finalize().as_bytes()
}

fn domain_hash(kind: u8, bytes: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_derive_key(TRANSPARENCY_MERKLE_V1_CONTEXT);
    hasher.update(&[kind]);
    hasher.update(bytes);
    *hasher.finalize().as_bytes()
}

fn constant_time_equal(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}
