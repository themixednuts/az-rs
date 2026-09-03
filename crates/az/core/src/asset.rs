//! Core AZ asset identity value types.

use std::error::Error;
use std::fmt;
use std::num::ParseIntError;
use std::str::FromStr;

use uuid::Uuid;

#[cfg(feature = "serde")]
use bevy_reflect::{ReflectDeserialize, ReflectSerialize};

use crate::{AzRtti, AzTypeInfo};

/// Native `AZ::Data::AssetData` RTTI base.
///
/// Concrete asset payloads use this marker only to preserve their native RTTI
/// hierarchy. Runtime asset behavior is expressed by the [`AssetData`] trait.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, bevy_reflect::Reflect, az_derive::AzRtti)]
#[az_rtti(
    name = "AssetData",
    crate::uuid::type_ids::AZ_DATA_ASSET_DATA,
    register
)]
pub struct AzAssetData;

/// `AZ::Data::AssetId`.
///
/// This is the canonical `(guid, sub_id)` product identifier used by `AzCore`,
/// asset catalogs, package manifests, and asset references.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, bevy_reflect::Reflect)]
#[reflect(opaque)]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AssetId {
    pub guid: Uuid,
    pub sub_id: u32,
}

impl AssetId {
    #[inline]
    #[must_use]
    pub const fn new(guid: Uuid, sub_id: u32) -> Self {
        Self { guid, sub_id }
    }

    #[inline]
    #[must_use]
    pub const fn nil() -> Self {
        Self {
            guid: Uuid::from_u128(0),
            sub_id: 0,
        }
    }

    #[inline]
    #[must_use]
    pub const fn null() -> Self {
        Self::nil()
    }

    /// Whether this id is exactly the default value.
    ///
    /// This is value equality with [`Self::nil`], not the native reference
    /// test. Use [`Self::is_valid`] to decide whether an id names an asset.
    #[inline]
    #[must_use]
    pub const fn is_nil(self) -> bool {
        self.sub_id == 0 && self.guid.is_nil()
    }

    #[inline]
    #[must_use]
    pub const fn is_null(self) -> bool {
        self.is_nil()
    }

    /// Native `AZ::Data::AssetId::IsValid`: an id names an asset only when its
    /// GUID is set.
    ///
    /// Shipping data carries references whose GUID is null while `sub_id` keeps
    /// whatever product sub-ID the field was last assigned.
    /// `SliceMetaDataSpawnerEntry::sliceAssetId` is the canonical case: the
    /// spawner resolves its target through `sliceName`, and the id is left
    /// unset with `subId == 2`. Such an id names nothing, yet [`Self::is_nil`]
    /// reports `false` for it. Anything that extracts references from reflected
    /// data — product dependencies above all — must key on this predicate.
    #[inline]
    #[must_use]
    pub const fn is_valid(self) -> bool {
        !self.guid.is_nil()
    }
}

impl AzTypeInfo for AssetId {
    const NAME: &'static str = "AZ::Data::AssetId";
    const TYPE_ID: Uuid = crate::uuid::type_ids::AZ_DATA_ASSET_ID;
}

impl AzRtti for AssetId {}

impl AsRef<Self> for AssetId {
    #[inline]
    fn as_ref(&self) -> &Self {
        self
    }
}

impl fmt::Debug for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssetId")
            .field("guid", &format_args!("{}", self.guid))
            .field("sub_id", &format_args!("{:#x}", self.sub_id))
            .finish()
    }
}

impl From<Uuid> for AssetId {
    #[inline]
    fn from(guid: Uuid) -> Self {
        Self { guid, sub_id: 0 }
    }
}

impl From<(Uuid, u32)> for AssetId {
    #[inline]
    fn from((guid, sub_id): (Uuid, u32)) -> Self {
        Self { guid, sub_id }
    }
}

impl TryFrom<&[u8]> for AssetId {
    type Error = AssetIdBytesError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        if value.len() < 20 {
            return Err(AssetIdBytesError {
                actual_len: value.len(),
            });
        }

        let mut guid = [0u8; 16];
        guid.copy_from_slice(&value[..16]);
        let mut sub_id = [0u8; 4];
        sub_id.copy_from_slice(&value[16..20]);

        Ok(Self {
            guid: Uuid::from_bytes(guid),
            sub_id: u32::from_be_bytes(sub_id),
        })
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{:x}",
            self.guid.as_braced().to_string().to_uppercase(),
            self.sub_id
        )
    }
}

