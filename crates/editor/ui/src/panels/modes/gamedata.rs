//! Game Data mode: left catalog rail, center data workbench, right inspector.
//!
//! All three panels render [`EditorGameDataProjection`] published by
//! editor-core and dispatch typed `GameData*` actions; no catalog or
//! project-host types appear here. Layout follows the Aether editor design
//! (`design/aether-handoff/project/Aether Editor.dc.html`, Game Data region).

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, Hsla, InteractiveElement,
    IntoElement, ParentElement, Render, Rgba, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Window, div, px,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::dock::Panel;
use gpui_component::theme::Theme;
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, StyledExt, h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    v_flex,
};

use crate::panels::gamedata_projection::{
    EditorGameDataProjection, GameDataDiagnosticProjection, GameDataFieldDetailProjection,
    GameDataFieldKind, GameDataGridProjection, GameDataGridRowState, GameDataInspectorTab,
    GameDataManagerCompositionProjection, GameDataManagerGroupKind, GameDataRailView,
    GameDataSchemaEditorProjection, GameDataTone, GameDataWorkbenchProjection,
};
use crate::panels::kit;
use crate::panels::modes::mode_panel;

fn projection(cx: &App) -> EditorGameDataProjection {
    cx.try_global::<EditorGameDataProjection>()
        .cloned()
        .unwrap_or_default()
}

fn tone_color(tone: GameDataTone, theme: &Theme) -> Hsla {
    match tone {
        GameDataTone::Neutral => theme.muted_foreground,
        GameDataTone::Accent => theme.primary,
        GameDataTone::Info => theme.info,
        GameDataTone::Success => theme.success,
        GameDataTone::Warning => theme.warning,
        GameDataTone::Danger => theme.danger,
    }
}

fn kind_tint(kind: GameDataFieldKind, theme: &Theme) -> Hsla {
    tone_color(kind.tone(), theme)
}

/// Parse a data-authored color literal (`#rgb` / `#rrggbb` / `#rrggbbaa`)
/// carried in cell text. This is document content, not chrome theming.
fn parse_data_color(value: &str) -> Option<Hsla> {
    let value = value.trim();
    if !value.starts_with('#') {
        return None;
    }
    Rgba::try_from(value).ok().map(Hsla::from)
}

fn element_id(prefix: &str, key: &str) -> SharedString {
    SharedString::from(format!(
        "{prefix}-{}",
        key.replace(|c: char| !c.is_ascii_alphanumeric(), "-")
    ))
}

fn dispatch_on_click<A>(row: gpui::Stateful<gpui::Div>, action: A) -> gpui::Stateful<gpui::Div>
where
    A: gpui::Action + Clone + 'static,
{
    row.on_click(move |_, window, cx| {
        cx.stop_propagation();
        window.dispatch_action(Box::new(action.clone()), cx);
    })
}

// ---------------------------------------------------------------------------
// Left rail: catalog panel
// ---------------------------------------------------------------------------

fn render_gamedata_catalog(cx: &App) -> impl IntoElement {
    let theme = cx.theme().clone();
    let data = projection(cx);

    if !data.catalog_loaded {
        return kit::panel_shell(
            "Game Data",
            Some("tables · schemas · managers".to_owned()),
            kit::empty_state(
                "GameData catalog not published yet.",
                Some("waiting for project-host descriptor registrations".to_owned()),
                &theme,
            ),
            &theme,
        )
        .into_any_element();
    }

    let rail = data.rail_view;
    let body = v_flex()
        .gap_1p5()
        .child(render_gamedata_rail_chips(&data, rail, &theme))
        .child(match rail {
            GameDataRailView::Tables => render_tables_rail(&data, &theme).into_any_element(),
            GameDataRailView::Schemas => render_schemas_rail(&data, &theme).into_any_element(),
            GameDataRailView::Managers => render_managers_rail(&data, &theme).into_any_element(),
        })
        .when(!data.diagnostics.is_empty(), |this| {
            this.child(kit::section_label("Catalog diagnostics", &theme))
                .children(
                    data.diagnostics
                        .iter()
                        .map(|diagnostic| render_catalog_diagnostic(diagnostic, &theme)),
                )
        });

    kit::panel_shell(
        "Game Data",
        Some("tables · schemas · managers".to_owned()),
        body,
        &theme,
    )
    .into_any_element()
}

/// The Tables / Schemas / Managers rail selector chips, each carrying its
/// registered count and dispatching [`crate::actions::SetGameDataRailView`].
fn render_gamedata_rail_chips(
    data: &EditorGameDataProjection,
    rail: GameDataRailView,
    theme: &Theme,
) -> impl IntoElement {
    h_flex()
        .gap_1()
        .child(
            kit::filter_chip(
                "gd-rail-tables",
                "Tables",
                None,
                None,
                Some(data.table_count.to_string()),
                rail == GameDataRailView::Tables,
                theme,
            )
            .on_click(|_, window, cx| {
                cx.stop_propagation();
                window.dispatch_action(
                    Box::new(crate::actions::SetGameDataRailView {
                        view: GameDataRailView::Tables,
                    }),
                    cx,
                );
            }),
        )
        .child(
            kit::filter_chip(
                "gd-rail-schemas",
                "Schemas",
                None,
                None,
                Some(data.schema_count.to_string()),
                rail == GameDataRailView::Schemas,
                theme,
            )
            .on_click(|_, window, cx| {
                cx.stop_propagation();
                window.dispatch_action(
                    Box::new(crate::actions::SetGameDataRailView {
                        view: GameDataRailView::Schemas,
                    }),
                    cx,
                );
            }),
        )
        .child(
            kit::filter_chip(
                "gd-rail-managers",
                "Managers",
                None,
                None,
                Some(data.manager_count.to_string()),
                rail == GameDataRailView::Managers,
                theme,
            )
            .on_click(|_, window, cx| {
                cx.stop_propagation();
                window.dispatch_action(
                    Box::new(crate::actions::SetGameDataRailView {
                        view: GameDataRailView::Managers,
                    }),
                    cx,
                );
            }),
        )
}

/// One catalog diagnostic row: warning icon plus target and message.
fn render_catalog_diagnostic(
    diagnostic: &GameDataDiagnosticProjection,
    theme: &Theme,
) -> impl IntoElement {
    h_flex()
        .gap_1p5()
        .items_start()
        .child(
            Icon::new(IconName::TriangleAlert)
                .with_size(px(13.0))
                .text_color(theme.warning),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(10.5))
                .text_color(theme.muted_foreground)
                .child(format!(
                    "{} — {}",
                    diagnostic.target_label, diagnostic.message
                )),
        )
}

