use az_federation::{
    ActivePolicyRef, AuthorityAtomId, AuthorityEpoch, CapabilityGrantRef,
    CertifiedExecutionBinding, ContentDigest, FederationId, FederationPrincipalId, GrantRevision,
    InstanceHostId, OperationId, OperatorId, PolicyRevision, RevocationCursor, RulesetBinding,
    RulesetNamespace, RuntimeReleaseBinding,
};
use az_world_instance::{PlacementFence, PlacementGeneration, WorldInstanceId};
use uuid::Uuid;

use crate::{CanonicalEnvelope, CanonicalEnvelopeError};

const MAGIC: [u8; 4] = *b"AZF1";
const SCHEMA_V1: u16 = 1;
const PORTABLE_MUTATION_KIND: u16 = 1;
const FIELD_COUNT: u16 = 16;
const HEADER_BYTES: usize = 10;
const FIELD_HEADER_BYTES: usize = 6;
const MAX_PARSED_FIELDS: u16 = FIELD_COUNT + 1;

/// Construction failure for a bounded portable mutation command.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PortableMutationCommandError {
    /// The game-owned canonical body exceeds its symbolic protocol limit.
    #[error("portable mutation body has {actual} bytes, maximum is {maximum}")]
    BodyTooLarge { actual: usize, maximum: usize },
}

/// Validated portable mutation input before game policy evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableMutationCommand {
    atom: AuthorityAtomId,
    operation: OperationId,
    principal: FederationPrincipalId,
    binding: CertifiedExecutionBinding,
    expected_state_version: u64,
    body: Box<[u8]>,
}

impl PortableMutationCommand {
    /// Symbolic first-version body bound. Ticket 013 must replace or confirm it
    /// with measured deployment evidence before production activation.
    pub const MAX_BODY_BYTES: usize = 64 * 1024;

    /// Constructs a command after checking the attacker-controlled body bound.
    ///
    /// # Errors
    ///
    /// Rejects bodies larger than [`Self::MAX_BODY_BYTES`].
    pub fn new(
        atom: AuthorityAtomId,
        operation: OperationId,
        principal: FederationPrincipalId,
        binding: CertifiedExecutionBinding,
        expected_state_version: u64,
        body: &[u8],
    ) -> Result<Self, PortableMutationCommandError> {
        if body.len() > Self::MAX_BODY_BYTES {
            return Err(PortableMutationCommandError::BodyTooLarge {
                actual: body.len(),
                maximum: Self::MAX_BODY_BYTES,
            });
        }
        Ok(Self {
            atom,
            operation,
            principal,
            binding,
            expected_state_version,
            body: body.into(),
        })
    }

