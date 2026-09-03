//! Faceted-component middleware shared by Azoth games.

use az_core::component::{
    AzComponent, Component as AzComponentBase, ComponentLoweringRegistration,
};
use az_core::rtti::AzTypeRegistration;
use az_derive::AzRtti;
use az_prefab::__private::bevy_ecs::{component::Component, reflect::ReflectComponent};
use az_prefab::{Prefab, PrefabType, ReflectPrefab};
use bevy_reflect::{
    FromReflect, PartialReflect, Reflect, ReflectDeserialize, ReflectRef, ReflectSerialize,
    std_traits::ReflectDefault,
};
use gridmate::{
    Marshaler,
    hub::{ActorId, ActorRequestId, FragmentKey, GdeRef, HubActorRef},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use az_core::Uid;

/// The comparison a bare `#[reflect(PartialEq)]` would generate, written out
/// once and named in the attribute as `#[reflect(PartialEq(reflect_partial_eq))]`.
///
/// The derive builds its default body with `quote_spanned!` pinned to the
/// `PartialEq` token, so the `if let/else` it expands is attributed to this
/// crate's source and trips `clippy::option_if_let_else` with a suggestion that
/// is not valid Rust. Supplying the function explicitly makes the generated body
/// a single call. Behaviour is unchanged: a failed downcast still compares
/// unequal rather than returning `None`.
fn reflect_partial_eq<T: PartialEq + Reflect>(this: &T, value: &dyn PartialReflect) -> bool {
    <dyn PartialReflect>::try_downcast_ref::<T>(value).is_some_and(|other| this == other)
}

/// Non-zero native replication index authored on an `MB::FacetedComponent`.
///
/// This is deliberately distinct from `GridMate`'s wire [`FragmentKey`]. A
/// component may retain a non-zero index without producing replicated state;
/// only a state-producing facet maps its index to a fragment key. Key zero is
/// the authored sentinel and is reserved for GDE metadata on the wire.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReplicationIndex(core::num::NonZeroU32);

impl ReplicationIndex {
    #[must_use]
    pub const fn new(value: u32) -> Option<Self> {
        match core::num::NonZeroU32::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }

    #[must_use]
    /// Maps a known state-producing facet's index to its wire key.
    ///
    /// A non-zero replication index alone does not prove that the component
    /// produces a fragment. Callers must establish that through the concrete
    /// component/state contract first.
    pub const fn fragment_key(self) -> FragmentKey {
        FragmentKey::new(self.get())
    }
}

impl From<ReplicationIndex> for u32 {
    fn from(value: ReplicationIndex) -> Self {
        value.get()
    }
}

impl From<ReplicationIndex> for FragmentKey {
    fn from(value: ReplicationIndex) -> Self {
        value.fragment_key()
    }
}

impl core::fmt::Display for ReplicationIndex {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.get().fmt(formatter)
    }
}

/// Native facet base shared by client and server facet references.
#[derive(
    AzRtti,
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Marshaler,
    Serialize,
    Deserialize,
    Reflect,
)]
#[az_rtti(name = "Facet", "9469C437-6529-489D-8CF8-63EEAB723A79", register)]
#[reflect(Serialize, Deserialize)]
pub struct Facet;

/// Native client-side facet base.
#[derive(
    AzRtti,
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Marshaler,
    Serialize,
    Deserialize,
    Reflect,
)]
#[az_rtti(
    name = "ClientFacet",
    "0643CDC7-B1C9-4721-92CE-7AC02E6175C9",
    Facet,
    register
)]
pub struct ClientFacet {
    #[serde(rename = "BaseClass1", default)]
    pub facet: Facet,
}

/// Native server-side facet base.
#[derive(
    AzRtti,
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Marshaler,
    Serialize,
    Deserialize,
    Reflect,
)]
#[az_rtti(
    name = "ServerFacet",
    "0392E589-5B61-47CC-835B-C3C254E76493",
    Facet,
    register
)]
pub struct ServerFacet {
    #[serde(rename = "BaseClass1", default)]
    pub facet: Facet,
}

