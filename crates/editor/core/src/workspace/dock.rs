use gpui::{App, AppContext, Context, Entity, Global, Subscription, Window, px};
use gpui_component::dock::{
    BottomDockExtent, DockArea, DockEvent, DockItem, DockPlacement, Panel, PanelEvent, PanelInfo,
    PanelRegistry, PanelState, PanelStyle, PanelView,
};
use gpui_component::tab::TabVariant;
use std::collections::BTreeMap;
use std::{collections::HashMap, sync::Arc};

use super::layout::{
    PanelPlacement, PanelVisibility, WorkbenchKind, WorkbenchPlacement, WorkspaceDock,
    WorkspaceLayoutProfile,
};
use crate::project_manager_preferences::{
    PersistedDockRegion, WORKSPACE_LAYOUT_VERSION, WorkspaceUiLayoutSnapshot,
};

#[derive(Default)]
struct WorkspacePanelCache {
    panels: HashMap<&'static str, Arc<dyn PanelView>>,
    subscriptions: Vec<Subscription>,
}

impl Global for WorkspacePanelCache {}

/// Notify one retained workspace panel without invalidating unrelated editor chrome.
///
/// Returns `false` when the panel has not been constructed or has an unexpected
/// concrete type, allowing callers to fall back to a full-window refresh.
pub fn notify_cached_panel<T: Panel>(panel_name: &'static str, cx: &mut gpui::App) -> bool {
    let Some(panel) = cx
        .try_global::<WorkspacePanelCache>()
        .and_then(|cache| cache.panels.get(panel_name))
        .cloned()
    else {
        return false;
    };
    let Ok(panel) = panel.view().downcast::<T>() else {
        return false;
    };
    panel.update(cx, |_, cx| cx.notify());
    true
}

/// Create a `DockArea` entity and initialize default left/right/bottom panels.
pub fn create_default_dock_area(
    window: &mut Window,
    cx: &mut Context<'_, super::super::app::EditorView>,
) -> Entity<DockArea> {
    let project_state = crate::ui_state_persistence::initial_project_state(cx);
    let mode = project_state.last_mode;
    let layout = super::layout::layout_profile_for_mode(&mode);
    let persisted = crate::ui_state_persistence::layout_for_mode(cx, &mode);
    let dock_area = cx.new(|cx| {
        DockArea::new("main", None, window, cx)
            .panel_style(PanelStyle::TabBar)
            .tab_variant(TabVariant::Dock)
            .tab_panel_actions_visible(false)
            .toggle_button_visible(false)
    });
    apply_workspace_layout_state(
        &dock_area,
        layout,
        persisted.as_ref(),
        &project_state.asset_view_mode,
        project_state.asset_folder_key,
        window,
        cx,
    );
    dock_area
}

/// Rebuild docks from a layout profile (mode / workbench switch).
///
/// Empty left/right/bottom slices remove that dock so modes like materials
/// and scripting match the design HTML (no Asset Browser / scene docks).
pub fn apply_workspace_layout(
    dock_area: &Entity<DockArea>,
    layout: &WorkspaceLayoutProfile,
    window: &mut Window,
    cx: &mut Context<'_, super::super::app::EditorView>,
) {
    apply_workspace_layout_state(dock_area, layout, None, "grid", None, window, cx);
}

pub fn apply_workspace_layout_for_mode(
    dock_area: &Entity<DockArea>,
    mode: &str,
    window: &mut Window,
    cx: &mut Context<'_, super::super::app::EditorView>,
) {
    let layout = super::layout::layout_profile_for_mode(mode);
    let persisted = crate::ui_state_persistence::layout_for_mode(cx, mode);
    let project = crate::ui_state_persistence::initial_project_state(cx);
    apply_workspace_layout_state(
        dock_area,
        layout,
        persisted.as_ref(),
        &project.asset_view_mode,
        project.asset_folder_key,
        window,
        cx,
    );
}

