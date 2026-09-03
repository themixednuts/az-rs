//! Game-data and gem catalog projections plus their interaction handlers.

use az_editor_ui::{EditorGemCatalog, EditorGemInfo, EditorGemSelection};
use az_proto_project::{GameDataCatalogSnapshot, GameDataTableDescriptor};
use gpui::{AppContext, Context, Window};
use gpui_component::ActiveTheme;

use crate::app::aether_common::{AetherItem, AetherStyle};
use crate::app::aether_editor_view::AetherEditorView;
use crate::game_data_catalog::EditorGameDataCatalog;

use super::presentation::{
    hsla_css, non_empty_string_or, set_item_style, themed_toggle_knob_style,
    themed_toggle_track_style, toolbar_action_item,
};

impl AetherEditorView {
    pub(crate) fn gem_detail_deps(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let Some(gem) = self.selected_gem(cx) else {
            return Vec::new();
        };
        gem.dependencies
            .iter()
            .map(|dependency| AetherItem {
                key: dependency.id.clone(),
                name: dependency.name.clone(),
                icon: "check_circle".to_owned(),
                ..AetherItem::default()
            })
            .collect()
    }
    pub(crate) fn gems_list(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        if let Some(catalog) = cx.try_global::<EditorGemCatalog>() {
            let theme = cx.theme().clone();
            return self.gems_from_catalog(catalog, cx.try_global::<EditorGemSelection>(), &theme);
        }
        Vec::new()
    }
    pub(crate) fn data_tool_items(&self) -> Vec<AetherItem> {
        vec![
            toolbar_action_item(
                "disabled",
                "import",
                "Import",
                "upload_file",
                "Import is unavailable until project-host exposes a table importer",
            ),
            toolbar_action_item(
                "disabled",
                "export-csv",
                "Export CSV",
                "download",
                "CSV export is unavailable until project-host exposes a destination contract",
            ),
            toolbar_action_item(
                "gamedata-validate",
                "validate",
                "Validate",
                "verified",
                "Refresh GameData descriptors and diagnostics",
            ),
        ]
    }
    pub(crate) fn mode_data(&self) -> bool {
        self.bool_value("modeData")
    }
    pub(crate) fn tab_gems(&self) -> bool {
        self.bool_value("tabGems")
    }
    pub(crate) fn gem_detail_cat(&self, cx: &mut Context<Self>) -> String {
        self.selected_gem(cx)
            .map(|gem| gem.category.clone())
            .unwrap_or_else(|| "No category".to_owned())
    }
    pub(crate) fn gem_detail_desc(&self, cx: &mut Context<Self>) -> String {
        self.selected_gem(cx)
            .map(|gem| non_empty_string_or(&gem.display_description(), "No description published."))
            .unwrap_or_else(|| "No gem selected.".to_owned())
    }
    pub(crate) fn gem_detail_icon(&self, cx: &mut Context<Self>) -> String {
        let selected_id = self.selected_gem_id(cx);
        if cx
            .try_global::<EditorGemSelection>()
            .is_some_and(|selection| selection.contains(&selected_id))
        {
            "check_circle".to_owned()
        } else {
            "extension".to_owned()
        }
    }
    pub(crate) fn gem_detail_color(&self, cx: &mut Context<Self>) -> String {
        let theme = cx.theme().clone();
        let selected_id = self.selected_gem_id(cx);
        if cx
            .try_global::<EditorGemSelection>()
            .is_some_and(|selection| selection.contains(&selected_id))
        {
            hsla_css(theme.accent)
        } else {
            hsla_css(theme.muted_foreground)
        }
    }
    pub(crate) fn gem_detail_name(&self, cx: &mut Context<Self>) -> String {
        self.selected_gem(cx)
            .map(|gem| gem.display_name())
            .unwrap_or_else(|| "No Gem Selected".to_owned())
    }
    pub(crate) fn gem_detail_ver(&self, cx: &mut Context<Self>) -> String {
        self.selected_gem(cx)
            .map(|gem| gem.version.clone())
            .unwrap_or_else(|| "—".to_owned())
    }
    fn selected_gem_id(&self, cx: &mut Context<Self>) -> String {
        cx.try_global::<EditorGemCatalog>()
            .and_then(|catalog| {
                selected_gem_id_from_catalog(
                    catalog,
                    cx.try_global::<EditorGemSelection>(),
                    self.state.gem_selection().id,
                )
            })
            .unwrap_or_default()
    }

    fn selected_gem(&self, cx: &mut Context<Self>) -> Option<EditorGemInfo> {
        let catalog = cx.try_global::<EditorGemCatalog>()?;
        let selected_id = selected_gem_id_from_catalog(
            catalog,
            cx.try_global::<EditorGemSelection>(),
            self.state.gem_selection().id,
        )?;
        catalog
            .gems
            .iter()
            .find(|gem| gem.id == selected_id)
            .cloned()
    }

