use az_federation::{
    ActivePolicyRef, AuthorityAtomId, AuthorityEpoch, CapabilityGrantRef,
    CertifiedExecutionBinding, ContentDigest, FederationId, FederationPrincipalId, GrantRevision,
    InstanceHostId, OperationId, OperatorId, PolicyRevision, RevocationCursor, RulesetBinding,
    RulesetNamespace, RuntimeReleaseBinding,
};
use az_proto_federation::{
    CanonicalEnvelopeError, PortableMutationCommand, decode_portable_mutation,
    encode_portable_mutation,
};
use az_world_instance::{PlacementFence, PlacementGeneration, WorldInstanceId};
use uuid::Uuid;

const fn digest(byte: u8) -> ContentDigest {
    ContentDigest::from_bytes([byte; 32])
}

fn command() -> PortableMutationCommand {
    PortableMutationCommand::new(
        AuthorityAtomId::from_digest(digest(0x01)),
        OperationId::try_from_uuid(Uuid::from_bytes([0x02; 16])).expect("operation"),
        FederationPrincipalId::from_digest(digest(0x03)),
        CertifiedExecutionBinding::new(
            FederationId::from_digest(digest(0x04)),
            AuthorityEpoch::try_from(1).expect("epoch"),
            OperatorId::from_digest(digest(0x05)),
            InstanceHostId::from_digest(digest(0x06)),
            PlacementFence::new(
                WorldInstanceId::try_from_uuid(Uuid::from_bytes([0x07; 16]))
                    .expect("world instance"),
                PlacementGeneration::try_from(2).expect("generation"),
            ),
            CapabilityGrantRef::new(
                digest(0x08),
                GrantRevision::try_from(3).expect("grant revision"),
            ),
            RuntimeReleaseBinding::new(digest(0x09)),
            RulesetBinding::new(
                RulesetNamespace::parse("example/open-world").expect("namespace"),
                digest(0x0a),
            ),
            ActivePolicyRef::new(
                digest(0x0b),
                PolicyRevision::try_from(4).expect("policy revision"),
            ),
            RevocationCursor::new(5),
        ),
        6,
        b"award-receipted-item".as_slice(),
    )
    .expect("bounded command")
}

fn encoded_fields(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut cursor = 10;
    let mut fields = Vec::new();
    while cursor < bytes.len() {
        let length = u32::from_be_bytes(
            bytes[cursor + 2..cursor + 6]
                .try_into()
                .expect("field length bytes"),
        ) as usize;
        let end = cursor + 6 + length;
        fields.push(bytes[cursor..end].to_vec());
        cursor = end;
    }
    fields
}

fn envelope_with_fields(canonical: &[u8], fields: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = canonical[..8].to_vec();
    bytes.extend_from_slice(
        &u16::try_from(fields.len())
            .expect("test field count")
            .to_be_bytes(),
    );
    for field in fields {
        bytes.extend_from_slice(field);
    }
    bytes
}

fn replace_field_payload(fields: &mut [Vec<u8>], tag: u16, payload: &[u8]) {
    let field = fields
        .iter_mut()
        .find(|field| field[..2] == tag.to_be_bytes())
        .expect("test field");
    field.truncate(2);
    field.extend_from_slice(
        &u32::try_from(payload.len())
            .expect("test payload length")
            .to_be_bytes(),
    );
    field.extend_from_slice(payload);
}

#[test]
fn canonical_bytes_are_stable_across_decode_and_encode() {
    let encoded = encode_portable_mutation(&command());
    let decoded = decode_portable_mutation(encoded.as_bytes()).expect("decode canonical command");
    let reencoded = encode_portable_mutation(&decoded);

    assert_eq!(encoded, reencoded);
    assert_eq!(decoded, command());
    assert_eq!(
        encoded.request_digest().as_bytes(),
        &[
            237, 108, 115, 232, 142, 101, 170, 215, 31, 58, 203, 78, 219, 90, 222, 227, 131, 94,
            79, 142, 141, 161, 194, 125, 167, 211, 26, 151, 218, 121, 191, 54,
        ]
    );
}

