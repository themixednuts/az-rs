use std::{fmt, num::NonZeroU32};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Visitor};
use uuid::Uuid;

/// Maximum canonical value accepted at a storage or wire boundary.
pub const MAX_CANONICAL_VALUE_BYTES: usize = 1_048_576;

/// Domain-separated content identity.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ContentDigest([u8; 32]);

impl ContentDigest {
    /// Constructs a digest from already verified bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Hashes one bounded value in an explicit semantic domain.
    #[must_use]
    pub fn hash(domain: &str, bytes: &[u8]) -> Self {
        let mut hasher = blake3::Hasher::new_derive_key(domain);
        hasher.update(bytes);
        Self(*hasher.finalize().as_bytes())
    }

    /// Returns the digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for ContentDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ContentDigest")
            .field(&self.0)
            .finish()
    }
}

/// Canonical bytes whose size was checked before allocation ownership escaped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalBytes(Box<[u8]>);

impl CanonicalBytes {
    /// Takes ownership of a bounded canonical value.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::Oversized`] when `bytes` exceeds the global cap.
    pub fn try_from_boxed(bytes: Box<[u8]>) -> Result<Self, CodecError> {
        if bytes.len() > MAX_CANONICAL_VALUE_BYTES {
            return Err(CodecError::Oversized {
                actual: bytes.len(),
                maximum: MAX_CANONICAL_VALUE_BYTES,
            });
        }
        Ok(Self(bytes))
    }

    /// Borrows the canonical bytes.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    /// Returns the encoded length.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns whether the value is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Transfers ownership to the caller.
    #[must_use]
    pub fn into_boxed(self) -> Box<[u8]> {
        self.0
    }
}

impl Serialize for CanonicalBytes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(&self.0)
    }
}

struct CanonicalBytesVisitor;

impl<'de> Visitor<'de> for CanonicalBytesVisitor {
    type Value = CanonicalBytes;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "at most {MAX_CANONICAL_VALUE_BYTES} canonical bytes"
        )
    }

    fn visit_bytes<E>(self, value: &[u8]) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        CanonicalBytes::try_from_boxed(value.into()).map_err(E::custom)
    }

    fn visit_byte_buf<E>(self, value: Vec<u8>) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        CanonicalBytes::try_from_boxed(value.into_boxed_slice()).map_err(E::custom)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        let capacity = sequence
            .size_hint()
            .unwrap_or_default()
            .min(MAX_CANONICAL_VALUE_BYTES);
        let mut bytes = Vec::with_capacity(capacity);
        while let Some(byte) = sequence.next_element::<u8>()? {
            if bytes.len() == MAX_CANONICAL_VALUE_BYTES {
                return Err(serde::de::Error::custom("canonical value exceeds bound"));
            }
            bytes.push(byte);
        }
        CanonicalBytes::try_from_boxed(bytes.into_boxed_slice()).map_err(serde::de::Error::custom)
    }
}

impl<'de> Deserialize<'de> for CanonicalBytes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_bytes(CanonicalBytesVisitor)
    }
}

/// Stable identity of one canonical codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CodecId(Uuid);

impl CodecId {
    /// Creates a non-nil codec identity.
    ///
    /// # Panics
    ///
    /// Panics when `value` is zero because nil is not a valid codec identity.
    #[must_use]
    pub const fn from_u128(value: u128) -> Self {
        assert!(value != 0, "codec identity cannot be nil");
        Self(Uuid::from_u128(value))
    }

    /// Returns the UUID representation.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

/// Nonzero schema version for canonical bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CodecVersion(NonZeroU32);

impl CodecVersion {
    /// Version one.
    pub const ONE: Self = Self(NonZeroU32::MIN);

    /// Returns the numeric version.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

impl TryFrom<u32> for CodecVersion {
    type Error = CodecError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        NonZeroU32::new(value)
            .map(Self)
            .ok_or(CodecError::Validation)
    }
}

/// Canonical codec boundary failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CodecError {
    /// Encoded bytes exceeded a declared bound.
    #[error("canonical value has {actual} bytes; maximum is {maximum}")]
    Oversized {
        /// Observed byte length.
        actual: usize,
        /// Accepted byte length.
        maximum: usize,
    },
    /// Bytes do not encode the declared value.
    #[error("canonical value is malformed")]
    Malformed,
    /// Stored bytes belong to a different codec.
    #[error("canonical value uses codec {actual:?}; expected {expected:?}")]
    WrongCodec {
        /// Expected codec identity.
        expected: CodecId,
        /// Stored codec identity.
        actual: CodecId,
    },
    /// Stored schema cannot be decoded by this implementation.
    #[error("codec version {stored:?} is unsupported; current is {current:?}")]
    UnsupportedVersion {
        /// Stored version.
        stored: CodecVersion,
        /// Current version.
        current: CodecVersion,
    },
    /// Decoded content violates the codec's domain invariant.
    #[error("canonical value failed validation")]
    Validation,
}

/// A deterministic bounded codec owned by a domain type.
pub trait DurableCodec: Sized + Send + Sync + 'static {
    /// Stable codec identity.
    const CODEC_ID: CodecId;
    /// Current schema version.
    const CURRENT_VERSION: CodecVersion;

    /// Encodes this value into its bounded canonical representation.
    ///
    /// # Errors
    ///
    /// Returns an error when the value cannot be represented canonically or exceeds the bound.
    fn encode_canonical(&self) -> Result<CanonicalBytes, CodecError>;

    /// Decodes an exact supported codec version without heuristic migration.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported versions, invalid bytes, or non-canonical encodings.
    fn decode_exact(version: CodecVersion, bytes: &[u8]) -> Result<Self, CodecError>;
}
