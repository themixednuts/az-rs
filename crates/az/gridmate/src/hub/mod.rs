//! Amazon Hub actor-fragment replication — Rust port of the vendor fragment,
//! replicated-state, and fixed-replicated-state spine.
//!
//! `FixedReplicatedState` uses a source `AZStd::bitset<NGroups>` group-present
//! prefix plus a VLQ-u64 field-presence mask inside each present group.

pub mod actor;
pub mod application_layer_seq;
pub mod client_messaging;
pub mod fixed_replicated_state;
pub mod fragment;
pub mod gde_metadata;
pub mod ids;
#[macro_use]
pub mod macros;
pub mod replicated_state;
pub mod replicated_state_bundle;
pub mod sequence_number;
pub mod state_bundle_builder;
pub mod time;
pub mod time_point;

pub use actor::{ActorBaseRef, ActorId, ActorRequestId, GdeRef, HubActorRef};
pub use application_layer_seq::{ApplicationLayerSeqAck, MAX_APPLICATION_LAYER_SEQ_ACK_ENTRIES};
pub use client_messaging::{
    AckQosConfig, AckQosConfigError, ApplicationLayerSeqAckOutcome, ClientMessaging,
    ClientMessagingError, ClientMessagingUpdate, FragmentBaselineUpdate, FragmentSyncKey,
    ReusableBundleSend, ReusableBundleSendKind, StateBundleDelivery, StateBundleLane,
    StateBundleSequenceTracker,
};
pub use fixed_replicated_state::{
    FieldGroup, FieldGroupMut, FieldVector, FieldVectorMut, FixedMergeOutcome,
    FixedReplicatedState, FixedReplicatedStateFields, FixedStateRegister, NamedField,
    NamedFieldMut,
};
pub use fragment::{
    FRAGMENT_POOL_SIZE_BYTES, Fragment, FragmentBase, FragmentCategory, FragmentCategoryBitset,
    FragmentRegistration, Fragments, I_FRAGMENT_TYPE_ID, MAXIMUM_REPLICATION_FRAGMENTS,
    MarshalContext, NUM_FRAGMENT_CATEGORIES, fragment_category_from_string,
    fragment_category_to_string,
};
pub use gde_metadata::GdeMetadataReplicatedState;
pub use ids::{BandwidthMode, ClientContextId, FragmentKey, InterestId};
pub use replicated_state::{
    ClientFilterContainer, ClientFilterContainerMarshalShim, ClientFilterField, DefaultBitsField,
    FilterValue, REPLICATED_STATE_TYPE_ID, ReplicatedDefaultBits, ReplicatedFieldInfo,
    ReplicatedFieldInfoMut, ReplicatedFilterGroup, ReplicatedMergeOutcome, ReplicatedState,
    ReplicatedStateConstants,
};
pub use replicated_state_bundle::{
    MAX_REPLICATED_STATE_BUNDLE_BUFFER_SIZE, MAX_REPLICATION_CONTROL_IDS, ReplicatedStateBundle,
    ReplicatedStateBundleView, ReplicationControlData, ReplicationPerformanceData,
    StateFragmentHeaderSpan, StateFragmentTypeId, StateFragmentWriteSpan, StateRecordHeader,
    StateRecordWriter, decode_state_fragment_contents, marshal_bundle_buffer, read_bundle_buffer,
    read_state_fragment_header, read_state_fragment_type_id, read_state_record_header,
    write_state_fragment_type_id, write_state_record,
};
pub use sequence_number::SequenceNumber;
pub use state_bundle_builder::StateBundleBuilder;
pub use time::{SyncedClock, SyncedClockError, SyncedTimestamp};
pub use time_point::TimePoint;