fn render_tables_rail(data: &EditorGameDataProjection, theme: &Theme) -> impl IntoElement {
    let mut body = v_flex().gap_0p5();
    if data.table_folders.is_empty() {
        return v_flex().child(kit::hint_text(
            "No GameData tables registered by project or engine gems.",
            theme,
        ));
    }
    for folder in &data.table_folders {
        body = body.child(
            h_flex()
                .h(px(kit::ROW_H))
                .items_center()
                .gap_1p5()
                .pr_2()
                .child(kit::row_caret(Some(true), theme))
                .child(kit::row_icon(
                    IconName::FolderClosed,
                    theme.muted_foreground,
                ))
                .child(kit::section_label(folder.name.clone(), theme))
                .child(div().flex_1())
                .child(kit::meta_text(folder.tables.len().to_string(), theme)),
        );
        for table in &folder.tables {
            body = body.child(dispatch_on_click(
                kit::list_row(element_id("gd-table", &table.key), theme, table.selected)
                    .pl(px(24.0))
                    .child(kit::row_icon(
                        IconName::GalleryVerticalEnd,
                        if table.selected {
                            theme.primary
                        } else {
                            theme.muted_foreground
                        },
                    ))
                    .child(kit::row_name(table.name.clone(), theme))
                    .child(kit::meta_text(table.count_label.clone(), theme)),
                crate::actions::SelectGameDataTable {
                    table_key: table.key.clone(),
                },
            ));
        }
    }
    body
}

fn render_schemas_rail(data: &EditorGameDataProjection, theme: &Theme) -> impl IntoElement {
    if data.schemas.is_empty() {
        return v_flex().child(kit::hint_text(
            "No row schemas referenced by GameData tables.",
            theme,
        ));
    }
    v_flex()
        .gap_0p5()
        .children(data.schemas.iter().map(|schema| {
            dispatch_on_click(
                kit::list_row(element_id("gd-schema", &schema.key), theme, schema.selected)
                    .pl_2()
                    .child(kit::row_icon(
                        IconName::LayoutDashboard,
                        if schema.selected {
                            theme.primary
                        } else if schema.resolved {
                            theme.muted_foreground
                        } else {
                            theme.warning
                        },
                    ))
                    .child(kit::row_name(schema.label.clone(), theme))
                    .child(kit::meta_text(schema.used_by_label.clone(), theme)),
                crate::actions::SelectGameDataSchema {
                    schema_key: schema.key.clone(),
                },
            )
        }))
}

fn render_managers_rail(data: &EditorGameDataProjection, theme: &Theme) -> impl IntoElement {
    let mut body = v_flex().gap_0p5();
    if data.manager_groups.is_empty() {
        return v_flex().child(kit::hint_text(
            "No manager descriptors in the resolved catalog.",
            theme,
        ));
    }
    for group in &data.manager_groups {
        let head_icon = match group.kind {
            GameDataManagerGroupKind::AutomaticProviders => IconName::SquareTerminal,
            GameDataManagerGroupKind::OwnerGem => IconName::FolderClosed,
        };
        body = body.child(
            h_flex()
                .h(px(kit::ROW_H))
                .items_center()
                .gap_1p5()
                .pr_2()
                .child(kit::row_caret(Some(true), theme))
                .child(kit::row_icon(head_icon, theme.muted_foreground))
                .child(kit::section_label(group.name.clone(), theme))
                .child(div().flex_1())
                .child(kit::meta_text(group.managers.len().to_string(), theme)),
        );
        for manager in &group.managers {
            let dot = if manager.has_diagnostics {
                theme.warning
            } else if manager.read_only {
                theme.muted_foreground
            } else {
                theme.primary
            };
            body = body.child(dispatch_on_click(
                kit::list_row(
                    element_id("gd-manager", &manager.key),
                    theme,
                    manager.selected,
                )
                .pl(px(24.0))
                .child(kit::status_dot(dot))
                .child(kit::row_name(manager.name.clone(), theme))
                .child(kit::tag_chip(
                    manager.kind_label.clone(),
                    if manager.read_only {
                        theme.muted_foreground
                    } else {
                        theme.primary
                    },
                )),
                crate::actions::SelectGameDataManager {
                    manager_key: manager.key.clone(),
                },
            ));
        }
    }
    body
}

// ---------------------------------------------------------------------------
// Center: data workbench panel
// ---------------------------------------------------------------------------

/// Center Game Data workbench. Hand-written (not `mode_panel!`) because it
/// owns the grid filter `InputState`.
pub struct GameDataWorkbenchPanel {
    focus_handle: FocusHandle,
    filter_input: Entity<InputState>,
    /// Last filter value pushed into the input, used to resync when the
    /// projection filter changes from outside.
    last_projected_filter: String,
    _subscriptions: Vec<Subscription>,
}

impl GameDataWorkbenchPanel {
    pub const NAME: &'static str = "game-data";

    pub fn init(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let filter_input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter rows…"));
        let subscriptions = vec![cx.subscribe_in(
            &filter_input,
            window,
            |_this: &mut Self, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    let filter = input.read(cx).value().to_string();
                    window.dispatch_action(
                        Box::new(crate::actions::SetGameDataGridFilter { filter }),
                        cx,
                    );
                }
            },
        )];
        Self {
            focus_handle: cx.focus_handle(),
            filter_input,
            last_projected_filter: String::new(),
            _subscriptions: subscriptions,
        }
    }
}

impl Render for GameDataWorkbenchPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if let Some(placeholder) =
            crate::panels::render_project_host_connection_placeholder("Game Data", cx)
        {
            return placeholder;
        }
        let theme = cx.theme().clone();
        let data = projection(cx);

        // Keep the locally-owned input in sync when the projection filter was
        // reset elsewhere (e.g. table switch clears the filter).
        if let GameDataWorkbenchProjection::Grid(grid) = &data.workbench
            && grid.filter != self.last_projected_filter
        {
            self.last_projected_filter.clone_from(&grid.filter);
            let value = grid.filter.clone();
            self.filter_input.update(cx, |state, cx| {
                if state.value() != value.as_str() {
                    state.set_value(value, window, cx);
                }
            });
        }

        match &data.workbench {
            GameDataWorkbenchProjection::Empty { message } => kit::workbench_shell(
                "Game Data",
                "catalog left · inspector right",
                kit::empty_state(message.clone(), None, &theme),
                &theme,
            )
            .into_any_element(),
            GameDataWorkbenchProjection::Grid(grid) => {
                render_grid_workbench(grid, &self.filter_input, &theme).into_any_element()
            }
            GameDataWorkbenchProjection::Schema(schema) => {
                render_schema_workbench(schema, &theme).into_any_element()
            }
            GameDataWorkbenchProjection::Manager(manager) => {
                render_manager_workbench(manager, &theme).into_any_element()
            }
        }
    }
}

