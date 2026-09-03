//! Attached-workspace asset browser projections, source authoring controls, and item styles.

use std::collections::BTreeMap;

use az_editor_inspector::ReflectedEntityInspection;
use az_editor_ui::panels::asset_creation::{
    CreatableAssetSourceData, creatable_asset_sources, default_target_folder_for_source,
};
use az_editor_ui::panels::{
    AssetBrowserEntryData, AssetBrowserEntryStatus, AssetBrowserFolderData, AssetBrowserJobStatus,
    AssetSourceFileWorkflowData, EditorAssetBrowserStatus, EditorAssetBuilderCatalog,
    EditorAssetSourceDependentsPreview, EditorAuthoredOutline, EditorReflectedSelectionState,
    asset_browser_entry_matches_folder, asset_browser_folder_for_key, asset_browser_folders,
};
use gpui::{AppContext, Context, Window};
use gpui_component::ActiveTheme;

use crate::app::aether_common::{AetherItem, AetherStyle};
use crate::app::aether_editor_model::{trace_aether_ui_state, trace_value};
use crate::app::aether_editor_view::AetherEditorView;

use super::AetherViewAction;

use super::authored_content::schema_presentation::{
    schema_color, schema_display_label, schema_icon,
};
use super::presentation::{hsla_css, non_empty_string_or, plural_count, set_item_style};

impl AetherEditorView {
    pub(crate) fn asset_files(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        if self.project_host_connecting(cx) {
            return Vec::new();
        }
        if let Some(status) = cx.try_global::<EditorAssetBrowserStatus>().cloned() {
            return self.asset_files_from_status(&status, cx);
        }
        Vec::new()
    }
    pub(crate) fn asset_folders(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        if self.project_host_connecting(cx) {
            return Vec::new();
        }
        if let Some(status) = cx.try_global::<EditorAssetBrowserStatus>() {
            return self.asset_folders_from_status(status);
        }
        Vec::new()
    }
    pub(crate) fn asset_search(&self) -> String {
        self.state.asset_browser_navigation().search.to_owned()
    }
    pub(crate) fn asset_result_count(&self, cx: &mut Context<Self>) -> String {
        if self.project_host_connecting(cx) {
            return "Connecting".to_owned();
        }
        let Some(status) = cx.try_global::<EditorAssetBrowserStatus>() else {
            return "0 assets".to_owned();
        };
        let folder = self.selected_asset_folder_filter(status);
        let total = asset_entries_for_folder(status, folder.as_ref()).count();
        let browser = self.state.asset_browser_navigation();
        let visible = filtered_asset_entries(status, folder.as_ref(), browser.search).count();
        if browser.search.trim().is_empty() {
            plural_count(visible, "asset")
        } else {
            format!("{visible} of {} matched", plural_count(total, "asset"))
        }
    }
    pub(crate) fn asset_empty_message(&self, cx: &mut Context<Self>) -> String {
        asset_browser_empty_message(
            self.project_host_connecting(cx),
            self.state.asset_browser_navigation().search,
        )
    }
    pub(crate) fn asset_can_go_back(&self) -> bool {
        self.state.asset_browser_navigation().can_go_back
    }
    pub(crate) fn asset_can_go_forward(&self) -> bool {
        self.state.asset_browser_navigation().can_go_forward
    }
    pub(crate) fn asset_back_button_style(&self, cx: &mut Context<Self>) -> AetherStyle {
        asset_history_button_style(self.asset_can_go_back(), cx.theme())
    }
    pub(crate) fn asset_forward_button_style(&self, cx: &mut Context<Self>) -> AetherStyle {
        asset_history_button_style(self.asset_can_go_forward(), cx.theme())
    }
    pub(crate) fn asset_folder_has_selection(&self, cx: &mut Context<Self>) -> bool {
        cx.try_global::<EditorAssetBrowserStatus>()
            .and_then(|status| self.selected_asset_folder_filter(status))
            .is_some()
    }
    pub(crate) fn file_type_chip_style(&self) -> AetherStyle {
        file_type_chip_style()
    }
    pub(crate) fn asset_grid(&self) -> bool {
        self.bool_value("assetGrid")
    }
    pub(crate) fn asset_list(&self) -> bool {
        self.bool_value("assetList")
    }
    pub(crate) fn asset_folder(&self, cx: &mut Context<Self>) -> String {
        if let Some(status) = cx.try_global::<EditorAssetBrowserStatus>() {
            return self.selected_asset_folder(status);
        }
        self.string_value("assetFolder")
    }
    pub(crate) fn file_type_icon(&self, cx: &mut Context<Self>) -> String {
        let selection = self.state.asset_selection();
        if !selection.icon.is_empty() {
            return selection.icon.to_owned();
        }
        selection_file_type_projection(
            cx.try_global::<EditorReflectedSelectionState>()
                .and_then(EditorReflectedSelectionState::current),
            cx.try_global::<EditorAuthoredOutline>(),
        )
        .icon
    }
    pub(crate) fn file_type_label(&self, cx: &mut Context<Self>) -> String {
        let selection = self.state.asset_selection();
        if !selection.schema_type.is_empty() {
            return schema_display_label(selection.schema_type);
        }
        selection_file_type_projection(
            cx.try_global::<EditorReflectedSelectionState>()
                .and_then(EditorReflectedSelectionState::current),
            cx.try_global::<EditorAuthoredOutline>(),
        )
        .label
    }
    pub(crate) fn file_type_color(&self, cx: &mut Context<Self>) -> String {
        let selection = self.state.asset_selection();
        if !selection.color.is_empty() {
            return selection.color.to_owned();
        }
        selection_file_type_projection(
            cx.try_global::<EditorReflectedSelectionState>()
                .and_then(EditorReflectedSelectionState::current),
            cx.try_global::<EditorAuthoredOutline>(),
        )
        .color
    }
    pub(crate) fn asset_on_search(
        &mut self,
        value: impl AsRef<str>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_input("asset_on_search", value.as_ref());
        cx.notify();
    }
    pub(crate) fn asset_create_on_name(
        &mut self,
        value: impl AsRef<str>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_input("asset_create_on_name", value.as_ref());
        cx.notify();
    }
    pub(crate) fn asset_create_on_folder(
        &mut self,
        value: impl AsRef<str>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_input("asset_create_on_folder", value.as_ref());
        cx.notify();
    }
    pub(crate) fn asset_rename_on_path(
        &mut self,
        value: impl AsRef<str>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_input("asset_rename_on_path", value.as_ref());
        cx.notify();
    }
    pub(crate) fn go_assets<E>(
        &mut self,
        _event: &E,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_action(AetherViewAction::GoAssets);
        cx.notify();
    }
    pub(crate) fn open_asset_processor<E>(
        &mut self,
        _event: &E,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_action(AetherViewAction::ClosePipe);
        window.dispatch_action(Box::new(az_editor_ui::actions::OpenAssetProcessor), cx);
        cx.notify();
    }
    pub(crate) fn on_grid_view<E>(
        &mut self,
        _event: &E,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_action(AetherViewAction::UseAssetGrid);
        cx.notify();
    }
    pub(crate) fn on_list_view<E>(
        &mut self,
        _event: &E,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_action(AetherViewAction::UseAssetList);
        cx.notify();
    }
    pub(crate) fn select_asset_folder(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.state.select_asset_folder_state(key) {
            return;
        }

        let before = self.state.trace_summary();
        trace_aether_ui_state(
            "asset.folder_select",
            format!("folder={} before={}", trace_value(key), before),
            &self.state,
        );
        cx.notify();
    }

