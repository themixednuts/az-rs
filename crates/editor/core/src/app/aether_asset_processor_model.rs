//! Runtime projection for the adopted Aether Asset Processor window.

use std::collections::{BTreeMap, BTreeSet};

use az_editor_ui::panels::{
    AssetBrowserEntryData, AssetBrowserEntryStatus, AssetBrowserJobData, AssetBrowserJobStatus,
    AssetBuilderData, AssetBuilderPatternData, AssetBuilderPatternKindData,
    AssetSourceSchemaAuthoringData, AssetSourceSchemaData, CatalogProductData,
    EditorAssetBrowserStatus, EditorAssetBuilderCatalog, EditorAssetProcessorActivity,
    EditorCatalogProductsStatus, EditorJobInspection, EditorSessionStateData, EditorSessionStatus,
    JobDependencyData, JobProductData, LogLevel, OutputLogMessage, OutputLogState,
    SessionProcessData, SessionProcessStateData, SessionServiceRoleData,
};
use az_editor_ui::status::ServiceHealthStateData;
use gpui::Context;

use crate::asset_processor::DEFAULT_ASSET_PRODUCT_PLATFORM;

use super::aether_asset_processor_view::AetherAssetProcessorView;
use super::aether_common::AetherItem;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AetherAssetProcessorTab {
    Jobs,
    Assets,
    Logs,
    Builders,
    Connections,
}

impl AetherAssetProcessorTab {
    const ALL: [Self; 5] = [
        Self::Jobs,
        Self::Assets,
        Self::Logs,
        Self::Builders,
        Self::Connections,
    ];