impl Focusable for GameDataWorkbenchPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for GameDataWorkbenchPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
        kit::tab_title(Some("table_chart"), "Game Data", kit::TabTone::Default)
    }

    fn inner_padding(&self, _cx: &App) -> bool {
        false
    }
}

impl gpui::EventEmitter<gpui_component::dock::PanelEvent> for GameDataWorkbenchPanel {}

fn render_grid_workbench(
    grid: &GameDataGridProjection,
    filter_input: &Entity<InputState>,
    theme: &Theme,
) -> impl IntoElement {
    let field_count = grid.columns.iter().filter(|column| !column.primary).count();
    v_flex()
        .size_full()
        .bg(theme.background)
        .child(render_grid_toolbar(grid, filter_input, theme))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_x_scrollbar()
                .child(render_grid_table(grid, theme)),
        )
        .child(render_grid_footer(grid, field_count, theme))
}

/// Grid toolbar: table title, the schema chip that jumps to the schema, and
/// the row filter input.
fn render_grid_toolbar(
    grid: &GameDataGridProjection,
    filter_input: &Entity<InputState>,
    theme: &Theme,
) -> impl IntoElement {
    kit::panel_toolbar(theme)
        .child(kit::panel_title(grid.table_name.clone(), theme))
        .child(dispatch_on_click(
            kit::filter_chip(
                "gd-grid-schema-chip",
                grid.schema_label.clone(),
                Some(IconName::LayoutDashboard),
                Some(theme.info),
                None,
                false,
                theme,
            ),
            crate::actions::SelectGameDataSchema {
                schema_key: grid.schema_key.clone(),
            },
        ))
        .child(div().flex_1())
        .child(
            Button::new("gd-undo")
                .ghost()
                .small()
                .label("Undo")
                .disabled(!grid.can_undo)
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(crate::actions::UndoGameDataRows {}), cx);
                }),
        )
        .child(
            Button::new("gd-redo")
                .ghost()
                .small()
                .label("Redo")
                .disabled(!grid.can_redo)
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(crate::actions::RedoGameDataRows {}), cx);
                }),
        )
        .child(
            Button::new("gd-close")
                .ghost()
                .small()
                .label("Close")
                .disabled(!grid.can_edit)
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(crate::actions::CloseGameDataTable {}), cx);
                }),
        )
        .child(
            h_flex().w(px(180.0)).child(kit::input_field(
                Input::new(filter_input)
                    .small()
                    .appearance(false)
                    .bordered(false)
                    .focus_bordered(false),
                theme,
            )),
        )
}

/// Grid footer: primary-key label, row and field counts, and the schema
/// validity summary.
fn render_grid_footer(
    grid: &GameDataGridProjection,
    field_count: usize,
    theme: &Theme,
) -> impl IntoElement {
    kit::count_footer(theme)
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .child(kit::status_dot(theme.warning))
                .child(grid.primary_key_label.clone()),
        )
        .child(format!("{} rows", grid.total_row_count))
        .child(format!("{field_count} fields"))
        .child(div().flex_1())
        .child(if grid.diagnostics.is_empty() {
            h_flex()
                .gap_1()
                .items_center()
                .text_color(theme.success)
                .child(kit::status_dot(theme.success))
                .child("schema valid")
        } else {
            h_flex()
                .gap_1()
                .items_center()
                .text_color(theme.warning)
                .child(kit::status_dot(theme.warning))
                .child(format!("{} diagnostics", grid.diagnostics.len()))
        })
}

fn render_grid_table(grid: &GameDataGridProjection, theme: &Theme) -> impl IntoElement {
    let mut table = v_flex().min_w(px(grid_min_width(grid)));

    table = table.child(render_grid_header_row(grid, theme));
    table = extend_with_grid_body_rows(table, grid, theme);
    table
}

/// Grid header row: one sortable, selectable cell per column, carrying the
/// column label over its type tag.
fn render_grid_header_row(grid: &GameDataGridProjection, theme: &Theme) -> impl IntoElement {
    let mut header = h_flex()
        .flex_none()
        .bg(theme.tab_bar)
        .border_b_1()
        .border_color(theme.border);
    for column in &grid.columns {
        let tint = kind_tint(column.kind, theme);
        header = header.child(dispatch_on_click(
            v_flex()
                .id(element_id("gd-col", &column.key))
                .w(px(f32::from(u16::try_from(column.width).unwrap_or(132))))
                .flex_none()
                .px_2()
                .py_1()
                .gap_0p5()
                .border_r_1()
                .border_color(theme.border)
                .cursor_pointer()
                .when(column.selected, |this| this.bg(theme.list_active))
                .hover(|this| this.bg(theme.list_hover))
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .when(column.primary, |this| {
                            this.child(kit::status_dot(theme.warning))
                        })
                        .child(
                            div()
                                .text_size(px(11.0))
                                .font_semibold()
                                .text_color(theme.foreground)
                                .truncate()
                                .child(column.label.clone()),
                        ),
                )
                .child(
                    div()
                        .text_size(px(9.0))
                        .text_color(tint)
                        .child(column.type_label.to_uppercase()),
                ),
            crate::actions::SelectGameDataField {
                field_key: column.key.clone(),
            },
        ));
    }
    header
}

