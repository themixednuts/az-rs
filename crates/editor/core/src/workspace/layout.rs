use gpui_component::dock::DockPlacement;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelRole {
    Navigator,
    Details,
    Diagnostics,
    Workbench,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelVisibility {
    Open,
    Closed,
}

impl PanelVisibility {
    #[must_use]
    pub const fn is_open(self) -> bool {
        matches!(self, Self::Open)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceDock {
    Left,
    Right,
    Bottom,
    Center,
}

impl WorkspaceDock {
    #[must_use]
    pub const fn placement(self) -> DockPlacement {
        match self {
            Self::Left => DockPlacement::Left,
            Self::Right => DockPlacement::Right,
            Self::Bottom => DockPlacement::Bottom,
            Self::Center => DockPlacement::Center,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum WorkbenchKind {
    LevelViewport,
    VisualGraph,
    Material,
    Script,
    GameData,
    Sequencer,
    Animation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelPlacement {
    pub panel_name: &'static str,
    pub role: PanelRole,
    pub dock: WorkspaceDock,
    pub visibility: PanelVisibility,
    pub active: bool,
}

impl PanelPlacement {
    #[must_use]
    pub const fn new(
        panel_name: &'static str,
        role: PanelRole,
        dock: WorkspaceDock,
        visibility: PanelVisibility,
        active: bool,
    ) -> Self {
        Self {
            panel_name,
            role,
            dock,
            visibility,
            active,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkbenchPlacement {
    pub kind: WorkbenchKind,
    pub panel_name: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceLayoutProfile {
    pub name: &'static str,
    pub center: &'static [WorkbenchPlacement],
    pub left: &'static [PanelPlacement],
    pub right: &'static [PanelPlacement],
    pub bottom: &'static [PanelPlacement],
}

// ---------------------------------------------------------------------------
// Scene (default loaded-project) — design/aether-editor.outline.md
// Left: Hierarchy | Layers
// Center: Viewport
// Right: Inspector
// Bottom: Assets | Console | Output | Profiler | Gems
// ---------------------------------------------------------------------------

const SCENE_CENTER: &[WorkbenchPlacement] = &[WorkbenchPlacement {
    kind: WorkbenchKind::LevelViewport,
    panel_name: az_editor_ui::panels::ViewportPanel::NAME,
}];

const SCENE_LEFT: &[PanelPlacement] = &[
    PanelPlacement::new(
        az_editor_ui::panels::AuthoredOutliner::NAME,
        PanelRole::Navigator,
        WorkspaceDock::Left,
        PanelVisibility::Open,
        true,
    ),
    PanelPlacement::new(
        az_editor_ui::panels::SceneLayersPanel::NAME,
        PanelRole::Navigator,
        WorkspaceDock::Left,
        PanelVisibility::Open,
        false,
    ),
];

const SCENE_RIGHT: &[PanelPlacement] = &[PanelPlacement::new(
    az_editor_ui::panels::AuthoredInspector::NAME,
    PanelRole::Details,
    WorkspaceDock::Right,
    PanelVisibility::Open,
    true,
)];

const SCENE_BOTTOM: &[PanelPlacement] = &[
    PanelPlacement::new(
        az_editor_ui::panels::AssetBrowser::NAME,
        PanelRole::Navigator,
        WorkspaceDock::Bottom,
        PanelVisibility::Open,
        true,
    ),
    PanelPlacement::new(
        az_editor_ui::panels::Console::NAME,
        PanelRole::Diagnostics,
        WorkspaceDock::Bottom,
        PanelVisibility::Open,
        false,
    ),
    PanelPlacement::new(
        az_editor_ui::panels::OutputLogPanel::NAME,
        PanelRole::Diagnostics,
        WorkspaceDock::Bottom,
        PanelVisibility::Open,
        false,
    ),
    PanelPlacement::new(
        az_editor_ui::panels::ProfilerPanel::NAME,
        PanelRole::Diagnostics,
        WorkspaceDock::Bottom,
        PanelVisibility::Open,
        false,
    ),
    PanelPlacement::new(
        az_editor_ui::panels::GemsPanel::NAME,
        PanelRole::Diagnostics,
        WorkspaceDock::Bottom,
        PanelVisibility::Open,
        false,
    ),
];

// ---------------------------------------------------------------------------
// Materials — design HTML: palette | graph canvas | preview+params
// NO bottom dock (Asset Browser does not belong here)
// ---------------------------------------------------------------------------

const MATERIAL_CENTER: &[WorkbenchPlacement] = &[WorkbenchPlacement {
    kind: WorkbenchKind::Material,
    panel_name: az_editor_ui::panels::MaterialWorkbenchPanel::NAME,
}];

const MATERIAL_LEFT: &[PanelPlacement] = &[PanelPlacement::new(
    az_editor_ui::panels::MaterialPalettePanel::NAME,
    PanelRole::Navigator,
    WorkspaceDock::Left,
    PanelVisibility::Open,
    true,
)];

const MATERIAL_RIGHT: &[PanelPlacement] = &[PanelPlacement::new(
    az_editor_ui::panels::MaterialParamsPanel::NAME,
    PanelRole::Details,
    WorkspaceDock::Right,
    PanelVisibility::Open,
    true,
)];

const EMPTY_BOTTOM: &[PanelPlacement] = &[];

// ---------------------------------------------------------------------------
// Scripting — design HTML: file tree | code | outline
// NO bottom dock
// ---------------------------------------------------------------------------

const SCRIPT_CENTER: &[WorkbenchPlacement] = &[WorkbenchPlacement {
    kind: WorkbenchKind::Script,
    panel_name: az_editor_ui::panels::ScriptEditorPanel::NAME,
}];

const SCRIPT_LEFT: &[PanelPlacement] = &[PanelPlacement::new(
    az_editor_ui::panels::ScriptFilesPanel::NAME,
    PanelRole::Navigator,
    WorkspaceDock::Left,
    PanelVisibility::Open,
    true,
)];

const SCRIPT_RIGHT: &[PanelPlacement] = &[PanelPlacement::new(
    az_editor_ui::panels::ScriptOutlinePanel::NAME,
    PanelRole::Details,
    WorkspaceDock::Right,
    PanelVisibility::Open,
    true,
)];

// ---------------------------------------------------------------------------
// Game Data — left catalog, center table workbench, right details
// NO bottom Asset Browser (capabilities: left+right, no bottom)
// ---------------------------------------------------------------------------

const GAMEDATA_CENTER: &[WorkbenchPlacement] = &[WorkbenchPlacement {
    kind: WorkbenchKind::GameData,
    panel_name: az_editor_ui::panels::GameDataWorkbenchPanel::NAME,
}];

const GAMEDATA_LEFT: &[PanelPlacement] = &[PanelPlacement::new(
    az_editor_ui::panels::GameDataCatalogPanel::NAME,
    PanelRole::Navigator,
    WorkspaceDock::Left,
    PanelVisibility::Open,
    true,
)];

const GAMEDATA_RIGHT: &[PanelPlacement] = &[PanelPlacement::new(
    az_editor_ui::panels::GameDataDetailsPanel::NAME,
    PanelRole::Details,
    WorkspaceDock::Right,
    PanelVisibility::Open,
    true,
)];

// ---------------------------------------------------------------------------
// Sequencer — HTML folds into scene body (modeScene includes sequencer).
// Keep scene docks; center is sequencer workbench.
// ---------------------------------------------------------------------------

const SEQUENCER_CENTER: &[WorkbenchPlacement] = &[WorkbenchPlacement {
    kind: WorkbenchKind::Sequencer,
    panel_name: az_editor_ui::panels::SequencerWorkbenchPanel::NAME,
}];

// ---------------------------------------------------------------------------
// Animation — left navigator (graph/motions/sets/skeleton + parameters),
// center anim-graph workbench, right mannequin viewport + attributes,
// bottom: motion tracks (primary) + assets/console secondary
// ---------------------------------------------------------------------------

const ANIMATION_CENTER: &[WorkbenchPlacement] = &[WorkbenchPlacement {
    kind: WorkbenchKind::Animation,
    panel_name: az_editor_ui::panels::AnimationWorkbenchPanel::NAME,
}];

const ANIMATION_LEFT: &[PanelPlacement] = &[PanelPlacement::new(
    az_editor_ui::panels::AnimationNavigatorPanel::NAME,
    PanelRole::Navigator,
    WorkspaceDock::Left,
    PanelVisibility::Open,
    true,
)];

// Right dock per design: mannequin viewport + attributes (primary), with the
// authored inspector as a secondary tab for authored animation-document edits
// (blend-space examples, Mannequin fragment options).
const ANIMATION_RIGHT: &[PanelPlacement] = &[
    PanelPlacement::new(
        az_editor_ui::panels::AnimationMannequinPanel::NAME,
        PanelRole::Details,
        WorkspaceDock::Right,
        PanelVisibility::Open,
        true,
    ),
    PanelPlacement::new(
        az_editor_ui::panels::AuthoredInspector::NAME,
        PanelRole::Details,
        WorkspaceDock::Right,
        PanelVisibility::Open,
        false,
    ),
];

const ANIMATION_BOTTOM: &[PanelPlacement] = &[
    PanelPlacement::new(
        az_editor_ui::panels::AnimationMotionTrackPanel::NAME,
        PanelRole::Diagnostics,
        WorkspaceDock::Bottom,
        PanelVisibility::Open,
        true,
    ),
    PanelPlacement::new(
        az_editor_ui::panels::AssetBrowser::NAME,
        PanelRole::Navigator,
        WorkspaceDock::Bottom,
        PanelVisibility::Open,
        false,
    ),
    PanelPlacement::new(
        az_editor_ui::panels::Console::NAME,
        PanelRole::Diagnostics,
        WorkspaceDock::Bottom,
        PanelVisibility::Open,
        false,
    ),
];

// ---------------------------------------------------------------------------
// Static profiles
// ---------------------------------------------------------------------------

static LOADED_PROJECT_WORKSPACE: WorkspaceLayoutProfile = WorkspaceLayoutProfile {
    name: "loaded-project",
    center: SCENE_CENTER,
    left: SCENE_LEFT,
    right: SCENE_RIGHT,
    bottom: SCENE_BOTTOM,
};

static MATERIAL_WORKSPACE: WorkspaceLayoutProfile = WorkspaceLayoutProfile {
    name: "materials",
    center: MATERIAL_CENTER,
    left: MATERIAL_LEFT,
    right: MATERIAL_RIGHT,
    bottom: EMPTY_BOTTOM,
};

static SCRIPT_WORKSPACE: WorkspaceLayoutProfile = WorkspaceLayoutProfile {
    name: "scripting",
    center: SCRIPT_CENTER,
    left: SCRIPT_LEFT,
    right: SCRIPT_RIGHT,
    bottom: EMPTY_BOTTOM,
};

static GAMEDATA_WORKSPACE: WorkspaceLayoutProfile = WorkspaceLayoutProfile {
    name: "gamedata",
    center: GAMEDATA_CENTER,
    left: GAMEDATA_LEFT,
    right: GAMEDATA_RIGHT,
    bottom: EMPTY_BOTTOM,
};

static SEQUENCER_WORKSPACE: WorkspaceLayoutProfile = WorkspaceLayoutProfile {
    name: "sequencer",
    center: SEQUENCER_CENTER,
    left: SCENE_LEFT,
    right: SCENE_RIGHT,
    bottom: SCENE_BOTTOM,
};

static ANIMATION_WORKSPACE: WorkspaceLayoutProfile = WorkspaceLayoutProfile {
    name: "animation",
    center: ANIMATION_CENTER,
    left: ANIMATION_LEFT,
    right: ANIMATION_RIGHT,
    bottom: ANIMATION_BOTTOM,
};

static PROFILER_WORKSPACE: WorkspaceLayoutProfile = WorkspaceLayoutProfile {
    name: "profiler",
    // HTML: profiler reuses scene body; bottom profiler tab is the focus.
    center: SCENE_CENTER,
    left: SCENE_LEFT,
    right: SCENE_RIGHT,
    bottom: SCENE_BOTTOM,
};

#[must_use]
pub fn loaded_project_workspace_layout() -> &'static WorkspaceLayoutProfile {
    &LOADED_PROJECT_WORKSPACE
}

#[must_use]
pub fn animation_workspace_layout() -> &'static WorkspaceLayoutProfile {
    &ANIMATION_WORKSPACE
}

#[must_use]
pub fn material_workspace_layout() -> &'static WorkspaceLayoutProfile {
    &MATERIAL_WORKSPACE
}

#[must_use]
pub fn script_workspace_layout() -> &'static WorkspaceLayoutProfile {
    &SCRIPT_WORKSPACE
}

#[must_use]
pub fn gamedata_workspace_layout() -> &'static WorkspaceLayoutProfile {
    &GAMEDATA_WORKSPACE
}

#[must_use]
pub fn sequencer_workspace_layout() -> &'static WorkspaceLayoutProfile {
    &SEQUENCER_WORKSPACE
}

/// Resolve the layout profile for an activity-rail mode key.
///
/// Matches design HTML mode bodies:
/// - scene / profiler → scene docks
/// - materials → palette | graph | params (no bottom)
/// - scripting → files | code | outline (no bottom)
/// - gamedata → catalog | table | details (no bottom)
/// - sequencer → scene docks + sequencer center
/// - animation → navigator | workbench | inspector + motion tracks bottom
#[must_use]
pub fn layout_profile_for_mode(mode: &str) -> &'static WorkspaceLayoutProfile {
    match mode {
        "animation" => animation_workspace_layout(),
        "materials" => material_workspace_layout(),
        "scripting" => script_workspace_layout(),
        "gamedata" => gamedata_workspace_layout(),
        "sequencer" => sequencer_workspace_layout(),
        "profiler" => &PROFILER_WORKSPACE,
        _ => loaded_project_workspace_layout(),
    }
}

#[must_use]
pub const fn visual_graph_workbench() -> WorkbenchPlacement {
    WorkbenchPlacement {
        kind: WorkbenchKind::VisualGraph,
        panel_name: az_editor_ui::panels::VisualGraphPanel::NAME,
    }
}

#[must_use]
pub const fn animation_workbench() -> WorkbenchPlacement {
    WorkbenchPlacement {
        kind: WorkbenchKind::Animation,
        panel_name: az_editor_ui::panels::AnimationWorkbenchPanel::NAME,
    }
}

#[must_use]
pub const fn material_workbench() -> WorkbenchPlacement {
    WorkbenchPlacement {
        kind: WorkbenchKind::Material,
        panel_name: az_editor_ui::panels::MaterialWorkbenchPanel::NAME,
    }
}

#[must_use]
pub const fn script_workbench() -> WorkbenchPlacement {
    WorkbenchPlacement {
        kind: WorkbenchKind::Script,
        panel_name: az_editor_ui::panels::ScriptEditorPanel::NAME,
    }
}

#[must_use]
pub const fn gamedata_workbench() -> WorkbenchPlacement {
    WorkbenchPlacement {
        kind: WorkbenchKind::GameData,
        panel_name: az_editor_ui::panels::GameDataWorkbenchPanel::NAME,
    }
}

#[must_use]
pub const fn sequencer_workbench() -> WorkbenchPlacement {
    WorkbenchPlacement {
        kind: WorkbenchKind::Sequencer,
        panel_name: az_editor_ui::panels::SequencerWorkbenchPanel::NAME,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_editor_ui::panels::{
        AnimationMotionTrackPanel, AnimationNavigatorPanel, AnimationWorkbenchPanel, AssetBrowser,
        AuthoredInspector, AuthoredOutliner, Console, GameDataCatalogPanel, GemsPanel,
        MaterialPalettePanel, MaterialParamsPanel, MaterialWorkbenchPanel, OutputLogPanel,
        ProfilerPanel, SceneLayersPanel, ScriptFilesPanel, ScriptOutlinePanel, ViewportPanel,
    };

    fn dock_of_profile(
        layout: &WorkspaceLayoutProfile,
        panel: &'static str,
    ) -> Option<WorkspaceDock> {
        layout
            .left
            .iter()
            .chain(layout.right)
            .chain(layout.bottom)
            .find(|placement| placement.panel_name == panel)
            .map(|placement| placement.dock)
    }

    #[test]
    fn scene_dock_ownership_matches_design() {
        let layout = loaded_project_workspace_layout();
        assert_eq!(
            dock_of_profile(layout, AuthoredOutliner::NAME),
            Some(WorkspaceDock::Left)
        );
        assert_eq!(
            dock_of_profile(layout, SceneLayersPanel::NAME),
            Some(WorkspaceDock::Left)
        );
        assert_eq!(
            dock_of_profile(layout, AuthoredInspector::NAME),
            Some(WorkspaceDock::Right)
        );
        assert_eq!(
            dock_of_profile(layout, AssetBrowser::NAME),
            Some(WorkspaceDock::Bottom)
        );
        assert_eq!(
            dock_of_profile(layout, Console::NAME),
            Some(WorkspaceDock::Bottom)
        );
        assert_eq!(
            dock_of_profile(layout, OutputLogPanel::NAME),
            Some(WorkspaceDock::Bottom)
        );
        assert_eq!(
            dock_of_profile(layout, ProfilerPanel::NAME),
            Some(WorkspaceDock::Bottom)
        );
        assert_eq!(
            dock_of_profile(layout, GemsPanel::NAME),
            Some(WorkspaceDock::Bottom)
        );
        assert!(dock_of_profile(layout, ViewportPanel::NAME).is_none());
        assert_eq!(layout.center[0].panel_name, ViewportPanel::NAME);
    }

    #[test]
    fn materials_has_no_asset_browser_and_uses_palette_params() {
        let layout = material_workspace_layout();
        assert!(layout.bottom.is_empty());
        assert_eq!(layout.left[0].panel_name, MaterialPalettePanel::NAME);
        assert_eq!(layout.right[0].panel_name, MaterialParamsPanel::NAME);
        assert_eq!(layout.center[0].panel_name, MaterialWorkbenchPanel::NAME);
        assert!(dock_of_profile(layout, AssetBrowser::NAME).is_none());
    }

    #[test]
    fn scripting_has_no_asset_browser_and_uses_files_outline() {
        let layout = script_workspace_layout();
        assert!(layout.bottom.is_empty());
        assert_eq!(layout.left[0].panel_name, ScriptFilesPanel::NAME);
        assert_eq!(layout.right[0].panel_name, ScriptOutlinePanel::NAME);
        assert!(dock_of_profile(layout, AssetBrowser::NAME).is_none());
    }

    #[test]
    fn gamedata_has_no_asset_browser() {
        let layout = gamedata_workspace_layout();
        assert!(layout.bottom.is_empty());
        assert_eq!(layout.left[0].panel_name, GameDataCatalogPanel::NAME);
        assert!(dock_of_profile(layout, AssetBrowser::NAME).is_none());
        assert!(dock_of_profile(layout, AuthoredOutliner::NAME).is_none());
    }

    #[test]
    fn animation_puts_motion_tracks_bottom_and_navigator_left() {
        let layout = animation_workspace_layout();
        assert_eq!(layout.left[0].panel_name, AnimationNavigatorPanel::NAME);
        assert_eq!(layout.center[0].panel_name, AnimationWorkbenchPanel::NAME);
        assert_eq!(layout.bottom[0].panel_name, AnimationMotionTrackPanel::NAME);
        assert!(layout.bottom[0].active);
        // Right dock: mannequin viewport + attributes (primary) per design,
        // authored inspector as the secondary tab.
        assert_eq!(
            layout.right[0].panel_name,
            az_editor_ui::panels::AnimationMannequinPanel::NAME
        );
        assert!(layout.right[0].active);
        assert_eq!(layout.right[1].panel_name, AuthoredInspector::NAME);
        assert!(!layout.right[1].active);
    }

    #[test]
    fn mode_profile_resolution() {
        assert_eq!(layout_profile_for_mode("scene").name, "loaded-project");
        assert_eq!(layout_profile_for_mode("animation").name, "animation");
        assert_eq!(layout_profile_for_mode("materials").name, "materials");
        assert_eq!(layout_profile_for_mode("scripting").name, "scripting");
        assert_eq!(layout_profile_for_mode("gamedata").name, "gamedata");
        assert!(layout_profile_for_mode("materials").bottom.is_empty());
        assert!(layout_profile_for_mode("gamedata").bottom.is_empty());
    }
}
