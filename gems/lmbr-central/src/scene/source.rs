use crate::{
    mesh::{InstancedMeshComponent, MeshComponent, SkinnedMeshComponent},
    non_empty_path,
};

/// Provides scene asset paths for components rendered through scene sync.
pub trait SceneComponentSource {
    const DEFAULT_NAME: &'static str;

    fn scene_asset_path(&self) -> Option<&str>;
    fn material_override_asset_path(&self) -> Option<&str> {
        None
    }
    fn visible(&self) -> bool;
}

impl SceneComponentSource for MeshComponent {
    const DEFAULT_NAME: &'static str = "MeshComponent";

    fn scene_asset_path(&self) -> Option<&str> {
        self.scene_asset_path()
    }

    fn material_override_asset_path(&self) -> Option<&str> {
        non_empty_path(self.render_node.material_override_asset_path.as_deref())
    }

    fn visible(&self) -> bool {
        self.render_node.visible
    }
}

impl SceneComponentSource for SkinnedMeshComponent {
    const DEFAULT_NAME: &'static str = "SkinnedMeshComponent";

    fn scene_asset_path(&self) -> Option<&str> {
        self.scene_asset_path()
    }

    fn visible(&self) -> bool {
        self.render_node.visible
    }
}

impl SceneComponentSource for InstancedMeshComponent {
    const DEFAULT_NAME: &'static str = "InstancedMeshComponent";

    fn scene_asset_path(&self) -> Option<&str> {
        self.scene_asset_path()
    }

    fn visible(&self) -> bool {
        self.render_node.mesh.visible
    }
}
