//! Editor-owned scene tool state controller.

use az_editor_ui::{
    EditorSceneToolState,
    actions::{
        SetScenePivot, SetSceneTool, SetSceneTransformSpace, ToggleSceneAngleSnap,
        ToggleSceneGridSnap,
    },
};
use gpui::App;
use tracing::{info, instrument};

#[instrument(skip(cx))]
pub fn install_scene_tool_controller(cx: &mut App) {
    cx.set_global(EditorSceneToolState::default());
    info!("installed editor scene tool controller");
}

pub fn install_scene_tool_action_handlers(cx: &mut App) {
    cx.on_action(|action: &SetSceneTool, cx| {
        let tool = action.tool;
        cx.default_global::<EditorSceneToolState>().set_tool(tool);
        info!(
            target: "az_editor::aether_ui",
            phase = "state",
            event = "scene_tool.selected",
            tool = tool.key(),
            "ui state"
        );
        cx.refresh_windows();
    });

    cx.on_action(|action: &SetScenePivot, cx| {
        let pivot = action.pivot;
        cx.default_global::<EditorSceneToolState>().set_pivot(pivot);
        info!(
            target: "az_editor::aether_ui",
            phase = "state",
            event = "scene_pivot.selected",
            pivot = pivot.key(),
            "ui state"
        );
        cx.refresh_windows();
    });

    cx.on_action(|action: &SetSceneTransformSpace, cx| {
        let space = action.space;
        cx.default_global::<EditorSceneToolState>().set_space(space);
        info!(
            target: "az_editor::aether_ui",
            phase = "state",
            event = "scene_space.selected",
            space = space.key(),
            "ui state"
        );
        cx.refresh_windows();
    });

    cx.on_action(|_: &ToggleSceneGridSnap, cx| {
        let state = cx.default_global::<EditorSceneToolState>();
        state.toggle_grid_snap();
        info!(
            target: "az_editor::aether_ui",
            phase = "state",
            event = "scene_grid_snap.toggled",
            enabled = state.grid_snap.enabled,
            step_meters = state.grid_snap.step_meters,
            "ui state"
        );
        cx.refresh_windows();
    });

    cx.on_action(|_: &ToggleSceneAngleSnap, cx| {
        let state = cx.default_global::<EditorSceneToolState>();
        state.toggle_angle_snap();
        info!(
            target: "az_editor::aether_ui",
            phase = "state",
            event = "scene_angle_snap.toggled",
            enabled = state.angle_snap.enabled,
            degrees = state.angle_snap.degrees,
            "ui state"
        );
        cx.refresh_windows();
    });
}
