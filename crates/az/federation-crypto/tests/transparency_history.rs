use az_federation_crypto::{
    CommitmentNonce, HistoryError, MerkleHistory, ReceiptCommitment, ReceiptDigest,
    verify_history_consistency, verify_receipt_inclusion,
};

const fn receipt(byte: u8) -> ReceiptDigest {
    ReceiptDigest::from_bytes([byte; 32])
}

const fn nonce(byte: u8) -> CommitmentNonce {
    CommitmentNonce::from_bytes([byte; 32])
}

fn commitment(byte: u8) -> ReceiptCommitment {
    ReceiptCommitment::new(nonce(byte.wrapping_add(0x40)), receipt(byte))
}

#[test]
fn every_leaf_verifies_at_each_committed_tree_size() {
    let mut history = MerkleHistory::new();
    let commitments: Vec<_> = (1..=8).map(commitment).collect();

    for (position, commitment) in commitments.iter().copied().enumerate() {
        history.append(commitment).expect("append");
        let checkpoint = history.checkpoint();
        for (included, expected) in commitments[..=position].iter().copied().enumerate() {
            let proof = history
                .inclusion_proof(included as u64, checkpoint.tree_size())
                .expect("proof");
            assert!(verify_receipt_inclusion(expected, &proof, checkpoint));
            assert!(proof.sibling_count() <= 64);
        }
    }
}

#[test]
fn a_past_checkpoint_and_proof_remain_valid_after_later_appends() {
    let mut history = MerkleHistory::new();
    for byte in 1..=3 {
        history.append(commitment(byte)).expect("append");
    }
    let checkpoint = history.checkpoint();
    let proof = history.inclusion_proof(1, 3).expect("proof");

    for byte in 4..=9 {
        history.append(commitment(byte)).expect("append");
    }

    assert!(verify_receipt_inclusion(commitment(2), &proof, checkpoint));
    assert_eq!(
        history.checkpoint_at(3).expect("past checkpoint"),
        checkpoint
    );
}

#[test]
fn changed_commitment_checkpoint_or_path_fails_verification() {
    let mut history = MerkleHistory::new();
    for byte in 1..=5 {
        history.append(commitment(byte)).expect("append");
    }
    let checkpoint = history.checkpoint();
    let proof = history.inclusion_proof(2, 5).expect("proof");

    assert!(!verify_receipt_inclusion(commitment(9), &proof, checkpoint));

    let mut other = MerkleHistory::new();
    for byte in [1, 2, 9, 4, 5] {
        other.append(commitment(byte)).expect("append");
    }
    assert_ne!(checkpoint, other.checkpoint());
    assert!(!verify_receipt_inclusion(
        commitment(3),
        &proof,
        other.checkpoint()
    ));
}

#[test]
fn opening_and_fixed_checkpoint_vector_are_stable() {
    let opened = commitment(1);
    assert!(opened.matches(nonce(0x41), receipt(1)));
    assert!(!opened.matches(nonce(0x42), receipt(1)));

    let mut history = MerkleHistory::new();
    for byte in 1..=5 {
        history.append(commitment(byte)).expect("append");
    }
    assert_eq!(history.checkpoint().tree_size(), 5);
    assert_eq!(
        history.checkpoint().root().as_bytes(),
        &[
            100, 55, 60, 29, 188, 91, 67, 224, 97, 177, 208, 14, 106, 155, 195, 217, 185, 190, 21,
            128, 136, 22, 131, 1, 75, 99, 143, 122, 114, 198, 84, 17,
        ]
    );
}

#[test]
fn invalid_tree_sizes_and_indexes_are_closed_errors() {
    let mut history = MerkleHistory::new();
    history.append(commitment(1)).expect("append");

    assert_eq!(
        history.inclusion_proof(1, 1),
        Err(HistoryError::LeafOutsideTree {
            leaf_index: 1,
            tree_size: 1,
        })
    );
    assert_eq!(
        history.checkpoint_at(2),
        Err(HistoryError::TreeSizeUnavailable {
            requested: 2,
            available: 1,
        })
    );
}

#[test]
fn every_committed_prefix_has_a_logarithmic_consistency_proof() {
    let mut history = MerkleHistory::new();
    for byte in 1..=16 {
        history.append(commitment(byte)).expect("append");
    }

    for old_size in 1..=16 {
        for new_size in old_size..=16 {
            let old = history.checkpoint_at(old_size).expect("old checkpoint");
            let new = history.checkpoint_at(new_size).expect("new checkpoint");
            let proof = history
                .consistency_proof(old_size, new_size)
                .expect("consistency proof");
            assert!(verify_history_consistency(old, new, &proof));
            assert!(proof.sibling_count() <= 64);
        }
    }
}

#[test]
fn rewritten_or_split_history_cannot_reuse_a_consistency_proof() {
    let mut history = MerkleHistory::new();
    for byte in 1..=8 {
        history.append(commitment(byte)).expect("append");
    }
    let old = history.checkpoint_at(4).expect("old checkpoint");
    let new = history.checkpoint_at(8).expect("new checkpoint");
    let proof = history.consistency_proof(4, 8).expect("proof");

    let mut split_view = MerkleHistory::new();
    for byte in [1, 2, 3, 4, 9, 6, 7, 8] {
        split_view.append(commitment(byte)).expect("append");
    }

    assert!(!verify_history_consistency(
        old,
        split_view.checkpoint(),
        &proof
    ));
    assert_ne!(new, split_view.checkpoint());
}

#[test]
fn invalid_consistency_ranges_are_closed_errors() {
    let mut history = MerkleHistory::new();
    history.append(commitment(1)).expect("append");

    assert_eq!(
        history.consistency_proof(1, 0),
        Err(HistoryError::InvalidConsistencyRange {
            old_size: 1,
            new_size: 0,
        })
    );
    assert_eq!(
        history.consistency_proof(1, 2),
        Err(HistoryError::TreeSizeUnavailable {
            requested: 2,
            available: 1,
        })
    );
}
