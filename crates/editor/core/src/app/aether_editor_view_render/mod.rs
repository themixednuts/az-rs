//! Aether view projections and rendering commands.
//!
//! The composer owns only region composition. Each domain module owns its
//! projections, handlers, and private presentation details.

pub(super) mod asset_browser;
pub(super) mod authored_content;
pub(super) mod diagnostics_preview;
pub(super) mod gamedata_gem;
pub(super) mod presentation;
pub(super) mod workspace_overlay;

#[derive(Debug, Clone, Copy)]
pub(in crate::app) enum AetherViewAction {
    GoAssets,
    GoConsole,
    GoOutput,
    GoProfiler,
    UseAssetGrid,
    UseAssetList,
    TogglePipe,
    ClosePipe,
    ToggleBuild,
    CloseBuild,
    CloseBuildAndMenu,
    ToggleLevelMenu,
    CloseLevelMenu,
    ToggleLayoutMenu,
    CloseLayoutMenu,
}