    pub(crate) fn select_asset_root(&mut self, cx: &mut Context<Self>) {
        self.select_asset_folder("", cx);
    }

    pub(crate) fn navigate_asset_back(&mut self, cx: &mut Context<Self>) {
        if self.state.navigate_asset_back_state() {
            trace_aether_ui_state("asset.folder_back", "history=back", &self.state);
            cx.notify();
        }
    }

    pub(crate) fn navigate_asset_forward(&mut self, cx: &mut Context<Self>) {
        if self.state.navigate_asset_forward_state() {
            trace_aether_ui_state("asset.folder_forward", "history=forward", &self.state);
            cx.notify();
        }
    }

    pub(crate) fn activate_asset_item(
        &mut self,
        item: &AetherItem,
        event: &gpui::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if item.kind != "asset" {
            self.activate_item(item, window, cx);
            return;
        }
        self.select_asset_item(item, cx);
        if event.click_count() < 2 {
            cx.notify();
            return;
        }

        if asset_item_is_authored_document(item) {
            tracing::info!(
                source_path = %item.src,
                schema_type = %item.type_label,
                "opening authored asset from Aether asset browser"
            );
            window.dispatch_action(
                Box::new(az_editor_ui::actions::SelectAuthoredDocument {
                    document_id: item.src.clone(),
                }),
                cx,
            );
            cx.stop_propagation();
        } else if let Ok(job_id) = item.idx.parse::<i64>() {
            let attempt_id = (!item.value.is_empty())
                .then(|| item.value.parse::<i64>().ok())
                .flatten();
            tracing::info!(
                source_path = %item.src,
                job_id,
                attempt_id = ?attempt_id,
                "inspecting job from Aether asset browser"
            );
            window.dispatch_action(
                Box::new(az_editor_ui::actions::InspectJob { job_id, attempt_id }),
                cx,
            );
            cx.stop_propagation();
        }
        cx.notify();
    }

    pub(crate) fn asset_create_open(&self) -> bool {
        {
            let overlay = self.state.overlay_projection();
            overlay.modal_open && overlay.modal_kind == "asset-create"
        }
    }

    pub(crate) fn open_asset_create_modal(&mut self, cx: &mut Context<Self>) {
        let sources = self.creatable_asset_sources(cx);
        let draft = self.state.asset_create_draft();
        let connecting = sources.is_empty() && self.project_host_connecting(cx);
        let schema_type = if connecting
            || draft.schema_type.is_empty()
            || !sources
                .iter()
                .any(|source| source.schema_type == draft.schema_type)
        {
            sources
                .first()
                .map(|source| source.schema_type.clone())
                .unwrap_or_default()
        } else {
            draft.schema_type.to_owned()
        };
        let folder = if connecting {
            String::new()
        } else if draft.folder.trim().is_empty() {
            self.selected_asset_create_source_for_schema(&schema_type, cx)
                .map(|source| {
                    let workflow = AssetSourceFileWorkflowData {
                        source_root: source.source_root.clone(),
                        default_path_prefix: source.default_path_prefix.clone(),
                        extensions: source.extensions.clone(),
                        can_create: true,
                        can_edit: true,
                    };
                    default_target_folder_for_source(&self.asset_folder(cx), &workflow)
                })
                .unwrap_or_default()
        } else {
            draft.folder.to_owned()
        };
        self.state.begin_asset_create_state(
            schema_type,
            folder,
            connecting.then(|| {
                "Project services are still connecting; asset source roots are not loaded yet."
                    .to_owned()
            }),
        );
        self.state.open_overlay_modal_state("asset-create");
        trace_aether_ui_state("asset.create.open", "open=true", &self.state);
        cx.notify();
    }
    pub(crate) fn asset_create_rows(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let theme = cx.theme().clone();
        let sources = self.creatable_asset_sources(cx);
        if sources.is_empty() && self.project_host_connecting(cx) {
            let mut row = AetherItem {
                kind: "asset-create-source".to_owned(),
                key: "project-services-connecting".to_owned(),
                label: "Connecting to project services".to_owned(),
                cat: "Project".to_owned(),
                src: "waiting for asset source roots".to_owned(),
                sub: "Asset-builder catalog has not attached yet".to_owned(),
                ext: String::new(),
                selected: true,
                active: false,
                ..AetherItem::default()
            };
            set_item_style(
                &mut row,
                "style",
                asset_create_source_row_style(true, &theme),
            );
            return vec![row];
        }
        let mut rows = Vec::new();
        let mut current_category = String::new();
        for source in sources {
            if source.category != current_category {
                current_category = source.category.clone();
                rows.push(AetherItem {
                    kind: "asset-create-category".to_owned(),
                    label: current_category.clone(),
                    sep: true,
                    ..AetherItem::default()
                });
            }
            let selected = source.schema_type == self.selected_asset_create_schema_type(cx);
            let extension_hint = source.extension_hint();
            let mut row = AetherItem {
                kind: "asset-create-source".to_owned(),
                key: source.schema_type,
                label: source.label,
                cat: source.category,
                src: source.source_root,
                sub: source.default_path_prefix.clone(),
                ext: extension_hint,
                selected,
                active: selected,
                ..AetherItem::default()
            };
            set_item_style(
                &mut row,
                "style",
                asset_create_source_row_style(selected, &theme),
            );
            rows.push(row);
        }
        rows
    }