/// Grid body: the loading / error / empty states, one row per record, and
/// the Add row action.
fn extend_with_grid_body_rows(
    mut table: gpui::Div,
    grid: &GameDataGridProjection,
    theme: &Theme,
) -> gpui::Div {
    match grid.row_state {
        GameDataGridRowState::Loading => {
            table = table.child(div().p_4().child(kit::hint_text(
                format!("Loading rows from {} …", grid.document_id),
                theme,
            )));
        }
        GameDataGridRowState::Error => {
            table = table.child(div().p_4().child(kit::hint_text(
                grid.status_detail.clone().unwrap_or_else(|| {
                    "Unable to load the selected GameData source. Select the table again after resolving the service error.".to_owned()
                }),
                theme,
            )));
        }
        GameDataGridRowState::Loaded => {
            if grid.rows.is_empty() {
                let message = if grid.filter.trim().is_empty() {
                    "Table has no rows.".to_owned()
                } else {
                    format!("No rows match \"{}\".", grid.filter.trim())
                };
                table = table.child(div().p_4().child(kit::hint_text(message, theme)));
            }
            for row in &grid.rows {
                let mut body_row = h_flex()
                    .flex_none()
                    .border_b_1()
                    .border_color(theme.border.opacity(0.5))
                    .hover(|this| this.bg(theme.list_hover));
                for cell in &row.cells {
                    let width = grid
                        .columns
                        .iter()
                        .find(|column| column.key == cell.column_key)
                        .map_or(132, |column| column.width);
                    body_row = body_row.child(
                        div()
                            .w(px(f32::from(u16::try_from(width).unwrap_or(132))))
                            .flex_none()
                            .px_2()
                            .py_1()
                            .border_r_1()
                            .border_color(theme.border.opacity(0.5))
                            .overflow_hidden()
                            .child(render_grid_cell(cell, theme)),
                    );
                }
                let duplicate_id = row.object_id.clone();
                let remove_id = row.object_id.clone();
                body_row = body_row
                    .child(
                        Button::new(element_id("gd-duplicate", &row.object_id))
                            .ghost()
                            .small()
                            .icon(IconName::Copy)
                            .disabled(!grid.can_edit)
                            .tooltip("Duplicate row")
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(
                                    Box::new(crate::actions::DuplicateGameDataRow {
                                        object_id: duplicate_id.clone(),
                                    }),
                                    cx,
                                );
                            }),
                    )
                    .child(
                        Button::new(element_id("gd-remove", &row.object_id))
                            .ghost()
                            .small()
                            .icon(IconName::Minus)
                            .disabled(!grid.can_edit)
                            .tooltip("Remove row")
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(
                                    Box::new(crate::actions::RemoveGameDataRow {
                                        object_id: remove_id.clone(),
                                    }),
                                    cx,
                                );
                            }),
                    );
                table = table.child(body_row);
            }
            table = table.child(
                Button::new("gd-add-row")
                    .ghost()
                    .small()
                    .icon(IconName::Plus)
                    .label("Add row")
                    .disabled(!grid.can_edit)
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(crate::actions::AddGameDataRow {}), cx);
                    }),
            );
        }
    }
    table
}

fn grid_min_width(grid: &GameDataGridProjection) -> f32 {
    grid.columns
        .iter()
        .map(|column| f32::from(u16::try_from(column.width).unwrap_or(132)))
        .sum::<f32>()
        .max(320.0)
}

fn render_grid_cell(
    cell: &crate::panels::gamedata_projection::GameDataGridCellProjection,
    theme: &Theme,
) -> gpui::AnyElement {
    match cell.kind {
        GameDataFieldKind::Key => div()
            .font_family(theme.mono_font_family.clone())
            .text_size(px(11.0))
            .text_color(theme.warning)
            .truncate()
            .child(cell.text.clone())
            .into_any_element(),
        GameDataFieldKind::Number => div()
            .font_family(theme.mono_font_family.clone())
            .text_size(px(11.0))
            .text_color(theme.foreground)
            .truncate()
            .child(cell.text.clone())
            .into_any_element(),
        GameDataFieldKind::Boolean => {
            let on = cell.bool_value.unwrap_or(false);
            h_flex()
                .gap_1()
                .items_center()
                .child(kit::status_dot(if on {
                    theme.success
                } else {
                    theme.muted_foreground.opacity(0.5)
                }))
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(theme.muted_foreground)
                        .child(if on { "true" } else { "false" }),
                )
                .into_any_element()
        }
        GameDataFieldKind::Reference | GameDataFieldKind::Object => h_flex()
            .gap_1()
            .items_center()
            .child(
                Icon::new(IconName::Replace)
                    .with_size(px(12.0))
                    .text_color(theme.primary),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(theme.foreground)
                    .truncate()
                    .child(cell.text.clone()),
            )
            .into_any_element(),
        GameDataFieldKind::Asset => h_flex()
            .gap_1()
            .items_center()
            .child(
                Icon::new(IconName::File)
                    .with_size(px(12.0))
                    .text_color(theme.info),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(theme.foreground)
                    .truncate()
                    .child(cell.text.clone()),
            )
            .into_any_element(),
        GameDataFieldKind::Color => {
            let swatch = parse_data_color(&cell.text);
            h_flex()
                .gap_1p5()
                .items_center()
                .when_some(swatch, |this, color| {
                    this.child(
                        div()
                            .size(px(12.0))
                            .rounded(px(3.0))
                            .border_1()
                            .border_color(theme.border)
                            .bg(color),
                    )
                })
                .child(
                    div()
                        .font_family(theme.mono_font_family.clone())
                        .text_size(px(10.5))
                        .text_color(theme.muted_foreground)
                        .child(cell.text.clone()),
                )
                .into_any_element()
        }
        GameDataFieldKind::Text | GameDataFieldKind::Unset => div()
            .text_size(px(11.0))
            .text_color(if cell.kind == GameDataFieldKind::Unset {
                theme.muted_foreground
            } else {
                theme.foreground
            })
            .truncate()
            .child(cell.text.clone())
            .into_any_element(),
    }
}

