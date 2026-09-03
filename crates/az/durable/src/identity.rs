use std::{
    any::{TypeId, type_name},
    collections::HashMap,
    fmt,
    hash::{Hash, Hasher},
    marker::PhantomData,
    num::NonZeroU64,
};

use arrayvec::ArrayString;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use crate::ContentDigest;

use crate::{CanonicalBytes, DurableCodec};

/// Invalid nil durable identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("durable identity cannot be nil")]
pub struct InvalidDurableIdentity;

/// Stable domain namespace for one durable subject type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DurableNamespaceId(Uuid);

impl DurableNamespaceId {
    /// Constructs a compile-time namespace and rejects zero.
    #[must_use]
    /// Creates a non-nil durable namespace identity.
    ///
    /// # Panics
    ///
    /// Panics when `value` is zero because nil is not a valid namespace.
    pub const fn from_u128(value: u128) -> Self {
        assert!(value != 0, "durable namespace cannot be nil");
        Self(Uuid::from_u128(value))
    }

    /// Validates a namespace UUID.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidDurableIdentity`] for nil.
    pub const fn try_from_uuid(value: Uuid) -> Result<Self, InvalidDurableIdentity> {
        if value.is_nil() {
            Err(InvalidDurableIdentity)
        } else {
            Ok(Self(value))
        }
    }

    /// Returns the namespace UUID.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

/// Stable key within one durable namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DurableSubjectKey([u8; 32]);

impl DurableSubjectKey {
    /// Constructs a key from deterministic bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the key bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Caller-owned stable operation identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct OperationId(Uuid);

impl OperationId {
    /// Validates an operation UUID.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidDurableIdentity`] for nil.
    pub const fn try_from_uuid(value: Uuid) -> Result<Self, InvalidDurableIdentity> {
        if value.is_nil() {
            Err(InvalidDurableIdentity)
        } else {
            Ok(Self(value))
        }
    }

    /// Returns the operation UUID.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

/// Stable position and hash of one committed journal record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RecordId {
    sequence: NonZeroU64,
    hash: ContentDigest,
}

/// Stable identity of one effect causally emitted by a subject operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EffectId {
    operation: OperationId,
    index: u16,
}

impl EffectId {
    /// Constructs the adapter-assigned effect identity for one operation-local ordinal.
    #[must_use]
    pub const fn new(operation: OperationId, index: u16) -> Self {
        Self { operation, index }
    }

    /// Returns the causal operation.
    #[must_use]
    pub const fn operation(self) -> OperationId {
        self.operation
    }

    /// Returns the zero-based operation-local ordinal.
    #[must_use]
    pub const fn index(self) -> u16 {
        self.index
    }
}

impl RecordId {
    /// Reconstructs a verified record identity from adapter-owned facts.
    #[must_use]
    pub const fn from_store_parts(sequence: NonZeroU64, hash: ContentDigest) -> Self {
        Self { sequence, hash }
    }

    /// Returns the per-subject sequence.
    #[must_use]
    pub const fn sequence(self) -> NonZeroU64 {
        self.sequence
    }

    /// Returns the hash that commits the record and its chain position.
    #[must_use]
    pub const fn hash(self) -> ContentDigest {
        self.hash
    }
}

/// Domain contract for one durable identity and state codec.
pub trait DurableSubject: Send + Sync + 'static {
    /// Stable namespace owned by the domain.
    const NAMESPACE: DurableNamespaceId;
    /// Canonical state representation.
    type State: DurableCodec;
}

/// Typed stable subject identity.
pub struct DurableSubjectId<T: DurableSubject> {
    namespace: DurableNamespaceId,
    key: DurableSubjectKey,
    marker: PhantomData<fn() -> T>,
}

impl<T: DurableSubject> DurableSubjectId<T> {
    /// Derives a stable key from the namespace and a canonical domain key.
    #[must_use]
    pub fn derive(stable_key: &CanonicalBytes) -> Self {
        let mut hasher = blake3::Hasher::new_derive_key("azoth durable subject identity v1");
        hasher.update(T::NAMESPACE.as_uuid().as_bytes());
        hasher.update(&1_u32.to_le_bytes());
        hasher.update(stable_key.as_slice());
        Self {
            namespace: T::NAMESPACE,
            key: DurableSubjectKey::from_bytes(*hasher.finalize().as_bytes()),
            marker: PhantomData,
        }
    }

    /// Creates an identity for a genuinely new subject from caller entropy.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidDurableIdentity`] for nil entropy.
    pub fn derive_from_entropy(entropy: Uuid) -> Result<Self, InvalidDurableIdentity> {
        if entropy.is_nil() {
            return Err(InvalidDurableIdentity);
        }
        let bytes = CanonicalBytes::try_from_boxed(entropy.as_bytes().to_vec().into_boxed_slice())
            .map_err(|_| InvalidDurableIdentity)?;
        Ok(Self::derive(&bytes))
    }

    /// Returns the namespace.
    #[must_use]
    pub const fn namespace(self) -> DurableNamespaceId {
        self.namespace
    }

    /// Returns the namespace-local key.
    #[must_use]
    pub const fn key(self) -> DurableSubjectKey {
        self.key
    }

    /// Erases only the Rust type marker for storage or wire dispatch.
    #[must_use]
    pub const fn erase(self) -> ErasedDurableSubjectId {
        ErasedDurableSubjectId {
            namespace: self.namespace,
            key: self.key,
        }
    }

    pub(crate) const fn from_erased(id: ErasedDurableSubjectId) -> Self {
        Self {
            namespace: id.namespace,
            key: id.key,
            marker: PhantomData,
        }
    }
}

