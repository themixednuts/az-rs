use az_federation_crypto::{
    PublicKeyId, RawSignature, RootRecovery, Signature, SignatureDomainVersion,
};

const MAGIC: [u8; 4] = *b"AZR1";
const SCHEMA_V1: u16 = 1;
const ROOT_RECOVERY_ROLE: u16 = 5;
const SIGNATURE_DOMAIN_V1: u16 = 1;
const ED25519_V1_SUITE: u16 = 1;
const HEADER_BYTES: usize = 12;
const PUBLIC_KEY_BYTES: usize = 32;
const SIGNATURE_BYTES: usize = 64;

/// Exact byte length of one root-recovery approval record.
pub const ROOT_RECOVERY_APPROVAL_BYTES: usize = HEADER_BYTES + PUBLIC_KEY_BYTES + SIGNATURE_BYTES;

/// Refusal while parsing one fixed-width root-recovery signature record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RecoveryApprovalError {
    /// The record is truncated or contains trailing bytes.
    #[error("root recovery approval has length {actual}, expected {expected}")]
    InvalidLength { actual: usize, expected: usize },
    /// The record is not a federation recovery approval.
    #[error("invalid root recovery approval magic")]
    InvalidMagic,
    /// The schema version is not supported.
    #[error("unsupported root recovery approval schema {found}")]
    UnsupportedSchema { found: u16 },
    /// An online or other signing role cannot substitute for offline roots.
    #[error("root recovery approval has invalid role {found}")]
    InvalidRole { found: u16 },
    /// The signature-domain construction is unknown.
    #[error("unsupported root recovery signature domain {found}")]
    UnsupportedDomain { found: u16 },
    /// The signature suite is unknown.
    #[error("unsupported root recovery signature suite {found}")]
    UnsupportedSuite { found: u16 },
}

/// Encodes one unverified `RootRecovery` signature in a fixed canonical record.
#[must_use]
pub fn encode_root_recovery_approval(
    approval: Signature<RootRecovery>,
) -> [u8; ROOT_RECOVERY_APPROVAL_BYTES] {
    let mut bytes = [0_u8; ROOT_RECOVERY_APPROVAL_BYTES];
    bytes[..4].copy_from_slice(&MAGIC);
    bytes[4..6].copy_from_slice(&SCHEMA_V1.to_be_bytes());
    bytes[6..8].copy_from_slice(&ROOT_RECOVERY_ROLE.to_be_bytes());
    bytes[8..10].copy_from_slice(&SIGNATURE_DOMAIN_V1.to_be_bytes());
    bytes[10..12].copy_from_slice(&ED25519_V1_SUITE.to_be_bytes());
    bytes[HEADER_BYTES..HEADER_BYTES + PUBLIC_KEY_BYTES]
        .copy_from_slice(approval.public_key().as_bytes());
    let RawSignature::Ed25519V1(signature) = approval.raw();
    bytes[HEADER_BYTES + PUBLIC_KEY_BYTES..].copy_from_slice(signature);
    bytes
}

/// Parses one unverified `RootRecovery` signature from its canonical record.
///
/// # Errors
///
/// Rejects any wrong length, magic, schema, role, domain, or suite before
/// cryptographic verification or federation policy sees the approval.
pub fn decode_root_recovery_approval(
    bytes: &[u8],
) -> Result<Signature<RootRecovery>, RecoveryApprovalError> {
    if bytes.len() != ROOT_RECOVERY_APPROVAL_BYTES {
        return Err(RecoveryApprovalError::InvalidLength {
            actual: bytes.len(),
            expected: ROOT_RECOVERY_APPROVAL_BYTES,
        });
    }
    if bytes[..4] != MAGIC {
        return Err(RecoveryApprovalError::InvalidMagic);
    }
    let schema = read_u16(&bytes[4..6]);
    if schema != SCHEMA_V1 {
        return Err(RecoveryApprovalError::UnsupportedSchema { found: schema });
    }
    let role = read_u16(&bytes[6..8]);
    if role != ROOT_RECOVERY_ROLE {
        return Err(RecoveryApprovalError::InvalidRole { found: role });
    }
    let domain = read_u16(&bytes[8..10]);
    if domain != SIGNATURE_DOMAIN_V1 {
        return Err(RecoveryApprovalError::UnsupportedDomain { found: domain });
    }
    let suite = read_u16(&bytes[10..12]);
    if suite != ED25519_V1_SUITE {
        return Err(RecoveryApprovalError::UnsupportedSuite { found: suite });
    }

    let public_key =
        read_array::<PUBLIC_KEY_BYTES>(&bytes[HEADER_BYTES..HEADER_BYTES + PUBLIC_KEY_BYTES]);
    let signature = read_array::<SIGNATURE_BYTES>(&bytes[HEADER_BYTES + PUBLIC_KEY_BYTES..]);
    Ok(Signature::from_parts(
        PublicKeyId::from_bytes(public_key),
        SignatureDomainVersion::V1,
        RawSignature::Ed25519V1(signature),
    ))
}

fn read_array<const N: usize>(bytes: &[u8]) -> [u8; N] {
    bytes
        .try_into()
        .expect("fixed recovery approval field length was checked")
}

const fn read_u16(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[0], bytes[1]])
}