/// Native base shared by all network-faceted components.
#[derive(
    Component,
    AzRtti,
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Marshaler,
    Serialize,
    Deserialize,
    Reflect,
    Prefab,
)]
#[az_rtti(
    name = "FacetedComponent",
    "65CD8F3E-73AA-43E9-8D9A-B5AE43F624F9",
    AzComponentBase,
    register
)]
#[reflect(Component, Default, Prefab)]
#[prefab(tag = "FacetedComponent", version = 1)]
pub struct FacetedComponent {
    #[serde(rename = "BaseClass1", default)]
    pub az_component: AzComponentBase,
    #[serde(rename = "m_clientFacetPtr", default)]
    pub client_facet_ptr: ClientFacet,
    #[serde(rename = "m_serverFacetPtr", default)]
    pub server_facet_ptr: ServerFacet,
    #[serde(rename = "m_replicationIndex", default)]
    pub replication_index: u32,
}

impl FacetedComponent {
    /// Raw authored non-zero replication index.
    ///
    /// `None` is the native zero sentinel. A non-zero value is retained even
    /// when this component has no replicated-state implementation.
    #[must_use]
    pub const fn replication_index(&self) -> Option<ReplicationIndex> {
        ReplicationIndex::new(self.replication_index)
    }
}

/// Finds the native faceted base inside a reflected prefab component.
///
/// Prefab processing already constructs the strongly typed reflected value.
/// Walking its native base-class structs here keeps replication metadata in
/// the product without requiring every generated component to publish an
/// erased callback or making the runtime inspect authoring fields.
#[must_use]
pub fn reflected_replication_index(value: &dyn PartialReflect) -> Option<ReplicationIndex> {
    if let Some(faceted) = value.try_downcast_ref::<FacetedComponent>() {
        return faceted.replication_index();
    }
    if value
        .get_represented_type_info()
        .is_some_and(|info| info.type_id() == std::any::TypeId::of::<FacetedComponent>())
    {
        return FacetedComponent::from_reflect(value)
            .and_then(|faceted| faceted.replication_index());
    }

    let ReflectRef::Struct(value) = value.reflect_ref() else {
        return None;
    };
    value
        .iter_fields()
        .find_map(|(_, field)| reflected_replication_index(field))
}

impl AzComponent for FacetedComponent {
    fn component_id(&self) -> az_core::component::ComponentId {
        self.az_component.id
    }
}

/// The AZ types this crate registers, for a host contribution to register.
#[must_use]
pub const fn types() -> [AzTypeRegistration; 7] {
    [
        ClientFacet::REGISTRATION,
        Facet::REGISTRATION,
        FacetedComponent::REGISTRATION,
        LocalComponentRefBase::REGISTRATION,
        RemoteServerContextRef::REGISTRATION,
        RemoteServerGDERef::REGISTRATION,
        ServerFacet::REGISTRATION,
    ]
}

/// The component lowerings this crate owns.
///
/// [`FacetedComponent`] is the middleware's own component, so its adapter is
/// written here rather than derived: the derive's `derived_component` would
/// pick the plain identity form under a Bevy-free `az-core`.
#[must_use]
pub const fn lowerings() -> [ComponentLoweringRegistration; 1] {
    [ComponentLoweringRegistration::bevy_component::<
        FacetedComponent,
    >()]
}

/// The Bevy-native reflected types this crate owns, for a host contribution to
/// register.
///
/// The middleware's reflected types are the one engine set that arrives
/// *composed* rather than through `engine_prefab_type_registry`'s direct-
/// application carve-out (asset-contract ticket 014, D7). The carve-out is for
/// crates every prefab host links unconditionally — az-prefab, az-transform,
/// az-render, az-framework — and this is not one of them: az-facet names
/// `gridmate`, so it rides the `runtime` bundle's nine roles like the rest of
/// the wire, and its entries carry that bundle's attribution.
#[must_use]
pub fn prefab_types() -> [PrefabType; 9] {
    [
        PrefabType::of::<Facet>(),
        PrefabType::of::<ClientFacet>(),
        PrefabType::of::<ServerFacet>(),
        PrefabType::of::<FacetedComponent>(),
        PrefabType::of::<GDEID>(),
        PrefabType::of::<RemoteServerContextRef>(),
        PrefabType::of::<RemoteServerGDERef>(),
        PrefabType::of::<LocalComponentRefBase>(),
        PrefabType::of::<RemoteTypelessServerFacetRef>(),
    ]
}

