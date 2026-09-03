use az_core::{AzTypeInfo, rtti::AzRtti};
use az_gem_slayer_script::source::{
    SlayerScriptData, SlayerScriptDataContainer, SlayerScriptSource,
};
use serde::{Deserialize, Serialize};
use uuid::{Uuid, uuid};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum ClosedProjectSource {
    Base(SlayerScriptData),
    ProjectValue(u32),
}

impl SlayerScriptSource for ClosedProjectSource {
    fn source_type_id(&self) -> Uuid {
        match self {
            Self::Base(_) => <SlayerScriptData as AzTypeInfo>::TYPE_ID,
            Self::ProjectValue(_) => uuid!("11111111-2222-3333-4444-555555555555"),
        }
    }
}

#[test]
fn generic_source_type_ids_and_versions_match_native_registration() {
    assert_eq!(
        <SlayerScriptData as AzTypeInfo>::TYPE_ID,
        uuid!("3CAD57DB-9179-456F-904F-1B3D68FAD90E")
    );
    assert_eq!(
        <SlayerScriptDataContainer<ClosedProjectSource> as AzTypeInfo>::TYPE_ID,
        uuid!("C9CCB4FB-44B8-4BE8-BCA8-4909C1C22B82")
    );
    assert!(
        <SlayerScriptDataContainer<ClosedProjectSource> as AzRtti>::is_type_of(uuid!(
            "C9CCB4FB-44B8-4BE8-BCA8-4909C1C22B82"
        ))
    );
    assert_eq!(SlayerScriptData::VERSION, 1);
    assert_eq!(SlayerScriptDataContainer::<ClosedProjectSource>::VERSION, 1);
}

#[test]
fn container_owns_a_cloneable_serializable_closed_project_projection() {
    let container = SlayerScriptDataContainer::new("project", ClosedProjectSource::ProjectValue(7));
    let cloned = container.clone();

    assert_eq!(cloned, container);
    assert_eq!(
        container.script_data.as_ref().unwrap().source_type_id(),
        uuid!("11111111-2222-3333-4444-555555555555")
    );
    assert_eq!(
        SlayerScriptDataContainer::<ClosedProjectSource>::SCRIPT_NAME_FIELD,
        "m_scriptName"
    );
    assert_eq!(
        SlayerScriptDataContainer::<ClosedProjectSource>::SCRIPT_DATA_FIELD,
        "m_scriptData"
    );
}
