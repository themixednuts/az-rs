//! Amazon Hub actor identities and actor-scoped routing headers.

use bevy_reflect::Reflect;
use uuid::Uuid;

use crate::Marshaler;

/// Newtype for Hub actor UUIDs.
#[derive(
    Copy,
    Clone,
    Debug,
    Default,
    PartialEq,
    Eq,
    Hash,
    Marshaler,
    derive_more::From,
    derive_more::Into,
)]
pub struct ActorId(pub Uuid);

impl ActorId {
    #[inline]
    #[must_use]
    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        self.0.as_bytes()
    }
}

/// Wire value carried by reflected metadata references.
///
/// The 16 bytes are also the actor identity for the portrayed GDE, but this
/// wrapper preserves the field's GDE-reference semantics at the replication
/// boundary. Native derives the root GDEID from the first little-endian word.
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Marshaler)]
pub struct GdeRef(Uuid);

impl GdeRef {
    #[inline]
    #[must_use]
    pub const fn new(value: Uuid) -> Self {
        Self(value)
    }

    #[inline]
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn actor_id(self) -> ActorId {
        ActorId(self.0)
    }

    #[inline]
    #[must_use]
    pub const fn root_id(self) -> u64 {
        let bytes = self.0.as_bytes();
        u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ])
    }
}

impl From<ActorId> for GdeRef {
    #[inline]
    fn from(value: ActorId) -> Self {
        Self(value.0)
    }
}

impl From<GdeRef> for ActorId {
    #[inline]
    fn from(value: GdeRef) -> Self {
        value.actor_id()
    }
}

impl From<Uuid> for GdeRef {
    #[inline]
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl From<GdeRef> for Uuid {
    #[inline]
    fn from(value: GdeRef) -> Self {
        value.0
    }
}

/// Runtime actor base reference used by native actor-scoped facet routing.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ActorBaseRef(u64);

impl ActorBaseRef {
    #[inline]
    #[must_use]
    pub const fn from_actor_id(actor: ActorId) -> Self {
        Self(source_actor_ref_from(actor))
    }

    #[inline]
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn target_local_id(self, target_id: u64) -> u64 {
        self.0.wrapping_add(target_id)
    }
}

/// 16-byte actor-routing header used by actor-scoped facet RPCs.
///
/// The first word targets a materialized actor facet. The second word
/// identifies the source actor context.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Marshaler)]
pub struct ActorRequestId {
    pub target_local_id: u64,
    pub source_actor_ref: u64,
}

impl ActorRequestId {
    /// Build an actor header from the two native route words.
    #[inline]
    #[must_use]
    pub const fn new(target_local_id: u64, source_actor_ref: u64) -> Self {
        Self {
            target_local_id,
            source_actor_ref,
        }
    }

    /// Build the actor-scoped facet header for a materialized target.
    #[inline]
    #[must_use]
    pub const fn for_facet(actor: ActorId, target_id: u64) -> Self {
        let base = ActorBaseRef::from_actor_id(actor);
        Self::new(base.target_local_id(target_id), base.value())
    }

    #[inline]
    #[must_use]
    pub const fn source_actor_ref_for(actor: ActorId) -> u64 {
        ActorBaseRef::from_actor_id(actor).value()
    }
}

const fn source_actor_ref_from(actor: ActorId) -> u64 {
    let bytes = actor.as_bytes();
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

/// Wire value for `Amazon::Hub::ActorRef`.
///
/// Native storage and the Hub message wire shape are a big-endian `u32`
/// followed by two raw UUIDs, for a fixed total of 36 bytes.
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    Reflect,
    Marshaler,
)]
pub struct HubActorRef {
    pub local_id: u32,
    pub actor_id: Uuid,
    pub context_id: Uuid,
}

impl HubActorRef {
    #[inline]
    #[must_use]
    pub const fn new(local_id: u32, actor_id: Uuid, context_id: Uuid) -> Self {
        Self {
            local_id,
            actor_id,
            context_id,
        }
    }

    #[inline]
    #[must_use]
    pub const fn from_actor_id(actor: ActorId) -> Self {
        Self::new(0, actor.0, Uuid::nil())
    }

    #[inline]
    #[must_use]
    pub const fn is_null(&self) -> bool {
        self.local_id == 0 && self.actor_id.is_nil() && self.context_id.is_nil()
    }

    #[inline]
    #[must_use]
    pub fn actor_id(&self) -> Option<ActorId> {
        (!self.actor_id.is_nil()).then_some(ActorId(self.actor_id))
    }

    #[inline]
    #[must_use]
    pub fn base_ref(self) -> Option<ActorBaseRef> {
        self.actor_id().map(ActorBaseRef::from_actor_id)
    }

    #[inline]
    #[must_use]
    pub fn actor_request_id(self, target_id: u64) -> Option<ActorRequestId> {
        self.actor_id()
            .map(|actor| ActorRequestId::for_facet(actor, target_id))
    }
}

impl From<ActorId> for HubActorRef {
    #[inline]
    fn from(actor: ActorId) -> Self {
        Self::from_actor_id(actor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialize::Marshaler as _;
    use crate::serialize::buffer::{CARRIER_ENDIAN, ReadBuffer, WriteBuffer};
    use uuid::uuid;

    #[test]
    fn actor_ref_wire_shape_is_u32_then_two_raw_uuids() {
        let actor_id = uuid!("11223344-5566-7788-99aa-bbccddeeff00");
        let context_id = uuid!("00112233-4455-6677-8899-aabbccddeeff");
        let actor_ref = HubActorRef::new(0x0102_0304, actor_id, context_id);

        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        actor_ref.marshal(&mut wb);

        let mut expected = Vec::new();
        expected.extend_from_slice(&0x0102_0304u32.to_be_bytes());
        expected.extend_from_slice(actor_id.as_bytes());
        expected.extend_from_slice(context_id.as_bytes());
        assert_eq!(wb.as_slice(), expected.as_slice());
        assert_eq!(wb.len(), 36);

        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, wb.as_slice());
        assert_eq!(HubActorRef::unmarshal(&mut rb).unwrap(), actor_ref);
        assert_eq!(rb.left(), 0);
    }

    #[test]
    fn source_actor_ref_is_the_little_endian_uuid_prefix() {
        let actor = ActorId(uuid!("11223344-5566-7788-99aa-bbccddeeff00"));
        assert_eq!(
            ActorRequestId::source_actor_ref_for(actor),
            0x8877_6655_4433_2211
        );
    }

    #[test]
    fn gde_ref_preserves_actor_bytes_and_exposes_the_root_word() {
        let actor = ActorId(uuid!("11223344-5566-7788-99aa-bbccddeeff00"));
        let gde_ref = GdeRef::from(actor);

        assert_eq!(gde_ref.actor_id(), actor);
        assert_eq!(gde_ref.root_id(), 0x8877_6655_4433_2211);

        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        gde_ref.marshal(&mut wb);
        assert_eq!(wb.as_slice(), actor.as_bytes());
    }
}
