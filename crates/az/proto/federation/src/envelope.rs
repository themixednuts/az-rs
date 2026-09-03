use az_federation::CanonicalRequestDigest;

/// BLAKE3 derive-key context for canonical request identity.
pub const REQUEST_DIGEST_V1_CONTEXT: &str =
    "azoth certified federation canonical request digest v1";

/// Owned canonical request bytes and their domain-separated identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalEnvelope {
    bytes: Box<[u8]>,
    request_digest: CanonicalRequestDigest,
}

impl CanonicalEnvelope {
    pub(crate) fn from_canonical_bytes(bytes: Vec<u8>) -> Self {
        let digest = blake3::derive_key(REQUEST_DIGEST_V1_CONTEXT, &bytes);
        Self {
            bytes: bytes.into_boxed_slice(),
            request_digest: CanonicalRequestDigest::from_bytes(digest),
        }
    }

    /// Returns the exact canonical bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the domain-separated digest of the entire envelope.
    #[must_use]
    pub const fn request_digest(&self) -> CanonicalRequestDigest {
        self.request_digest
    }
}

/// Failure to validate a canonical federation envelope.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CanonicalEnvelopeError {
    /// The fixed federation magic is absent.
    #[error("invalid federation envelope magic")]
    InvalidMagic,
    /// The schema version is not supported.
    #[error("unsupported federation schema version {found}")]
    UnsupportedSchema { found: u16 },
    /// The message kind is not supported by this decoder.
    #[error("unsupported federation message kind {found}")]
    UnsupportedMessageKind { found: u16 },
    /// The declared field count exceeds the parser's bounded work limit.
    #[error("federation envelope declares too many fields: {found}")]
    TooManyFields { found: u16 },
    /// A field appears twice.
    #[error("duplicate federation field {tag}")]
    DuplicateField { tag: u16 },
    /// Fields are not in strictly increasing tag order.
    #[error("noncanonical federation field order: {previous} before {current}")]
    NonCanonicalFieldOrder { previous: u16, current: u16 },
    /// The message kind does not define this tag.
    #[error("unknown federation field {tag}")]
    UnknownField { tag: u16 },
    /// A required field is absent.
    #[error("missing federation field {tag}")]
    MissingField { tag: u16 },
    /// A fixed-width field has the wrong length.
    #[error("federation field {tag} has length {actual}, expected {expected}")]
    InvalidFieldLength {
        tag: u16,
        expected: usize,
        actual: usize,
    },
    /// A UUID identity is nil.
    #[error("federation field {tag} contains a nil identity")]
    NilIdentity { tag: u16 },
    /// The mutation body exceeds its protocol limit.
    #[error("portable mutation body has {actual} bytes, maximum is {maximum}")]
    BodyTooLarge { actual: usize, maximum: usize },
    /// A revision, sequence, epoch, or generation violates its domain range.
    #[error("federation field {tag} contains an invalid monotonic value")]
    InvalidMonotonicValue { tag: u16 },
    /// A recovery statement did not advance the visible authority epoch.
    #[error("recovery authority epoch {proposed} does not advance {current}")]
    NonSuccessorAuthorityEpoch { current: u64, proposed: u64 },
    /// The trusted and divergent recovery checkpoints were identical.
    #[error("recovery statement contains no divergent checkpoint")]
    MissingForkDivergence,
    /// The Ruleset namespace is malformed or noncanonical.
    #[error("federation Ruleset namespace is invalid")]
    InvalidRulesetNamespace,
    /// Input ended before the declared structure was complete.
    #[error("truncated federation envelope")]
    Truncated,
    /// Bytes remain after the declared fields.
    #[error("federation envelope contains trailing bytes")]
    TrailingBytes,
}