    const fn key(self) -> &'static str {
        match self {
            Self::Jobs => "jobs",
            Self::Assets => "assets",
            Self::Logs => "logs",
            Self::Builders => "builders",
            Self::Connections => "connections",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Jobs => "Jobs",
            Self::Assets => "Assets",
            Self::Logs => "Logs",
            Self::Builders => "Builders",
            Self::Connections => "Connections",
        }
    }

    const fn icon(self) -> &'static str {
        match self {
            Self::Jobs => "work_history",
            Self::Assets => "inventory_2",
            Self::Logs => "description",
            Self::Builders => "construction",
            Self::Connections => "lan",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AetherAssetProcessorJobFilter {
    All,
    Active,
    Queued,
    Succeeded,
    Warnings,
    Failed,
}

impl AetherAssetProcessorJobFilter {
    const ALL: [Self; 6] = [
        Self::All,
        Self::Succeeded,
        Self::Active,
        Self::Queued,
        Self::Warnings,
        Self::Failed,
    ];

    const fn key(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Active => "active",
            Self::Queued => "queued",
            Self::Succeeded => "done",
            Self::Warnings => "warnings",
            Self::Failed => "failed",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Active => "Active",
            Self::Queued => "Queued",
            Self::Succeeded => "Done",
            Self::Warnings => "Warn",
            Self::Failed => "Failed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AetherAssetProcessorDiagnosticFilter {
    All,
    Messages,
    Warnings,
    Errors,
}

impl AetherAssetProcessorDiagnosticFilter {
    const ALL: [Self; 4] = [Self::All, Self::Messages, Self::Warnings, Self::Errors];

    const fn key(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Messages => "messages",
            Self::Warnings => "warnings",
            Self::Errors => "errors",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Messages => "Messages",
            Self::Warnings => "Warnings",
            Self::Errors => "Errors",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AetherAssetProcessorAction {
    ShowTab(AetherAssetProcessorTab),
    SetJobQuery(String),
    SetSourceQuery(String),
    SetPlatformFilter(String),
    SetJobFilter(AetherAssetProcessorJobFilter),
    SetDiagnosticFilter(AetherAssetProcessorDiagnosticFilter),
    SelectJob(i64),
    SelectSource {
        source_key: String,
        job_id: Option<i64>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AetherAssetProcessorState {
    pub(crate) tab: AetherAssetProcessorTab,
    pub(crate) selected_job_id: Option<i64>,
    pub(crate) selected_source_key: Option<String>,
    pub(crate) job_query: String,
    pub(crate) source_query: String,
    pub(crate) platform_filter: String,
    pub(crate) job_filter: AetherAssetProcessorJobFilter,
    pub(crate) diagnostic_filter: AetherAssetProcessorDiagnosticFilter,
}

impl Default for AetherAssetProcessorState {
    fn default() -> Self {
        Self {
            tab: AetherAssetProcessorTab::Jobs,
            selected_job_id: None,
            selected_source_key: None,
            job_query: String::new(),
            source_query: String::new(),
            platform_filter: "all".to_owned(),
            job_filter: AetherAssetProcessorJobFilter::All,
            diagnostic_filter: AetherAssetProcessorDiagnosticFilter::All,
        }
    }
}

impl AetherAssetProcessorState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn apply_action(&mut self, action: AetherAssetProcessorAction) {
        match action {
            AetherAssetProcessorAction::ShowTab(tab) => self.tab = tab,
            AetherAssetProcessorAction::SetJobQuery(query) => self.job_query = query,
            AetherAssetProcessorAction::SetSourceQuery(query) => self.source_query = query,
            AetherAssetProcessorAction::SetPlatformFilter(platform) => {
                self.platform_filter = if platform.trim().is_empty() {
                    "all".to_owned()
                } else {
                    platform
                };
            }
            AetherAssetProcessorAction::SetJobFilter(filter) => self.job_filter = filter,
            AetherAssetProcessorAction::SetDiagnosticFilter(filter) => {
                self.diagnostic_filter = filter;
            }
            AetherAssetProcessorAction::SelectJob(job_id) => {
                self.selected_job_id = Some(job_id);
                self.tab = AetherAssetProcessorTab::Jobs;
            }
            AetherAssetProcessorAction::SelectSource { source_key, job_id } => {
                self.selected_source_key = Some(source_key);
                self.selected_job_id = job_id;
                self.tab = AetherAssetProcessorTab::Assets;
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AetherAssetProcessorCounts {
    pub(crate) sources: usize,
    pub(crate) jobs: usize,
    pub(crate) active: usize,
    pub(crate) queued: usize,
    pub(crate) succeeded: usize,
    pub(crate) warnings: usize,
    pub(crate) failed: usize,
    pub(crate) builders: usize,
    pub(crate) products: usize,
    pub(crate) connections: usize,
    pub(crate) roots: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AetherAssetProcessorHeader {
    pub(crate) icon: &'static str,
    pub(crate) title: String,
    pub(crate) subtitle: String,
    pub(crate) completed_label: String,
    pub(crate) percent_label: String,
    pub(crate) percent: u8,
    pub(crate) progress_known: bool,
    pub(crate) busy: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AetherAssetProcessorSourceGroup {
    pub(crate) label: String,
    pub(crate) items: Vec<AetherItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AetherAssetProcessorSourceDetail {
    pub(crate) source_key: String,
    pub(crate) source_path: String,
    pub(crate) source_root: String,
    pub(crate) source_root_path: String,
    pub(crate) title: String,
    pub(crate) folder: String,
    pub(crate) icon: String,
    pub(crate) status_label: String,
    pub(crate) status_icon: String,
    pub(crate) meta: Vec<AetherItem>,
    pub(crate) products: Vec<AetherItem>,
    pub(crate) dependencies: Vec<AetherItem>,
    pub(crate) products_pending_inspection: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AetherAssetProcessorBuilderProjection {
    pub(crate) builders: Vec<AetherItem>,
    pub(crate) schemas: Vec<AetherItem>,
    pub(crate) processes: Vec<AetherItem>,
    pub(crate) allow_list: Vec<String>,
    pub(crate) reject_list: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AetherAssetProcessorProductProjection {
    pub(crate) platform: String,
    pub(crate) products: Vec<AetherItem>,
    pub(crate) error: Option<String>,
}

impl AetherAssetProcessorView {
    pub(crate) fn apply_action(&mut self, action: AetherAssetProcessorAction) {
        self.state.apply_action(action);
    }

    pub(crate) fn counts(&self, cx: &mut Context<Self>) -> AetherAssetProcessorCounts {
        let asset_status = cx.try_global::<EditorAssetBrowserStatus>();
        let catalog = cx.try_global::<EditorAssetBuilderCatalog>();
        let products = cx.try_global::<EditorCatalogProductsStatus>();
        let session = cx.try_global::<EditorSessionStatus>();
        let activity = cx.try_global::<EditorAssetProcessorActivity>();
        asset_processor_counts(asset_status, catalog, products, session, activity)
    }

    pub(crate) fn header(&self, cx: &mut Context<Self>) -> AetherAssetProcessorHeader {
        let counts = self.counts(cx);
        asset_processor_header(&counts, cx.try_global::<EditorAssetProcessorActivity>())
    }

    pub(crate) fn tabs(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let counts = self.counts(cx);
        AetherAssetProcessorTab::ALL
            .into_iter()
            .map(|tab| {
                let badge = match tab {
                    AetherAssetProcessorTab::Jobs => counts.active + counts.queued,
                    AetherAssetProcessorTab::Assets => counts.sources + counts.products,
                    AetherAssetProcessorTab::Logs => counts.failed + counts.warnings,
                    AetherAssetProcessorTab::Builders => counts.builders,
                    AetherAssetProcessorTab::Connections => counts.connections,
                };
                AetherItem {
                    key: tab.key().to_owned(),
                    label: tab.label().to_owned(),
                    icon: tab.icon().to_owned(),
                    badge: badge.to_string(),
                    has_badge: badge > 0,
                    selected: self.state.tab == tab,
                    ..AetherItem::default()
                }
            })
            .collect()
    }

    pub(crate) fn status_chips(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let counts = self.counts(cx);
        AetherAssetProcessorJobFilter::ALL
            .into_iter()
            .map(|filter| {
                let count = match filter {
                    AetherAssetProcessorJobFilter::All => counts.jobs,
                    AetherAssetProcessorJobFilter::Active => counts.active,
                    AetherAssetProcessorJobFilter::Queued => counts.queued,
                    AetherAssetProcessorJobFilter::Succeeded => counts.succeeded,
                    AetherAssetProcessorJobFilter::Warnings => counts.warnings,
                    AetherAssetProcessorJobFilter::Failed => counts.failed,
                };
                AetherItem {
                    key: filter.key().to_owned(),
                    label: filter.label().to_owned(),
                    count: count.to_string(),
                    active: self.state.job_filter == filter,
                    ..AetherItem::default()
                }
            })
            .collect()
    }

    pub(crate) fn diagnostic_tabs(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let mut count_state = self.state.clone();
        count_state.diagnostic_filter = AetherAssetProcessorDiagnosticFilter::All;
        let rows = diagnostic_rows(
            &count_state,
            cx.try_global::<EditorAssetBrowserStatus>(),
            cx.try_global::<OutputLogState>(),
        );
        AetherAssetProcessorDiagnosticFilter::ALL
            .into_iter()
            .map(|filter| {
                let count = rows
                    .iter()
                    .filter(|row| diagnostic_filter_matches(filter, row.kind.as_str()))
                    .count();
                AetherItem {
                    key: filter.key().to_owned(),
                    label: filter.label().to_owned(),
                    count: count.to_string(),
                    active: self.state.diagnostic_filter == filter,
                    ..AetherItem::default()
                }
            })
            .collect()
    }

    pub(crate) fn platform_options(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        platform_options(
            cx.try_global::<EditorAssetBrowserStatus>(),
            cx.try_global::<EditorCatalogProductsStatus>(),
        )
    }

    pub(crate) fn job_rows(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        job_rows(
            &self.state,
            cx.try_global::<EditorAssetBrowserStatus>(),
            cx.try_global::<EditorAssetProcessorActivity>(),
        )
    }

    pub(crate) fn source_groups(
        &self,
        cx: &mut Context<Self>,
    ) -> Vec<AetherAssetProcessorSourceGroup> {
        cx.try_global::<EditorAssetBrowserStatus>()
            .map_or_else(Vec::new, |status| source_groups(&self.state, status))
    }

    pub(crate) fn selected_source_detail(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AetherAssetProcessorSourceDetail> {
        selected_source_detail(
            &self.state,
            cx.try_global::<EditorAssetBrowserStatus>(),
            cx.try_global::<EditorJobInspection>(),
        )
    }

    pub(crate) fn selected_job_events(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        selected_job_events(
            &self.state,
            cx.try_global::<EditorAssetBrowserStatus>(),
            cx.try_global::<OutputLogState>(),
        )
    }

    pub(crate) fn selected_job_summary(&self, cx: &mut Context<Self>) -> Option<AetherItem> {
        selected_job_summary(&self.state, cx.try_global::<EditorAssetBrowserStatus>())
    }

    pub(crate) fn diagnostic_rows(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        diagnostic_rows(
            &self.state,
            cx.try_global::<EditorAssetBrowserStatus>(),
            cx.try_global::<OutputLogState>(),
        )
    }

    pub(crate) fn builders(&self, cx: &mut Context<Self>) -> AetherAssetProcessorBuilderProjection {
        builder_projection(
            cx.try_global::<EditorAssetBuilderCatalog>(),
            cx.try_global::<EditorSessionStatus>(),
        )
    }

    pub(crate) fn catalog_products(
        &self,
        cx: &mut Context<Self>,
    ) -> AetherAssetProcessorProductProjection {
        product_projection(&self.state, cx.try_global::<EditorCatalogProductsStatus>())
    }
}

fn asset_processor_counts(
    status: Option<&EditorAssetBrowserStatus>,
    catalog: Option<&EditorAssetBuilderCatalog>,
    products: Option<&EditorCatalogProductsStatus>,
    session: Option<&EditorSessionStatus>,
    activity: Option<&EditorAssetProcessorActivity>,
) -> AetherAssetProcessorCounts {
    let mut counts = AetherAssetProcessorCounts {
        builders: catalog.map_or(0, |catalog| catalog.builders.len()),
        connections: session.map_or(0, |session| asset_processor_connection_rows(session).len()),
        roots: status.map_or(0, |status| status.roots.len()),
        products: products.map_or(0, |products| products.entries.len()),
        ..AetherAssetProcessorCounts::default()
    };

    let Some(status) = status else {
        return counts;
    };
    counts.sources = visible_sources(status).count();
    for (entry, job) in status
        .entries
        .iter()
        .filter_map(|entry| entry.latest_job.as_ref().map(|job| (entry, job)))
    {
        counts.jobs += 1;
        match job_display_kind(entry, job).as_bytes() {
            b"queued" => counts.queued += 1,
            b"active" => counts.active += 1,
            b"warning" => counts.warnings += 1,
            b"succeeded" => counts.succeeded += 1,
            b"failed" => counts.failed += 1,
            _ => {}
        }
    }
    counts
}

fn asset_processor_header(
    counts: &AetherAssetProcessorCounts,
    activity: Option<&EditorAssetProcessorActivity>,
) -> AetherAssetProcessorHeader {
    if let Some(activity) = activity {
        if activity.degraded || !activity.ready || activity.state == ServiceHealthStateData::Failed
        {
            return AetherAssetProcessorHeader {
                icon: "warning",
                title: "Asset processor health needs attention".to_owned(),
                subtitle: activity.message.clone(),
                completed_label: activity_label(activity),
                percent_label: "health".to_owned(),
                percent: 0,
                progress_known: false,
                busy: false,
            };
        }
    }

    let live_activity = activity.filter(|activity| activity.busy());
    let source_reconcile_busy = activity_is_source_reconcile_busy(activity);
    let busy = counts.active > 0 || counts.queued > 0 || live_activity.is_some();
    let attention = counts.failed > 0 || counts.warnings > 0;
    let completed = counts.succeeded + counts.warnings;
    let percent = if counts.jobs == 0 {
        0
    } else {
        ((completed * 100) / counts.jobs).min(100) as u8
    };
    let connected = asset_processor_has_state(counts, activity);
    let subtitle = if let Some(activity) = live_activity
        && !activity.message.trim().is_empty()
    {
        activity.message.clone()
    } else if !connected {
        "No project asset processor activity is available".to_owned()
    } else if counts.jobs == 0 {
        format!(
            "{} source roots · {} products · {} builders · {} connections",
            counts.roots, counts.products, counts.builders, counts.connections
        )
    } else {
        format!(
            "{} active · {} queued · {} failed · {} warnings · {} connections",
            counts.active, counts.queued, counts.failed, counts.warnings, counts.connections
        )
    };
    AetherAssetProcessorHeader {
        icon: if !connected {
            "lan"
        } else if busy {
            "sync"
        } else if attention {
            "warning"
        } else {
            "check_circle"
        },
        title: if !connected {
            "Asset processor disconnected".to_owned()
        } else if source_reconcile_busy {
            "Indexing source assets".to_owned()
        } else if busy {
            "Processing assets".to_owned()
        } else if attention {
            "Pipeline needs attention".to_owned()
        } else {
            "Asset pipeline idle".to_owned()
        },
        subtitle,
        completed_label: if source_reconcile_busy && counts.jobs == 0 {
            live_activity.map(activity_label).unwrap_or_default()
        } else {
            format!("{completed} of {} completed", counts.jobs)
        },
        percent_label: if source_reconcile_busy && counts.jobs == 0 {
            "live".to_owned()
        } else {
            format!("{percent}%")
        },
        percent,
        // Only source reconciliation is an unbounded live operation. An empty
        // or disconnected job queue has a known 0/0 state and must not render
        // as an indeterminate operation.
        progress_known: !source_reconcile_busy || counts.jobs > 0,
        busy,
    }
}

fn activity_is_source_reconcile_busy(activity: Option<&EditorAssetProcessorActivity>) -> bool {
    activity.is_some_and(|activity| {
        activity.busy() && activity.operation.eq_ignore_ascii_case("source-reconcile")
    })
}

fn asset_processor_has_state(
    counts: &AetherAssetProcessorCounts,
    activity: Option<&EditorAssetProcessorActivity>,
) -> bool {
    activity.is_some()
        || counts.sources > 0
        || counts.jobs > 0
        || counts.products > 0
        || counts.builders > 0
        || counts.connections > 0
        || counts.roots > 0
}

fn activity_label(activity: &EditorAssetProcessorActivity) -> String {
    if activity.operation.is_empty() {
        activity.state.label().to_owned()
    } else {
        format!("{} · {}", activity.state.label(), activity.operation)
    }
}

fn job_rows(
    state: &AetherAssetProcessorState,
    status: Option<&EditorAssetBrowserStatus>,
    _activity: Option<&EditorAssetProcessorActivity>,
) -> Vec<AetherItem> {
    let mut rows = status.map_or_else(Vec::new, |status| {
        status
            .entries
            .iter()
            .filter_map(|entry| entry.latest_job.as_ref().map(|job| (entry, job)))
            .filter(|(entry, job)| job_matches_filters(state, entry, job))
            .map(|(entry, job)| job_row(entry, job, state.selected_job_id))
            .collect::<Vec<_>>()
    });
    rows.sort_by(|left, right| {
        job_sort_key(left.kind.as_str())
            .cmp(&job_sort_key(right.kind.as_str()))
            .then_with(|| left.src.cmp(&right.src))
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.tag.cmp(&right.tag))
    });
    rows
}

fn job_matches_filters(
    state: &AetherAssetProcessorState,
    entry: &AssetBrowserEntryData,
    job: &AssetBrowserJobData,
) -> bool {
    if state.platform_filter != "all" && job.platform != state.platform_filter {
        return false;
    }
    if !job_filter_matches(state.job_filter, entry, job) {
        return false;
    }
    let query = state.job_query.trim().to_ascii_lowercase();
    query.is_empty()
        || entry.source_path.to_ascii_lowercase().contains(&query)
        || job.job_key.to_ascii_lowercase().contains(&query)
        || job.platform.to_ascii_lowercase().contains(&query)
        || entry
            .schema_type
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .contains(&query)
}

fn job_filter_matches(
    filter: AetherAssetProcessorJobFilter,
    entry: &AssetBrowserEntryData,
    job: &AssetBrowserJobData,
) -> bool {
    let kind = job_display_kind(entry, job);
    match filter {
        AetherAssetProcessorJobFilter::All => true,
        AetherAssetProcessorJobFilter::Active => kind == "active",
        AetherAssetProcessorJobFilter::Queued => kind == "queued",
        AetherAssetProcessorJobFilter::Succeeded => kind == "succeeded",
        AetherAssetProcessorJobFilter::Warnings => kind == "warning",
        AetherAssetProcessorJobFilter::Failed => kind == "failed",
    }
}

fn job_row(
    entry: &AssetBrowserEntryData,
    job: &AssetBrowserJobData,
    selected_job_id: Option<i64>,
) -> AetherItem {
    let kind = job_display_kind(entry, job);
    AetherItem {
        key: job.job_id.to_string(),
        src: entry.source_path.clone(),
        name: asset_display_name(&entry.source_path),
        label: job.job_key.clone(),
        tag: job.platform.clone(),
        status: job_display_label(kind, job.status).to_owned(),
        kind: kind.to_owned(),
        icon: job_display_icon(kind).to_owned(),
        time: job_completion_label(job.status).to_owned(),
        unit: job_duration_label(job.status).to_owned(),
        count: diagnostic_summary(entry, job),
        idx: job
            .attempt_id
            .map_or_else(String::new, |attempt_id| attempt_id.to_string()),
        selected: selected_job_id == Some(job.job_id),
        ..AetherItem::default()
    }
}

fn job_sort_key(kind: &str) -> u8 {
    match kind.as_bytes() {
        b"active" => 0,
        b"queued" => 1,
        b"failed" => 2,
        b"warning" => 3,
        b"succeeded" => 4,
        _ => 4,
    }
}

fn source_groups(
    state: &AetherAssetProcessorState,
    status: &EditorAssetBrowserStatus,
) -> Vec<AetherAssetProcessorSourceGroup> {
    let mut by_folder = BTreeMap::<String, Vec<AetherItem>>::new();
    for entry in visible_sources(status).filter(|entry| source_matches_filter(state, entry)) {
        let folder = source_folder(&entry.source_path);
        by_folder
            .entry(if folder.is_empty() {
                "Root".to_owned()
            } else {
                folder
            })
            .or_default()
            .push(source_row(entry, state.selected_source_key.as_deref()));
    }
    by_folder
        .into_iter()
        .map(|(label, mut items)| {
            items.sort_by(|left, right| left.name.cmp(&right.name));
            AetherAssetProcessorSourceGroup { label, items }
        })
        .collect()
}

fn source_matches_filter(state: &AetherAssetProcessorState, entry: &AssetBrowserEntryData) -> bool {
    let query = state.source_query.trim().to_ascii_lowercase();
    query.is_empty()
        || entry.source_path.to_ascii_lowercase().contains(&query)
        || entry
            .schema_type
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .contains(&query)
}

fn source_row(entry: &AssetBrowserEntryData, selected_source_key: Option<&str>) -> AetherItem {
    let latest_job = entry.latest_job.as_ref();
    let status = latest_job
        .map(|job| source_status_label(entry, job))
        .unwrap_or_else(|| entry.status.label().to_owned());
    let source_key = source_key(entry);
    AetherItem {
        key: source_key.clone(),
        name: asset_display_name(&entry.source_path),
        src: entry.source_path.clone(),
        sub: entry
            .schema_type
            .clone()
            .unwrap_or_else(|| "source".to_owned()),
        icon: asset_icon(entry).to_owned(),
        status,
        kind: latest_job
            .map(|job| job_display_kind(entry, job).to_owned())
            .unwrap_or_else(|| "indexed".to_owned()),
        tag: latest_job
            .map(|job| job.platform.clone())
            .unwrap_or_default(),
        count: latest_job.map_or_else(String::new, |job| diagnostic_summary(entry, job)),
        selected: selected_source_key == Some(source_key.as_str()),
        ..AetherItem::default()
    }
}

fn selected_source_detail(
    state: &AetherAssetProcessorState,
    status: Option<&EditorAssetBrowserStatus>,
    inspection: Option<&EditorJobInspection>,
) -> Option<AetherAssetProcessorSourceDetail> {
    let status = status?;
    let entry = state
        .selected_source_key
        .as_deref()
        .and_then(|key| visible_sources(status).find(|entry| source_key(entry) == key))
        .or_else(|| visible_sources(status).next())?;
    let latest_job = entry.latest_job.as_ref();
    let inspected =
        latest_job.and_then(|job| inspection.filter(|inspection| inspection.job_id == job.job_id));
    let products = inspected
        .map(|inspection| {
            inspection
                .products
                .iter()
                .map(product_row)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let dependencies = inspected
        .map(|inspection| {
            inspection
                .dependencies
                .iter()
                .map(dependency_row)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let products_pending_inspection = latest_job.is_some() && inspected.is_none();
    Some(AetherAssetProcessorSourceDetail {
        source_key: source_key(entry),
        source_path: entry.source_path.clone(),
        source_root: source_root_selector(status, entry).unwrap_or_default(),
        source_root_path: source_root_path(status, entry).unwrap_or_default(),
        title: asset_display_name(&entry.source_path),
        folder: source_folder(&entry.source_path),
        icon: asset_icon(entry).to_owned(),
        status_label: latest_job
            .map(|job| source_status_label(entry, job))
            .unwrap_or_else(|| entry.status.label().to_owned()),
        status_icon: latest_job
            .map(|job| job_display_icon(job_display_kind(entry, job)).to_owned())
            .unwrap_or_else(|| "inventory_2".to_owned()),
        meta: source_meta_rows(status, entry, latest_job),
        products,
        dependencies,
        products_pending_inspection,
    })
}

fn source_meta_rows(
    status: &EditorAssetBrowserStatus,
    entry: &AssetBrowserEntryData,
    latest_job: Option<&AssetBrowserJobData>,
) -> Vec<AetherItem> {
    let mut rows = vec![
        meta_row("Source", &entry.source_path),
        meta_row(
            "Source root",
            &source_root_selector(status, entry).unwrap_or_default(),
        ),
        meta_row(
            "Filesystem root",
            &source_root_path(status, entry).unwrap_or_default(),
        ),
        meta_row("Schema", entry.schema_type.as_deref().unwrap_or("unknown")),
        meta_row("Content hash", &entry.content_hash),
        meta_row("Entry status", entry.status.label()),
    ];
    if let Some(job) = latest_job {
        rows.extend([
            meta_row("Job key", &job.job_key),
            meta_row("Platform", &job.platform),
            meta_row(
                "Attempt",
                &job.ordinal
                    .map_or_else(|| "none".to_owned(), |ordinal| ordinal.to_string()),
            ),
        ]);
    }
    rows
}

fn product_row(product: &JobProductData) -> AetherItem {
    AetherItem {
        key: product.product_id.to_string(),
        name: product.product_path.clone(),
        sub: format!(
            "{} v{}",
            product.product_format, product.product_format_version
        ),
        tag: product.asset_type.clone(),
        size: format_bytes(product.byte_length),
        icon: "deployed_code".to_owned(),
        ..AetherItem::default()
    }
}

fn dependency_row(dependency: &JobDependencyData) -> AetherItem {
    AetherItem {
        key: dependency.job_edge_id.to_string(),
        name: dependency.target.clone(),
        label: dependency.job_key.clone(),
        tag: dependency.platform.clone(),
        kind: dependency.dependency_kind.clone(),
        icon: "link".to_owned(),
        ..AetherItem::default()
    }
}

fn selected_job_events(
    state: &AetherAssetProcessorState,
    status: Option<&EditorAssetBrowserStatus>,
    output: Option<&OutputLogState>,
) -> Vec<AetherItem> {
    let selected = state
        .selected_job_id
        .and_then(|id| status.and_then(|status| find_job_entry(status, id)));
    let mut rows = output.map_or_else(Vec::new, |output| {
        let mut rows = output
            .messages()
            .iter()
            .filter(|message| output_message_is_asset_processor(message))
            .filter(|message| {
                selected.is_none_or(|(entry, job)| {
                    message.message.contains(&entry.source_path)
                        || message
                            .message
                            .contains(&asset_display_name(&entry.source_path))
                        || message.message.contains(&job.job_key)
                })
            })
            .map(|message| output_message_row(message))
            .collect::<Vec<_>>();
        rows.reverse();
        rows
    });
    if rows.is_empty()
        && let Some((entry, job)) = selected
    {
        rows = job_status_event_rows(entry, job);
    }
    rows
}

fn selected_job_summary(
    state: &AetherAssetProcessorState,
    status: Option<&EditorAssetBrowserStatus>,
) -> Option<AetherItem> {
    let status = status?;
    let (entry, job) = state
        .selected_job_id
        .and_then(|id| find_job_entry(status, id))
        .or_else(|| {
            status
                .entries
                .iter()
                .filter_map(|entry| entry.latest_job.as_ref().map(|job| (entry, job)))
                .next()
        })?;
    let mut row = job_row(entry, job, state.selected_job_id);
    row.value = source_key(entry);
    row.from = source_root_selector(status, entry).unwrap_or_default();
    row.to = source_root_path(status, entry).unwrap_or_default();
    Some(row)
}

fn diagnostic_rows(
    state: &AetherAssetProcessorState,
    status: Option<&EditorAssetBrowserStatus>,
    output: Option<&OutputLogState>,
) -> Vec<AetherItem> {
    let mut rows = Vec::new();
    if let Some(status) = status {
        for (entry, job) in status
            .entries
            .iter()
            .filter_map(|entry| entry.latest_job.as_ref().map(|job| (entry, job)))
        {
            if entry.diagnostics_count > 0 || job.error_count > 0 || job.warning_count > 0 {
                let kind = if job.error_count > 0
                    || matches!(
                        job.status,
                        AssetBrowserJobStatus::Failed | AssetBrowserJobStatus::Abandoned
                    ) {
                    "error"
                } else {
                    "warning"
                };
                rows.push(AetherItem {
                    key: format!("job-{}", job.job_id),
                    time: "job".to_owned(),
                    kind: kind.to_owned(),
                    tag: diagnostic_tag(kind).to_owned(),
                    src: "asset-processor".to_owned(),
                    name: entry.source_path.clone(),
                    msg: diagnostic_summary(entry, job),
                    icon: diagnostic_icon(kind).to_owned(),
                    ..AetherItem::default()
                });
            }
        }
    }
    if let Some(output) = output {
        rows.extend(
            output
                .messages()
                .iter()
                .filter(|message| output_message_is_asset_processor(message))
                .map(|message| output_message_row(message)),
        );
    }
    rows.retain(|row| diagnostic_filter_matches(state.diagnostic_filter, row.kind.as_str()));
    rows.reverse();
    rows
}

fn builder_projection(
    catalog: Option<&EditorAssetBuilderCatalog>,
    session: Option<&EditorSessionStatus>,
) -> AetherAssetProcessorBuilderProjection {
    let builders = catalog.map_or_else(Vec::new, |catalog| builder_rows(&catalog.builders));
    let schemas = catalog.map_or_else(Vec::new, |catalog| schema_rows(&catalog.source_schemas));
    let processes = session.map_or_else(Vec::new, asset_processor_connection_rows);
    let allow_list = session.map_or_else(Vec::new, asset_processor_allow_list);
    let reject_list = session.map_or_else(Vec::new, asset_processor_reject_list);
    AetherAssetProcessorBuilderProjection {
        builders,
        schemas,
        processes,
        allow_list,
        reject_list,
    }
}

fn product_projection(
    state: &AetherAssetProcessorState,
    products: Option<&EditorCatalogProductsStatus>,
) -> AetherAssetProcessorProductProjection {
    let Some(products) = products else {
        return AetherAssetProcessorProductProjection {
            platform: DEFAULT_ASSET_PRODUCT_PLATFORM.to_owned(),
            products: Vec::new(),
            error: None,
        };
    };
    let platform = products.platform.clone();
    let mut rows = products
        .entries
        .iter()
        .filter(|entry| catalog_product_matches_filter(state, entry))
        .map(catalog_product_row)
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.src.cmp(&right.src))
            .then_with(|| left.tag.cmp(&right.tag))
            .then_with(|| left.key.cmp(&right.key))
    });
    AetherAssetProcessorProductProjection {
        platform,
        products: rows,
        error: products.status_error.clone(),
    }
}

fn catalog_product_matches_filter(
    state: &AetherAssetProcessorState,
    entry: &CatalogProductData,
) -> bool {
    if state.platform_filter != "all" && entry.platform != state.platform_filter {
        return false;
    }
    let query = state.source_query.trim().to_ascii_lowercase();
    query.is_empty()
        || entry.product_path.to_ascii_lowercase().contains(&query)
        || entry.source_path.to_ascii_lowercase().contains(&query)
        || entry.job_key.to_ascii_lowercase().contains(&query)
        || entry.product_format.to_ascii_lowercase().contains(&query)
        || entry.asset_type.to_ascii_lowercase().contains(&query)
}

fn catalog_product_row(entry: &CatalogProductData) -> AetherItem {
    let version = if entry.product_format_version == 0 {
        String::new()
    } else {
        format!("v{}", entry.product_format_version)
    };
    AetherItem {
        key: entry.product_id.to_string(),
        name: entry.product_path.clone(),
        src: entry.source_path.clone(),
        label: entry.product_format.clone(),
        tag: entry.platform.clone(),
        status: entry.job_key.clone(),
        kind: "succeeded".to_owned(),
        sub: entry.asset_type.clone(),
        count: format_bytes(entry.byte_length),
        value: entry.content_hash.clone(),
        unit: version,
        id: entry.asset_guid.clone(),
        idx: entry.sub_id.to_string(),
        icon: "deployed_code".to_owned(),
        ..AetherItem::default()
    }
}

fn builder_rows(builders: &[AssetBuilderData]) -> Vec<AetherItem> {
    let mut rows = builders
        .iter()
        .map(|builder| AetherItem {
            key: builder.builder_guid.clone(),
            name: builder.name.clone(),
            label: format!("v{}", builder.version),
            sub: asset_builder_patterns_label(&builder.patterns),
            count: format_count(builder.source_schema_types.len(), "schema"),
            tag: if builder.source_schema_types.is_empty() {
                "any".to_owned()
            } else {
                builder.source_schema_types.join(", ")
            },
            icon: "build".to_owned(),
            ..AetherItem::default()
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left.name.cmp(&right.name));
    rows
}

fn schema_rows(schemas: &[AssetSourceSchemaData]) -> Vec<AetherItem> {
    let mut rows = schemas
        .iter()
        .map(|schema| AetherItem {
            key: schema.schema_type.clone(),
            name: schema.label.clone(),
            label: schema.category.clone(),
            sub: schema_authoring_label(schema),
            tag: schema.owner.clone(),
            count: format_count(schema.file_templates.len(), "template"),
            icon: "schema".to_owned(),
            ..AetherItem::default()
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.label
            .cmp(&right.label)
            .then_with(|| left.name.cmp(&right.name))
    });
    rows
}

fn asset_processor_connection_rows(status: &EditorSessionStatus) -> Vec<AetherItem> {
    let mut rows = current_session_processes(status)
        .into_iter()
        .map(|process| AetherItem {
            key: format!(
                "{}:{}:{}",
                process.owner_id, process.owner_root, process.service_name
            ),
            name: process.service_name.clone(),
            label: asset_processor_connection_type(process.role).to_owned(),
            tag: asset_processor_connection_platform(process.role).to_owned(),
            sub: process
                .pid
                .map(|pid| format!("pid {pid}"))
                .unwrap_or_else(|| format!("run {}", &process.run.to_string()[..8])),
            status: process.state.label().to_owned(),
            kind: process_state_kind(process.state).to_owned(),
            value: process.structured_log.clone(),
            icon: asset_processor_connection_icon(process.role).to_owned(),
            active: matches!(
                process.state,
                SessionProcessStateData::Planned
                    | SessionProcessStateData::Starting
                    | SessionProcessStateData::Running
            ),
            ..AetherItem::default()
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left.name.cmp(&right.name));
    rows
}

fn current_session_processes(status: &EditorSessionStatus) -> Vec<&SessionProcessData> {
    let mut processes = status.processes.iter().collect::<Vec<_>>();
    processes.sort_by(|left, right| {
        left.service_name
            .cmp(&right.service_name)
            .then_with(|| left.owner_id.cmp(&right.owner_id))
            .then_with(|| left.owner_root.cmp(&right.owner_root))
    });
    processes
}

fn asset_processor_allow_list(status: &EditorSessionStatus) -> Vec<String> {
    let mut items = vec![format!("session: {}", status.session_slug)];
    if !status.project_root.trim().is_empty() {
        items.push(format!("project: {}", status.project_root));
    }
    if !status.workspace_root.trim().is_empty() {
        items.push(format!("workspace: {}", status.workspace_root));
    }
    items.push(format_count(
        current_session_processes(status).len(),
        "managed endpoint",
    ));
    items
}

fn asset_processor_reject_list(status: &EditorSessionStatus) -> Vec<String> {
    let mut items = current_session_processes(status)
        .into_iter()
        .filter(|process| process.state == SessionProcessStateData::Failed)
        .map(|process| {
            process
                .failure
                .as_deref()
                .filter(|failure| !failure.trim().is_empty())
                .map_or_else(
                    || format!("{} failed", process.service_name),
                    |failure| format!("{}: {failure}", process.service_name),
                )
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        items.push("No rejected endpoints reported".to_owned());
    }
    items
}

fn platform_options(
    status: Option<&EditorAssetBrowserStatus>,
    products: Option<&EditorCatalogProductsStatus>,
) -> Vec<AetherItem> {
    let mut platforms = BTreeSet::new();
    if let Some(status) = status {
        for job in status
            .entries
            .iter()
            .filter_map(|entry| entry.latest_job.as_ref())
        {
            if !job.platform.trim().is_empty() {
                platforms.insert(job.platform.clone());
            }
        }
    }
    if let Some(products) = products {
        if !products.platform.trim().is_empty() {
            platforms.insert(products.platform.clone());
        }
        for product in &products.entries {
            if !product.platform.trim().is_empty() {
                platforms.insert(product.platform.clone());
            }
        }
    }
    std::iter::once(AetherItem {
        key: "all".to_owned(),
        label: "All platforms".to_owned(),
        ..AetherItem::default()
    })
    .chain(platforms.into_iter().map(|platform| AetherItem {
        key: platform.clone(),
        label: platform,
        ..AetherItem::default()
    }))
    .collect()
}

fn find_job_entry(
    status: &EditorAssetBrowserStatus,
    job_id: i64,
) -> Option<(&AssetBrowserEntryData, &AssetBrowserJobData)> {
    status.entries.iter().find_map(|entry| {
        entry
            .latest_job
            .as_ref()
            .filter(|job| job.job_id == job_id)
            .map(|job| (entry, job))
    })
}

fn visible_sources(
    status: &EditorAssetBrowserStatus,
) -> impl Iterator<Item = &AssetBrowserEntryData> {
    status.entries.iter().filter(|entry| {
        entry
            .schema_type
            .as_deref()
            .is_some_and(|schema| !schema.trim().is_empty())
    })
}

fn source_key(entry: &AssetBrowserEntryData) -> String {
    format!("{}:{}", entry.root_id, entry.source_path)
}

fn source_root_selector(
    status: &EditorAssetBrowserStatus,
    entry: &AssetBrowserEntryData,
) -> Option<String> {
    status
        .roots
        .iter()
        .find(|root| root.root_id == entry.root_id)
        .map(|root| root.portable_key.clone())
}

fn source_root_path(
    status: &EditorAssetBrowserStatus,
    entry: &AssetBrowserEntryData,
) -> Option<String> {
    status
        .roots
        .iter()
        .find(|root| root.root_id == entry.root_id)
        .map(|root| root.source_root.clone())
}

fn source_folder(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    normalized
        .rsplit_once('/')
        .map_or_else(String::new, |(folder, _)| folder.to_owned())
}

fn asset_display_name(path: &str) -> String {
    path.replace('\\', "/")
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_owned()
}

fn asset_icon(entry: &AssetBrowserEntryData) -> &'static str {
    let schema = entry
        .schema_type
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let path = entry.source_path.to_ascii_lowercase();
    if schema.contains("prefab") || path.ends_with(".prefab") || path.ends_with(".prefab.ron") {
        "widgets"
    } else if schema.contains("material") || path.ends_with(".material") {
        "palette"
    } else if schema.contains("texture")
        || path.ends_with(".png")
        || path.ends_with(".jpg")
        || path.ends_with(".tif")
        || path.ends_with(".dds")
    {
        "image"
    } else if schema.contains("script") || path.ends_with(".lua") || path.ends_with(".rs") {
        "code"
    } else if schema.contains("audio") || path.ends_with(".wav") || path.ends_with(".ogg") {
        "graphic_eq"
    } else if schema.contains("shader") {
        "gradient"
    } else if schema.contains("animation") || schema.contains("motion") {
        "animation"
    } else {
        "inventory_2"
    }
}

const fn job_status_icon(status: AssetBrowserJobStatus) -> &'static str {
    match status {
        AssetBrowserJobStatus::Queued => "history",
        AssetBrowserJobStatus::Leased => "sync",
        AssetBrowserJobStatus::Succeeded => "check_circle",
        AssetBrowserJobStatus::Failed => "error",
        AssetBrowserJobStatus::Abandoned => "warning",
    }
}

fn job_display_kind(entry: &AssetBrowserEntryData, job: &AssetBrowserJobData) -> &'static str {
    match job.status {
        AssetBrowserJobStatus::Queued => "queued",
        AssetBrowserJobStatus::Leased => "active",
        AssetBrowserJobStatus::Succeeded
            if job.warning_count > 0 || entry.diagnostics_count > 0 =>
        {
            "warning"
        }
        AssetBrowserJobStatus::Succeeded => "succeeded",
        AssetBrowserJobStatus::Failed | AssetBrowserJobStatus::Abandoned => "failed",
    }
}

fn job_display_label(kind: &str, status: AssetBrowserJobStatus) -> &'static str {
    match kind {
        // "warning" and the unrecognised-kind fallback are display states the
        // job pipeline synthesises; the rest are the job status itself.
        "warning" => "warning",
        "queued" | "active" | "succeeded" | "failed" => status.label(),
        _ => "unknown",
    }
}

fn job_display_icon(kind: &str) -> &'static str {
    match kind {
        "queued" => "schedule",
        "active" => "sync",
        "warning" => "warning",
        "succeeded" => "check_circle",
        "failed" => "error",
        _ => "work_history",
    }
}

const fn job_duration_label(_status: AssetBrowserJobStatus) -> &'static str {
    "-"
}

const fn job_completion_label(status: AssetBrowserJobStatus) -> &'static str {
    match status {
        AssetBrowserJobStatus::Queued => "queued",
        AssetBrowserJobStatus::Leased => "in progress",
        AssetBrowserJobStatus::Succeeded => "latest",
        AssetBrowserJobStatus::Failed | AssetBrowserJobStatus::Abandoned => "-",
    }
}

fn source_status_label(entry: &AssetBrowserEntryData, job: &AssetBrowserJobData) -> String {
    if entry.diagnostics_count > 0 || job.error_count > 0 || job.warning_count > 0 {
        // Diagnostics outrank the job status: a succeeded job with warnings
        // still needs a look.
        return if job.error_count > 0 {
            "failed".to_owned()
        } else {
            "warning".to_owned()
        };
    }
    job.status.label().to_owned()
}

fn diagnostic_summary(entry: &AssetBrowserEntryData, job: &AssetBrowserJobData) -> String {
    let mut parts = Vec::new();
    if job.error_count > 0 {
        parts.push(format_count(job.error_count as usize, "error"));
    }
    if job.warning_count > 0 {
        parts.push(format_count(job.warning_count as usize, "warning"));
    }
    if entry.diagnostics_count > 0 {
        parts.push(format_count(entry.diagnostics_count as usize, "diagnostic"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        parts.join(" · ")
    }
}

fn output_message_is_asset_processor(message: &OutputLogMessage) -> bool {
    let source = message.source.to_ascii_lowercase();
    source.contains("assetprocessor")
        || source.contains("asset-processor")
        || source.contains("asset processor")
        || source.contains("builder")
        || source.contains("job scheduler")
        || source.contains("jobscheduler")
        || source.contains("file watcher")
        || source.contains("filewatcher")
}

fn job_status_event_rows(
    entry: &AssetBrowserEntryData,
    job: &AssetBrowserJobData,
) -> Vec<AetherItem> {
    let kind = job_display_kind(entry, job);
    let tag = if kind == "warning" {
        diagnostic_tag("warning")
    } else if kind == "failed" {
        diagnostic_tag("error")
    } else {
        diagnostic_tag("message")
    };
    let diagnostic = diagnostic_summary(entry, job);
    let mut rows = vec![AetherItem {
        key: format!("job-status-{}", job.job_id),
        time: job_completion_label(job.status).to_owned(),
        kind: if kind == "failed" {
            "error".to_owned()
        } else if kind == "warning" {
            "warning".to_owned()
        } else {
            "message".to_owned()
        },
        tag: tag.to_owned(),
        src: "asset-processor".to_owned(),
        msg: format!(
            "{} [{}] · {}",
            entry.source_path,
            job.platform,
            job_display_label(kind, job.status)
        ),
        icon: if kind == "failed" {
            diagnostic_icon("error")
        } else if kind == "warning" {
            diagnostic_icon("warning")
        } else {
            diagnostic_icon("message")
        }
        .to_owned(),
        ..AetherItem::default()
    }];
    if !diagnostic.is_empty() {
        rows.push(AetherItem {
            key: format!("job-diagnostics-{}", job.job_id),
            time: "job".to_owned(),
            kind: if job.error_count > 0 {
                "error".to_owned()
            } else {
                "warning".to_owned()
            },
            tag: if job.error_count > 0 { "ERR" } else { "WARN" }.to_owned(),
            src: "asset-processor".to_owned(),
            msg: diagnostic,
            icon: if job.error_count > 0 {
                "error".to_owned()
            } else {
                "warning".to_owned()
            },
            ..AetherItem::default()
        });
    }
    rows
}

fn output_message_row(message: &OutputLogMessage) -> AetherItem {
    let kind = diagnostic_kind_for_level(message.level);
    AetherItem {
        key: format!(
            "{:?}-{}-{}",
            message.timestamp, message.source, message.message
        ),
        time: format_system_time(message.timestamp),
        kind: kind.to_owned(),
        tag: diagnostic_tag(kind).to_owned(),
        src: message.source.clone(),
        msg: message.message.clone(),
        icon: diagnostic_icon(kind).to_owned(),
        ..AetherItem::default()
    }
}

const fn diagnostic_kind_for_level(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Error => "error",
        LogLevel::Warn => "warning",
        LogLevel::Trace | LogLevel::Debug | LogLevel::Info => "message",
    }
}

fn diagnostic_tag(kind: &str) -> &'static str {
    match kind.as_bytes() {
        b"error" => "ERR",
        b"warning" => "WARN",
        _ => "MSG",
    }
}

fn diagnostic_icon(kind: &str) -> &'static str {
    match kind.as_bytes() {
        b"error" => "error",
        b"warning" => "warning",
        _ => "info",
    }
}

fn diagnostic_filter_matches(filter: AetherAssetProcessorDiagnosticFilter, kind: &str) -> bool {
    match filter {
        AetherAssetProcessorDiagnosticFilter::All => true,
        AetherAssetProcessorDiagnosticFilter::Messages => kind == "message",
        AetherAssetProcessorDiagnosticFilter::Warnings => kind == "warning",
        AetherAssetProcessorDiagnosticFilter::Errors => kind == "error",
    }
}

fn meta_row(label: &str, value: &str) -> AetherItem {
    AetherItem {
        label: label.to_owned(),
        value: value.to_owned(),
        ..AetherItem::default()
    }
}

fn asset_builder_patterns_label(patterns: &[AssetBuilderPatternData]) -> String {
    if patterns.is_empty() {
        return "patterns: none".to_owned();
    }
    patterns
        .iter()
        .map(|pattern| {
            format!(
                "{} {}",
                asset_builder_pattern_kind_label(pattern.kind),
                pattern.pattern
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

const fn asset_builder_pattern_kind_label(kind: AssetBuilderPatternKindData) -> &'static str {
    match kind {
        AssetBuilderPatternKindData::Wildcard => "wildcard",
        AssetBuilderPatternKindData::Regex => "regex",
    }
}

fn schema_authoring_label(schema: &AssetSourceSchemaData) -> String {
    match &schema.authoring {
        AssetSourceSchemaAuthoringData::File { workflow } => {
            let extensions = if workflow.extensions.is_empty() {
                "no extensions".to_owned()
            } else {
                workflow.extensions.join(", ")
            };
            format!(
                "file · root {} · {} · create {}",
                workflow.source_root,
                extensions,
                if workflow.can_create { "yes" } else { "no" }
            )
        }
        AssetSourceSchemaAuthoringData::ProjectDocument { schema_type } => {
            format!("project document · {schema_type}")
        }
    }
}

fn format_count(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

fn format_bytes(bytes: i64) -> String {
    if bytes < 0 {
        return "-".to_owned();
    }
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= GB {
        format!("{:.1} GB", bytes / GB)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes / MB)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes / KB)
    } else {
        format!("{} B", bytes as i64)
    }
}

fn format_system_time(timestamp: std::time::SystemTime) -> String {
    timestamp.duration_since(std::time::UNIX_EPOCH).map_or_else(
        |_| "time".to_owned(),
        |duration| {
            let seconds = duration.as_secs() % 86_400;
            format!(
                "{:02}:{:02}:{:02}",
                seconds / 3_600,
                (seconds / 60) % 60,
                seconds % 60
            )
        },
    )
}

const fn process_state_kind(state: SessionProcessStateData) -> &'static str {
    match state {
        SessionProcessStateData::Planned | SessionProcessStateData::Exited => "idle",
        SessionProcessStateData::Starting | SessionProcessStateData::Running => "active",
        SessionProcessStateData::Failed => "failed",
    }
}

const fn asset_processor_connection_type(role: SessionServiceRoleData) -> &'static str {
    match role {
        SessionServiceRoleData::AssetProcessor | SessionServiceRoleData::Worker => "Builder",
        SessionServiceRoleData::RuntimeHost => "Game",
        SessionServiceRoleData::Editor
        | SessionServiceRoleData::Daemon
        | SessionServiceRoleData::SessionSupervisor
        | SessionServiceRoleData::ProjectHost => "Editor",
        SessionServiceRoleData::Unknown => "Service",
    }
}

const fn asset_processor_connection_platform(role: SessionServiceRoleData) -> &'static str {
    match role {
        SessionServiceRoleData::RuntimeHost
        | SessionServiceRoleData::AssetProcessor
        | SessionServiceRoleData::Worker => "pc",
        _ => "-",
    }
}

const fn asset_processor_connection_icon(role: SessionServiceRoleData) -> &'static str {
    match role {
        SessionServiceRoleData::AssetProcessor | SessionServiceRoleData::Worker => {
            "precision_manufacturing"
        }
        SessionServiceRoleData::RuntimeHost => "sports_esports",
        SessionServiceRoleData::Editor
        | SessionServiceRoleData::Daemon
        | SessionServiceRoleData::SessionSupervisor
        | SessionServiceRoleData::ProjectHost => "edit_document",
        SessionServiceRoleData::Unknown => "lan",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_editor_ui::panels::{
        AssetSourceFileWorkflowData, AssetSourceSchemaAuthoringData, WorkspaceRootData,
    };

    #[test]
    fn asset_processor_header_has_no_project_workflow_input() {
        let header = asset_processor_header(&AetherAssetProcessorCounts::default(), None);

        assert_eq!(header.icon, "lan");
        assert_eq!(header.title, "Asset processor disconnected");
        assert_eq!(
            header.subtitle,
            "No project asset processor activity is available"
        );
        assert_eq!(header.completed_label, "0 of 0 completed");
        assert_eq!(header.percent_label, "0%");
        assert_eq!(header.percent, 0);
        assert!(header.progress_known);
        assert!(!header.busy);
    }

    #[test]
    fn asset_processor_header_reports_source_index_without_fake_progress() {
        let activity = EditorAssetProcessorActivity::new(
            "session-a",
            ServiceHealthStateData::Busy,
            "source-reconcile",
            "scanned 240000 entries in Project Assets",
        );
        let counts = AetherAssetProcessorCounts {
            roots: 2,
            sources: 120,
            products: 12,
            ..AetherAssetProcessorCounts::default()
        };

        let header = asset_processor_header(&counts, Some(&activity));

        assert_eq!(header.title, "Indexing source assets");
        assert_eq!(header.subtitle, "scanned 240000 entries in Project Assets");
        assert_eq!(header.percent_label, "live");
        assert!(!header.progress_known);
        assert!(header.busy);
    }

    #[test]
    fn asset_processor_header_uses_loaded_asset_state_directly() {
        let counts = AetherAssetProcessorCounts {
            roots: 2,
            sources: 64,
            products: 16,
            builders: 9,
            connections: 2,
            ..AetherAssetProcessorCounts::default()
        };

        let header = asset_processor_header(&counts, None);

        assert_eq!(header.title, "Asset pipeline idle");
        assert!(header.subtitle.contains("2 source roots"));
        assert!(header.subtitle.contains("16 products"));
        assert!(!header.busy);
    }

    #[test]
    fn asset_processor_jobs_list_projects_latest_job_states() {
        let status = EditorAssetBrowserStatus::new(
            "session-a",
            Vec::new(),
            vec![
                asset_entry_with_job(
                    1,
                    "textures/rock.png",
                    "Texture",
                    "pc",
                    AssetBrowserJobStatus::Succeeded,
                ),
                asset_entry_with_job(
                    2,
                    "prefabs/player.prefab",
                    "Prefab",
                    "pc",
                    AssetBrowserJobStatus::Leased,
                ),
            ],
            None,
        );

        let rows = job_rows(&AetherAssetProcessorState::new(), Some(&status), None);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].key, "2");
        assert_eq!(rows[0].status, "leased");
        assert_eq!(rows[0].kind, "active");
        assert_eq!(rows[1].key, "1");
        assert_eq!(rows[1].status, "succeeded");
    }

    #[test]
    fn asset_processor_builder_list_projects_catalog_patterns() {
        let catalog = EditorAssetBuilderCatalog::new(
            vec![AssetBuilderData {
                name: "Texture Builder".to_owned(),
                builder_guid: "texture-builder".to_owned(),
                version: 1,
                patterns: vec![AssetBuilderPatternData {
                    kind: AssetBuilderPatternKindData::Wildcard,
                    pattern: "*.png".to_owned(),
                }],
                source_schema_types: vec!["az.texture.Source".to_owned()],
            }],
            vec![AssetSourceSchemaData {
                schema_type: "az.texture.Source".to_owned(),
                owner: "azoth.render".to_owned(),
                label: "Texture".to_owned(),
                category: "Rendering".to_owned(),
                authoring: AssetSourceSchemaAuthoringData::File {
                    workflow: AssetSourceFileWorkflowData {
                        source_root: "project".to_owned(),
                        default_path_prefix: "textures".to_owned(),
                        extensions: vec!["png".to_owned(), "tif".to_owned()],
                        can_create: true,
                        can_edit: true,
                    },
                },
                file_templates: Vec::new(),
            }],
        );

        let projection = builder_projection(Some(&catalog), None);

        assert_eq!(projection.builders.len(), 1);
        assert_eq!(projection.builders[0].name, "Texture Builder");
        assert_eq!(projection.builders[0].sub, "wildcard *.png");
        assert_eq!(projection.builders[0].count, "1 schema");
        assert_eq!(projection.schemas.len(), 1);
        assert!(projection.schemas[0].sub.contains("root project"));
    }

    #[test]
    fn asset_processor_connections_render_one_current_row_per_service() {
        let status = EditorSessionStatus {
            session_id: "session-a".to_owned(),
            project_id: "project-a".to_owned(),
            session_slug: "main".to_owned(),
            project_root: "projects/sample-game".to_owned(),
            workspace_root: "projects/sample-game".to_owned(),
            run_dir: "data/sessions/session-a".to_owned(),
            state: EditorSessionStateData::Active,
            failure_reason: None,
            services_count: 2,
            processes: vec![
                session_process(
                    "asset-processor",
                    SessionServiceRoleData::AssetProcessor,
                    2,
                    SessionProcessStateData::Running,
                    Some(24_408),
                ),
                session_process(
                    "project-host",
                    SessionServiceRoleData::ProjectHost,
                    2,
                    SessionProcessStateData::Running,
                    Some(39_508),
                ),
            ],
        };

        let rows = asset_processor_connection_rows(&status);
        let keys = rows.iter().map(|row| row.key.as_str()).collect::<Vec<_>>();

        assert_eq!(
            keys,
            [
                "azoth:projects/sample-game:asset-processor",
                "azoth:projects/sample-game:project-host",
            ]
        );
        assert!(rows.iter().all(|row| row.status == "running"));
        assert_eq!(
            asset_processor_allow_list(&status)
                .last()
                .map(String::as_str),
            Some("2 managed endpoints")
        );
    }

    #[test]
    fn asset_processor_filter_state_filters_jobs_by_query_status_and_platform() {
        let status = EditorAssetBrowserStatus::new(
            "session-a",
            Vec::new(),
            vec![
                asset_entry_with_job(
                    1,
                    "textures/rock.png",
                    "Texture",
                    "pc",
                    AssetBrowserJobStatus::Succeeded,
                ),
                asset_entry_with_job(
                    2,
                    "textures/sky.tif",
                    "Texture",
                    "android",
                    AssetBrowserJobStatus::Failed,
                ),
                asset_entry_with_job(
                    3,
                    "prefabs/player.prefab",
                    "Prefab",
                    "pc",
                    AssetBrowserJobStatus::Leased,
                ),
            ],
            None,
        );
        let mut state = AetherAssetProcessorState::new();
        state.apply_action(AetherAssetProcessorAction::SetJobQuery(
            "texture".to_owned(),
        ));
        state.apply_action(AetherAssetProcessorAction::SetPlatformFilter(
            "android".to_owned(),
        ));
        state.apply_action(AetherAssetProcessorAction::SetJobFilter(
            AetherAssetProcessorJobFilter::Failed,
        ));

        let rows = job_rows(&state, Some(&status), None);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, "2");
        assert_eq!(rows[0].src, "textures/sky.tif");
    }

    #[test]
    fn asset_processor_jobs_list_only_includes_real_asset_jobs() {
        let activity = EditorAssetProcessorActivity::new(
            "session-a",
            ServiceHealthStateData::Busy,
            "source-reconcile",
            "scanned 512 entries in Project Assets",
        );

        let rows = job_rows(&AetherAssetProcessorState::new(), None, Some(&activity));

        assert!(rows.is_empty());
    }

    #[test]
    fn asset_processor_selected_source_uses_inspected_products_and_dependencies() {
        let status = EditorAssetBrowserStatus::new(
            "session-a",
            vec![WorkspaceRootData {
                workspace_root_id: 1,
                root_id: 1,
                declared_root_id: "project.assets".to_owned(),
                owner_id: "project".to_owned(),
                source_root: "project/assets".to_owned(),
                display_name: "Project Assets".to_owned(),
                portable_key: "project".to_owned(),
                output_prefix: "assets".to_owned(),
            }],
            vec![asset_entry_with_job(
                42,
                "materials/rock.material",
                "Material",
                "pc",
                AssetBrowserJobStatus::Succeeded,
            )],
            None,
        );
        let mut state = AetherAssetProcessorState::new();
        state.apply_action(AetherAssetProcessorAction::SelectSource {
            source_key: "1:materials/rock.material".to_owned(),
            job_id: Some(42),
        });
        let inspection = EditorJobInspection::new(
            42,
            Some(7),
            "materials/rock.material",
            "Material",
            "pc",
            Some(1),
            AssetBrowserJobStatus::Succeeded,
            vec![JobDependencyData {
                job_edge_id: 9,
                target: "textures/rock.png".to_owned(),
                job_key: "Texture".to_owned(),
                platform: "pc".to_owned(),
                dependency_kind: "fingerprint".to_owned(),
            }],
            vec![JobProductData {
                product_id: 8,
                product_path: "materials/rock.azmaterial".to_owned(),
                asset_type: "azmaterial".to_owned(),
                sub_id: 0,
                product_format: "azoth.material".to_owned(),
                product_format_version: 1,
                content_hash: "abc".to_owned(),
                byte_length: 2048,
            }],
        );

        let detail = selected_source_detail(&state, Some(&status), Some(&inspection)).unwrap();

        assert_eq!(detail.title, "rock.material");
        assert_eq!(detail.products[0].name, "materials/rock.azmaterial");
        assert_eq!(detail.products[0].size, "2.0 KB");
        assert_eq!(detail.dependencies[0].name, "textures/rock.png");
        assert!(!detail.products_pending_inspection);
    }

    #[test]
    fn asset_processor_catalog_products_project_product_assets() {
        let products = EditorCatalogProductsStatus::new(
            "session-a",
            "pc",
            vec![
                CatalogProductData {
                    job_id: 10,
                    product_id: 20,
                    asset_guid: "11111111-1111-1111-1111-111111111111".to_owned(),
                    source_path: "materials/rock.material".to_owned(),
                    builder_guid: "22222222-2222-2222-2222-222222222222".to_owned(),
                    job_key: "Material".to_owned(),
                    platform: "pc".to_owned(),
                    product_path: "materials/rock.azmaterial".to_owned(),
                    asset_type: "33333333-3333-3333-3333-333333333333".to_owned(),
                    sub_id: 7,
                    product_format: "azoth.material".to_owned(),
                    product_format_version: 2,
                    content_hash: "ab".repeat(32),
                    byte_length: 4096,
                },
                CatalogProductData {
                    job_id: 11,
                    product_id: 21,
                    asset_guid: "44444444-4444-4444-4444-444444444444".to_owned(),
                    source_path: "textures/rock.png".to_owned(),
                    builder_guid: "55555555-5555-5555-5555-555555555555".to_owned(),
                    job_key: "Texture".to_owned(),
                    platform: "android".to_owned(),
                    product_path: "textures/rock.ktx2".to_owned(),
                    asset_type: "66666666-6666-6666-6666-666666666666".to_owned(),
                    sub_id: 0,
                    product_format: "azoth.texture".to_owned(),
                    product_format_version: 1,
                    content_hash: "cd".repeat(32),
                    byte_length: 2048,
                },
            ],
        );
        let mut state = AetherAssetProcessorState::new();
        state.apply_action(AetherAssetProcessorAction::SetPlatformFilter(
            "pc".to_owned(),
        ));
        state.apply_action(AetherAssetProcessorAction::SetSourceQuery(
            "material".to_owned(),
        ));

        let projection = product_projection(&state, Some(&products));

        assert_eq!(projection.platform, "pc");
        assert_eq!(projection.products.len(), 1);
        assert_eq!(projection.products[0].name, "materials/rock.azmaterial");
        assert_eq!(projection.products[0].src, "materials/rock.material");
        assert_eq!(projection.products[0].label, "azoth.material");
        assert_eq!(projection.products[0].unit, "v2");
        assert_eq!(projection.products[0].idx, "7");
        assert_eq!(projection.products[0].count, "4.0 KB");
        assert_eq!(projection.error, None);

        let failed = EditorCatalogProductsStatus::error("session-a", "pc", "products offline");
        let projection = product_projection(&AetherAssetProcessorState::new(), Some(&failed));
        assert_eq!(projection.error.as_deref(), Some("products offline"));
    }

    fn session_process(
        service_name: &str,
        role: SessionServiceRoleData,
        run_label: u8,
        state: SessionProcessStateData,
        pid: Option<u32>,
    ) -> SessionProcessData {
        SessionProcessData {
            owner_id: "azoth".to_owned(),
            owner_root: "projects/sample-game".to_owned(),
            service_name: service_name.to_owned(),
            role,
            run: uuid::Uuid::from_bytes([run_label; 16]),
            state,
            pid,
            exit_code: None,
            failure: None,
            structured_log: format!("run {run_label}"),
        }
    }

    fn asset_entry_with_job(
        attempt_id: i64,
        source_path: &str,
        job_key: &str,
        platform: &str,
        status: AssetBrowserJobStatus,
    ) -> AssetBrowserEntryData {
        AssetBrowserEntryData {
            entry_id: attempt_id,
            workspace_id: 1,
            asset_guid: format!("00000000-0000-0000-0000-{attempt_id:012}"),
            root_id: 1,
            source_path: source_path.to_owned(),
            schema_type: Some(format!("az.{job_key}.Source")),
            content_hash: format!("hash-{attempt_id}"),
            status: AssetBrowserEntryStatus::Clean,
            diagnostics_count: 0,
            latest_job: Some(AssetBrowserJobData {
                job_id: attempt_id,
                attempt_id: Some(attempt_id),
                job_key: job_key.to_owned(),
                platform: platform.to_owned(),
                ordinal: Some(1),
                status,
                error_count: if status == AssetBrowserJobStatus::Failed {
                    1
                } else {
                    0
                },
                warning_count: 0,
            }),
        }
    }
}