fn apply_workspace_layout_state(
    dock_area: &Entity<DockArea>,
    layout: &WorkspaceLayoutProfile,
    persisted: Option<&WorkspaceUiLayoutSnapshot>,
    asset_view_mode: &str,
    asset_folder_key: Option<String>,
    window: &Window,
    cx: &mut Context<'_, super::super::app::EditorView>,
) {
    window.defer(cx, {
        let dock_area = dock_area.clone();
        let layout_name = layout.name;
        let center = layout.center;
        let left = layout.left;
        let right = layout.right;
        let bottom = layout.bottom;
        let persisted = persisted.cloned();
        let asset_view_mode = asset_view_mode.to_owned();
        move |window, cx| {
            dock_area.update(cx, |this, cx| {
                let weak = dock_area.downgrade();
                let info = PanelInfo::tabs(0);
                let state = PanelState::default();

                this.set_bottom_dock_extent(BottomDockExtent::BoundedByRight, cx);

                apply_side_dock(
                    this,
                    SideDockSpec {
                        side: SideDock::Left,
                        placements: left,
                        dock_area: &weak,
                        persisted: persisted.as_ref(),
                        info: &info,
                    },
                    window,
                    cx,
                );

                this.set_center(
                    dock_tabs_for_workbenches(
                        center,
                        &weak,
                        &state,
                        &info,
                        window,
                        cx,
                        persisted
                            .as_ref()
                            .and_then(|state| state.center_active_panel.as_deref()),
                    ),
                    window,
                    cx,
                );

                apply_side_dock(
                    this,
                    SideDockSpec {
                        side: SideDock::Right,
                        placements: right,
                        dock_area: &weak,
                        persisted: persisted.as_ref(),
                        info: &info,
                    },
                    window,
                    cx,
                );

                apply_side_dock(
                    this,
                    SideDockSpec {
                        side: SideDock::Bottom,
                        placements: bottom,
                        dock_area: &weak,
                        persisted: persisted.as_ref(),
                        info: &info,
                    },
                    window,
                    cx,
                );

                tracing::debug!(layout = layout_name, "workspace dock layout applied");
            });
            restore_cached_asset_browser(&asset_view_mode, asset_folder_key, cx);
        }
    });
}

/// The three side docks a workspace layout installs. `WorkspaceDock` also names
/// the center, which is placed on its own and never through this path.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SideDock {
    Left,
    Right,
    Bottom,
}

impl SideDock {
    const fn placement(self) -> WorkspaceDock {
        match self {
            Self::Left => WorkspaceDock::Left,
            Self::Right => WorkspaceDock::Right,
            Self::Bottom => WorkspaceDock::Bottom,
        }
    }

    const fn region(self, snapshot: &WorkspaceUiLayoutSnapshot) -> Option<&PersistedDockRegion> {
        match self {
            Self::Left => snapshot.left.as_ref(),
            Self::Right => snapshot.right.as_ref(),
            Self::Bottom => snapshot.bottom.as_ref(),
        }
    }
}

/// One side dock's inputs: the panels the layout places there, plus the
/// persisted snapshot that overrides its size, open state, and active tab.
#[derive(Clone, Copy)]
struct SideDockSpec<'a> {
    side: SideDock,
    placements: &'a [PanelPlacement],
    dock_area: &'a gpui::WeakEntity<DockArea>,
    persisted: Option<&'a WorkspaceUiLayoutSnapshot>,
    info: &'a PanelInfo,
}

/// Installs one side dock from `spec`, or removes that dock when the layout
/// places no panels on the side.
fn apply_side_dock(
    area: &mut DockArea,
    spec: SideDockSpec<'_>,
    window: &mut Window,
    cx: &mut Context<'_, DockArea>,
) {
    if spec.placements.is_empty() {
        match spec.side {
            SideDock::Left => area.remove_left_dock(window, cx),
            SideDock::Right => area.remove_right_dock(window, cx),
            SideDock::Bottom => area.remove_bottom_dock(window, cx),
        }
        return;
    }

    let region = spec
        .persisted
        .and_then(|snapshot| spec.side.region(snapshot));
    let tabs = dock_tabs_for_panel_placements(
        spec,
        region.and_then(|region| region.active_panel.as_deref()),
        window,
        cx,
    );
    let size = region.map(|region| px(region.size));
    let open = region.map_or_else(
        || dock_visibility(spec.placements).is_open(),
        |region| region.open,
    );
    match spec.side {
        SideDock::Left => area.set_left_dock(tabs, size, open, window, cx),
        SideDock::Right => area.set_right_dock(tabs, size, open, window, cx),
        SideDock::Bottom => area.set_bottom_dock(tabs, size, open, window, cx),
    }
}

pub(crate) fn capture_workspace_layout(
    dock_area: &Entity<DockArea>,
    cx: &App,
) -> WorkspaceUiLayoutSnapshot {
    let dock_area = dock_area.read(cx);
    WorkspaceUiLayoutSnapshot {
        version: WORKSPACE_LAYOUT_VERSION,
        center_active_panel: active_panel_name(dock_area.center(), cx),
        left: dock_area.left_dock().map(|dock| dock_region(dock, cx)),
        right: dock_area.right_dock().map(|dock| dock_region(dock, cx)),
        bottom: dock_area.bottom_dock().map(|dock| dock_region(dock, cx)),
        panel_states: capture_cached_panel_states(cx),
    }
}