impl FromStr for AssetId {
    type Err = AssetIdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (guid_part, sub_id_part) = value
            .rsplit_once(':')
            .ok_or(AssetIdParseError::MissingSeparator)?;
        let guid = Uuid::parse_str(guid_part.trim_start_matches('{').trim_end_matches('}'))?;
        let sub_id = u32::from_str_radix(sub_id_part, 16)?;
        Ok(Self { guid, sub_id })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetIdBytesError {
    pub actual_len: usize,
}

impl fmt::Display for AssetIdBytesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "asset id bytes must be at least 20 bytes, got {}",
            self.actual_len
        )
    }
}

impl Error for AssetIdBytesError {}

#[derive(Debug)]
pub enum AssetIdParseError {
    MissingSeparator,
    BadGuid(uuid::Error),
    BadSubId(ParseIntError),
}

impl fmt::Display for AssetIdParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSeparator => f.write_str("missing ':' separator between guid and sub_id"),
            Self::BadGuid(error) => write!(f, "invalid guid: {error}"),
            Self::BadSubId(error) => write!(f, "invalid sub_id: {error}"),
        }
    }
}

impl Error for AssetIdParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::MissingSeparator => None,
            Self::BadGuid(error) => Some(error),
            Self::BadSubId(error) => Some(error),
        }
    }
}

impl From<uuid::Error> for AssetIdParseError {
    #[inline]
    fn from(error: uuid::Error) -> Self {
        Self::BadGuid(error)
    }
}

impl From<ParseIntError> for AssetIdParseError {
    #[inline]
    fn from(error: ParseIntError) -> Self {
        Self::BadSubId(error)
    }
}

/// The UUID of an asset class.
///
/// This is the canonical runtime product type id, equivalent to
/// `AZ::Data::AssetType`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(transparent)]
pub struct AssetType(pub Uuid);

impl AzTypeInfo for AssetType {
    const NAME: &'static str = "AZ::Uuid";
    const TYPE_ID: Uuid = crate::uuid::type_ids::AZ_UUID;
}

impl AzRtti for AssetType {}

impl AssetType {
    #[inline]
    #[must_use]
    pub const fn new(uuid: Uuid) -> Self {
        Self(uuid)
    }

    #[inline]
    #[must_use]
    pub const fn nil() -> Self {
        Self(Uuid::nil())
    }

    #[inline]
    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }

    #[inline]
    #[must_use]
    pub const fn into_inner(self) -> Uuid {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn into_uuid(self) -> Uuid {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn is_nil(self) -> bool {
        self.0.is_nil()
    }

    #[inline]
    #[must_use]
    pub const fn is_null(self) -> bool {
        self.is_nil()
    }
}

impl fmt::Display for AssetType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.as_braced().to_string().to_uppercase())
    }
}

impl FromStr for AssetType {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value.trim_start_matches('{').trim_end_matches('}')).map(Self)
    }
}

impl From<Uuid> for AssetType {
    #[inline]
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl From<AssetType> for Uuid {
    #[inline]
    fn from(value: AssetType) -> Self {
        value.0
    }
}

impl AsRef<Uuid> for AssetType {
    #[inline]
    fn as_ref(&self) -> &Uuid {
        &self.0
    }
}

/// Runtime asset payload marker.
///
/// Concrete product payload types implement this to expose their canonical
/// [`AssetType`].
pub trait AssetData: AzTypeInfo + AzRtti + Send + Sync + 'static {
    /// Stable asset type name used in manifests, registries, and editor UI.
    ///
    /// Use a reverse-DNS-style name such as `azoth.material.material` instead
    /// of a Rust module path or display label.
    const STABLE_NAME: &'static str;

    /// Runtime asset class UUID.
    const ASSET_TYPE: AssetType = AssetType::new(Self::TYPE_ID);

    #[inline]
    #[must_use]
    fn asset_type() -> AssetType {
        Self::ASSET_TYPE
    }
}

/// Metadata for one asset type contributed by an engine, project, or gem crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetTypeRegistration {
    stable_name: &'static str,
    asset_type: AssetType,
    owner: &'static str,
}

impl AssetTypeRegistration {
    /// Register a concrete asset type at a runtime/wire boundary.
    ///
    /// Normal asset type registration should use [`Self::for_asset`] so the
    /// stable name and UUID come from the `AssetData` implementation.
    #[inline]
    #[must_use]
    pub const fn from_dynamic_parts(stable_name: &'static str, asset_type: AssetType) -> Self {
        Self {
            stable_name,
            asset_type,
            owner: "",
        }
    }