/// Register this crate's AZ types and component lowerings into a composing
/// host.
pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<AzTypeRegistration>().register_many(types());
    ctx.registrar::<ComponentLoweringRegistration>()
        .register_many(lowerings());
}

/// Native game-data entity identifier.
// `#[reflect(PartialEq)]` expands to a bevy_reflect comparison respanned onto
// the `PartialEq` token, so clippy reads it as local code and suggests
// `PartialEq.map_or(Reflect, |value| Reflect)` — not valid Rust. There is no
// user-written `if let/else` here to rewrite.
#[derive(
    az_derive::AzTypeInfo,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Marshaler,
    Serialize,
    Deserialize,
    Reflect,
)]
#[az_type_info(name = "GDEID", "07CE17BA-C4B7-4B42-81C1-79AF6A61F9A5")]
#[reflect(opaque)]
#[reflect(Hash, PartialEq(reflect_partial_eq))]
#[reflect(Serialize, Deserialize)]
pub struct GDEID {
    #[serde(rename = "ID", default)]
    pub id: u64,
}

impl GDEID {
    pub const INVALID: Self = Self {
        id: u32::MAX as u64,
    };

    #[inline]
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self { id: value }
    }

    #[inline]
    #[must_use]
    pub const fn value(self) -> u64 {
        self.id
    }

    #[inline]
    #[must_use]
    pub const fn is_invalid(self) -> bool {
        self.id == Self::INVALID.id
    }
}

impl Default for GDEID {
    #[inline]
    fn default() -> Self {
        Self::INVALID
    }
}

/// One online GDE root and the actor identity used to portray it remotely.
///
/// Native `MB::Context::GenerateGDEID` rejects the reserved zero-high-word
/// namespace and clears the low bit for version-2 online IDs. Actor identity
/// construction then places that GDEID in the first little-endian UUID word
/// and uses independent random data for the second word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GdeActorIdentity {
    gde_id: GDEID,
    actor_id: ActorId,
}

impl GdeActorIdentity {
    /// Mint an online GDE/actor identity using the operating system RNG.
    ///
    /// # Errors
    ///
    /// Returns [`GdeIdentityGenerationError`] if the operating system RNG
    /// (`getrandom::fill`) cannot supply entropy. Random draws that land in the
    /// reserved zero-high-word namespace are retried rather than reported.
    pub fn generate_online() -> Result<Self, GdeIdentityGenerationError> {
        loop {
            let mut random = [0_u8; 16];
            getrandom::fill(&mut random).map_err(GdeIdentityGenerationError)?;
            if let Some(identity) = Self::from_random_bytes(random) {
                return Ok(identity);
            }
        }
    }

    /// Build the native version-2 identity for an authored offline GDE.
    ///
    /// The authored UUID halves are interpreted as big-endian numeric words.
    /// Native folds them into the offline namespace, then preserves the
    /// supplied `baseGdeId` as the `ActorID`'s second little-endian wire word.
    #[must_use]
    pub fn from_offline_uuid(authored_id: Uuid, base_gde_id: u64) -> Self {
        let (high, low) = authored_id.as_u64_pair();
        let gde_id = GDEID::new(((high ^ low) & !4) | 3);
        let mut actor_id = [0_u8; 16];
        actor_id[..8].copy_from_slice(&gde_id.value().to_le_bytes());
        actor_id[8..].copy_from_slice(&base_gde_id.to_le_bytes());
        Self {
            gde_id,
            actor_id: ActorId(Uuid::from_bytes(actor_id)),
        }
    }

    #[inline]
    #[must_use]
    pub const fn gde_id(self) -> GDEID {
        self.gde_id
    }

    #[inline]
    #[must_use]
    pub const fn actor_id(self) -> ActorId {
        self.actor_id
    }

    #[inline]
    #[must_use]
    pub fn gde_ref(self) -> GdeRef {
        self.actor_id.into()
    }

