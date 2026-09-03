use az_federation::{
    AuthorityEpoch, AuthorityRecoveryStatement, ContentDigest, FederationId, HistoryCheckpointRef,
    RecoveryStatementError,
};

use crate::{CanonicalEnvelope, CanonicalEnvelopeError};

const MAGIC: [u8; 4] = *b"AZF1";
const SCHEMA_V2: u16 = 2;
const AUTHORITY_RECOVERY_KIND: u16 = 2;
const FIELD_COUNT: u16 = 9;
const HEADER_BYTES: usize = 10;
const FIELD_HEADER_BYTES: usize = 6;

/// Encodes a root/fork recovery statement with strictly ordered fixed fields.
#[must_use]
pub fn encode_authority_recovery(statement: &AuthorityRecoveryStatement) -> CanonicalEnvelope {
    let mut bytes = Vec::with_capacity(328);
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(&SCHEMA_V2.to_be_bytes());
    bytes.extend_from_slice(&AUTHORITY_RECOVERY_KIND.to_be_bytes());
    bytes.extend_from_slice(&FIELD_COUNT.to_be_bytes());
    push_field(&mut bytes, 1, statement.federation().digest().as_bytes());
    push_field(
        &mut bytes,
        2,
        &statement.current_epoch().get().to_be_bytes(),
    );
    push_field(
        &mut bytes,
        3,
        &statement.successor_epoch().get().to_be_bytes(),
    );
    push_checkpoint(&mut bytes, 4, statement.trusted_checkpoint());
    push_checkpoint(&mut bytes, 5, statement.divergent_checkpoint());
    push_checkpoint(&mut bytes, 6, statement.selected_checkpoint());
    push_field(&mut bytes, 7, statement.successor_key_policy().as_bytes());
    push_field(&mut bytes, 8, statement.recovery_policy().as_bytes());
    push_field(&mut bytes, 9, statement.recovery_manifest().as_bytes());
    CanonicalEnvelope::from_canonical_bytes(bytes)
}

/// Parses and validates an exact root/fork recovery statement.
///
/// # Errors
///
/// Rejects malformed, duplicate, reordered, unknown, incomplete, or
/// non-forward statements before governance or signing code sees them.
pub fn decode_authority_recovery(
    bytes: &[u8],
) -> Result<AuthorityRecoveryStatement, CanonicalEnvelopeError> {
    let fields = parse_fields(bytes)?;
    let current = read_epoch(required(&fields, 2)?, 2)?;
    let successor = read_epoch(required(&fields, 3)?, 3)?;
    AuthorityRecoveryStatement::new(
        FederationId::from_digest(read_digest(required(&fields, 1)?, 1)?),
        current,
        successor,
        read_checkpoint(required(&fields, 4)?, 4)?,
        read_checkpoint(required(&fields, 5)?, 5)?,
        read_checkpoint(required(&fields, 6)?, 6)?,
        read_digest(required(&fields, 7)?, 7)?,
        read_digest(required(&fields, 8)?, 8)?,
        read_digest(required(&fields, 9)?, 9)?,
    )
    .map_err(|failure| match failure {
        RecoveryStatementError::NonSuccessorEpoch { current, proposed } => {
            CanonicalEnvelopeError::NonSuccessorAuthorityEpoch { current, proposed }
        }
        RecoveryStatementError::NoForkDivergence => CanonicalEnvelopeError::MissingForkDivergence,
    })
}