#[test]
fn duplicate_reordered_unknown_and_oversized_fields_fail_before_domain_use() {
    let canonical = encode_portable_mutation(&command());
    let mut duplicate_fields = encoded_fields(canonical.as_bytes());
    duplicate_fields.insert(3, duplicate_fields[2].clone());
    let duplicate = envelope_with_fields(canonical.as_bytes(), &duplicate_fields);
    let mut reordered_fields = encoded_fields(canonical.as_bytes());
    reordered_fields.swap(2, 3);
    let reordered = envelope_with_fields(canonical.as_bytes(), &reordered_fields);
    let mut unknown_fields = encoded_fields(canonical.as_bytes());
    unknown_fields.push(
        [
            99_u16.to_be_bytes().as_slice(),
            0_u32.to_be_bytes().as_slice(),
        ]
        .concat(),
    );
    let unknown = envelope_with_fields(canonical.as_bytes(), &unknown_fields);

    assert_eq!(
        decode_portable_mutation(&duplicate),
        Err(CanonicalEnvelopeError::DuplicateField { tag: 3 })
    );
    assert_eq!(
        decode_portable_mutation(&reordered),
        Err(CanonicalEnvelopeError::NonCanonicalFieldOrder {
            previous: 4,
            current: 3,
        })
    );
    assert_eq!(
        decode_portable_mutation(&unknown),
        Err(CanonicalEnvelopeError::UnknownField { tag: 99 })
    );

    let oversized = vec![0_u8; PortableMutationCommand::MAX_BODY_BYTES + 1];
    assert!(
        PortableMutationCommand::new(
            command().atom(),
            command().operation(),
            command().principal(),
            command().binding().clone(),
            command().expected_state_version(),
            &oversized,
        )
        .is_err()
    );
}

#[test]
fn trailing_and_truncated_bytes_are_rejected() {
    let canonical = encode_portable_mutation(&command());
    let mut trailing = canonical.as_bytes().to_vec();
    trailing.push(0xff);
    let truncated = &canonical.as_bytes()[..canonical.as_bytes().len() - 1];

    assert_eq!(
        decode_portable_mutation(&trailing),
        Err(CanonicalEnvelopeError::TrailingBytes)
    );
    assert_eq!(
        decode_portable_mutation(truncated),
        Err(CanonicalEnvelopeError::Truncated)
    );
}

#[test]
fn schema_kind_and_domain_identity_invariants_are_checked_at_decode() {
    let canonical = encode_portable_mutation(&command());

    let mut unsupported_schema = canonical.as_bytes().to_vec();
    unsupported_schema[4..6].copy_from_slice(&0_u16.to_be_bytes());
    assert_eq!(
        decode_portable_mutation(&unsupported_schema),
        Err(CanonicalEnvelopeError::UnsupportedSchema { found: 0 })
    );

    let mut unsupported_kind = canonical.as_bytes().to_vec();
    unsupported_kind[6..8].copy_from_slice(&2_u16.to_be_bytes());
    assert_eq!(
        decode_portable_mutation(&unsupported_kind),
        Err(CanonicalEnvelopeError::UnsupportedMessageKind { found: 2 })
    );

    for tag in [2, 8] {
        let mut fields = encoded_fields(canonical.as_bytes());
        replace_field_payload(&mut fields, tag, &[0; 16]);
        let encoded = envelope_with_fields(canonical.as_bytes(), &fields);
        assert_eq!(
            decode_portable_mutation(&encoded),
            Err(CanonicalEnvelopeError::NilIdentity { tag })
        );
    }

    let mut fields = encoded_fields(canonical.as_bytes());
    let ruleset = fields
        .iter_mut()
        .find(|field| field[..2] == 12_u16.to_be_bytes())
        .expect("ruleset field");
    ruleset[8] = b'N';
    let encoded = envelope_with_fields(canonical.as_bytes(), &fields);
    assert_eq!(
        decode_portable_mutation(&encoded),
        Err(CanonicalEnvelopeError::InvalidRulesetNamespace)
    );
}

#[test]
fn every_truncation_and_bounded_arbitrary_input_fails_without_panicking() {
    let canonical = encode_portable_mutation(&command());
    for length in 0..canonical.as_bytes().len() {
        assert!(decode_portable_mutation(&canonical.as_bytes()[..length]).is_err());
    }

    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    for length in 0..1024 {
        let mut bytes = vec![0_u8; length];
        for byte in &mut bytes {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *byte = state.to_le_bytes()[0];
        }
        let _ = decode_portable_mutation(&bytes);
    }
}