    fn gems_from_catalog(
        &self,
        catalog: &EditorGemCatalog,
        selection: Option<&EditorGemSelection>,
        theme: &gpui_component::theme::Theme,
    ) -> Vec<AetherItem> {
        let mut gems = catalog.gems.clone();
        gems.sort_by(|left, right| {
            left.category
                .cmp(&right.category)
                .then_with(|| left.name.cmp(&right.name))
        });
        let selected_gem_id =
            selected_gem_id_from_catalog(catalog, selection, self.state.gem_selection().id);
        gems.into_iter()
            .map(|gem| {
                let active = selection.is_some_and(|selection| selection.contains(&gem.id));
                let selected = selected_gem_id.as_deref() == Some(gem.id.as_str());
                let display_name = gem.display_name();
                let display_description = gem.display_description();
                let mut item = AetherItem {
                    key: gem.id,
                    kind: "gem".to_owned(),
                    name: display_name,
                    ver: gem.version,
                    desc: !display_description.is_empty(),
                    body: display_description,
                    cat: gem.category,
                    active,
                    selected,
                    icon: if active { "check_circle" } else { "extension" }.to_owned(),
                    icon_color: hsla_css(if active {
                        theme.accent
                    } else {
                        theme.muted_foreground
                    }),
                    name_color: hsla_css(if selected {
                        theme.foreground
                    } else {
                        theme.sidebar_foreground
                    }),
                    ..AetherItem::default()
                };
                set_item_style(&mut item, "rowStyle", gem_row_style(selected, theme));
                set_item_style(
                    &mut item,
                    "trackStyle",
                    themed_toggle_track_style(active, theme),
                );
                set_item_style(
                    &mut item,
                    "knobStyle",
                    themed_toggle_knob_style(active, theme),
                );
                item
            })
            .collect()
    }

    pub(super) fn activate_gamedata_item_with_window(
        &mut self,
        item: &AetherItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match item.id.as_str() {
            "gamedata-table" | "gamedata-table-link" => {
                let table = cx
                    .try_global::<EditorGameDataCatalog>()
                    .and_then(|catalog| game_data_table_for_item(&catalog.catalog, item))
                    .cloned();
                if let Some(table) = table {
                    self.select_gamedata_table_with_dispatch(&table, window, cx);
                    return true;
                }
            }
            "gamedata-schema" => {
                self.state.select_gamedata_schema_state(&item.key);
                cx.stop_propagation();
                cx.notify();
                return true;
            }
            _ => {}
        }
        false
    }

    fn select_gamedata_table_with_dispatch(
        &mut self,
        table: &GameDataTableDescriptor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.select_gamedata_table_state(table);
        window.dispatch_action(
            Box::new(az_editor_ui::actions::SelectGameDataTable {
                table_key: table.name.clone(),
            }),
            cx,
        );
        cx.stop_propagation();
        cx.notify();
    }
}

pub(crate) fn selected_gem_id_from_catalog(
    catalog: &EditorGemCatalog,
    selection: Option<&EditorGemSelection>,
    requested: &str,
) -> Option<String> {
    if !requested.is_empty() && catalog.gems.iter().any(|gem| gem.id == requested) {
        return Some(requested.to_owned());
    }
    selection
        .and_then(|selection| {
            selection
                .enabled
                .iter()
                .find(|id| catalog.gems.iter().any(|gem| gem.id == **id))
                .cloned()
        })
        .or_else(|| catalog.gems.first().map(|gem| gem.id.clone()))
}

fn gem_row_style(selected: bool, theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[(
        "background",
        hsla_css(if selected {
            theme.accent.opacity(0.13)
        } else {
            theme.transparent
        }),
    )])
}

fn game_data_table_by_key<'a>(
    catalog: &'a GameDataCatalogSnapshot,
    key: &str,
) -> Option<&'a GameDataTableDescriptor> {
    if key.trim().is_empty() {
        return None;
    }
    catalog.tables.iter().find(|table| {
        table.name == key
            || table.document_id == key
            || table.source_path == key
            || table.source_root == key
    })
}

pub(crate) fn game_data_table_for_item<'a>(
    catalog: &'a GameDataCatalogSnapshot,
    item: &AetherItem,
) -> Option<&'a GameDataTableDescriptor> {
    match item.id.as_str() {
        "gamedata-table" | "gamedata-table-link" => game_data_table_by_key(catalog, &item.key)
            .or_else(|| game_data_table_by_key(catalog, &item.file))
            .or_else(|| game_data_table_by_key(catalog, &item.name)),
        _ => None,
    }
}