    pub(crate) fn asset_create_name(&self) -> String {
        self.state.asset_create_draft().name.to_owned()
    }

    pub(crate) fn asset_create_folder(&self) -> String {
        self.state.asset_create_draft().folder.to_owned()
    }

    pub(crate) fn asset_create_error(&self) -> String {
        self.state.asset_create_draft().error.to_owned()
    }

    pub(crate) fn asset_create_preview_path(&self, cx: &mut Context<Self>) -> String {
        self.build_asset_create_request(cx)
            .map(|request| request.source_path)
            .unwrap_or_else(|_| "choose a type and name".to_owned())
    }

    pub(crate) fn asset_create_source_root_label(&self, cx: &mut Context<Self>) -> String {
        if self.project_host_connecting(cx) && self.creatable_asset_sources(cx).is_empty() {
            return "waiting for asset source roots".to_owned();
        }
        self.selected_asset_create_source(cx)
            .map(|source| source.source_root)
            .unwrap_or_else(|| "no source root".to_owned())
    }

    pub(crate) fn asset_create_can_submit(&self, cx: &mut Context<Self>) -> bool {
        self.build_asset_create_request(cx).is_ok()
    }

    pub(crate) fn select_asset_create_schema(&mut self, schema_type: &str, cx: &mut Context<Self>) {
        let default_folder = self
            .selected_asset_create_source_for_schema(schema_type, cx)
            .map(|source| source.default_path_prefix);
        if self
            .state
            .choose_asset_create_schema_state(schema_type, default_folder)
        {
            cx.notify();
        }
    }

    pub(crate) fn submit_asset_create(
        &mut self,
        _event: &gpui::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.build_asset_create_request(cx) {
            Ok(request) => {
                tracing::info!(
                    schema_type = %request.schema_type,
                    source_root = %request.source_root,
                    source_path = %request.source_path,
                    "creating asset source file from Aether asset browser"
                );
                self.state.close_overlay_modal_state();
                self.state.commit_asset_create_state(
                    request.schema_type.clone(),
                    request.source_path.clone(),
                    hsla_css(cx.theme().accent),
                );
                window.dispatch_action(
                    Box::new(az_editor_ui::actions::CreateAssetSourceFile {
                        schema_type: request.schema_type,
                        source_root: request.source_root,
                        source_path: request.source_path,
                    }),
                    cx,
                );
                window.dispatch_action(Box::new(az_editor_ui::actions::RefreshAssets), cx);
                cx.stop_propagation();
                cx.notify();
            }
            Err(err) => {
                self.state.reject_asset_create_state(err.to_string());
                cx.notify();
            }
        }
    }

    pub(crate) fn asset_rename_open(&self) -> bool {
        {
            let overlay = self.state.overlay_projection();
            overlay.modal_open && overlay.modal_kind == "asset-rename"
        }
    }

    pub(crate) fn open_asset_rename_modal(&mut self, item: &AetherItem, cx: &mut Context<Self>) {
        self.select_asset_item(item, cx);
        self.state
            .begin_asset_rename_state(asset_item_source_root(item), item.src.clone());
        self.state.open_overlay_modal_state("asset-rename");
        trace_aether_ui_state("asset.rename.open", "open=true", &self.state);
        cx.notify();
    }

    pub(crate) fn asset_rename_from_path(&self) -> String {
        self.state.asset_rename_draft().from_path.to_owned()
    }

    pub(crate) fn asset_rename_to_path(&self) -> String {
        self.state.asset_rename_draft().to_path.to_owned()
    }

    pub(crate) fn asset_rename_source_root_label(&self) -> String {
        non_empty_string_or(
            self.state.asset_rename_draft().source_root,
            "no source root",
        )
    }

    pub(crate) fn asset_rename_error(&self) -> String {
        self.state.asset_rename_draft().error.to_owned()
    }

    pub(crate) fn asset_rename_can_submit(&self) -> bool {
        self.build_asset_rename_request().is_ok()
    }