    /// Register an [`AssetData`] implementor using its own stable name and UUID.
    #[inline]
    #[must_use]
    pub const fn for_asset<T: AssetData>() -> Self {
        Self::from_dynamic_parts(T::STABLE_NAME, T::ASSET_TYPE)
    }

    /// Record the crate, header, or schema that owns this asset type id.
    #[inline]
    #[must_use]
    pub const fn with_owner(mut self, owner: &'static str) -> Self {
        self.owner = owner;
        self
    }

    #[inline]
    #[must_use]
    pub const fn stable_name(&self) -> &'static str {
        self.stable_name
    }

    #[inline]
    #[must_use]
    pub const fn asset_type(&self) -> AssetType {
        self.asset_type
    }

    #[inline]
    #[must_use]
    pub const fn owner(&self) -> &'static str {
        self.owner
    }
}

/// One registration per asset type id. The id is the identity products
/// carry, and the stable name is the catalog's name for it; two contributions
/// claiming one id disagree about what those bytes are, which the old
/// first-match lookup settled by link order.
impl az_gem_contract::RegistryEntry for AssetTypeRegistration {
    type Key = AssetType;
    type Requires = az_gem_contract::Unconditional;

    fn registry_name() -> &'static str {
        "asset-type"
    }

    fn key(&self) -> Self::Key {
        self.asset_type
    }
}

/// Composed asset types in deterministic order.
#[must_use]
pub fn composed_asset_types(
    registries: &az_gem_contract::Registries,
) -> Vec<AssetTypeRegistration> {
    let mut registrations = registries
        .get::<AssetTypeRegistration>()
        .map(|registry| registry.entries().copied().collect::<Vec<_>>())
        .unwrap_or_default();
    registrations.sort_by(|left, right| {
        left.stable_name
            .cmp(right.stable_name)
            .then_with(|| left.asset_type.cmp(&right.asset_type))
    });
    registrations
}

/// The composed asset type with this UUID.
#[must_use]
pub fn composed_asset_type(
    registries: &az_gem_contract::Registries,
    asset_type: AssetType,
) -> Option<AssetTypeRegistration> {
    composed_asset_types(registries)
        .into_iter()
        .find(|registration| registration.asset_type == asset_type)
}

/// The composed asset type with this stable name.
#[must_use]
pub fn composed_asset_type_by_name(
    registries: &az_gem_contract::Registries,
    stable_name: &str,
) -> Option<AssetTypeRegistration> {
    composed_asset_types(registries)
        .into_iter()
        .find(|registration| registration.stable_name == stable_name)
}

/// The asset types `az-core` itself owns, for a host contribution to register.
#[must_use]
pub const fn asset_types() -> [AssetTypeRegistration; 6] {
    [
        AssetTypeRegistration::for_asset::<SliceAsset>().with_owner("AzCore/Slice/SliceAsset.h"),
        AssetTypeRegistration::for_asset::<DynamicSliceAsset>()
            .with_owner("AzCore/Slice/SliceAsset.h"),
        AssetTypeRegistration::for_asset::<ScriptAsset>().with_owner("AzCore/Script/ScriptAsset.h"),
        AssetTypeRegistration::for_asset::<ConfigAsset>().with_owner("az-core"),
        AssetTypeRegistration::for_asset::<AssetCatalogAsset>().with_owner("az-core"),
        AssetTypeRegistration::for_asset::<PakArchiveAsset>().with_owner("az-core"),
    ]
}

/// Register `az-core`'s own asset types into a composing host.
pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<AssetTypeRegistration>()
        .register_many(asset_types());
}

/// Core and SDK-generic `AssetType` UUIDs.
///
/// Project, game, gem, and compatibility asset classes should define their
/// ids in the owning crate and submit [`AssetTypeRegistration`] entries.
pub mod ids {
    use super::AssetType;
    use uuid::uuid;

    /// `AZ::SliceAsset` — O3DE reference:
    /// `Code/Framework/AzCore/AzCore/Slice/SliceAsset.h`.
    pub const SLICE_ASSET: AssetType =
        AssetType::new(uuid!("c62c7a87-9c09-4148-a985-12f2c99c0a45"));

    /// `AZ::DynamicSliceAsset` — O3DE reference:
    /// `Code/Framework/AzCore/AzCore/Slice/SliceAsset.h`.
    pub const DYNAMIC_SLICE_ASSET: AssetType =
        AssetType::new(uuid!("78802abf-9595-463a-8d2b-d022f906f9b1"));