fn render_schema_workbench(
    schema: &GameDataSchemaEditorProjection,
    theme: &Theme,
) -> impl IntoElement {
    let mut body = v_flex().gap_2p5();
    if let Some(rust_type) = &schema.rust_type {
        body = body.child(kit::status_row("Rust row type", rust_type.clone(), theme));
    }
    if let Some(description) = &schema.description {
        body = body.child(kit::hint_text(description.clone(), theme));
    }
    if !schema.resolved {
        body = body.child(kit::hint_text(
            "Schema key is referenced by GameData tables but is not present in the published schema catalog.",
            theme,
        ));
    }

    body = body.child(kit::section_label("Fields", theme));
    if schema.fields.is_empty() {
        body = body.child(kit::hint_text("No visible fields.", theme));
    } else {
        let mut list = v_flex()
            .border_1()
            .border_color(theme.border)
            .rounded(px(6.0))
            .overflow_hidden();
        for field in &schema.fields {
            let tint = kind_tint(field.kind, theme);
            list = list.child(
                h_flex()
                    .h(px(30.0))
                    .items_center()
                    .gap_2()
                    .px_2()
                    .border_b_1()
                    .border_color(theme.border.opacity(0.5))
                    .child(kit::status_dot(tint))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(11.5))
                            .text_color(theme.foreground)
                            .truncate()
                            .child(field.label.clone()),
                    )
                    .child(
                        div()
                            .font_family(theme.mono_font_family.clone())
                            .text_size(px(10.0))
                            .text_color(theme.muted_foreground)
                            .child(field.key.clone()),
                    )
                    .child(kit::tag_chip(field.type_label.clone(), tint))
                    .when(field.read_only, |this| {
                        this.child(kit::tag_chip("read-only", theme.muted_foreground))
                    }),
            );
        }
        body = body.child(list);
    }

    body = body.child(kit::section_label("Tables using this schema", theme));
    if schema.used_by.is_empty() {
        body = body.child(kit::hint_text("No tables use this schema.", theme));
    } else {
        for usage in &schema.used_by {
            body = body.child(dispatch_on_click(
                kit::list_row(element_id("gd-schema-used", &usage.table_key), theme, false)
                    .pl_2()
                    .child(kit::row_icon(
                        IconName::GalleryVerticalEnd,
                        theme.muted_foreground,
                    ))
                    .child(kit::row_name(usage.name.clone(), theme))
                    .child(kit::meta_text(
                        format!("{} · {}", usage.category, usage.count_label),
                        theme,
                    )),
                crate::actions::SelectGameDataTable {
                    table_key: usage.table_key.clone(),
                },
            ));
        }
    }

    kit::workbench_shell(
        schema.label.clone(),
        "row schema · shared by tables",
        body,
        theme,
    )
}

fn render_manager_workbench(
    manager: &GameDataManagerCompositionProjection,
    theme: &Theme,
) -> impl IntoElement {
    let status_tint = if manager.status_ok {
        theme.success
    } else {
        theme.warning
    };

    let mut body = v_flex().gap_2p5();
    body = body.child(
        h_flex()
            .gap_1p5()
            .items_center()
            .child(kit::tag_chip(
                manager.kind_label.clone(),
                if manager.read_only {
                    theme.muted_foreground
                } else {
                    theme.primary
                },
            ))
            .when(manager.read_only, |this| {
                this.child(kit::tag_chip("auto", theme.info))
            })
            .child(kit::meta_text(manager.owner.clone(), theme))
            .child(kit::meta_text(manager.row_type.clone(), theme))
            .child(div().flex_1())
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(kit::status_dot(status_tint))
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(status_tint)
                            .child(manager.status_label.clone()),
                    ),
            ),
    );
    body = body.child(kit::hint_text(manager.summary.clone(), theme));

    body = extend_with_manager_pipeline(body, manager, theme);
    body = extend_with_manager_resolution(body, manager, theme);
    body = extend_with_manager_key_policy(body, manager, theme);
    body = extend_with_manager_validation(body, manager, theme);
    body = extend_with_manager_descriptor(body, manager, theme);

    kit::workbench_shell(
        manager.name.clone(),
        format!("{} · {}", manager.kind_label, manager.owner),
        div().w_full().overflow_y_scrollbar().child(body),
        theme,
    )
}

/// Access pipeline section: one chip per stage the manager routes through.
fn extend_with_manager_pipeline(
    mut body: gpui::Div,
    manager: &GameDataManagerCompositionProjection,
    theme: &Theme,
) -> gpui::Div {
    // Access pipeline.
    body = body.child(kit::section_label("Access pipeline", theme));
    let mut pipeline = h_flex().gap_1().items_stretch().flex_wrap();
    for (index, stage) in manager.stages.iter().enumerate() {
        let tint = tone_color(stage.tone, theme);
        if index > 0 {
            pipeline = pipeline.child(
                div().flex_none().flex().items_center().child(
                    Icon::new(IconName::ChevronRight)
                        .with_size(px(14.0))
                        .text_color(theme.muted_foreground),
                ),
            );
        }
        pipeline = pipeline.child(
            v_flex()
                .flex_1()
                .min_w(px(96.0))
                .gap_0p5()
                .p_2()
                .rounded(px(6.0))
                .border_1()
                .border_color(tint.opacity(0.4))
                .bg(tint.opacity(0.07))
                .child(kit::section_label(stage.title.clone(), theme))
                .child(
                    div()
                        .font_family(theme.mono_font_family.clone())
                        .text_size(px(10.5))
                        .text_color(theme.foreground)
                        .truncate()
                        .child(stage.detail.clone()),
                ),
        );
    }
    body = body.child(pipeline);
    body = body.child(kit::hint_text(
        format!(
            "Game logic calls {}::get(key) and receives a {} — it never reaches into the underlying tables.",
            manager.name, manager.output_type
        ),
        theme,
    ));
    body
}

/// Sources and resolution sections: where the manager's rows come from and
/// how conflicts between them resolve.
fn extend_with_manager_resolution(
    mut body: gpui::Div,
    manager: &GameDataManagerCompositionProjection,
    theme: &Theme,
) -> gpui::Div {
    // Sources.
    body = body.child(
        h_flex()
            .items_center()
            .gap_1p5()
            .child(kit::section_label("Sources", theme))
            .child(div().flex_1())
            .child(kit::meta_text(manager.sources.len().to_string(), theme)),
    );
    if manager.sources.is_empty() {
        body = body.child(kit::hint_text("No declared source inputs.", theme));
    } else {
        for source in &manager.sources {
            let row = kit::list_row(element_id("gd-mgr-src", &source.key), theme, false)
                .pl_2()
                .child(kit::row_icon(
                    if source.manager_key.is_some() {
                        IconName::SquareTerminal
                    } else {
                        IconName::GalleryVerticalEnd
                    },
                    theme.info,
                ))
                .child(kit::row_name(source.name.clone(), theme))
                .child(kit::meta_text(
                    format!("{} · {}", source.kind, source.detail),
                    theme,
                ));
            body = body.child(if let Some(table_key) = &source.table_key {
                dispatch_on_click(
                    row,
                    crate::actions::SelectGameDataTable {
                        table_key: table_key.clone(),
                    },
                )
            } else if let Some(manager_key) = &source.manager_key {
                dispatch_on_click(
                    row,
                    crate::actions::SelectGameDataManager {
                        manager_key: manager_key.clone(),
                    },
                )
            } else {
                row
            });
        }
    }

    extend_with_manager_projection_sources(body, manager, theme)
}