    pub(crate) fn submit_asset_rename(
        &mut self,
        _event: &gpui::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.build_asset_rename_request() {
            Ok((source_root, from_source_path, to_source_path)) => {
                tracing::info!(
                    source_root = %source_root,
                    from_source_path = %from_source_path,
                    to_source_path = %to_source_path,
                    "renaming asset source file from Aether asset browser"
                );
                self.state.close_overlay_modal_state();
                let was_authored_document = self
                    .state
                    .commit_asset_rename_state(&from_source_path, &to_source_path)
                    && from_source_path.to_ascii_lowercase().ends_with(".ron");
                if was_authored_document {
                    self.state.record_authored_source_path_move(
                        from_source_path.clone(),
                        to_source_path.clone(),
                    );
                }
                window.dispatch_action(
                    Box::new(az_editor_ui::actions::RenameAssetSource {
                        source_root,
                        from_source_path: from_source_path.clone(),
                        to_source_path: to_source_path.clone(),
                    }),
                    cx,
                );
                if was_authored_document {
                    window.dispatch_action(
                        Box::new(az_editor_ui::actions::RefreshAuthoredOutline),
                        cx,
                    );
                }
                window.dispatch_action(Box::new(az_editor_ui::actions::RefreshAssets), cx);
                cx.stop_propagation();
                cx.notify();
            }
            Err(err) => {
                self.state.reject_asset_rename_state(err);
                cx.notify();
            }
        }
    }

    pub(crate) fn asset_delete_open(&self) -> bool {
        {
            let overlay = self.state.overlay_projection();
            overlay.modal_open && overlay.modal_kind == "asset-delete"
        }
    }

    pub(crate) fn open_asset_delete_modal(
        &mut self,
        item: &AetherItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_asset_item(item, cx);
        self.state
            .begin_asset_delete_state(asset_item_source_root(item), item.src.clone());
        self.state.open_overlay_modal_state("asset-delete");
        let draft = self.state.asset_delete_draft();
        if draft.source_root.trim().is_empty() || draft.source_path.trim().is_empty() {
            self.state
                .reject_asset_delete_state("asset source root or path is unavailable".to_owned());
        } else {
            window.dispatch_action(
                Box::new(az_editor_ui::actions::PreviewDeleteAssetSource {
                    source_root: draft.source_root.to_owned(),
                    source_path: draft.source_path.to_owned(),
                }),
                cx,
            );
        }
        trace_aether_ui_state("asset.delete.open", "open=true", &self.state);
        cx.notify();
    }

    pub(crate) fn asset_delete_source_path(&self) -> String {
        self.state.asset_delete_draft().source_path.to_owned()
    }

    pub(crate) fn asset_delete_source_root_label(&self) -> String {
        non_empty_string_or(
            self.state.asset_delete_draft().source_root,
            "no source root",
        )
    }

    pub(crate) fn asset_delete_error(&self, cx: &mut Context<Self>) -> String {
        let draft = self.state.asset_delete_draft();
        if !draft.error.is_empty() {
            return draft.error.to_owned();
        }
        self.matching_asset_delete_preview(cx)
            .and_then(|preview| preview.error.clone())
            .unwrap_or_default()
    }

    pub(crate) fn asset_delete_summary(&self, cx: &mut Context<Self>) -> String {
        let Some(preview) = self.matching_asset_delete_preview(cx) else {
            return "Checking dependents...".to_owned();
        };
        if preview.loading {
            return "Checking dependents...".to_owned();
        }
        let count = preview.total_dependents();
        if count == 0 {
            return "No recorded dependents. The source file and current products will be retired."
                .to_owned();
        }
        format!(
            "Deleting {} breaks {} dependents:",
            self.state.asset_delete_draft().source_path,
            count,
        )
    }

    pub(crate) fn asset_delete_dependent_rows(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let Some(preview) = self.matching_asset_delete_preview(cx) else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        for dependent in &preview.source_dependents {
            rows.push(AetherItem {
                kind: "asset-delete-dependent".to_owned(),
                icon: "description".to_owned(),
                name: dependent.source_path.clone(),
                meta: format!("source relation · {}", dependent.relation),
                color: hsla_css(cx.theme().accent),
                ..AetherItem::default()
            });
        }
        for dependent in &preview.job_dependents {
            let products = if dependent.product_paths.is_empty() {
                "product dependency".to_owned()
            } else {
                dependent.product_paths.join(", ")
            };
            rows.push(AetherItem {
                kind: "asset-delete-dependent".to_owned(),
                icon: "deployed_code".to_owned(),
                name: dependent.source_path.clone(),
                meta: format!(
                    "{}:{} · {}",
                    dependent.job_key, dependent.platform, products
                ),
                color: hsla_css(cx.theme().warning),
                ..AetherItem::default()
            });
        }
        rows
    }

    pub(crate) fn asset_delete_can_submit(&self, cx: &mut Context<Self>) -> bool {
        self.state.asset_delete_draft().error.is_empty()
            && self
                .matching_asset_delete_preview(cx)
                .is_some_and(|preview| !preview.loading && preview.error.is_none())
    }

    pub(crate) fn submit_asset_delete(
        &mut self,
        _event: &gpui::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.asset_delete_can_submit(cx) {
            self.state.reject_asset_delete_state(
                "wait for the dependents check before deleting".to_owned(),
            );
            cx.notify();
            return;
        }
        let draft = self.state.asset_delete_draft();
        let source_root = draft.source_root.to_owned();
        let source_path = draft.source_path.to_owned();
        tracing::info!(
            source_root = %source_root,
            source_path = %source_path,
            "deleting asset source file from Aether asset browser"
        );
        let was_authored_document = self.state.commit_asset_delete_state(&source_path)
            && source_path.to_ascii_lowercase().ends_with(".ron");
        self.state.close_overlay_modal_state();
        window.dispatch_action(
            Box::new(az_editor_ui::actions::DeleteAssetSource {
                source_root,
                source_path,
            }),
            cx,
        );
        if was_authored_document {
            window.dispatch_action(Box::new(az_editor_ui::actions::RefreshAuthoredOutline), cx);
        }
        window.dispatch_action(Box::new(az_editor_ui::actions::RefreshAssets), cx);
        cx.stop_propagation();
        cx.notify();
    }

