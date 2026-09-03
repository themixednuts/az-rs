// Root menu module, groups sub-menus (file/edit/view/...) when added.
// Kept minimal for now.

pub mod edit;
pub mod file;
pub mod view;

use gpui::{Menu, MenuItem};

#[must_use]
pub fn build_app_menus() -> Vec<Menu> {
    use crate::actions;
    vec![
        Menu::new("File").items([
            MenuItem::action("New Project…", actions::NewProject),
            MenuItem::action("Open Project…", actions::OpenProject),
            MenuItem::separator(),
            MenuItem::action("Save", actions::Save),
            MenuItem::action("Refresh Assets", actions::RefreshAssets),
            MenuItem::action(
                "Refresh Reflected Inspection",
                actions::RefreshReflectedInspection,
            ),
            MenuItem::separator(),
            MenuItem::action("Back to Project Launcher", actions::BackToProjectLauncher),
            MenuItem::action("Exit", actions::Quit),
        ]),
        Menu::new("Edit").items([
            MenuItem::action("Undo", actions::Undo),
            MenuItem::action("Redo", actions::Redo),
            MenuItem::separator(),
            MenuItem::action("Preferences...", actions::Preferences),
        ]),
        // Panel / workspace navigation (the design's "Window" menu).
        Menu::new("View").items([
            MenuItem::action("Hierarchy", actions::ToggleOutliner),
            MenuItem::action("Inspector", actions::ToggleInspector),
            MenuItem::action("Asset Browser", actions::ToggleAssetBrowser),
            MenuItem::action("Asset Processor", actions::OpenAssetProcessor),
            MenuItem::action("Console", actions::ToggleConsole),
            MenuItem::action("Session", actions::ToggleSessionPanel),
            MenuItem::separator(),
            MenuItem::action("Visual Graph Workbench", actions::ToggleGraphPanel),
            MenuItem::separator(),
            MenuItem::action("Refresh Outliner", actions::RefreshAuthoredOutline),
            MenuItem::separator(),
            MenuItem::action("Toggle Fullscreen", actions::ToggleFullscreen),
            MenuItem::separator(),
            MenuItem::action("Reset Layout", actions::ResetLayout),
        ]),
        // Editor-world runtime (the design's "Build & Run").
        Menu::new("Run").items([
            MenuItem::action("Launch Editor World", actions::LaunchEditorWorld),
            MenuItem::action(
                "Stop Editor World",
                actions::StopEditorWorld { preserve: false },
            ),
            MenuItem::separator(),
            MenuItem::action("Refresh Runtime Status", actions::RefreshRuntimeStatus),
            MenuItem::action("Refresh Viewport Frame", actions::RefreshViewportFrame),
            MenuItem::action(
                "Refresh Runtime Projections",
                actions::RefreshRuntimeProjections,
            ),
        ]),
        // Session supervision lifecycle.
        Menu::new("Session").items([
            MenuItem::action("Refresh Status", actions::RefreshSessionStatus),
            MenuItem::separator(),
            MenuItem::action("Start Services", actions::StartSessionServices),
            MenuItem::action("Stop Services", actions::StopSessionServices),
            MenuItem::action("Recover", actions::RecoverSession),
            MenuItem::action("Force Recover", actions::ForceRecoverSession),
        ]),
        Menu::new("Help").items([MenuItem::action("About Azoth", actions::About)]),
    ]
}