/// Projection sub-section of the manager workbench: the row type the
/// composition projects into and the fields that reach it.
fn extend_with_manager_projection_sources(
    mut body: gpui::Div,
    manager: &GameDataManagerCompositionProjection,
    theme: &Theme,
) -> gpui::Div {
    body = body.child(
        h_flex()
            .items_center()
            .gap_1p5()
            .child(kit::section_label(
                format!("Projection → {}", manager.output_type),
                theme,
            ))
            .child(div().flex_1())
            .child(kit::meta_text(manager.projection_count.to_string(), theme)),
    );
    if manager.projection_rows.is_empty() {
        body = body.child(kit::hint_text("Implicit identity row projection.", theme));
    } else {
        let mut list = v_flex()
            .border_1()
            .border_color(theme.border)
            .rounded(px(6.0))
            .overflow_hidden();
        for row in &manager.projection_rows {
            list = list.child(
                h_flex()
                    .h(px(28.0))
                    .items_center()
                    .gap_2()
                    .px_2()
                    .border_b_1()
                    .border_color(theme.border.opacity(0.5))
                    .child(
                        div()
                            .w(px(140.0))
                            .flex_none()
                            .font_family(theme.mono_font_family.clone())
                            .text_size(px(10.5))
                            .text_color(theme.foreground)
                            .truncate()
                            .child(row.output_field.clone()),
                    )
                    .child(
                        Icon::new(IconName::ChevronRight)
                            .with_size(px(12.0))
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(theme.mono_font_family.clone())
                            .text_size(px(10.0))
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(row.source_label.clone()),
                    )
                    .when(row.computed, |this| {
                        this.child(kit::tag_chip("computed", theme.primary))
                    }),
            );
        }
        body = body.child(list);
    }
    body
}

/// Key policy section: the key rule, duplicate handling, and active filters.
fn extend_with_manager_key_policy(
    mut body: gpui::Div,
    manager: &GameDataManagerCompositionProjection,
    theme: &Theme,
) -> gpui::Div {
    // Key policy / duplicates / filters.
    body = body.child(kit::section_label("Key policy", theme));
    body = body.child(kit::status_row(
        "Lowering",
        manager.key_policy_summary.clone(),
        theme,
    ));
    body = body.child(kit::status_row(
        "Duplicate keys",
        manager.duplicate_key_policy.clone(),
        theme,
    ));
    if !manager.transform_chips.is_empty() {
        body = body.child(
            h_flex().gap_1().flex_wrap().children(
                manager
                    .transform_chips
                    .iter()
                    .map(|label| kit::tag_chip(label.clone(), theme.primary)),
            ),
        );
    }
    if !manager.row_filters.is_empty() {
        body = body.child(kit::section_label("Row filters", theme));
        for filter in &manager.row_filters {
            body = body.child(kit::status_row(
                filter.field.clone(),
                if filter.compare_field.is_empty() {
                    filter.predicate.clone()
                } else {
                    format!("{} {}", filter.predicate, filter.compare_field)
                },
                theme,
            ));
        }
    }

    // Consumers.
    body = body.child(
        h_flex()
            .items_center()
            .gap_1p5()
            .child(kit::section_label("Accessed by", theme))
            .child(div().flex_1())
            .child(kit::meta_text(manager.consumers.len().to_string(), theme)),
    );
    if manager.consumers.is_empty() {
        body = body.child(kit::hint_text(
            "No downstream managers depend on this projection.",
            theme,
        ));
    } else {
        body = body.child(
            h_flex()
                .gap_1()
                .flex_wrap()
                .children(manager.consumers.iter().map(|chip| {
                    dispatch_on_click(
                        kit::filter_chip(
                            element_id("gd-mgr-consumer", &chip.key),
                            chip.label.clone(),
                            Some(IconName::SquareTerminal),
                            Some(if chip.provider {
                                theme.muted_foreground
                            } else {
                                theme.primary
                            }),
                            None,
                            false,
                            theme,
                        ),
                        crate::actions::SelectGameDataManager {
                            manager_key: chip.key.clone(),
                        },
                    )
                })),
        );
    }
    body
}

/// Validation section: the diagnostics the project host published for this
/// manager.
fn extend_with_manager_validation(
    mut body: gpui::Div,
    manager: &GameDataManagerCompositionProjection,
    theme: &Theme,
) -> gpui::Div {
    // Validation.
    body = body.child(kit::section_label("Validation", theme));
    for row in &manager.validation {
        let tint = if row.ok { theme.success } else { theme.warning };
        body = body.child(
            h_flex()
                .gap_1p5()
                .items_start()
                .child(kit::status_dot(tint))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(10.5))
                        .text_color(theme.muted_foreground)
                        .child(if row.target_label.is_empty() {
                            row.message.clone()
                        } else {
                            format!("{} — {}", row.target_label, row.message)
                        }),
                ),
        );
    }

    // Descriptor source (service gap: project-host publishes resolved
    body
}

/// Generated descriptor section. Service gap: project-host publishes resolved
/// descriptors, not registration source text.
fn extend_with_manager_descriptor(
    mut body: gpui::Div,
    manager: &GameDataManagerCompositionProjection,
    theme: &Theme,
) -> gpui::Div {
    body = body.child(kit::section_label("Generated descriptor", theme));
    body = body.child(
        div()
            .p_2p5()
            .rounded(px(6.0))
            .border_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .child(kit::hint_text(manager.descriptor_note.clone(), theme)),
    );
    body
}

// ---------------------------------------------------------------------------
// Right rail: details / inspector panel
// ---------------------------------------------------------------------------

const fn inspector_tab_label(tab: GameDataInspectorTab) -> &'static str {
    match tab {
        GameDataInspectorTab::Field => "Field",
        GameDataInspectorTab::Table => "Table",
        GameDataInspectorTab::Schema => "Schema",
        GameDataInspectorTab::Manager => "Manager",
    }
}