    /// `AZ::ScriptAsset` (Lua) — O3DE reference:
    /// `Code/Framework/AzCore/AzCore/Script/ScriptAsset.h`.
    pub const SCRIPT_ASSET: AssetType =
        AssetType::new(uuid!("82557326-4ae3-416c-95d6-c70635ab7588"));

    /// SDK-generic config asset (settings/config-shaped compiled products
    /// owned by format crates such as `crates/formats/bink` and
    /// `cry-system-assets`).
    pub const CONFIG: AssetType = AssetType::new(uuid!("f0177614-18ef-49c3-7b43-be809979ad0f"));

    /// SDK-generic asset catalog product.
    pub const ASSET_CATALOG: AssetType =
        AssetType::new(uuid!("01288725-29f0-4ad4-8c54-cf91aa8abe10"));

    /// SDK-generic package archive product.
    pub const PAK_ARCHIVE: AssetType =
        AssetType::new(uuid!("12399836-3a01-4be5-9d65-d0a2bb9bcf21"));
}

pub struct SliceAsset;

impl AzTypeInfo for SliceAsset {
    const NAME: &'static str = "AZ::SliceAsset";
    const TYPE_ID: Uuid = ids::SLICE_ASSET.0;
}

impl AzRtti for SliceAsset {}

impl AssetData for SliceAsset {
    const STABLE_NAME: &'static str = "az.core.slice";
}

pub struct DynamicSliceAsset;

impl AzTypeInfo for DynamicSliceAsset {
    const NAME: &'static str = "AZ::DynamicSliceAsset";
    const TYPE_ID: Uuid = ids::DYNAMIC_SLICE_ASSET.0;
}

impl AzRtti for DynamicSliceAsset {}

impl AssetData for DynamicSliceAsset {
    const STABLE_NAME: &'static str = "az.core.dynamic-slice";
}

pub struct ScriptAsset;

impl AzTypeInfo for ScriptAsset {
    const NAME: &'static str = "AZ::ScriptAsset";
    const TYPE_ID: Uuid = ids::SCRIPT_ASSET.0;
}

impl AzRtti for ScriptAsset {}

impl AssetData for ScriptAsset {
    const STABLE_NAME: &'static str = "az.core.script";
}

pub struct ConfigAsset;

impl AzTypeInfo for ConfigAsset {
    const NAME: &'static str = "Azoth::ConfigAsset";
    const TYPE_ID: Uuid = ids::CONFIG.0;
}

impl AzRtti for ConfigAsset {}

impl AssetData for ConfigAsset {
    const STABLE_NAME: &'static str = "azoth.sdk.config";
}

pub struct AssetCatalogAsset;

impl AzTypeInfo for AssetCatalogAsset {
    const NAME: &'static str = "Azoth::AssetCatalogAsset";
    const TYPE_ID: Uuid = ids::ASSET_CATALOG.0;
}

impl AzRtti for AssetCatalogAsset {}

impl AssetData for AssetCatalogAsset {
    const STABLE_NAME: &'static str = "azoth.sdk.asset-catalog";
}

pub struct PakArchiveAsset;

impl AzTypeInfo for PakArchiveAsset {
    const NAME: &'static str = "Azoth::PakArchiveAsset";
    const TYPE_ID: Uuid = ids::PAK_ARCHIVE.0;
}

impl AzRtti for PakArchiveAsset {}

impl AssetData for PakArchiveAsset {
    const STABLE_NAME: &'static str = "azoth.sdk.pak-archive";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn const_constructors() {
        const NIL: AssetId = AssetId::nil();
        const FROM_PARTS: AssetId = AssetId::new(Uuid::from_u128(0xABCD), 7);