    fn select_asset_item(&mut self, item: &AetherItem, cx: &mut Context<Self>) {
        self.state.select_asset_state(item);
        tracing::info!(
            source_path = %item.src,
            schema_type = %item.type_label,
            "selected asset in Aether asset browser"
        );
        cx.notify();
    }

    fn creatable_asset_sources(&self, cx: &mut Context<Self>) -> Vec<CreatableAssetSourceData> {
        cx.try_global::<EditorAssetBuilderCatalog>()
            .map(creatable_asset_sources)
            .unwrap_or_default()
    }

    pub(super) fn project_host_connecting(&self, cx: &mut Context<Self>) -> bool {
        az_editor_ui::panels::editor_project_host_connecting(cx)
    }

    fn selected_asset_create_schema_type(&self, cx: &mut Context<Self>) -> String {
        let draft = self.state.asset_create_draft();
        if !draft.schema_type.is_empty() {
            return draft.schema_type.to_owned();
        }
        self.creatable_asset_sources(cx)
            .first()
            .map(|source| source.schema_type.clone())
            .unwrap_or_default()
    }

    pub(super) fn selected_asset_create_source(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<CreatableAssetSourceData> {
        let selected = self.selected_asset_create_schema_type(cx);
        self.creatable_asset_sources(cx)
            .into_iter()
            .find(|source| source.schema_type == selected)
            .or_else(|| self.creatable_asset_sources(cx).into_iter().next())
    }

    fn selected_asset_create_source_for_schema(
        &self,
        schema_type: &str,
        cx: &mut Context<Self>,
    ) -> Option<CreatableAssetSourceData> {
        self.creatable_asset_sources(cx)
            .into_iter()
            .find(|source| source.schema_type == schema_type)
            .or_else(|| self.creatable_asset_sources(cx).into_iter().next())
    }

    fn matching_asset_delete_preview(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<EditorAssetSourceDependentsPreview> {
        let preview = cx.try_global::<EditorAssetSourceDependentsPreview>()?;
        let draft = self.state.asset_delete_draft();
        (preview.source_root == draft.source_root && preview.source_path == draft.source_path)
            .then(|| preview.clone())
    }

    fn asset_folders_from_status(&self, status: &EditorAssetBrowserStatus) -> Vec<AetherItem> {
        let folders = asset_browser_folders(status);
        let selected = self.selected_asset_folder_filter(status);
        folders
            .into_iter()
            .map(|folder| {
                let category = asset_folder_category(status, &folder);
                let active = selected
                    .as_ref()
                    .is_some_and(|selected| selected.key == folder.key);
                let mut item = AetherItem {
                    key: folder.key.clone(),
                    name: folder.name.clone(),
                    icon: category.folder_icon.to_owned(),
                    color: category.color.to_owned(),
                    count: folder.count.to_string(),
                    file: folder.breadcrumb(),
                    active,
                    selected: active,
                    ..AetherItem::default()
                };
                set_item_style(&mut item, "style", asset_folder_style(active));
                item
            })
            .collect()
    }

    fn asset_files_from_status(
        &self,
        status: &EditorAssetBrowserStatus,
        cx: &mut Context<Self>,
    ) -> Vec<AetherItem> {
        let selected = self.selected_asset_folder_filter(status);
        let theme = cx.theme().clone();
        let source_root_by_scan_folder_id = status
            .roots
            .iter()
            .map(|root| (root.root_id, root.portable_key.clone()))
            .collect::<BTreeMap<_, _>>();
        let browser = self.state.asset_browser_navigation();
        filtered_asset_entries(status, selected.as_ref(), browser.search)
            .map(|entry| {
                let selected = self.asset_entry_is_selected(entry);
                let source_root = source_root_by_scan_folder_id
                    .get(&entry.root_id)
                    .cloned()
                    .unwrap_or_default();
                asset_entry_item(entry, selected, &theme, source_root)
            })
            .collect()
    }

    fn selected_asset_folder(&self, status: &EditorAssetBrowserStatus) -> String {
        if self
            .state
            .asset_browser_navigation()
            .folder
            .trim()
            .is_empty()
        {
            return "All".to_owned();
        }
        self.selected_asset_folder_filter(status)
            .map(|folder| folder.breadcrumb())
            .unwrap_or_else(|| "All".to_owned())
    }

    fn selected_asset_folder_filter(
        &self,
        status: &EditorAssetBrowserStatus,
    ) -> Option<AssetBrowserFolderData> {
        let browser = self.state.asset_browser_navigation();
        if browser.folder.trim().is_empty() {
            return None;
        }
        let folders = asset_browser_folders(status);
        asset_browser_folder_for_key(&folders, browser.folder)
            .cloned()
            .or_else(|| folders.first().cloned())
    }

    fn asset_entry_is_selected(&self, entry: &AssetBrowserEntryData) -> bool {
        let selection = self.state.asset_selection();
        (!selection.key.is_empty() && selection.key == entry.entry_id.to_string())
            || (!selection.source_path.is_empty() && selection.source_path == entry.source_path)
    }

    pub(super) fn pipe_jobs_from_asset_status(
        &self,
        status: &EditorAssetBrowserStatus,
        theme: &gpui_component::theme::Theme,
    ) -> Vec<AetherItem> {
        status
            .entries
            .iter()
            .filter_map(|entry| entry.latest_job.as_ref().map(|job| (entry, job)))
            .take(16)
            .map(|(entry, job)| {
                let running = matches!(
                    job.status,
                    AssetBrowserJobStatus::Queued | AssetBrowserJobStatus::Leased
                );
                let mut item = AetherItem {
                    key: job.job_id.to_string(),
                    idx: job
                        .attempt_id
                        .map_or_else(String::new, |attempt_id| attempt_id.to_string()),
                    name: asset_display_name(&entry.source_path),
                    icon: asset_entry_category(entry).file_icon.to_owned(),
                    color: hsla_css(job.status.tone().color(theme)),
                    stage: format!("{} · {}", job.platform, job.job_key),
                    status: job.status.label().to_owned(),
                    running,
                    ..AetherItem::default()
                };
                set_item_style(&mut item, "barStyle", pipe_job_bar_style(job.status, theme));
                set_item_style(
                    &mut item,
                    "statusStyle",
                    pipe_job_status_style(job.status, theme),
                );
                item
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SelectionFileTypeProjection {
    pub(crate) icon: String,
    pub(crate) label: String,
    pub(crate) color: String,
}

pub(crate) fn selection_file_type_projection(
    inspection: Option<&ReflectedEntityInspection>,
    outline: Option<&EditorAuthoredOutline>,
) -> SelectionFileTypeProjection {
    let schema = inspection
        .and_then(|inspection| inspection.components.first())
        .map(|component| component.component.type_path.as_str())
        .or_else(|| {
            outline.and_then(|outline| {
                outline
                    .data
                    .documents
                    .iter()
                    .find(|document| document.objects.iter().any(|object| object.selected))
                    .map(|document| document.schema_type.as_str())
            })
        });
    if let Some(schema) = schema {
        return SelectionFileTypeProjection {
            icon: schema_icon(schema).to_owned(),
            label: schema_display_label(schema),
            color: schema_color(schema).to_owned(),
        };
    }
    SelectionFileTypeProjection {
        icon: "description".to_owned(),
        label: "Selection".to_owned(),
        color: "#7a8aa0".to_owned(),
    }
}

fn asset_item_is_authored_document(item: &AetherItem) -> bool {
    !item.type_label.trim().is_empty() && item.src.to_ascii_lowercase().ends_with(".ron")
}

fn file_type_chip_style() -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("gap", "4px".to_owned()),
        ("fontSize", "9.5px".to_owned()),
        ("color", "#9aa1ac".to_owned()),
        ("background", "#23272e".to_owned()),
        ("border", "1px solid #2c313a".to_owned()),
        ("borderRadius", "3px".to_owned()),
        ("padding", "1px 6px".to_owned()),
    ])
}

fn pipe_job_bar_style(
    status: AssetBrowserJobStatus,
    theme: &gpui_component::theme::Theme,
) -> AetherStyle {
    AetherStyle::from_pairs(&[
        (
            "width",
            match status {
                AssetBrowserJobStatus::Queued => "20%",
                AssetBrowserJobStatus::Leased => "35%",
                AssetBrowserJobStatus::Succeeded => "100%",
                AssetBrowserJobStatus::Failed | AssetBrowserJobStatus::Abandoned => "100%",
            }
            .to_owned(),
        ),
        ("background", hsla_css(status.tone().color(theme))),
    ])
}

fn pipe_job_status_style(
    status: AssetBrowserJobStatus,
    theme: &gpui_component::theme::Theme,
) -> AetherStyle {
    AetherStyle::from_pairs(&[("color", hsla_css(status.tone().color(theme)))])
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct AssetCategory {
    pub(crate) label: &'static str,
    pub(crate) folder_icon: &'static str,
    pub(crate) file_icon: &'static str,
    pub(crate) color: &'static str,
}

const ASSET_CATEGORIES: &[AssetCategory] = &[
    AssetCategory {
        label: "Meshes",
        folder_icon: "category",
        file_icon: "deployed_code",
        color: "#7a8aa0",
    },
    AssetCategory {
        label: "Materials",
        folder_icon: "gradient",
        file_icon: "texture",
        color: "#d6a23b",
    },
    AssetCategory {
        label: "Textures",
        folder_icon: "image",
        file_icon: "image",
        color: "#8fc97e",
    },
    AssetCategory {
        label: "Prefabs",
        folder_icon: "widgets",
        file_icon: "widgets",
        color: "#b78fd6",
    },
    AssetCategory {
        label: "Scripts",
        folder_icon: "code",
        file_icon: "code",
        color: "#4fd1c5",
    },
    AssetCategory {
        label: "Audio",
        folder_icon: "graphic_eq",
        file_icon: "graphic_eq",
        color: "#d88ccf",
    },
    AssetCategory {
        label: "Levels",
        folder_icon: "map",
        file_icon: "map",
        color: "#f5a742",
    },
    AssetCategory {
        label: "Animations",
        folder_icon: "animation",
        file_icon: "animation",
        color: "#65c7f7",
    },
    AssetCategory {
        label: "Other",
        folder_icon: "insert_drive_file",
        file_icon: "insert_drive_file",
        color: "#8b919c",
    },
];

pub(crate) fn asset_category_counts(
    status: &EditorAssetBrowserStatus,
) -> Vec<(AssetCategory, usize)> {
    let mut counts = BTreeMap::<&'static str, usize>::new();
    for entry in visible_asset_entries(status) {
        let category = asset_entry_category(entry);
        *counts.entry(category.label).or_default() += 1;
    }

    ASSET_CATEGORIES
        .iter()
        .filter_map(|category| {
            counts
                .get(category.label)
                .copied()
                .filter(|count| *count > 0)
                .map(|count| (*category, count))
        })
        .collect()
}

pub(crate) fn asset_folder_category(
    status: &EditorAssetBrowserStatus,
    folder: &AssetBrowserFolderData,
) -> AssetCategory {
    asset_entries_for_folder(status, Some(folder))
        .next()
        .map(asset_entry_category)
        .unwrap_or_else(|| asset_category("Other"))
}

pub(super) fn visible_asset_entries(
    status: &EditorAssetBrowserStatus,
) -> impl Iterator<Item = &AssetBrowserEntryData> {
    status.entries.iter().filter(|entry| {
        entry
            .schema_type
            .as_deref()
            .is_some_and(|schema| !schema.trim().is_empty())
    })
}

pub(crate) fn asset_entries_for_folder<'a>(
    status: &'a EditorAssetBrowserStatus,
    folder: Option<&AssetBrowserFolderData>,
) -> impl Iterator<Item = &'a AssetBrowserEntryData> {
    visible_asset_entries(status).filter(move |entry| {
        folder.is_none_or(|folder| asset_browser_entry_matches_folder(entry, folder))
    })
}

pub(crate) fn filtered_asset_entries<'a>(
    status: &'a EditorAssetBrowserStatus,
    folder: Option<&AssetBrowserFolderData>,
    search: &str,
) -> impl Iterator<Item = &'a AssetBrowserEntryData> {
    let search = search.trim().to_ascii_lowercase();
    let mut entries = asset_entries_for_folder(status, folder)
        .filter(move |entry| {
            search.is_empty()
                || entry
                    .source_path
                    .to_ascii_lowercase()
                    .contains(search.as_str())
                || entry
                    .schema_type
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .contains(search.as_str())
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.source_path.cmp(&right.source_path));
    entries.into_iter()
}

pub(crate) fn asset_browser_empty_message(project_host_connecting: bool, search: &str) -> String {
    if project_host_connecting {
        return "Connecting to project services...".to_owned();
    }
    let search = search.trim();
    if search.is_empty() {
        "No assets in this folder".to_owned()
    } else {
        format!("No assets match `{search}`")
    }
}

pub(crate) fn asset_entry_category(entry: &AssetBrowserEntryData) -> AssetCategory {
    let schema = entry
        .schema_type
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let path = entry.source_path.to_ascii_lowercase().replace('\\', "/");

    if schema == "azoth.scene.scene" || path.contains("/scenes/") || path.ends_with(".scene.ron") {
        return asset_category("Levels");
    }
    if schema.contains("prefab")
        || path.contains("/prefabs/")
        || path.ends_with(".prefab")
        || path.ends_with(".prefab.ron")
    {
        return asset_category("Prefabs");
    }
    if schema.contains("material")
        || path.contains("/materials/")
        || path.ends_with(".material")
        || path.ends_with(".material.ron")
        || path.ends_with(".mat")
        || path.ends_with(".mtl")
    {
        return asset_category("Materials");
    }
    if schema.contains("texture")
        || path.contains("/textures/")
        || path.ends_with(".dds")
        || path.ends_with(".png")
        || path.ends_with(".jpg")
        || path.ends_with(".jpeg")
        || path.ends_with(".tga")
        || path.ends_with(".tif")
        || path.ends_with(".tiff")
        || path.ends_with(".ktx")
        || path.ends_with(".exr")
    {
        return asset_category("Textures");
    }
    if schema.contains("mesh")
        || path.contains("/meshes/")
        || path.ends_with(".mesh")
        || path.ends_with(".cgf")
        || path.ends_with(".fbx")
        || path.ends_with(".gltf")
        || path.ends_with(".glb")
        || path.ends_with(".obj")
    {
        return asset_category("Meshes");
    }
    if schema.contains("script")
        || path.contains("/scripts/")
        || path.ends_with(".lua")
        || path.ends_with(".luac")
    {
        return asset_category("Scripts");
    }
    if schema.contains("audio")
        || path.contains("/audio/")
        || path.ends_with(".wav")
        || path.ends_with(".wem")
        || path.ends_with(".ogg")
        || path.ends_with(".mp3")
    {
        return asset_category("Audio");
    }
    if schema.contains("level")
        || path.contains("/levels/")
        || path.ends_with(".level")
        || path.ends_with(".level.ron")
        || path.ends_with(".cry")
    {
        return asset_category("Levels");
    }
    if schema.contains("animation")
        || path.contains("/animations/")
        || path.ends_with(".anim")
        || path.ends_with(".caf")
        || path.ends_with(".i_caf")
        || path.ends_with(".motion")
        || path.ends_with(".bspace")
    {
        return asset_category("Animations");
    }

    asset_category("Other")
}

fn asset_category(label: &str) -> AssetCategory {
    ASSET_CATEGORIES
        .iter()
        .copied()
        .find(|category| category.label == label)
        .unwrap_or(ASSET_CATEGORIES[ASSET_CATEGORIES.len() - 1])
}

pub(crate) fn asset_entry_item(
    entry: &AssetBrowserEntryData,
    selected: bool,
    theme: &gpui_component::theme::Theme,
    source_root: String,
) -> AetherItem {
    let category = asset_entry_category(entry);
    let mut item = AetherItem {
        kind: "asset".to_owned(),
        key: entry.entry_id.to_string(),
        id: entry.entry_id.to_string(),
        src: entry.source_path.clone(),
        sub: source_root,
        type_label: entry.schema_type.clone().unwrap_or_default(),
        idx: entry
            .latest_job
            .as_ref()
            .map(|job| job.job_id.to_string())
            .unwrap_or_default(),
        value: entry
            .latest_job
            .as_ref()
            .and_then(|job| job.attempt_id)
            .map_or_else(String::new, |attempt_id| attempt_id.to_string()),
        name: asset_display_name(&entry.source_path),
        icon: category.file_icon.to_owned(),
        color: category.color.to_owned(),
        ext: asset_display_extension(&entry.source_path),
        size: entry.status.label().to_owned(),
        r#mod: asset_entry_activity_label(entry),
        status: entry.status.label().to_owned(),
        selected,
        active: selected,
        ..AetherItem::default()
    };
    item.card_style = asset_card_style(selected, theme);
    item.thumb_style = asset_thumb_style();
    item.row_style = asset_row_style(selected, theme);
    item.style_fields
        .insert("cardStyle".to_owned(), item.card_style.clone());
    item.style_fields
        .insert("thumbStyle".to_owned(), item.thumb_style.clone());
    item.style_fields
        .insert("rowStyle".to_owned(), item.row_style.clone());
    item
}

pub(crate) fn asset_item_source_root(item: &AetherItem) -> String {
    item.sub.trim().to_owned()
}

pub(super) fn asset_display_name(source_path: &str) -> String {
    source_path
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(source_path)
        .to_owned()
}

fn asset_display_extension(source_path: &str) -> String {
    let lower = source_path.to_ascii_lowercase();
    for extension in [".prefab.ron", ".material.ron", ".level.ron"] {
        if lower.ends_with(extension) {
            return extension.to_owned();
        }
    }

    source_path
        .rsplit(['/', '\\'])
        .next()
        .and_then(|name| name.rsplit_once('.').map(|(_, extension)| extension))
        .filter(|extension| !extension.is_empty())
        .map_or_else(String::new, |extension| format!(".{extension}"))
}

pub(super) fn asset_entry_activity_label(entry: &AssetBrowserEntryData) -> String {
    if entry.diagnostics_count > 0 {
        return format!("{} issues", entry.diagnostics_count);
    }
    // An entry with no job at all has still been seen by the processor.
    entry
        .latest_job
        .as_ref()
        .map_or_else(|| "indexed".to_owned(), |job| job.status.label().to_owned())
}

pub(super) fn asset_category_for_path(path: &str) -> AssetCategory {
    let entry = AssetBrowserEntryData {
        entry_id: 0,
        workspace_id: 0,
        asset_guid: String::new(),
        root_id: 0,
        source_path: path.to_owned(),
        schema_type: None,
        content_hash: String::new(),
        status: AssetBrowserEntryStatus::Clean,
        diagnostics_count: 0,
        latest_job: None,
    };
    asset_entry_category(&entry)
}

fn asset_folder_style(active: bool) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("gap", "8px".to_owned()),
        ("height", "25px".to_owned()),
        ("padding", "0 10px".to_owned()),
        ("cursor", "pointer".to_owned()),
        ("fontSize", "11.5px".to_owned()),
        (
            "color",
            if active { "#e4e7ec" } else { "#aeb4bd" }.to_owned(),
        ),
        (
            "background",
            if active {
                "rgba(65,136,224,0.13)"
            } else {
                "transparent"
            }
            .to_owned(),
        ),
        (
            "borderLeft",
            if active {
                "2px solid #4188e0"
            } else {
                "2px solid transparent"
            }
            .to_owned(),
        ),
    ])
}