pub(crate) fn capture_cached_asset_browser(cx: &App) -> Option<(String, Option<String>)> {
    let panel = cx
        .try_global::<WorkspacePanelCache>()
        .and_then(|cache| cache.panels.get(az_editor_ui::panels::AssetBrowser::NAME))?;
    let Ok(panel) = panel
        .view()
        .downcast::<az_editor_ui::panels::AssetBrowser>()
    else {
        return None;
    };
    let panel = panel.read(cx);
    Some((
        panel.persisted_view_mode().to_owned(),
        panel.persisted_folder_key().map(ToOwned::to_owned),
    ))
}

/// Return the retained Asset Browser entity so the editor view can persist
/// panel-owned navigation state directly when it changes.
pub(crate) fn cached_asset_browser(cx: &App) -> Option<Entity<az_editor_ui::panels::AssetBrowser>> {
    cx.try_global::<WorkspacePanelCache>()
        .and_then(|cache| cache.panels.get(az_editor_ui::panels::AssetBrowser::NAME))
        .and_then(|panel| {
            panel
                .view()
                .downcast::<az_editor_ui::panels::AssetBrowser>()
                .ok()
        })
}

pub fn open_center_workbench(
    dock_area: &Entity<DockArea>,
    workbench: WorkbenchPlacement,
    window: &mut Window,
    cx: &mut Context<'_, super::super::app::EditorView>,
) {
    dock_area.update(cx, |this, cx| {
        let weak = cx.entity().downgrade();
        let info = PanelInfo::tabs(0);
        let state = PanelState::default();
        let panel = cached_panel(workbench.panel_name, weak, &state, &info, window, cx);
        this.add_panel(panel, DockPlacement::Center, None, window, cx);
    });
}

fn dock_tabs_for_workbenches(
    workbenches: &[WorkbenchPlacement],
    dock_area: &gpui::WeakEntity<DockArea>,
    state: &PanelState,
    info: &PanelInfo,
    window: &mut Window,
    cx: &mut gpui::App,
    persisted_active_panel: Option<&str>,
) -> DockItem {
    let active_index = persisted_active_panel
        .and_then(|active| {
            workbenches
                .iter()
                .position(|workbench| workbench.panel_name == active)
        })
        .or_else(|| {
            workbenches.iter().position(|workbench| {
                matches!(
                    workbench.kind,
                    WorkbenchKind::LevelViewport
                        | WorkbenchKind::Animation
                        | WorkbenchKind::Material
                        | WorkbenchKind::Script
                        | WorkbenchKind::GameData
                        | WorkbenchKind::Sequencer
                )
            })
        })
        .unwrap_or(0);
    let panels = workbenches
        .iter()
        .map(|workbench| {
            cached_panel(
                workbench.panel_name,
                dock_area.clone(),
                state,
                info,
                window,
                cx,
            )
        })
        .collect();

    DockItem::tabs(panels, dock_area, window, cx).active_index(active_index, cx)
}

fn dock_tabs_for_panel_placements(
    spec: SideDockSpec<'_>,
    persisted_active_panel: Option<&str>,
    window: &mut Window,
    cx: &mut gpui::App,
) -> DockItem {
    let SideDockSpec {
        side,
        placements,
        dock_area,
        persisted,
        info,
    } = spec;
    let expected_dock = side.placement();
    let persisted_panel_states = persisted.map(|snapshot| &snapshot.panel_states);
    let active_index = persisted_active_panel
        .and_then(|active| {
            placements
                .iter()
                .position(|placement| placement.panel_name == active)
        })
        .or_else(|| placements.iter().position(|placement| placement.active))
        .unwrap_or(0);
    let panels = placements
        .iter()
        .map(|placement| {
            debug_assert_eq!(placement.dock, expected_dock);
            let state = PanelState {
                panel_name: placement.panel_name.to_owned(),
                children: Vec::new(),
                info: PanelInfo::Panel(
                    persisted_panel_states
                        .and_then(|states| states.get(placement.panel_name))
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                ),
            };
            cached_panel(
                placement.panel_name,
                dock_area.clone(),
                &state,
                info,
                window,
                cx,
            )
        })
        .collect();

    DockItem::tabs(panels, dock_area, window, cx).active_index(active_index, cx)
}