    #[inline]
    #[must_use]
    pub const fn remote_ref(self) -> RemoteServerGDERef {
        RemoteServerGDERef::new(
            RemoteServerContextRef::new(Uid::new(self.actor_id.0)),
            self.gde_id,
        )
    }

    fn from_random_bytes(mut random: [u8; 16]) -> Option<Self> {
        let candidate = u64::from_le_bytes(random[..8].try_into().ok()?);
        if candidate & 0xffff_ffff_0000_0000 == 0 {
            return None;
        }
        let gde_id = GDEID::new(candidate & !1);
        random[..8].copy_from_slice(&gde_id.value().to_le_bytes());
        Some(Self {
            gde_id,
            actor_id: ActorId(Uuid::from_bytes(random)),
        })
    }
}

#[derive(Debug)]
pub struct GdeIdentityGenerationError(getrandom::Error);

impl core::fmt::Display for GdeIdentityGenerationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            formatter,
            "generate GDE identity from operating-system randomness: {}",
            self.0
        )
    }
}

impl std::error::Error for GdeIdentityGenerationError {}

/// Actor context portion of a remote server reference.
#[derive(
    AzRtti,
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Marshaler,
    Serialize,
    Deserialize,
    Reflect,
)]
#[az_rtti(
    name = "RemoteServerContextRef",
    "5BEDCB9D-F3BE-4300-80AF-B4E17A7F4646",
    register
)]
pub struct RemoteServerContextRef {
    #[serde(rename = "m_actorId", default)]
    pub actor_id: Uid,
}

impl RemoteServerContextRef {
    #[inline]
    #[must_use]
    pub const fn new(actor_id: Uid) -> Self {
        Self { actor_id }
    }

    #[inline]
    #[must_use]
    pub const fn has_actor_id(self) -> bool {
        !self.actor_id.is_zero()
    }

    #[inline]
    #[must_use]
    pub fn actor_id(self) -> Option<ActorId> {
        self.has_actor_id()
            .then_some(ActorId(self.actor_id.value()))
    }

    #[inline]
    #[must_use]
    pub fn actor_ref(self) -> Option<HubActorRef> {
        self.actor_id().map(HubActorRef::from_actor_id)
    }
}

/// Native actor-context plus game-data entity target.
#[derive(
    AzRtti,
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Marshaler,
    Serialize,
    Deserialize,
    Reflect,
)]
#[az_rtti(
    name = "RemoteServerGDERef",
    "17207DAC-730B-4793-B975-23591A950260",
    register
)]
#[reflect(opaque)]
#[reflect(Hash, PartialEq(reflect_partial_eq))]
#[reflect(Serialize, Deserialize)]
pub struct RemoteServerGDERef {
    #[serde(rename = "m_remoteServerContext", default)]
    pub remote_server_context: RemoteServerContextRef,
    #[serde(rename = "m_targetId", default)]
    pub target_id: GDEID,
}

impl RemoteServerGDERef {
    #[inline]
    #[must_use]
    pub const fn new(remote_server_context: RemoteServerContextRef, target_id: GDEID) -> Self {
        Self {
            remote_server_context,
            target_id,
        }
    }

    #[inline]
    #[must_use]
    pub const fn has_target(self) -> bool {
        !self.target_id.is_invalid()
    }

    #[inline]
    #[must_use]
    pub fn actor_id(self) -> Option<ActorId> {
        self.remote_server_context.actor_id()
    }

    #[inline]
    #[must_use]
    pub fn actor_ref(self) -> Option<HubActorRef> {
        self.remote_server_context.actor_ref()
    }

    #[inline]
    #[must_use]
    pub const fn from_gde_ref(gde_ref: GdeRef) -> Self {
        Self::new(
            RemoteServerContextRef::new(Uid::new(gde_ref.as_uuid())),
            GDEID::new(gde_ref.root_id()),
        )
    }
}

/// Native base shared by typed local component references.
#[derive(
    AzRtti,
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Marshaler,
    Serialize,
    Deserialize,
    Reflect,
)]
#[az_rtti(
    name = "LocalComponentRefBase",
    "EB20D4F6-82A2-4FEC-B020-CD10833E9793",
    register
)]
pub struct LocalComponentRefBase {
    #[serde(rename = "EntityId", default)]
    pub entity_id: az_core::EntityId,
    #[serde(rename = "InterfaceType", default)]
    pub interface_type: Uuid,
}