fn render_gamedata_details(cx: &App) -> impl IntoElement {
    let theme = cx.theme().clone();
    let data = projection(cx);

    if !data.catalog_loaded {
        return kit::panel_shell(
            "Details",
            Some("field · table · schema · manager".to_owned()),
            kit::empty_state("GameData catalog not published yet.", None, &theme),
            &theme,
        )
        .into_any_element();
    }

    let tabs = h_flex()
        .gap_1()
        .children(data.inspector.tabs.iter().map(|tab| {
            let tab = *tab;
            kit::filter_chip(
                element_id("gd-ins-tab", inspector_tab_label(tab)),
                inspector_tab_label(tab),
                None,
                None,
                None,
                data.inspector.tab == tab,
                &theme,
            )
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                window.dispatch_action(
                    Box::new(crate::actions::SetGameDataInspectorTab { tab }),
                    cx,
                );
            })
        }));

    let content = match data.inspector.tab {
        GameDataInspectorTab::Field => render_field_inspector(&data, &theme),
        GameDataInspectorTab::Table => render_table_inspector(&data, &theme),
        GameDataInspectorTab::Schema => render_schema_inspector(&data, &theme),
        GameDataInspectorTab::Manager => render_manager_inspector(&data, &theme),
    };

    kit::panel_shell(
        "Details",
        None,
        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .gap_2()
            .child(tabs)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .child(content),
            ),
        &theme,
    )
    .into_any_element()
}

const fn grid_of(data: &EditorGameDataProjection) -> Option<&GameDataGridProjection> {
    match &data.workbench {
        GameDataWorkbenchProjection::Grid(grid) => Some(grid),
        _ => None,
    }
}

fn render_field_inspector(data: &EditorGameDataProjection, theme: &Theme) -> gpui::AnyElement {
    let Some(field) = grid_of(data).and_then(|grid| grid.selected_field.as_ref()) else {
        return kit::empty_state("Select a field in the data grid header.", None, theme)
            .into_any_element();
    };
    render_field_detail(field, theme).into_any_element()
}

fn render_field_detail(field: &GameDataFieldDetailProjection, theme: &Theme) -> impl IntoElement {
    let tint = kind_tint(field.kind, theme);
    let mut body = v_flex()
        .gap_2()
        .child(kit::section_label("Field", theme))
        .child(
            div()
                .text_size(px(13.0))
                .font_semibold()
                .text_color(theme.foreground)
                .child(field.label.clone()),
        )
        .child(
            div()
                .font_family(theme.mono_font_family.clone())
                .text_size(px(10.5))
                .text_color(theme.muted_foreground)
                .child(field.key.clone()),
        )
        .child(
            h_flex()
                .gap_1p5()
                .items_center()
                .child(kit::tag_chip(field.type_label.clone(), tint))
                .when(field.read_only, |this| {
                    this.child(kit::tag_chip("read-only", theme.muted_foreground))
                }),
        )
        .child(kit::status_row("Schema", field.schema_label.clone(), theme));

    if field.shared_table_count > 1 {
        body = body.child(kit::hint_text(
            format!(
                "This field lives on the {} schema — it is shared by {} tables.",
                field.schema_label, field.shared_table_count
            ),
            theme,
        ));
    }
    if let Some(value) = &field.value_preview {
        body = body.child(kit::status_row(
            "Value (selected row)",
            value.clone(),
            theme,
        ));
    }
    body = body.child(kit::section_label("Description", theme));
    body = body.child(field.description.as_ref().map_or_else(
        || kit::hint_text("No schema description.", theme),
        |description| kit::hint_text(description.clone(), theme),
    ));
    // Service gap: reflected field descriptors do not publish defaults or
    // required/unique constraints yet.
    body = body.child(kit::hint_text(
        "Defaults and required/unique constraints are not published by the schema catalog yet.",
        theme,
    ));
    body
}

fn render_table_inspector(data: &EditorGameDataProjection, theme: &Theme) -> gpui::AnyElement {
    let Some(grid) = grid_of(data) else {
        return kit::empty_state("Select a table in the Game Data catalog.", None, theme)
            .into_any_element();
    };
    let field_count = grid.columns.iter().filter(|column| !column.primary).count();
    let mut body = v_flex()
        .gap_2()
        .child(
            div()
                .text_size(px(13.0))
                .font_semibold()
                .text_color(theme.foreground)
                .child(grid.table_name.clone()),
        )
        .child(kit::status_row(
            "Key",
            grid.primary_key_label.clone(),
            theme,
        ))
        .child(kit::status_row(
            "Rows",
            grid.total_row_count.to_string(),
            theme,
        ))
        .child(kit::status_row("Fields", field_count.to_string(), theme))
        .child(kit::status_row("Category", grid.category.clone(), theme))
        .child(kit::status_row("Owner", grid.owner.clone(), theme))
        .child(kit::section_label("Schema", theme))
        .child(dispatch_on_click(
            kit::filter_chip(
                "gd-ins-schema-jump",
                grid.schema_label.clone(),
                Some(IconName::LayoutDashboard),
                Some(theme.info),
                None,
                false,
                theme,
            ),
            crate::actions::SelectGameDataSchema {
                schema_key: grid.schema_key.clone(),
            },
        ));

    if !grid.families.is_empty() {
        body = body.child(kit::section_label("Table families", theme));
        body = body.child(
            h_flex().gap_1().flex_wrap().children(
                grid.families
                    .iter()
                    .map(|family| kit::tag_chip(family.clone(), theme.info)),
            ),
        );
    }

    if !grid.diagnostics.is_empty() {
        body = body.child(kit::section_label("Diagnostics", theme));
        for diagnostic in &grid.diagnostics {
            body = body.child(
                h_flex()
                    .gap_1p5()
                    .items_start()
                    .child(kit::status_dot(theme.warning))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(10.5))
                            .text_color(theme.muted_foreground)
                            .child(diagnostic.message.clone()),
                    ),
            );
        }
    }

    // Service gaps kept explicit instead of mock affordances.
    body = body.child(kit::section_label("GameData components", theme));
    body = body.child(kit::hint_text(
        "Asset bindings to this table are not published by project-host yet.",
        theme,
    ));
    body = body.child(kit::section_label("Referenced by", theme));
    body = body.child(kit::hint_text(
        "Foreign-key reverse references are not published by the catalog yet.",
        theme,
    ));
    body.into_any_element()
}

