use uuid::{Uuid, uuid};

/// Deprecated `MeshComponent` AZ type UUID retained for compatibility.
pub const DEPRECATED_MESH_COMPONENT_TYPE_ID: Uuid = uuid!("9697D425-3D28-4414-93DD-1890E576AB4B");

/// Lumberyard `LmbrCentral::MeshComponent` AZ component UUID.
pub const MESH_COMPONENT_TYPE_ID: Uuid = uuid!("2F4BAD46-C857-4DCB-A454-C412DE67852A");

/// Lumberyard `LmbrCentral::MeshComponentRenderNode` type UUID.
pub const MESH_COMPONENT_RENDER_NODE_TYPE_ID: Uuid = uuid!("46FF2BC4-BEF9-4CC4-9456-36C127C310D7");

/// Lumberyard `LmbrCentral::MeshRenderOptions` type UUID.
pub const MESH_RENDER_OPTIONS_TYPE_ID: Uuid = uuid!("EFF77BEB-CB99-44A3-8F15-111B0200F50D");

/// Lumberyard `LmbrCentral::SkinnedMeshComponent` AZ component UUID.
pub const SKINNED_MESH_COMPONENT_TYPE_ID: Uuid = uuid!("C99EB110-CA74-4D95-83F0-2FCDD1FF418B");

/// Lumberyard `LmbrCentral::SkinnedMeshComponentRenderNode` type UUID.
pub const SKINNED_MESH_COMPONENT_RENDER_NODE_TYPE_ID: Uuid =
    uuid!("AE5CFE2B-6CFF-4B66-9B9C-C514BFDB8A88");

/// Lumberyard `LmbrCentral::SkinnedRenderOptions` type UUID.
pub const SKINNED_RENDER_OPTIONS_TYPE_ID: Uuid = uuid!("33E69F1C-518F-4DD2-88D1-DF6D12ECA54E");

/// `InstancedMeshComponent` AZ type UUID.
pub const INSTANCED_MESH_COMPONENT_TYPE_ID: Uuid = uuid!("6B5FCA5E-D112-4114-8D77-5CECC0078ACA");

/// `InstancedMeshComponentRenderNode` AZ type UUID.
pub const INSTANCED_MESH_COMPONENT_RENDER_NODE_TYPE_ID: Uuid =
    uuid!("A2522C48-084F-42BD-AE0F-2570F917CEA8");