impl LocalComponentRefBase {
    pub const INVALID: Self = Self {
        entity_id: az_core::EntityId::INVALID,
        interface_type: Uuid::nil(),
    };

    #[must_use]
    pub const fn new(entity_id: az_core::EntityId, interface_type: Uuid) -> Self {
        Self {
            entity_id,
            interface_type,
        }
    }

    #[must_use]
    pub const fn invalid() -> Self {
        Self::INVALID
    }

    #[must_use]
    pub const fn is_invalid(self) -> bool {
        self.entity_id.is_invalid()
    }

    #[must_use]
    pub const fn is_valid(self) -> bool {
        !self.is_invalid()
    }
}

/// Native untyped remote server-facet target.
#[derive(
    az_derive::AzTypeInfo,
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Marshaler,
    Serialize,
    Deserialize,
    Reflect,
)]
#[az_type_info(
    name = "RemoteTypelessServerFacetRef",
    "6328304A-B754-4AD3-BF78-87236958B55B"
)]
#[reflect(opaque)]
#[reflect(Hash, PartialEq(reflect_partial_eq))]
#[reflect(Serialize, Deserialize)]
pub struct RemoteTypelessServerFacetRef {
    #[serde(rename = "m_remoteServerGDERef", default)]
    pub remote_server_gde_ref: RemoteServerGDERef,
    #[serde(rename = "m_targetId", default)]
    pub target_id: u64,
}

impl RemoteTypelessServerFacetRef {
    #[must_use]
    pub const fn new(remote_server_gde_ref: RemoteServerGDERef, target_id: u64) -> Self {
        Self {
            remote_server_gde_ref,
            target_id,
        }
    }

    #[must_use]
    pub const fn has_target(self) -> bool {
        self.target_id != 0 || self.remote_server_gde_ref.has_target()
    }

