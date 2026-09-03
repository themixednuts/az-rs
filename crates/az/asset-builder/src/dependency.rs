//! Dependency descriptors emitted by builders.
//!
//! Mirrors the dependency concepts from `AssetBuilderSDK::SourceFileDependency`,
//! `JobDependency`, and `ProductDependency` (`AssetBuilderSDK.h:300-410`).
//! Source/job dependencies flow into the asset registry's dependency tables;
//! product dependencies travel with verified build products and are persisted as
//! product facts/catalog metadata instead of the old AP mirror tables.

use az_asset::AssetRef;
use az_core::AssetData;
use uuid::Uuid;

use crate::AssetId;

/// Mirrors `SourceFileDependency` (`AssetBuilderSDK.h:300`).
///
/// "If source file `Foo` references file `Bar`, AP needs to know so
/// changing `Bar` re-fingerprints `Foo`." Identified either by path
/// or by source UUID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceFileDependency {
    Path(String),
    Uuid(Uuid),
}

/// Mirrors `JobDependency` (`AssetBuilderSDK.h:354`).
///
/// "Job A on Source X must run after Job B on Source Y." Used when
/// build order matters across sources (e.g. material override jobs
/// run after the base material).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobDependency {
    /// Source the dependency targets.
    pub source: SourceFileDependency,
    /// Job key on that source we depend on.
    pub job_key: String,
    /// Platform the dependency applies to (`""` = all).
    pub platform: String,
    /// Type of dependency (mirrors `JobDependencyType`).
    pub kind: JobDependencyType,
}

/// Mirrors `JobDependencyType` (`AssetBuilderSDK.h:344`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobDependencyType {
    /// Order only. The other job must run first; we don't read its
    /// outputs.
    Order,
    /// Order + read. The other job's outputs influence ours; we
    /// re-fingerprint when they change.
    Fingerprint,
    /// Used in `CreateJobs` only: emit a job once we see the
    /// dependency target is processed.
    OrderOnly,
}

/// Mirrors `ProductDependency` (`AssetBuilderSDK.h:399`).
///
/// "This product references that other product at runtime." The asset processor
/// records it with the verified product facts/catalog entry for the producing
/// job attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductDependency {
    /// Target asset id (guid + `sub_id`).
    pub dependency_id: AssetId,
    /// Bitmap mirroring `ProductDependencyFlags`.
    pub flags: ProductDependencyFlags,
}

impl ProductDependency {
    /// Create an ordinary runtime product dependency edge.
    #[inline]
    #[must_use]
    pub const fn new(dependency_id: AssetId) -> Self {
        Self {
            dependency_id,
            flags: ProductDependencyFlags(0),
        }
    }

    /// Create a dependency edge from a typed product reference.
    ///
    /// Only the reference's canonical asset id is persisted. Its diagnostic
    /// path hint is deliberately not part of the build dependency graph.
    #[inline]
    #[must_use]
    pub const fn from_asset_ref<T: AssetData>(reference: &AssetRef<T>) -> Self {
        Self::new(reference.id())
    }

    #[inline]
    #[must_use]
    pub const fn with_flags(mut self, flags: ProductDependencyFlags) -> Self {
        self.flags = flags;
        self
    }
}

impl<T: AssetData> From<&AssetRef<T>> for ProductDependency {
    #[inline]
    fn from(reference: &AssetRef<T>) -> Self {
        Self::from_asset_ref(reference)
    }
}

/// Mirrors `ProductDependencyFlags` bit pattern
/// (`AssetBuilderSDK.h:380`). Stored as a `u32` to match what we
/// write into the database; named accessors below.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct ProductDependencyFlags(pub u32);

impl ProductDependencyFlags {
    /// `0x0001` — load on referenced platform only.
    pub const NO_LOAD: Self = Self(0x0001);

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[cfg(test)]
mod tests {
    use az_core::{AzRtti, AzTypeInfo};

    use super::*;

    struct ReferencedProduct;

    impl AzTypeInfo for ReferencedProduct {
        const NAME: &'static str = "Azoth::ReferencedProduct";
        const TYPE_ID: Uuid = Uuid::from_u128(0x1255_71c0_062e_46a9_bccc_f0bb_ed43_4ddc);
    }

    impl AzRtti for ReferencedProduct {}

    impl AssetData for ReferencedProduct {
        const STABLE_NAME: &'static str = "az.test.referenced-product";
    }

    #[test]
    fn product_dependency_from_asset_ref_uses_canonical_asset_id() {
        let asset_id = AssetId::new(Uuid::from_u128(7), 21);
        let reference =
            AssetRef::<ReferencedProduct>::new(asset_id, Some("wrong/diagnostic-only.product"));

        let dependency = ProductDependency::from_asset_ref(&reference);

        assert_eq!(dependency.dependency_id, asset_id);
        assert_eq!(dependency.flags, ProductDependencyFlags::default());
    }
}
