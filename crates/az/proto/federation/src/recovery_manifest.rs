use az_federation::authority::{
    AuthorityRecoveryManifest, AuthorityRecoveryTarget, RecoveryManifestError,
};
use az_federation::{
    AuthorityAtomId, AuthorityEpoch, ContentDigest, FederationId, HistoryCheckpointRef, OperationId,
};
use uuid::Uuid;

use crate::{CanonicalEnvelope, CanonicalEnvelopeError};

const MAGIC: [u8; 4] = *b"AZF1";
const SCHEMA_V1: u16 = 1;
const KIND: u16 = 4;
const FIELD_COUNT: u16 = 8;
const HEADER_BYTES: usize = 10;
const FIELD_HEADER_BYTES: usize = 6;
const TARGET_BYTES: usize = 72;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RecoveryManifestDecodeError {
    #[error(transparent)]
    Envelope(#[from] CanonicalEnvelopeError),
    #[error(transparent)]
    Manifest(#[from] RecoveryManifestError),
}

#[must_use]
/// Encodes one validated recovery manifest into its canonical envelope.
///
/// # Panics
///
/// Panics only if an in-memory target slice exceeds `u32::MAX` entries, which
/// cannot be allocated within the supported process address space.
pub fn encode_recovery_manifest(manifest: &AuthorityRecoveryManifest) -> CanonicalEnvelope {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(&SCHEMA_V1.to_be_bytes());
    bytes.extend_from_slice(&KIND.to_be_bytes());
    bytes.extend_from_slice(&FIELD_COUNT.to_be_bytes());
    push(&mut bytes, 1, manifest.federation().digest().as_bytes());
    push(&mut bytes, 2, &manifest.current_epoch().get().to_be_bytes());
    push(
        &mut bytes,
        3,
        &manifest.successor_epoch().get().to_be_bytes(),
    );
    let checkpoint = manifest.selected_checkpoint();
    let mut checkpoint_bytes = [0; 40];
    checkpoint_bytes[..8].copy_from_slice(&checkpoint.tree_size().to_be_bytes());
    checkpoint_bytes[8..].copy_from_slice(checkpoint.statement_digest().as_bytes());
    push(&mut bytes, 4, &checkpoint_bytes);
    push(&mut bytes, 5, manifest.operation().as_uuid().as_bytes());
    push(&mut bytes, 6, manifest.impact_inventory().as_bytes());
    push(&mut bytes, 7, manifest.history_amendment().as_bytes());
    let mut targets = Vec::with_capacity(4 + manifest.targets().len() * TARGET_BYTES);
    targets.extend_from_slice(
        &u32::try_from(manifest.targets().len())
            .expect("manifest target count fits u32")
            .to_be_bytes(),
    );
    for target in manifest.targets() {
        targets.extend_from_slice(target.atom().digest().as_bytes());
        targets.extend_from_slice(&target.through_sequence().to_be_bytes());
        targets.extend_from_slice(target.state_digest().as_bytes());
    }
    push(&mut bytes, 8, &targets);
    CanonicalEnvelope::from_canonical_bytes(bytes)
}

#[must_use]
pub fn recovery_manifest_digest(manifest: &AuthorityRecoveryManifest) -> ContentDigest {
    ContentDigest::from_bytes(
        *encode_recovery_manifest(manifest)
            .request_digest()
            .as_bytes(),
    )
}

/// Decodes and validates one canonical recovery manifest.
///
/// # Errors
///
/// Rejects malformed envelopes, invalid fixed-width fields, nil operation
/// identities, non-successor epochs, and empty, duplicate, or reordered targets.
pub fn decode_recovery_manifest(
    bytes: &[u8],
) -> Result<AuthorityRecoveryManifest, RecoveryManifestDecodeError> {
    let fields = parse(bytes)?;
    let federation = FederationId::from_digest(digest(required(&fields, 1)?, 1)?);
    let current = epoch(required(&fields, 2)?, 2)?;
    let successor = epoch(required(&fields, 3)?, 3)?;
    let checkpoint_bytes = required(&fields, 4)?;
    if checkpoint_bytes.len() != 40 {
        return Err(CanonicalEnvelopeError::InvalidFieldLength {
            tag: 4,
            expected: 40,
            actual: checkpoint_bytes.len(),
        }
        .into());
    }
    let checkpoint = HistoryCheckpointRef::new(
        u64::from_be_bytes(array(&checkpoint_bytes[..8], 4)?),
        ContentDigest::from_bytes(array(&checkpoint_bytes[8..], 4)?),
    );
    let operation_bytes = required(&fields, 5)?;
    let operation = OperationId::try_from_uuid(Uuid::from_bytes(array(operation_bytes, 5)?))
        .map_err(|_| CanonicalEnvelopeError::NilIdentity { tag: 5 })?;
    let target_bytes = required(&fields, 8)?;
    if target_bytes.len() < 4 {
        return Err(CanonicalEnvelopeError::Truncated.into());
    }
    let count = u32::from_be_bytes(array(&target_bytes[..4], 8)?) as usize;
    let expected = 4usize
        .checked_add(
            count
                .checked_mul(TARGET_BYTES)
                .ok_or(CanonicalEnvelopeError::Truncated)?,
        )
        .ok_or(CanonicalEnvelopeError::Truncated)?;
    if target_bytes.len() != expected {
        return Err(CanonicalEnvelopeError::InvalidFieldLength {
            tag: 8,
            expected,
            actual: target_bytes.len(),
        }
        .into());
    }
    let mut targets = Vec::with_capacity(count);
    for chunk in target_bytes[4..].chunks_exact(TARGET_BYTES) {
        targets.push(AuthorityRecoveryTarget::new(
            AuthorityAtomId::from_digest(ContentDigest::from_bytes(array(&chunk[..32], 8)?)),
            u64::from_be_bytes(array(&chunk[32..40], 8)?),
            ContentDigest::from_bytes(array(&chunk[40..], 8)?),
        ));
    }
    Ok(AuthorityRecoveryManifest::new(
        federation,
        current,
        successor,
        checkpoint,
        operation,
        digest(required(&fields, 6)?, 6)?,
        digest(required(&fields, 7)?, 7)?,
        targets.into_boxed_slice(),
    )?)
}

fn parse(bytes: &[u8]) -> Result<[Option<&[u8]>; 9], CanonicalEnvelopeError> {
    if bytes.len() < HEADER_BYTES {
        return Err(CanonicalEnvelopeError::Truncated);
    }
    if bytes[..4] != MAGIC {
        return Err(CanonicalEnvelopeError::InvalidMagic);
    }
    let schema = u16::from_be_bytes(array(&bytes[4..6], 0)?);
    if schema != SCHEMA_V1 {
        return Err(CanonicalEnvelopeError::UnsupportedSchema { found: schema });
    }
    let kind = u16::from_be_bytes(array(&bytes[6..8], 0)?);
    if kind != KIND {
        return Err(CanonicalEnvelopeError::UnsupportedMessageKind { found: kind });
    }
    let count = u16::from_be_bytes(array(&bytes[8..10], 0)?);
    if count > FIELD_COUNT {
        return Err(CanonicalEnvelopeError::TooManyFields { found: count });
    }
    let mut fields = [None; 9];
    let mut cursor = HEADER_BYTES;
    let mut previous = 0;
    for _ in 0..count {
        if cursor + FIELD_HEADER_BYTES > bytes.len() {
            return Err(CanonicalEnvelopeError::Truncated);
        }
        let tag = u16::from_be_bytes(array(&bytes[cursor..cursor + 2], 0)?);
        let length = u32::from_be_bytes(array(&bytes[cursor + 2..cursor + 6], 0)?) as usize;
        if tag == 0 || tag > FIELD_COUNT {
            return Err(CanonicalEnvelopeError::UnknownField { tag });
        }
        if tag == previous {
            return Err(CanonicalEnvelopeError::DuplicateField { tag });
        }
        if tag < previous {
            return Err(CanonicalEnvelopeError::NonCanonicalFieldOrder {
                previous,
                current: tag,
            });
        }
        let end = cursor + FIELD_HEADER_BYTES + length;
        if end > bytes.len() {
            return Err(CanonicalEnvelopeError::Truncated);
        }
        fields[tag as usize] = Some(&bytes[cursor + FIELD_HEADER_BYTES..end]);
        previous = tag;
        cursor = end;
    }
    if cursor != bytes.len() {
        return Err(CanonicalEnvelopeError::TrailingBytes);
    }
    Ok(fields)
}

fn push(bytes: &mut Vec<u8>, tag: u16, value: &[u8]) {
    bytes.extend_from_slice(&tag.to_be_bytes());
    bytes.extend_from_slice(
        &u32::try_from(value.len())
            .expect("field length fits u32")
            .to_be_bytes(),
    );
    bytes.extend_from_slice(value);
}
fn required<'a>(
    fields: &'a [Option<&'a [u8]>],
    tag: u16,
) -> Result<&'a [u8], CanonicalEnvelopeError> {
    fields[tag as usize].ok_or(CanonicalEnvelopeError::MissingField { tag })
}
fn digest(bytes: &[u8], tag: u16) -> Result<ContentDigest, CanonicalEnvelopeError> {
    Ok(ContentDigest::from_bytes(array(bytes, tag)?))
}
fn epoch(bytes: &[u8], tag: u16) -> Result<AuthorityEpoch, CanonicalEnvelopeError> {
    AuthorityEpoch::try_from(u64::from_be_bytes(array(bytes, tag)?))
        .map_err(|_| CanonicalEnvelopeError::InvalidMonotonicValue { tag })
}
fn array<const N: usize>(bytes: &[u8], tag: u16) -> Result<[u8; N], CanonicalEnvelopeError> {
    bytes
        .try_into()
        .map_err(|_| CanonicalEnvelopeError::InvalidFieldLength {
            tag,
            expected: N,
            actual: bytes.len(),
        })
}