    #[must_use]
    pub fn actor_request_id(self) -> Option<ActorRequestId> {
        if self.target_id == 0 {
            return None;
        }
        self.remote_server_gde_ref
            .actor_ref()?
            .actor_request_id(self.target_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_core::{AzTypeInfo, component::ComponentId};
    use bevy_reflect::Reflect;
    use gridmate::serialize::Marshaler;
    use gridmate::serialize::buffer::{CARRIER_ENDIAN, ReadBuffer, WriteBuffer};

    #[derive(Reflect)]
    struct GeneratedComponentShape {
        faceted_component: FacetedComponent,
    }

    #[test]
    fn faceted_component_roundtrips_exact_native_fields() {
        let value = FacetedComponent {
            az_component: AzComponentBase {
                id: ComponentId::new(0x0123_4567_89ab_cdef),
            },
            client_facet_ptr: ClientFacet { facet: Facet },
            server_facet_ptr: ServerFacet { facet: Facet },
            replication_index: 37,
        };
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        value.marshal(&mut wb);
        let bytes = wb.into_vec();
        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &bytes);

        assert_eq!(FacetedComponent::unmarshal(&mut rb).unwrap(), value);
        assert!(rb.is_empty());
        assert_eq!(bytes.len(), u64::MARSHAL_SIZE + u32::MARSHAL_SIZE);
    }

    #[test]
    fn reflected_replication_index_finds_nested_faceted_base() {
        let expected = ReplicationIndex::new(544_828_704).expect("non-zero replication index");
        let component = GeneratedComponentShape {
            faceted_component: FacetedComponent {
                replication_index: expected.get(),
                ..Default::default()
            },
        };

        assert_eq!(reflected_replication_index(&component), Some(expected));
        assert_eq!(
            reflected_replication_index(component.to_dynamic().as_ref()),
            Some(expected),
        );
    }

    #[test]
    fn game_data_ids_default_to_the_native_invalid_sentinel() {
        assert_eq!(GDEID::default(), GDEID::INVALID);
        assert!(GDEID::default().is_invalid());
    }

    #[test]
    fn online_gde_identity_uses_the_native_v2_namespace_and_actor_layout() {
        let identity = GdeActorIdentity::from_random_bytes([
            0x05, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
            0xff, 0x10,
        ])
        .expect("non-zero high word");

        assert_eq!(identity.gde_id().value(), 0x8877_6655_4433_2204);
        assert_eq!(
            &identity.actor_id().as_bytes()[..8],
            &identity.gde_id().value().to_le_bytes()
        );
        assert_eq!(identity.gde_ref().actor_id(), identity.actor_id());
        assert_eq!(identity.remote_ref().target_id, identity.gde_id());
        assert_eq!(identity.remote_ref().actor_id(), Some(identity.actor_id()));
    }

    #[test]
    fn online_gde_identity_rejects_the_reserved_zero_high_word_namespace() {
        assert!(GdeActorIdentity::from_random_bytes([0; 16]).is_none());
    }
    #[test]
    fn offline_gde_identity_matches_native_authored_capital_fixtures() {
        let base_gde_id = 0xf6e4_d4dc_6a74_d185_u64;
        let fixtures = [
            (
                "02f0b3b6-4175-14a5-6bfc-6d3638207009",
                "ab645579-80de-0c69-85d1-746adcd4e4f6",
            ),
            (
                "6f180209-9f35-a280-b813-e86e0e049613",
                "93343191-67ea-0bd7-85d1-746adcd4e4f6",
            ),
            (
                "559a165f-a477-2495-80ca-cc1b6eb3eb9d",
                "0bcfc4ca-44da-50d5-85d1-746adcd4e4f6",
            ),
            (
                "1bedb102-8762-16b8-9cc5-aa634de22b9a",
                "233d80ca-611b-2887-85d1-746adcd4e4f6",
            ),
            (
                "ae26e6b1-a84d-0f7a-f6d1-a1f6f3f1fe01",
                "7bf1bc5b-4747-f758-85d1-746adcd4e4f6",
            ),
            (
                "1c4b9e65-5ada-959a-b6f5-f714f71ac679",
                "e353c0ad-7169-beaa-85d1-746adcd4e4f6",
            ),
        ];

        for (authored, expected) in fixtures {
            let identity = GdeActorIdentity::from_offline_uuid(
                Uuid::parse_str(authored).expect("authored capital UUID"),
                base_gde_id,
            );
            assert_eq!(
                identity.gde_ref().as_uuid(),
                Uuid::parse_str(expected).unwrap()
            );
            assert_eq!(identity.gde_id().value() & 7, 3);
            assert_eq!(
                &identity.actor_id().as_bytes()[8..],
                &base_gde_id.to_le_bytes()
            );
        }
    }

    #[test]
    fn component_reference_types_match_native_identity_and_invalid_defaults() {
        assert_eq!(
            LocalComponentRefBase::TYPE_ID,
            uuid::uuid!("EB20D4F6-82A2-4FEC-B020-CD10833E9793")
        );
        assert_eq!(
            RemoteTypelessServerFacetRef::TYPE_ID,
            uuid::uuid!("6328304A-B754-4AD3-BF78-87236958B55B")
        );
        assert!(LocalComponentRefBase::default().is_invalid());
        assert_eq!(
            RemoteTypelessServerFacetRef::default()
                .remote_server_gde_ref
                .target_id,
            GDEID::INVALID
        );
        assert!(!RemoteTypelessServerFacetRef::default().has_target());
        assert_eq!(
            RemoteTypelessServerFacetRef::default().actor_request_id(),
            None
        );
    }

    #[test]
    fn typeless_server_facet_targets_use_native_actor_request_ids() {
        let actor_id = ActorId(Uuid::from_bytes([
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
            0x0f, 0x10,
        ]));
        let target = RemoteTypelessServerFacetRef::new(
            RemoteServerGDERef::new(
                RemoteServerContextRef::new(Uid::new(actor_id.0)),
                GDEID::default(),
            ),
            0x20,
        );

        assert!(target.has_target());
        assert_eq!(
            target.actor_request_id(),
            Some(ActorRequestId::new(
                0x0807_0605_0403_0221,
                0x0807_0605_0403_0201,
            ))
        );
    }
}