fn asset_card_style(selected: bool, theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("display", "flex".to_owned()),
        ("flexDirection", "column".to_owned()),
        ("gap", "6px".to_owned()),
        ("alignItems", "center".to_owned()),
        ("padding", "6px".to_owned()),
        ("borderRadius", "6px".to_owned()),
        ("cursor", "default".to_owned()),
        (
            "border",
            if selected {
                format!("1px solid {}", hsla_css(theme.accent))
            } else {
                "1px solid transparent".to_owned()
            },
        ),
        (
            "background",
            if selected {
                hsla_css(theme.accent.opacity(0.10))
            } else {
                "transparent".to_owned()
            },
        ),
    ])
}

fn asset_thumb_style() -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("position", "relative".to_owned()),
        ("width", "100%".to_owned()),
        ("aspectRatio", "1/1".to_owned()),
        ("borderRadius", "5px".to_owned()),
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("justifyContent", "center".to_owned()),
        (
            "background",
            "repeating-linear-gradient(135deg,#1e2228 0 8px,#1a1d22 8px 16px)".to_owned(),
        ),
        ("border", "1px solid #2c313a".to_owned()),
    ])
}

fn asset_row_style(selected: bool, theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("height", "25px".to_owned()),
        ("padding", "0 12px".to_owned()),
        ("fontSize", "11.5px".to_owned()),
        ("cursor", "default".to_owned()),
        (
            "color",
            hsla_css(if selected {
                theme.foreground
            } else {
                theme.muted_foreground
            }),
        ),
        (
            "background",
            if selected {
                hsla_css(theme.accent.opacity(0.13))
            } else {
                "transparent".to_owned()
            },
        ),
    ])
}