        assert_eq!(NIL, AssetId::default());
        assert_eq!(FROM_PARTS.sub_id, 7);
        assert!(AssetId::null().is_null());
    }

    #[test]
    fn display_round_trips_az_canonical_form() {
        let id = AssetId::new(
            Uuid::parse_str("c9000220-b6d4-506d-b1a0-08bf4d6845dc").unwrap(),
            0x20000,
        );

        let text = id.to_string();
        assert_eq!(text, "{C9000220-B6D4-506D-B1A0-08BF4D6845DC}:20000");
        assert_eq!(text.parse::<AssetId>().unwrap(), id);
    }

    #[test]
    fn ordering_uses_guid_then_subid() {
        let a = AssetId::new(Uuid::from_u128(1), 0);
        let b = AssetId::new(Uuid::from_u128(1), 1);
        let c = AssetId::new(Uuid::from_u128(2), 0);

        assert!(a < b);
        assert!(b < c);
    }

    #[test]
    fn from_uuid_uses_zero_subid() {
        let uuid = Uuid::from_u128(0xdead);
        let id = AssetId::from(uuid);

        assert_eq!(id.guid, uuid);
        assert_eq!(id.sub_id, 0);
    }

    #[test]
    fn bytes_parse_guid_and_subid() {
        let mut bytes = [0u8; 20];
        bytes[..16].copy_from_slice(Uuid::from_u128(0xABCD).as_bytes());
        bytes[16..20].copy_from_slice(&7u32.to_be_bytes());

        assert_eq!(
            AssetId::try_from(bytes.as_slice()).unwrap(),
            AssetId::new(Uuid::from_u128(0xABCD), 7)
        );
        assert_eq!(
            AssetId::try_from(&bytes[..19]).unwrap_err(),
            AssetIdBytesError { actual_len: 19 }
        );
    }

    #[test]
    fn asset_id_matches_az_type_info() {
        assert_eq!(
            <AssetId as AzTypeInfo>::TYPE_ID,
            crate::uuid::type_ids::AZ_DATA_ASSET_ID
        );
        assert_eq!(<AssetId as AzTypeInfo>::NAME, "AZ::Data::AssetId");
    }

    #[cfg(feature = "serde")]
    #[test]
    fn asset_id_registers_reflect_serde_type_data() {
        let mut registry = bevy_reflect::TypeRegistry::new();
        registry.register::<AssetId>();
        let registration = registry
            .get(std::any::TypeId::of::<AssetId>())
            .expect("AssetId registration");

        assert!(
            registration
                .data::<bevy_reflect::ReflectSerialize>()
                .is_some()
        );
        assert!(
            registration
                .data::<bevy_reflect::ReflectDeserialize>()
                .is_some()
        );
    }

    #[test]
    fn lumberyard_asset_type_uuids_match_header_declarations() {
        assert_eq!(
            ids::SLICE_ASSET.0.to_string(),
            "c62c7a87-9c09-4148-a985-12f2c99c0a45",
        );
        assert_eq!(
            ids::SCRIPT_ASSET.0.to_string(),
            "82557326-4ae3-416c-95d6-c70635ab7588",
        );
        assert_eq!(
            ids::CONFIG.0.to_string(),
            "f0177614-18ef-49c3-7b43-be809979ad0f",
        );
    }

    /// Native `AZ::Data::AssetId::IsValid` keys on the GUID alone. Shipping
    /// `SliceMetaData` spawners carry `{null, 2}` - an id that names nothing
    /// yet is not the default value.
    #[test]
    fn an_unset_guid_names_no_asset_even_when_the_sub_id_survives() {
        let unset = AssetId::new(Uuid::nil(), 2);

        assert!(!unset.is_valid());
        assert!(!unset.is_nil());
        assert!(!AssetId::nil().is_valid());
        assert!(AssetId::nil().is_nil());
        assert!(AssetId::new(Uuid::from_u128(1), 0).is_valid());
    }

    #[test]
    fn asset_type_uses_az_canonical_text_shape() {
        let asset_type: AssetType = "c5d443e1-41ff-4263-8654-9438bc888cb7".parse().unwrap();

        assert_eq!(
            asset_type.to_string(),
            "{C5D443E1-41FF-4263-8654-9438BC888CB7}"
        );
        assert_eq!(
            asset_type,
            "{C5D443E1-41FF-4263-8654-9438BC888CB7}".parse().unwrap()
        );
        assert_eq!(AssetType::default(), AssetType::nil());
        assert!(AssetType::nil().is_nil());
        assert!(AssetType::nil().is_null());
        assert_eq!(asset_type.as_uuid(), &asset_type.0);
        assert_eq!(asset_type.into_uuid(), asset_type.0);
        assert_eq!(
            <AssetType as AzTypeInfo>::TYPE_ID,
            crate::uuid::type_ids::AZ_UUID
        );
        assert_eq!(<AssetType as AzTypeInfo>::NAME, "AZ::Uuid");
    }

    #[test]
    fn asset_data_registration_uses_asset_data_identity() {
        struct TestAsset;

        impl AzTypeInfo for TestAsset {
            const NAME: &'static str = "AzCoreTests::TestAsset";
            const TYPE_ID: Uuid = uuid::uuid!("11111111-2222-3333-4444-555555555555");
        }

        impl AzRtti for TestAsset {}

        impl AssetData for TestAsset {
            const STABLE_NAME: &'static str = "az.core.tests.test-asset";
        }

        assert_eq!(
            TestAsset::asset_type(),
            AssetType::new(uuid::uuid!("11111111-2222-3333-4444-555555555555"))
        );

        let registration =
            AssetTypeRegistration::for_asset::<TestAsset>().with_owner("az-core-test");
        assert_eq!(registration.stable_name(), "az.core.tests.test-asset");
        assert_eq!(registration.asset_type(), TestAsset::asset_type());
        assert_eq!(registration.owner(), "az-core-test");
    }

    az_gem_contract::declare_caps!(EngineCaps:);

    struct Engine;

    impl az_gem_contract::Contribution for Engine {
        type Caps = EngineCaps;

        fn descriptor(&self) -> az_gem_contract::ContributionDescriptor {
            az_gem_contract::ContributionDescriptor {
                gem: az_gem_contract::GemId::new("azoth.core"),
                contribution: az_gem_contract::ContributionId::new("assets"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut az_gem_contract::GemContext<'_, EngineCaps>) {
            register(ctx);
        }
    }

    fn composed() -> az_gem_contract::Composer {
        let mut composer =
            az_gem_contract::Composer::new(az_gem_contract::GemTargetRole::AssetWorker);
        composer
            .add(Engine, az_gem_contract::ProductActivation::default())
            .expect("an empty floor composes");
        composer
    }

    #[test]
    fn composed_asset_types_are_sorted_and_searchable() {
        let composer = composed();
        let registries = composer.registries();
        let registrations = composed_asset_types(registries);
        let names = registrations
            .iter()
            .map(AssetTypeRegistration::stable_name)
            .collect::<Vec<_>>();
        let mut sorted_names = names.clone();
        sorted_names.sort_unstable();
        assert_eq!(names, sorted_names);

        let slice = composed_asset_type(registries, ids::SLICE_ASSET)
            .expect("slice asset type is composed");
        assert_eq!(slice.stable_name(), "az.core.slice");
        assert_eq!(slice.asset_type(), ids::SLICE_ASSET);
        assert_eq!(slice.owner(), "AzCore/Slice/SliceAsset.h");
        assert_eq!(
            composed_asset_type_by_name(registries, "az.core.slice"),
            Some(slice)
        );

        // A host sees exactly the asset types its own composition registered,
        // not every type linked into the process.
        let mut expected = asset_types()
            .iter()
            .map(AssetTypeRegistration::stable_name)
            .collect::<Vec<_>>();
        expected.sort_unstable();
        assert_eq!(names, expected);
    }

    #[test]
    fn every_composed_asset_type_is_attributed_to_its_contribution() {
        let report = composed().finalize().expect("composition is valid");
        let entries = report
            .entries
            .iter()
            .filter(|entry| entry.registry == "asset-type")
            .collect::<Vec<_>>();

        assert_eq!(entries.len(), asset_types().len());
        assert!(entries.iter().all(|entry| {
            entry.instance.gem.as_str() == "azoth.core"
                && entry.instance.contribution.as_str() == "assets"
        }));
    }

    #[test]
    fn two_contributions_claiming_one_asset_type_fail_composition() {
        struct Rival;

        impl az_gem_contract::Contribution for Rival {
            type Caps = EngineCaps;

            fn descriptor(&self) -> az_gem_contract::ContributionDescriptor {
                az_gem_contract::ContributionDescriptor {
                    gem: az_gem_contract::GemId::new("azoth.core-rival"),
                    contribution: az_gem_contract::ContributionId::new("assets"),
                    roles: &[],
                }
            }

            fn register(&self, ctx: &mut az_gem_contract::GemContext<'_, EngineCaps>) {
                ctx.registrar::<AssetTypeRegistration>().register(
                    AssetTypeRegistration::for_asset::<SliceAsset>().with_owner("azoth.core-rival"),
                );
            }
        }

        let mut composer = composed();
        composer
            .add(Rival, az_gem_contract::ProductActivation::default())
            .unwrap();

        let az_gem_contract::ComposeError::Duplicate {
            registry,
            key,
            first,
            second,
        } = composer.finalize().unwrap_err()
        else {
            panic!("a repeated asset type must fail composition");
        };
        assert_eq!(registry, "asset-type");
        assert_eq!(key, ids::SLICE_ASSET.to_string());
        assert_eq!(first.gem.as_str(), "azoth.core");
        assert_eq!(second.gem.as_str(), "azoth.core-rival");
    }
}