    /// Returns the game-defined authority atom.
    #[must_use]
    pub const fn atom(&self) -> AuthorityAtomId {
        self.atom
    }
    /// Returns the stable idempotency identity.
    #[must_use]
    pub const fn operation(&self) -> OperationId {
        self.operation
    }
    /// Returns the authenticated principal.
    #[must_use]
    pub const fn principal(&self) -> FederationPrincipalId {
        self.principal
    }
    /// Returns the complete certified execution binding.
    #[must_use]
    pub const fn binding(&self) -> &CertifiedExecutionBinding {
        &self.binding
    }
    /// Returns the atom version observed before game policy evaluation.
    #[must_use]
    pub const fn expected_state_version(&self) -> u64 {
        self.expected_state_version
    }
    /// Returns the bounded canonical game body.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

/// Encodes a validated command with strictly ordered, unique TLV fields.
#[must_use]
pub fn encode_portable_mutation(command: &PortableMutationCommand) -> CanonicalEnvelope {
    let mut bytes = Vec::with_capacity(HEADER_BYTES + command.body.len() + 512);
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(&SCHEMA_V1.to_be_bytes());
    bytes.extend_from_slice(&PORTABLE_MUTATION_KIND.to_be_bytes());
    bytes.extend_from_slice(&FIELD_COUNT.to_be_bytes());

    push_field(&mut bytes, 1, command.atom.digest().as_bytes());
    push_field(&mut bytes, 2, command.operation.as_uuid().as_bytes());
    push_field(&mut bytes, 3, command.principal.digest().as_bytes());
    push_field(
        &mut bytes,
        4,
        command.binding.federation().digest().as_bytes(),
    );
    push_field(
        &mut bytes,
        5,
        &command.binding.authority_epoch().get().to_be_bytes(),
    );
    push_field(
        &mut bytes,
        6,
        command.binding.operator().digest().as_bytes(),
    );
    push_field(&mut bytes, 7, command.binding.host().digest().as_bytes());
    push_field(
        &mut bytes,
        8,
        command
            .binding
            .placement()
            .world_instance()
            .as_uuid()
            .as_bytes(),
    );
    push_field(
        &mut bytes,
        9,
        &command.binding.placement().generation().get().to_be_bytes(),
    );

    let mut grant = [0_u8; 40];
    grant[..32].copy_from_slice(command.binding.grant().id().as_bytes());
    grant[32..].copy_from_slice(&command.binding.grant().revision().get().to_be_bytes());
    push_field(&mut bytes, 10, &grant);
    push_field(
        &mut bytes,
        11,
        command.binding.runtime_release().digest().as_bytes(),
    );

    let namespace = command.binding.ruleset().namespace().as_str().as_bytes();
    let namespace_length = bounded_namespace_length(namespace);
    let mut ruleset = Vec::with_capacity(2 + namespace.len() + 32);
    ruleset.extend_from_slice(&namespace_length.to_be_bytes());
    ruleset.extend_from_slice(namespace);
    ruleset.extend_from_slice(command.binding.ruleset().digest().as_bytes());
    push_field(&mut bytes, 12, &ruleset);

    let mut policy = [0_u8; 40];
    policy[..32].copy_from_slice(command.binding.active_policy().id().as_bytes());
    policy[32..].copy_from_slice(
        &command
            .binding
            .active_policy()
            .revision()
            .get()
            .to_be_bytes(),
    );
    push_field(&mut bytes, 13, &policy);
    push_field(
        &mut bytes,
        14,
        &command.binding.revocation_view().get().to_be_bytes(),
    );
    push_field(
        &mut bytes,
        15,
        &command.expected_state_version.to_be_bytes(),
    );
    push_field(&mut bytes, 16, &command.body);
    CanonicalEnvelope::from_canonical_bytes(bytes)
}

/// Parses and validates a portable mutation before application code sees it.
///
/// # Errors
///
/// Rejects malformed headers, unknown or noncanonical fields, invalid domain
/// values, truncated input, trailing bytes, and oversized bodies.
pub fn decode_portable_mutation(
    bytes: &[u8],
) -> Result<PortableMutationCommand, CanonicalEnvelopeError> {
    let fields = parse_field_table(bytes)?;
    decode_fields(&fields)
}

fn parse_field_table(
    bytes: &[u8],
) -> Result<[Option<&[u8]>; FIELD_COUNT as usize + 1], CanonicalEnvelopeError> {
    if bytes.len() < HEADER_BYTES {
        return Err(CanonicalEnvelopeError::Truncated);
    }
    if bytes[..4] != MAGIC {
        return Err(CanonicalEnvelopeError::InvalidMagic);
    }
    let schema = read_u16(&bytes[4..6]);
    if schema != SCHEMA_V1 {
        return Err(CanonicalEnvelopeError::UnsupportedSchema { found: schema });
    }
    let kind = read_u16(&bytes[6..8]);
    if kind != PORTABLE_MUTATION_KIND {
        return Err(CanonicalEnvelopeError::UnsupportedMessageKind { found: kind });
    }
    let count = read_u16(&bytes[8..10]);
    if count > MAX_PARSED_FIELDS {
        return Err(CanonicalEnvelopeError::TooManyFields { found: count });
    }

    let mut fields: [Option<&[u8]>; FIELD_COUNT as usize + 1] = [None; FIELD_COUNT as usize + 1];
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
        if tag == 16 && length > PortableMutationCommand::MAX_BODY_BYTES {
            return Err(CanonicalEnvelopeError::BodyTooLarge {
                actual: length,
                maximum: PortableMutationCommand::MAX_BODY_BYTES,
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

fn decode_fields(
    fields: &[Option<&[u8]>; FIELD_COUNT as usize + 1],
) -> Result<PortableMutationCommand, CanonicalEnvelopeError> {
    let atom = AuthorityAtomId::from_digest(read_digest(required(fields, 1)?, 1)?);
    let operation = OperationId::try_from_uuid(read_uuid(required(fields, 2)?, 2)?)
        .map_err(|_| CanonicalEnvelopeError::NilIdentity { tag: 2 })?;
    let principal = FederationPrincipalId::from_digest(read_digest(required(fields, 3)?, 3)?);
    let federation = FederationId::from_digest(read_digest(required(fields, 4)?, 4)?);
    let authority_epoch = AuthorityEpoch::try_from(read_fixed_u64(required(fields, 5)?, 5)?)
        .map_err(|_| CanonicalEnvelopeError::InvalidMonotonicValue { tag: 5 })?;
    let operator = OperatorId::from_digest(read_digest(required(fields, 6)?, 6)?);
    let host = InstanceHostId::from_digest(read_digest(required(fields, 7)?, 7)?);
    let world_instance = WorldInstanceId::try_from_uuid(read_uuid(required(fields, 8)?, 8)?)
        .map_err(|_| CanonicalEnvelopeError::NilIdentity { tag: 8 })?;
    let generation = PlacementGeneration::try_from(read_fixed_u64(required(fields, 9)?, 9)?)
        .map_err(|_| CanonicalEnvelopeError::InvalidMonotonicValue { tag: 9 })?;
    let grant = parse_grant(required(fields, 10)?)?;
    let release = RuntimeReleaseBinding::new(read_digest(required(fields, 11)?, 11)?);
    let ruleset = parse_ruleset(required(fields, 12)?)?;
    let policy = parse_policy(required(fields, 13)?)?;
    let revocation = RevocationCursor::new(read_fixed_u64(required(fields, 14)?, 14)?);
    let expected_state_version = read_fixed_u64(required(fields, 15)?, 15)?;
    let body = required(fields, 16)?;
    PortableMutationCommand::new(
        atom,
        operation,
        principal,
        CertifiedExecutionBinding::new(
            federation,
            authority_epoch,
            operator,
            host,
            PlacementFence::new(world_instance, generation),
            grant,
            release,
            ruleset,
            policy,
            revocation,
        ),
        expected_state_version,
        body,
    )
    .map_err(
        |PortableMutationCommandError::BodyTooLarge { actual, maximum }| {
            CanonicalEnvelopeError::BodyTooLarge { actual, maximum }
        },
    )
}

fn bounded_namespace_length(namespace: &[u8]) -> u16 {
    debug_assert!(namespace.len() <= az_federation::MAX_RULESET_NAMESPACE_BYTES);
    #[allow(clippy::cast_possible_truncation)]
    {
        namespace.len() as u16
    }
}

fn push_field(bytes: &mut Vec<u8>, tag: u16, value: &[u8]) {
    bytes.extend_from_slice(&tag.to_be_bytes());
    bytes.extend_from_slice(
        &u32::try_from(value.len())
            .expect("validated federation fields fit in u32")
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

fn read_digest(bytes: &[u8], tag: u16) -> Result<ContentDigest, CanonicalEnvelopeError> {
    let value = read_array::<32>(bytes, tag)?;
    Ok(ContentDigest::from_bytes(value))
}

fn read_uuid(bytes: &[u8], tag: u16) -> Result<Uuid, CanonicalEnvelopeError> {
    let value = read_array::<16>(bytes, tag)?;
    Ok(Uuid::from_bytes(value))
}

fn read_fixed_u64(bytes: &[u8], tag: u16) -> Result<u64, CanonicalEnvelopeError> {
    Ok(u64::from_be_bytes(read_array::<8>(bytes, tag)?))
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

fn parse_grant(bytes: &[u8]) -> Result<CapabilityGrantRef, CanonicalEnvelopeError> {
    if bytes.len() != 40 {
        return Err(CanonicalEnvelopeError::InvalidFieldLength {
            tag: 10,
            expected: 40,
            actual: bytes.len(),
        });
    }
    let id = read_digest(&bytes[..32], 10)?;
    let revision = GrantRevision::try_from(read_fixed_u64(&bytes[32..], 10)?)
        .map_err(|_| CanonicalEnvelopeError::InvalidMonotonicValue { tag: 10 })?;
    Ok(CapabilityGrantRef::new(id, revision))
}

fn parse_ruleset(bytes: &[u8]) -> Result<RulesetBinding, CanonicalEnvelopeError> {
    if bytes.len() < 34 {
        return Err(CanonicalEnvelopeError::Truncated);
    }
    let namespace_length = usize::from(read_u16(&bytes[..2]));
    let expected = 2_usize
        .checked_add(namespace_length)
        .and_then(|value| value.checked_add(32))
        .ok_or(CanonicalEnvelopeError::InvalidRulesetNamespace)?;
    if bytes.len() != expected {
        return Err(CanonicalEnvelopeError::InvalidFieldLength {
            tag: 12,
            expected,
            actual: bytes.len(),
        });
    }
    let namespace = std::str::from_utf8(&bytes[2..2 + namespace_length])
        .map_err(|_| CanonicalEnvelopeError::InvalidRulesetNamespace)
        .and_then(|value| {
            RulesetNamespace::parse(value)
                .map_err(|_| CanonicalEnvelopeError::InvalidRulesetNamespace)
        })?;
    let digest = read_digest(&bytes[2 + namespace_length..], 12)?;
    Ok(RulesetBinding::new(namespace, digest))
}

fn parse_policy(bytes: &[u8]) -> Result<ActivePolicyRef, CanonicalEnvelopeError> {
    if bytes.len() != 40 {
        return Err(CanonicalEnvelopeError::InvalidFieldLength {
            tag: 13,
            expected: 40,
            actual: bytes.len(),
        });
    }
    let id = read_digest(&bytes[..32], 13)?;
    let revision = PolicyRevision::try_from(read_fixed_u64(&bytes[32..], 13)?)
        .map_err(|_| CanonicalEnvelopeError::InvalidMonotonicValue { tag: 13 })?;
    Ok(ActivePolicyRef::new(id, revision))
}

fn read_u16(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[0], bytes[1]])
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}
