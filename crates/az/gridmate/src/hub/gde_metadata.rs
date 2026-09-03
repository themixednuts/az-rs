//! Actor identity metadata carried by `GdeMetadataReplicatedState`.

use az_core::asset::AssetId;
use az_derive::AzTypeInfo;

use super::{FragmentCategory, GdeRef};
use crate::serialize::ReplicatedFieldHandler;
use crate::{ClassDesc, ReplicatedState};

/// Metadata fragment that establishes an interest's actor identity and source
/// asset before ordinary actor state is applied.
#[derive(AzTypeInfo, Debug, Clone, Default, ClassDesc, ReplicatedState)]
#[class_desc(type_index = 10)]
#[az_type_info("203DC8C7-0C60-454B-A46F-566114314B84")]
#[replicated_state(metadata, category_field = "replication_category")]
pub struct GdeMetadataReplicatedState {
    #[replicated_state(name = "AssetId")]
    pub asset_id: ReplicatedFieldHandler<AssetId>,
    #[replicated_state(name = "GdeRef")]
    pub gde_ref: ReplicatedFieldHandler<GdeRef>,
    #[replicated_state(attribute, name = "ReplicationCategory")]
    pub replication_category: ReplicatedFieldHandler<FragmentCategory>,

    pub hub: super::ReplicatedState,
}

impl GdeMetadataReplicatedState {
    #[must_use]
    pub fn with_asset(asset_id: AssetId, gde_ref: GdeRef, category: FragmentCategory) -> Self {
        let mut state = Self::with_ref(gde_ref, category);
        state.asset_id.set_value(asset_id);
        state
    }

    #[must_use]
    pub fn with_ref(gde_ref: GdeRef, category: FragmentCategory) -> Self {
        let mut state = Self::default();
        state.gde_ref.set_value(gde_ref);
        state.replication_category.set_value(category);
        state
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::hub::{Fragment, MarshalContext};
    use crate::serialize::{CARRIER_ENDIAN, ReadBuffer, WriteBuffer};

    fn metadata() -> GdeMetadataReplicatedState {
        GdeMetadataReplicatedState::with_asset(
            AssetId::new(
                Uuid::from_bytes([
                    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
                    0x0e, 0x0f, 0x10,
                ]),
                0x1122_3344,
            ),
            GdeRef::new(Uuid::from_bytes([
                0x1a, 0x95, 0x4a, 0xbc, 0x4b, 0x31, 0x85, 0xbf, 0xbe, 0x37, 0xc3, 0xd8, 0x59, 0x26,
                0x18, 0xe0,
            ])),
            FragmentCategory::PlayerCharacter,
        )
    }

    #[test]
    fn metadata_body_uses_canonical_asset_and_gde_reference_types() {
        let state = metadata();
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);

        assert!(state.marshal_contents_with(&MarshalContext::default(), &mut wb));

        assert_eq!(
            wb.as_slice(),
            &[
                0x01, 0x03, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
                0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x22, 0x33, 0x44, 0x1a, 0x95, 0x4a, 0xbc, 0x4b, 0x31,
                0x85, 0xbf, 0xbe, 0x37, 0xc3, 0xd8, 0x59, 0x26, 0x18, 0xe0,
            ]
        );
        assert!(state.is_metadata());
        assert_eq!(state.category(), FragmentCategory::PlayerCharacter);
    }

    #[test]
    fn registered_category_attribute_roundtrips_through_replicated_state() {
        let state = metadata();
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);

        assert!(state.marshal_attributes(&MarshalContext::default(), &mut wb));

        let mut decoded = GdeMetadataReplicatedState::default();
        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, wb.as_slice());
        assert!(decoded.unmarshal_attributes(&mut rb).unwrap());
        assert_eq!(rb.left(), 0);
        assert_eq!(decoded.category(), FragmentCategory::PlayerCharacter);
    }
}