fn asset_create_source_row_style(
    selected: bool,
    theme: &gpui_component::theme::Theme,
) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("display", "flex".to_owned()),
        ("flexDirection", "column".to_owned()),
        ("gap", "2px".to_owned()),
        ("padding", "8px 10px".to_owned()),
        ("borderRadius", "6px".to_owned()),
        ("cursor", "pointer".to_owned()),
        (
            "color",
            hsla_css(if selected {
                theme.foreground
            } else {
                theme.muted_foreground
            }),
        ),
        (
            "background",
            if selected {
                hsla_css(theme.accent.opacity(0.14))
            } else {
                "transparent".to_owned()
            },
        ),
        (
            "border",
            if selected {
                format!("1px solid {}", hsla_css(theme.accent))
            } else {
                format!("1px solid {}", hsla_css(theme.border))
            },
        ),
    ])
}

fn asset_history_button_style(enabled: bool, theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[
        (
            "cursor",
            if enabled { "pointer" } else { "default" }.to_owned(),
        ),
        (
            "color",
            hsla_css(if enabled {
                theme.foreground
            } else {
                theme.muted_foreground
            }),
        ),
        ("opacity", if enabled { "1" } else { "0.45" }.to_owned()),
    ])
}

pub(super) fn asset_entry_looks_like_skeleton(entry: &AssetBrowserEntryData) -> bool {
    let schema = entry
        .schema_type
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let path = entry.source_path.to_ascii_lowercase().replace('\\', "/");
    schema.contains("skeleton")
        || schema.contains("skel")
        || schema.contains("mannequin")
        || path.contains("/skeleton")
        || path.ends_with(".skel")
        || path.ends_with(".skeleton")
        || path.ends_with(".chr")
        || path.ends_with(".cdf")
}