fn parse_fields(
    bytes: &[u8],
) -> Result<[Option<&[u8]>; FIELD_COUNT as usize + 1], CanonicalEnvelopeError> {
    if bytes.len() < HEADER_BYTES {
        return Err(CanonicalEnvelopeError::Truncated);
    }
    if bytes[..4] != MAGIC {
        return Err(CanonicalEnvelopeError::InvalidMagic);
    }
    let schema = read_u16(&bytes[4..6]);
    if schema != SCHEMA_V2 {
        return Err(CanonicalEnvelopeError::UnsupportedSchema { found: schema });
    }
    let kind = read_u16(&bytes[6..8]);
    if kind != AUTHORITY_RECOVERY_KIND {
        return Err(CanonicalEnvelopeError::UnsupportedMessageKind { found: kind });
    }
    let count = read_u16(&bytes[8..10]);
    if count > FIELD_COUNT {
        return Err(CanonicalEnvelopeError::TooManyFields { found: count });
    }

    let mut fields = [None; FIELD_COUNT as usize + 1];
    let mut cursor = HEADER_BYTES;
    let mut previous = 0_u16;
    for _ in 0..count {
        let header_end = cursor
            .checked_add(FIELD_HEADER_BYTES)
            .ok_or(CanonicalEnvelopeError::Truncated)?;
        if header_end > bytes.len() {
            return Err(CanonicalEnvelopeError::Truncated);
        }
        let tag = read_u16(&bytes[cursor..cursor + 2]);
        let length = read_u32(&bytes[cursor + 2..header_end]) as usize;
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
        let end = header_end
            .checked_add(length)
            .ok_or(CanonicalEnvelopeError::Truncated)?;
        if end > bytes.len() {
            return Err(CanonicalEnvelopeError::Truncated);
        }
        fields[usize::from(tag)] = Some(&bytes[header_end..end]);
        previous = tag;
        cursor = end;
    }
    if cursor != bytes.len() {
        return Err(CanonicalEnvelopeError::TrailingBytes);
    }
    Ok(fields)
}

fn push_checkpoint(bytes: &mut Vec<u8>, tag: u16, checkpoint: HistoryCheckpointRef) {
    let mut value = [0_u8; 40];
    value[..8].copy_from_slice(&checkpoint.tree_size().to_be_bytes());
    value[8..].copy_from_slice(checkpoint.statement_digest().as_bytes());
    push_field(bytes, tag, &value);
}

fn push_field(bytes: &mut Vec<u8>, tag: u16, value: &[u8]) {
    bytes.extend_from_slice(&tag.to_be_bytes());
    bytes.extend_from_slice(
        &u32::try_from(value.len())
            .expect("fixed recovery fields fit in u32")
            .to_be_bytes(),
    );
    bytes.extend_from_slice(value);
}

fn required<'a>(
    fields: &'a [Option<&'a [u8]>],
    tag: u16,
) -> Result<&'a [u8], CanonicalEnvelopeError> {
    fields[usize::from(tag)].ok_or(CanonicalEnvelopeError::MissingField { tag })
}

fn read_checkpoint(bytes: &[u8], tag: u16) -> Result<HistoryCheckpointRef, CanonicalEnvelopeError> {
    if bytes.len() != 40 {
        return Err(CanonicalEnvelopeError::InvalidFieldLength {
            tag,
            expected: 40,
            actual: bytes.len(),
        });
    }
    Ok(HistoryCheckpointRef::new(
        u64::from_be_bytes(read_array::<8>(&bytes[..8], tag)?),
        ContentDigest::from_bytes(read_array::<32>(&bytes[8..], tag)?),
    ))
}

fn read_epoch(bytes: &[u8], tag: u16) -> Result<AuthorityEpoch, CanonicalEnvelopeError> {
    AuthorityEpoch::try_from(u64::from_be_bytes(read_array::<8>(bytes, tag)?))
        .map_err(|_| CanonicalEnvelopeError::InvalidMonotonicValue { tag })
}

fn read_digest(bytes: &[u8], tag: u16) -> Result<ContentDigest, CanonicalEnvelopeError> {
    Ok(ContentDigest::from_bytes(read_array::<32>(bytes, tag)?))
}

fn read_array<const N: usize>(bytes: &[u8], tag: u16) -> Result<[u8; N], CanonicalEnvelopeError> {
    bytes
        .try_into()
        .map_err(|_| CanonicalEnvelopeError::InvalidFieldLength {
            tag,
            expected: N,
            actual: bytes.len(),
        })
}

const fn read_u16(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[0], bytes[1]])
}

const fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}