impl<T: DurableSubject> Copy for DurableSubjectId<T> {}

impl<T: DurableSubject> Clone for DurableSubjectId<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: DurableSubject> fmt::Debug for DurableSubjectId<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DurableSubjectId")
            .field("namespace", &self.namespace)
            .field("key", &self.key)
            .finish()
    }
}

impl<T: DurableSubject> PartialEq for DurableSubjectId<T> {
    fn eq(&self, other: &Self) -> bool {
        self.namespace == other.namespace && self.key == other.key
    }
}

impl<T: DurableSubject> Eq for DurableSubjectId<T> {}

impl<T: DurableSubject> PartialOrd for DurableSubjectId<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: DurableSubject> Ord for DurableSubjectId<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.namespace, self.key).cmp(&(other.namespace, other.key))
    }
}

impl<T: DurableSubject> Hash for DurableSubjectId<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.namespace.hash(state);
        self.key.hash(state);
    }
}

impl<T: DurableSubject> Serialize for DurableSubjectId<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.erase().serialize(serializer)
    }
}

impl<'de, T: DurableSubject> Deserialize<'de> for DurableSubjectId<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let erased = ErasedDurableSubjectId::deserialize(deserializer)?;
        if erased.namespace != T::NAMESPACE {
            return Err(serde::de::Error::custom(
                "durable subject namespace mismatch",
            ));
        }
        Ok(Self::from_erased(erased))
    }
}

/// Subject identity without a Rust domain marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ErasedDurableSubjectId {
    /// Domain namespace.
    pub namespace: DurableNamespaceId,
    /// Namespace-local stable key.
    pub key: DurableSubjectKey,
}

/// Namespace/type binding failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NamespaceRegistrationError {
    /// A namespace is already owned by a registered Rust type.
    #[error("durable namespace {namespace:?} is already bound")]
    NamespaceAlreadyBound {
        /// Conflicting namespace.
        namespace: DurableNamespaceId,
    },
    /// A Rust subject type is already registered.
    #[error("durable subject type {type_name} is already bound")]
    TypeAlreadyBound {
        /// Conflicting Rust type.
        type_name: &'static str,
    },
    /// Reification requested a type that was not registered.
    #[error("durable subject type {type_name} is not registered")]
    TypeNotRegistered {
        /// Missing Rust type.
        type_name: &'static str,
    },
    /// Erased identity belongs to another namespace.
    #[error("durable subject namespace {actual:?} does not match {expected:?}")]
    NamespaceMismatch {
        /// Expected namespace.
        expected: DurableNamespaceId,
        /// Stored namespace.
        actual: DurableNamespaceId,
    },
}

/// Checked two-way mapping between durable namespaces and Rust domain types.
#[derive(Default)]
pub struct DurableNamespaceRegistry {
    namespaces: HashMap<DurableNamespaceId, &'static str>,
    types: HashMap<TypeId, DurableNamespaceId>,
}

impl DurableNamespaceRegistry {
    /// Constructs an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one subject type exactly once.
    ///
    /// # Errors
    ///
    /// Rejects duplicate namespace or Rust-type ownership.
    pub fn register<T: DurableSubject>(&mut self) -> Result<(), NamespaceRegistrationError> {
        if self.namespaces.contains_key(&T::NAMESPACE) {
            return Err(NamespaceRegistrationError::NamespaceAlreadyBound {
                namespace: T::NAMESPACE,
            });
        }
        let rust_type = TypeId::of::<T>();
        if self.types.contains_key(&rust_type) {
            return Err(NamespaceRegistrationError::TypeAlreadyBound {
                type_name: type_name::<T>(),
            });
        }
        self.namespaces.insert(T::NAMESPACE, type_name::<T>());
        self.types.insert(rust_type, T::NAMESPACE);
        Ok(())
    }

    /// Reifies an erased ID only for its registered namespace owner.
    ///
    /// # Errors
    ///
    /// Rejects unregistered types and cross-namespace identities.
    pub fn reify<T: DurableSubject>(
        &self,
        id: ErasedDurableSubjectId,
    ) -> Result<DurableSubjectId<T>, NamespaceRegistrationError> {
        let Some(registered) = self.types.get(&TypeId::of::<T>()) else {
            return Err(NamespaceRegistrationError::TypeNotRegistered {
                type_name: type_name::<T>(),
            });
        };
        if *registered != id.namespace || T::NAMESPACE != id.namespace {
            return Err(NamespaceRegistrationError::NamespaceMismatch {
                expected: T::NAMESPACE,
                actual: id.namespace,
            });
        }
        Ok(DurableSubjectId::from_erased(id))
    }
}

/// Bounded diagnostic-only identity; never authority currency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedDiagnosticId(ArrayString<128>);

impl TryFrom<&str> for BoundedDiagnosticId {
    type Error = DiagnosticIdError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let mut bounded = ArrayString::new();
        bounded
            .try_push_str(value)
            .map_err(|_| DiagnosticIdError::TooLong {
                actual: value.len(),
                maximum: 128,
            })?;
        if bounded.is_empty() {
            return Err(DiagnosticIdError::Empty);
        }
        Ok(Self(bounded))
    }
}

/// Invalid diagnostic identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DiagnosticIdError {
    /// Empty diagnostics do not identify a holder.
    #[error("diagnostic identity cannot be empty")]
    Empty,
    /// Diagnostic text exceeded its fixed bound.
    #[error("diagnostic identity has {actual} bytes; maximum is {maximum}")]
    TooLong {
        /// Actual UTF-8 length.
        actual: usize,
        /// Maximum UTF-8 length.
        maximum: usize,
    },
}