fn capture_cached_panel_states(cx: &App) -> BTreeMap<String, serde_json::Value> {
    cx.try_global::<WorkspacePanelCache>()
        .map(|cache| {
            cache
                .panels
                .values()
                .filter_map(|panel| match panel.dump(cx).info {
                    PanelInfo::Panel(value) if !value.is_null() => {
                        Some((panel.panel_name(cx).to_owned(), value))
                    }
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn dock_region(dock: &Entity<gpui_component::dock::Dock>, cx: &App) -> PersistedDockRegion {
    let dock = dock.read(cx);
    PersistedDockRegion {
        size: f32::from(dock.size()),
        open: dock.is_open(),
        active_panel: active_panel_name(dock.panel(), cx),
    }
}

fn active_panel_name(item: &DockItem, cx: &App) -> Option<String> {
    match item {
        DockItem::Tabs {
            items, active_ix, ..
        } => items
            .get(*active_ix)
            .or_else(|| items.first())
            .map(|panel| panel.panel_name(cx).to_owned()),
        DockItem::Split { items, .. } => items.iter().find_map(|item| active_panel_name(item, cx)),
        DockItem::Panel { view, .. } => Some(view.panel_name(cx).to_owned()),
        DockItem::Tiles { .. } => None,
    }
}

fn restore_cached_asset_browser(view_mode: &str, folder_key: Option<String>, cx: &mut App) {
    let Some(panel) = cx
        .try_global::<WorkspacePanelCache>()
        .and_then(|cache| cache.panels.get(az_editor_ui::panels::AssetBrowser::NAME))
        .cloned()
    else {
        return;
    };
    let Ok(panel) = panel
        .view()
        .downcast::<az_editor_ui::panels::AssetBrowser>()
    else {
        return;
    };
    panel.update(cx, |panel, cx| {
        panel.restore_persisted_state(view_mode, folder_key, cx);
    });
}

pub(crate) fn restore_cached_asset_browser_from_project_state(cx: &mut App) {
    let project = crate::ui_state_persistence::initial_project_state(cx);
    restore_cached_asset_browser(&project.asset_view_mode, project.asset_folder_key, cx);
}

fn dock_visibility(placements: &[PanelPlacement]) -> PanelVisibility {
    placements
        .iter()
        .map(|placement| placement.visibility)
        .find(|visibility| visibility.is_open())
        .unwrap_or(PanelVisibility::Closed)
}

fn cached_panel(
    panel_name: &'static str,
    dock_area: gpui::WeakEntity<DockArea>,
    state: &PanelState,
    info: &PanelInfo,
    window: &mut Window,
    cx: &mut gpui::App,
) -> Arc<dyn PanelView> {
    if let Some(panel) = cx
        .try_global::<WorkspacePanelCache>()
        .and_then(|cache| cache.panels.get(panel_name))
        .cloned()
    {
        return panel;
    }

    let dock_events = dock_area.clone();
    let panel: Arc<dyn PanelView> = Arc::from(PanelRegistry::build_panel(
        panel_name, dock_area, state, info, window, cx,
    ));
    let panel_subscription = if panel_name == az_editor_ui::panels::AssetBrowser::NAME
        && let Ok(asset_browser) = panel
            .view()
            .downcast::<az_editor_ui::panels::AssetBrowser>()
    {
        Some(
            cx.subscribe(&asset_browser, move |_, event: &PanelEvent, cx| {
                if matches!(event, PanelEvent::LayoutChanged)
                    && let Some(dock_area) = dock_events.upgrade()
                {
                    dock_area.update(cx, |_, cx| cx.emit(DockEvent::LayoutChanged));
                }
            }),
        )
    } else if panel_name == az_editor_ui::panels::AuthoredInspector::NAME
        && let Ok(inspector) = panel
            .view()
            .downcast::<az_editor_ui::panels::AuthoredInspector>()
    {
        Some(cx.subscribe(&inspector, move |_, event: &PanelEvent, cx| {
            if matches!(event, PanelEvent::LayoutChanged)
                && let Some(dock_area) = dock_events.upgrade()
            {
                dock_area.update(cx, |_, cx| cx.emit(DockEvent::LayoutChanged));
            }
        }))
    } else {
        None
    };
    let cache = cx.default_global::<WorkspacePanelCache>();
    cache.panels.insert(panel_name, panel.clone());
    cache.subscriptions.extend(panel_subscription);
    panel
}