fn render_schema_inspector(data: &EditorGameDataProjection, theme: &Theme) -> gpui::AnyElement {
    let GameDataWorkbenchProjection::Schema(schema) = &data.workbench else {
        return kit::empty_state("Select a schema in the Game Data catalog.", None, theme)
            .into_any_element();
    };
    let mut body = v_flex()
        .gap_2()
        .child(
            div()
                .text_size(px(13.0))
                .font_semibold()
                .text_color(theme.foreground)
                .child(schema.label.clone()),
        )
        .child(kit::status_row(
            "Row type",
            schema
                .rust_type
                .clone()
                .unwrap_or_else(|| schema.schema_key.clone()),
            theme,
        ))
        .child(kit::status_row(
            "Fields",
            schema.fields.len().to_string(),
            theme,
        ));
    if let Some(description) = &schema.description {
        body = body.child(kit::hint_text(description.clone(), theme));
    }
    body = body.child(kit::section_label("Tables using this schema", theme));
    if schema.used_by.is_empty() {
        body = body.child(kit::hint_text("No tables use this schema.", theme));
    } else {
        for usage in &schema.used_by {
            body = body.child(dispatch_on_click(
                kit::list_row(
                    element_id("gd-ins-schema-used", &usage.table_key),
                    theme,
                    false,
                )
                .pl_2()
                .child(kit::row_icon(
                    IconName::GalleryVerticalEnd,
                    theme.muted_foreground,
                ))
                .child(kit::row_name(usage.name.clone(), theme))
                .child(kit::meta_text(
                    format!("{} · {}", usage.category, usage.count_label),
                    theme,
                )),
                crate::actions::SelectGameDataTable {
                    table_key: usage.table_key.clone(),
                },
            ));
        }
    }
    body.into_any_element()
}

fn render_manager_inspector(data: &EditorGameDataProjection, theme: &Theme) -> gpui::AnyElement {
    let GameDataWorkbenchProjection::Manager(manager) = &data.workbench else {
        return kit::empty_state("Select a manager in the Game Data catalog.", None, theme)
            .into_any_element();
    };
    let status_tint = if manager.status_ok {
        theme.success
    } else {
        theme.warning
    };
    let mut body = v_flex()
        .gap_2()
        .child(kit::section_label("Data manager", theme))
        .child(
            div()
                .text_size(px(13.0))
                .font_semibold()
                .text_color(theme.foreground)
                .child(manager.name.clone()),
        )
        .child(
            h_flex()
                .gap_1p5()
                .items_center()
                .child(kit::tag_chip(
                    manager.kind_label.clone(),
                    if manager.read_only {
                        theme.muted_foreground
                    } else {
                        theme.primary
                    },
                ))
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(kit::status_dot(status_tint))
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(status_tint)
                                .child(manager.status_label.clone()),
                        ),
                ),
        )
        .child(kit::hint_text(manager.summary.clone(), theme));

    if manager.read_only {
        body = body.child(kit::hint_text(
            "Auto-generated and read-only. To change its shape, edit the backing table or its schema.",
            theme,
        ));
    }
    if let (Some(table_key), Some(table_name)) =
        (&manager.backing_table_key, &manager.backing_table_name)
    {
        body = body.child(kit::section_label("Backing table", theme));
        body = body.child(dispatch_on_click(
            kit::list_row(element_id("gd-ins-mgr-table", table_key), theme, false)
                .pl_2()
                .child(kit::row_icon(IconName::GalleryVerticalEnd, theme.info))
                .child(kit::row_name(table_name.clone(), theme))
                .child(
                    Icon::new(IconName::ChevronRight)
                        .with_size(px(13.0))
                        .text_color(theme.muted_foreground),
                ),
            crate::actions::SelectGameDataTable {
                table_key: table_key.clone(),
            },
        ));
    }

    body = body
        .child(kit::status_row("Owner", manager.owner.clone(), theme))
        .child(kit::status_row(
            "Output row type",
            manager.row_type.clone(),
            theme,
        ))
        .child(kit::status_row(
            "Key policy",
            manager.key_policy_summary.clone(),
            theme,
        ))
        .child(kit::status_row(
            "Duplicate keys",
            manager.duplicate_key_policy.clone(),
            theme,
        ));

    body = extend_with_manager_projection(body, manager, theme);
    body = extend_with_manager_validation_rows(body, manager, theme);
    body.into_any_element()
}

/// Inspector projection section: the row type the manager projects into and
/// the source chips that feed it.
fn extend_with_manager_projection(
    mut body: gpui::Div,
    manager: &GameDataManagerCompositionProjection,
    theme: &Theme,
) -> gpui::Div {
    body = body.child(
        h_flex()
            .items_center()
            .gap_1p5()
            .child(kit::section_label(
                format!("Projection → {}", manager.row_type),
                theme,
            ))
            .child(div().flex_1())
            .child(kit::meta_text(manager.projection_count.to_string(), theme)),
    );
    for row in &manager.projection_rows {
        body = body.child(kit::status_row(
            row.output_field.clone(),
            row.source_label.clone(),
            theme,
        ));
    }

    if !manager.consumers.is_empty() {
        body = body.child(kit::section_label("Referenced by", theme));
        for chip in &manager.consumers {
            body = body.child(dispatch_on_click(
                kit::list_row(element_id("gd-ins-mgr-dep", &chip.key), theme, false)
                    .pl_2()
                    .child(kit::status_dot(if chip.provider {
                        theme.muted_foreground
                    } else {
                        theme.primary
                    }))
                    .child(kit::row_name(chip.label.clone(), theme))
                    .child(kit::meta_text(chip.kind.clone(), theme)),
                crate::actions::SelectGameDataManager {
                    manager_key: chip.key.clone(),
                },
            ));
        }
    }
    body
}

/// Inspector validation section: one tinted row per published diagnostic.
fn extend_with_manager_validation_rows(
    mut body: gpui::Div,
    manager: &GameDataManagerCompositionProjection,
    theme: &Theme,
) -> gpui::Div {
    body = body.child(kit::section_label("Validation", theme));
    for row in &manager.validation {
        let tint = if row.ok { theme.success } else { theme.warning };
        body = body.child(
            h_flex()
                .gap_1p5()
                .items_start()
                .child(kit::status_dot(tint))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(10.5))
                        .text_color(theme.muted_foreground)
                        .child(row.message.clone()),
                ),
        );
    }
    body
}

mode_panel!(
    GameDataCatalogPanel,
    "gamedata-catalog",
    "Catalog",
    render_gamedata_catalog,
    Some("storage"),
    kit::TabTone::Muted
);
mode_panel!(
    GameDataDetailsPanel,
    "gamedata-details",
    "Details",
    render_gamedata_details,
    Some("view_column"),
    kit::TabTone::Default
);
