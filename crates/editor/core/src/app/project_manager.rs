// Expansion drops `cfg(test)`-only names and adds unused ones; it does not compile.
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::project_manager_preferences::{
    ProjectManagerPreferenceSnapshot, ProjectManagerPreferenceStore,
};

/// Which section of the project manager is selected in the nav rail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ManagerScreen {
    Recent,
    New,
    Samples,
    Learn,
}

impl ManagerScreen {
    pub(super) const ALL: [Self; 4] = [Self::Recent, Self::New, Self::Samples, Self::Learn];

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Recent => "Recent Projects",
            Self::New => "New Project",
            Self::Samples => "Samples",
            Self::Learn => "Learn",
        }
    }

    pub(super) const fn icon(self) -> IconName {
        match self {
            Self::Recent => IconName::FolderOpen,
            Self::New => IconName::Plus,
            Self::Samples | Self::Learn => IconName::BookOpen,
        }
    }
}

/// Layout selected for the Recent Projects list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ManagerRecentLayout {
    List,
    Grid,
}

impl ManagerRecentLayout {
    const fn as_preference_value(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Grid => "grid",
        }
    }

    const fn from_preference_value(value: &str) -> Self {
        match value.as_bytes() {
            b"grid" => Self::Grid,
            _ => Self::List,
        }
    }
}

/// Sort keys the Recent list can actually order by. Template and gem count are
/// absent because no project data backs them: the manifest records no template
/// at all, and its `gems` list is unreachable from the editor shell, which reads
/// projects through `az_project_scaffold::ProjectSummary` (id/name/engine
/// version) precisely so it takes no direct `az-project` dependency. Their
/// columns were removed with them. A preference naming a deleted key falls back
/// to `LastOpened` through `from_preference_value`, so no schema change is
/// needed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ManagerRecentSort {
    LastOpened,
    Name,
    Engine,
}

impl ManagerRecentSort {
    const ALL: [Self; 3] = [Self::LastOpened, Self::Name, Self::Engine];

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::LastOpened => "Last opened",
            Self::Name => "Name",
            Self::Engine => "Engine",
        }
    }

    const fn as_preference_value(self) -> &'static str {
        match self {
            Self::LastOpened => "last_opened",
            Self::Name => "name",
            Self::Engine => "engine",
        }
    }

    const fn from_preference_value(value: &str) -> Self {
        match value.as_bytes() {
            b"name" => Self::Name,
            b"engine" => Self::Engine,
            _ => Self::LastOpened,
        }
    }

    pub(super) fn next(self) -> Self {
        let index = Self::ALL.iter().position(|sort| *sort == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }
}

#[derive(Clone)]
pub(super) struct ProjectManagerRecentProject {
    id: String,
    name: String,
    path: String,
    engine_version: String,
    /// Unix milliseconds of the recorded open, carried raw so ordering and the
    /// rendered text derive from the same fact.
    last_opened_unix_ms: i64,
    icon: IconName,
    pinned: bool,
}

impl std::fmt::Debug for ProjectManagerRecentProject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectManagerRecentProject")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("path", &self.path)
            .field("engine_version", &self.engine_version)
            .field("last_opened_unix_ms", &self.last_opened_unix_ms)
            .field("pinned", &self.pinned)
            // `icon` is a render detail with no `Debug` value worth printing.
            .finish_non_exhaustive()
    }
}

pub(super) struct ProjectManagerState {
    pub(super) preferences: Option<ProjectManagerPreferenceStore>,
    pub(super) recent_layout: ManagerRecentLayout,
    pub(super) recent_sort: ManagerRecentSort,
    pub(super) recent_projects: Vec<ProjectManagerRecentProject>,
}

pub(super) fn load_project_manager_state() -> ProjectManagerState {
    let preferences = match ProjectManagerPreferenceStore::open_global() {
        Ok(store) => Some(store),
        Err(error) => {
            warn!(%error, "failed to open project manager preference store");
            None
        }
    };
    let loaded_snapshot = preferences.as_ref().and_then(|store| match store.load() {
        Ok(snapshot) => Some(snapshot),
        Err(error) => {
            warn!(%error, "failed to load project manager preferences");
            None
        }
    });
    let snapshot = loaded_snapshot.clone().unwrap_or_default();
    // Recent shows ONLY real projects the user has opened — recorded the moment
    // they pick a valid project folder (so it persists even if the open later
    // fails or is cancelled). No illustrative/demo placeholders.
    let mut recent_projects = Vec::new();
    merge_recorded_recent_projects(&mut recent_projects);
    if let Some(snapshot) = &loaded_snapshot {
        for project in &mut recent_projects {
            project.pinned = snapshot.pinned_project_ids.contains(&project.id);
        }
    }

    ProjectManagerState {
        preferences,
        recent_layout: ManagerRecentLayout::from_preference_value(&snapshot.recent_layout),
        recent_sort: ManagerRecentSort::from_preference_value(&snapshot.recent_sort),
        recent_projects,
    }
}

/// Populate Recent from the real recorded recent-opened projects (deduped by
/// path, most-recent first), so a just-opened project shows in Recent even if
/// its open later fails or is cancelled. `recent_projects` starts empty today;
/// any pre-existing entries are de-duplicated against the recorded set and kept
/// after the recorded ones, where their own timestamp orders them.
fn merge_recorded_recent_projects(recent_projects: &mut Vec<ProjectManagerRecentProject>) {
    let recorded = crate::recent_projects::load_recent_projects();
    if recorded.is_empty() {
        return;
    }

    let normalize = |path: &str| {
        path.trim()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_ascii_lowercase()
    };
    let recorded_paths: std::collections::HashSet<String> = recorded
        .iter()
        .map(|entry| normalize(&entry.path))
        .collect();
    // Drop any default placeholder that the user has actually opened.
    recent_projects.retain(|project| !recorded_paths.contains(&normalize(&project.path)));

    let mut merged = recorded
        .into_iter()
        .map(|entry| ProjectManagerRecentProject {
            id: entry.id,
            name: entry.name,
            path: entry.path,
            engine_version: entry.engine_version,
            last_opened_unix_ms: entry.last_opened_unix_ms,
            icon: IconName::FolderOpen,
            pinned: false,
        })
        .collect::<Vec<_>>();
    merged.append(recent_projects);
    *recent_projects = merged;
}

impl ProjectManagerView {
    /// "Open…" call-to-action: open the OS folder picker directly and, on
    /// selection, dispatch the open workflow (which validates the manifest,
    /// records Recent, and transitions into the connecting workspace). This is
    /// the single entry point for opening an existing project — no separate
    /// "browse then confirm" step.
    // GPUI's `Context::listener` fixes the click-handler shape, so `self` stays
    // in the signature even though the picker needs nothing from the view.
    #[allow(clippy::unused_self)]
    pub(super) fn open_existing_project_via_picker(
        &mut self,
        _event: &gpui::ClickEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open Azoth project".into()),
        });
        cx.spawn_in(window, async move |_, window| {
            let path = paths.await.ok()?.ok()??.into_iter().next()?;
            let project_root = path.to_string_lossy().to_string();
            window
                .update(|window, cx| {
                    window.dispatch_action(
                        Box::new(actions::OpenProjectWorkflowEditorSession {
                            project_root,
                            session_name: "main".to_string(),
                        }),
                        cx,
                    );
                })
                .ok()
        })
        .detach();
    }

    pub(super) fn toggle_recent_project_pin(&mut self, project_id: &str) {
        if let Some(project) = self
            .recent_projects
            .iter_mut()
            .find(|project| project.id == project_id)
        {
            project.pinned = !project.pinned;
            self.persist_project_manager_preferences();
        }
    }

    pub(super) fn set_recent_sort(&mut self, sort: ManagerRecentSort) {
        self.recent_sort = sort;
        self.persist_project_manager_preferences();
    }

    pub(super) fn set_recent_layout(&mut self, layout: ManagerRecentLayout) {
        self.recent_layout = layout;
        self.persist_project_manager_preferences();
    }

    fn persist_project_manager_preferences(&self) {
        let Some(store) = &self.project_manager_preferences else {
            return;
        };
        let snapshot = ProjectManagerPreferenceSnapshot {
            recent_layout: self.recent_layout.as_preference_value().to_string(),
            recent_sort: self.recent_sort.as_preference_value().to_string(),
            pinned_project_ids: self
                .recent_projects
                .iter()
                .filter(|project| project.pinned)
                .map(|project| project.id.clone())
                .collect(),
        };
        if let Err(error) = store.save(&snapshot) {
            warn!(%error, "failed to save project manager preferences");
        }
    }
}

fn project_open_action(
    project: &ProjectManagerRecentProject,
) -> actions::OpenProjectWorkflowEditorSession {
    actions::OpenProjectWorkflowEditorSession {
        project_root: project.path.clone(),
        session_name: "main".to_string(),
    }
}

/// The Recent list's only ordering: pinned first, then the active sort key,
/// then name as a stable tiebreak. Both the rendered list and the default-open
/// action read the head of this ordering, so they cannot disagree.
fn sorted_recent_projects(
    projects: &[ProjectManagerRecentProject],
    sort: ManagerRecentSort,
) -> Vec<&ProjectManagerRecentProject> {
    let mut sorted = projects.iter().collect::<Vec<_>>();
    sorted.sort_by(|left, right| {
        right
            .pinned
            .cmp(&left.pinned)
            .then_with(|| match sort {
                // Newest first; the rendered text is derived from this same
                // timestamp, never sorted on.
                ManagerRecentSort::LastOpened => {
                    right.last_opened_unix_ms.cmp(&left.last_opened_unix_ms)
                }
                ManagerRecentSort::Name => left.name.cmp(&right.name),
                ManagerRecentSort::Engine => left.engine_version.cmp(&right.engine_version),
            })
            .then_with(|| left.name.cmp(&right.name))
    });
    sorted
}

pub(super) fn default_recent_project_open_action()
-> Option<actions::OpenProjectWorkflowEditorSession> {
    let state = load_project_manager_state();
    sorted_recent_projects(&state.recent_projects, state.recent_sort)
        .into_iter()
        .next()
        .map(project_open_action)
}

/// Human-facing "last opened" text for the Recent list.
///
/// `now_unix_ms` is a parameter rather than a clock read so the bucket
/// boundaries are testable.
fn format_last_opened(now_unix_ms: i64, last_opened_unix_ms: i64) -> String {
    // Entries persisted before the timestamp field existed carry 0. Say the
    // time is unknown rather than inventing one.
    if last_opened_unix_ms <= 0 {
        return "Unknown".to_string();
    }
    // A stamp written under clock skew can read as the future; clamp to 0 so it
    // lands in "just now" instead of underflowing into a huge elapsed span.
    let elapsed_ms = now_unix_ms.saturating_sub(last_opened_unix_ms).max(0);
    let minutes = elapsed_ms / 60_000;
    let hours = minutes / 60;
    let days = hours / 24;

    if minutes < 1 {
        "just now".to_string()
    } else if minutes < 60 {
        format!("{minutes} min ago")
    } else if hours < 24 {
        format!("{hours} h ago")
    } else if days < 2 {
        "yesterday".to_string()
    } else if days < 30 {
        format!("{days} days ago")
    } else {
        civil_date_utc(last_opened_unix_ms)
    }
}

/// `YYYY-MM-DD` in UTC for a Unix-millisecond stamp, via Howard Hinnant's
/// `civil_from_days`. Hand-rolled because the editor shell carries no date
/// crate and this is the only calendar arithmetic it needs.
fn civil_date_utc(unix_ms: i64) -> String {
    // Euclidean division so pre-epoch stamps floor to the correct day.
    let days = unix_ms.div_euclid(86_400_000);
    // Shift the epoch to 0000-03-01 so leap days land at the end of the cycle.
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let march_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * march_month + 2) / 5 + 1;
    let month = if march_month < 10 {
        march_month + 3
    } else {
        march_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

// ---------------------------------------------------------------------------
// ProjectManagerView: the project-manager view entity (state + screens). The
// app shell slides this in when no project is attached. Its `render` defers to
// the adopted aether layout in project_manager_view.rs.
// ---------------------------------------------------------------------------

pub struct ProjectManagerView {
    pub(super) active_screen: ManagerScreen,
    pub(super) recent_layout: ManagerRecentLayout,
    pub(super) recent_sort: ManagerRecentSort,
    pub(super) recent_projects: Vec<ProjectManagerRecentProject>,
    project_manager_preferences:
        Option<crate::project_manager_preferences::ProjectManagerPreferenceStore>,
    pub(super) gem_catalog: Vec<az_editor_ui::EditorGemInfo>,
    pub(super) template_step: ProjectTemplateStep,
    pub(super) selected_template: ProjectTemplateKind,
    pub(super) project_name: String,
    pub(super) project_location: String,
    pub(super) lore_url: String,
    pub(super) gem_query: String,
    pub(super) topology: ProjectTopologyChoice,
    pub(super) renderer_backend: ProjectRendererBackend,
    pub(super) render_pipeline: ProjectRenderPipeline,
    pub(super) color_space: ProjectColorSpace,
    pub(super) target_platforms: BTreeSet<ProjectTargetPlatform>,
    pub(super) enabled_gems: BTreeSet<String>,
}

impl ProjectManagerView {
    pub fn new(_window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let selected_template = ProjectTemplateKind::ThreeD;
        let project_name = "MyGame".to_string();
        let project_location = default_project_location();
        let lore_url = String::new();
        let gem_query = String::new();

        let project_manager_state = load_project_manager_state();
        let gem_catalog = cx
            .try_global::<az_editor_ui::EditorGemCatalog>()
            .map(|catalog| catalog.gems.clone())
            .unwrap_or_default();
        let enabled_gems = template_enabled_gems(selected_template, cx);
        cx.set_global(az_editor_ui::EditorGemSelection::new(
            enabled_gems.iter().cloned().collect(),
        ));

        Self {
            active_screen: ManagerScreen::Recent,
            recent_layout: project_manager_state.recent_layout,
            recent_sort: project_manager_state.recent_sort,
            recent_projects: project_manager_state.recent_projects,
            project_manager_preferences: project_manager_state.preferences,
            gem_catalog,
            template_step: ProjectTemplateStep::Template,
            selected_template,
            project_name,
            project_location,
            lore_url,
            gem_query,
            topology: selected_template.topology(),
            renderer_backend: selected_template.renderer(),
            render_pipeline: selected_template.pipeline(),
            color_space: ProjectColorSpace::Linear,
            target_platforms: selected_template.platforms().iter().copied().collect(),
            enabled_gems,
        }
    }

    pub(super) fn select_template(
        &mut self,
        template: ProjectTemplateKind,
        cx: &mut Context<'_, Self>,
    ) {
        self.selected_template = template;
        self.topology = template.topology();
        self.renderer_backend = template.renderer();
        self.render_pipeline = template.pipeline();
        self.target_platforms = template.platforms().iter().copied().collect();
        self.enabled_gems = template_enabled_gems(template, cx);
        self.publish_gem_selection(cx);
    }

    pub(super) fn toggle_project_gem(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        if self.enabled_gems.contains(id) {
            self.enabled_gems.remove(id);
        } else {
            self.enabled_gems.insert(id.to_string());
        }
        self.publish_gem_selection(cx);
    }

    pub(super) fn publish_gem_selection(&self, cx: &mut Context<'_, Self>) {
        cx.set_global(az_editor_ui::EditorGemSelection::new(
            self.enabled_gems.iter().cloned().collect(),
        ));
    }

    pub(super) fn dispatch_template_create_project(
        &self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let platform_ids = self
            .target_platforms
            .iter()
            .map(|platform| platform.as_manifest_id())
            .collect::<Vec<_>>();
        info!(
            template = self.selected_template.name(),
            topology = self.topology.as_manifest_id(),
            renderer = self.renderer_backend.as_manifest_id(),
            pipeline = self.render_pipeline.as_manifest_id(),
            color_space = self.color_space.as_manifest_id(),
            platforms = ?platform_ids,
            gem_count = self.enabled_gems.len(),
            "dispatching project template creation"
        );
        window.dispatch_action(
            Box::new(actions::CreateProject {
                name: template_project_name(&self.project_name),
                path: template_project_path(&self.project_name, &self.project_location),
                lore_url: template_lore_url(&self.lore_url),
                topology: self.topology.as_manifest_id().to_string(),
                enabled_gems: self.enabled_gems.iter().cloned().collect(),
            }),
            cx,
        );
    }
}

impl Render for ProjectManagerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        // Adopted aether project-manager layout (design/aether-project-manager.html):
        // titlebar + nav rail + screens, theme-driven; Recent is fully wired and
        // New/Samples/Learn delegate to the existing functional screens.
        self.render_aether_manager(window, cx)
    }
}

// ===========================================================================
// ProjectManagerView view-model: the data + handlers the hand-owned render
// regions bind to via accessors and item fields.
// Each item carries a deferred click handler so the codegen's
// `item.on_click(event, window, cx)` can mutate the view without re-borrowing it.
// ===========================================================================

type ManagerClick =
    std::rc::Rc<dyn Fn(&mut ProjectManagerView, &mut Window, &mut Context<ProjectManagerView>)>;

fn manager_defer(handler: &ManagerClick, window: &Window, cx: &mut Context<ProjectManagerView>) {
    let handler = handler.clone();
    cx.defer_in(window, move |this, window, cx| handler(this, window, cx));
}

fn gem_category_icon(category: &str) -> IconName {
    match category {
        "System" | "Physics" => IconName::Cpu,
        "Data" => IconName::File,
        "Rendering" => IconName::Palette,
        "UI" => IconName::LayoutDashboard,
        "Audio" => IconName::Bell,
        "Gameplay" => IconName::Play,
        "World" => IconName::Globe,
        "Platform" => IconName::Network,
        _ => IconName::Star,
    }
}

#[derive(Clone)]
pub(super) struct NavItem {
    pub(super) icon: IconName,
    pub(super) label: String,
    pub(super) active: bool,
    handler: ManagerClick,
}
impl NavItem {
    pub(super) fn activate(&self, window: &Window, cx: &mut Context<ProjectManagerView>) {
        manager_defer(&self.handler, window, cx);
    }

    pub(super) fn on_click(
        &self,
        _e: &gpui::ClickEvent,
        window: &Window,
        cx: &mut Context<ProjectManagerView>,
    ) {
        self.activate(window, cx);
    }
}

#[derive(Clone)]
pub(super) struct RecentItem {
    pub(super) id: String,
    pub(super) icon: IconName,
    pub(super) name: String,
    pub(super) path: String,
    pub(super) ver: String,
    pub(super) opened: String,
    pub(super) pinned: bool,
    handler: ManagerClick,
}
impl RecentItem {
    pub(super) fn on_click(
        &self,
        _e: &gpui::ClickEvent,
        window: &Window,
        cx: &mut Context<ProjectManagerView>,
    ) {
        manager_defer(&self.handler, window, cx);
    }
}

#[derive(Clone)]
pub(super) struct Step {
    pub(super) num: u8,
    pub(super) label: String,
    pub(super) done: bool,
    pub(super) not_done: bool,
    handler: ManagerClick,
}
impl Step {
    pub(super) fn on_click(
        &self,
        _e: &gpui::ClickEvent,
        window: &Window,
        cx: &mut Context<ProjectManagerView>,
    ) {
        manager_defer(&self.handler, window, cx);
    }
}

#[derive(Clone)]
pub(super) struct Template {
    pub(super) selected: bool,
    pub(super) icon: IconName,
    pub(super) name: String,
    pub(super) desc: String,
    handler: ManagerClick,
}
impl Template {
    pub(super) fn on_click(
        &self,
        _e: &gpui::ClickEvent,
        window: &Window,
        cx: &mut Context<ProjectManagerView>,
    ) {
        manager_defer(&self.handler, window, cx);
    }
}

#[derive(Clone)]
pub(super) struct Opt {
    pub(super) icon: IconName,
    pub(super) label: String,
    handler: ManagerClick,
}
impl Opt {
    pub(super) fn on_click(
        &self,
        _e: &gpui::ClickEvent,
        window: &Window,
        cx: &mut Context<ProjectManagerView>,
    ) {
        manager_defer(&self.handler, window, cx);
    }
}

#[derive(Clone)]
pub(super) struct Gem {
    pub(super) name: String,
    pub(super) desc: String,
    pub(super) ver: String,
    pub(super) locked: bool,
    handler: ManagerClick,
}
impl Gem {
    pub(super) fn on_toggle(
        &self,
        _e: &gpui::ClickEvent,
        window: &Window,
        cx: &mut Context<ProjectManagerView>,
    ) {
        manager_defer(&self.handler, window, cx);
    }
}

#[derive(Clone)]
pub(super) struct GemGroup {
    pub(super) cat: String,
    pub(super) icon: IconName,
    pub(super) count: usize,
    pub(super) items: Vec<Gem>,
}

#[derive(Clone)]
pub(super) struct SumRow {
    pub(super) label: String,
    pub(super) value: String,
}

#[derive(Clone)]
pub(super) struct SumGem {
    pub(super) icon: IconName,
    pub(super) name: String,
}

impl ProjectManagerView {
    // --- screen visibility ---
    pub(super) fn view_recent(&self) -> bool {
        self.active_screen == ManagerScreen::Recent
    }
    pub(super) fn view_new(&self) -> bool {
        self.active_screen == ManagerScreen::New
    }
    pub(super) fn step1(&self) -> bool {
        self.template_step == ProjectTemplateStep::Template
    }
    pub(super) fn step2(&self) -> bool {
        self.template_step == ProjectTemplateStep::Configure
    }
    pub(super) fn can_back(&self) -> bool {
        self.template_step != ProjectTemplateStep::Template
    }
    pub(super) fn last_step(&self) -> bool {
        self.template_step == ProjectTemplateStep::Gems
    }

    // --- scalar text / counts ---
    pub(super) fn name(&self) -> String {
        self.project_name.clone()
    }
    pub(super) fn path_preview(&self) -> String {
        template_project_path(&self.project_name, &self.project_location)
    }
    pub(super) const fn recent_layout(&self) -> ManagerRecentLayout {
        self.recent_layout
    }
    pub(super) const fn recent_sort(&self) -> ManagerRecentSort {
        self.recent_sort
    }
    pub(super) fn gem_count(&self) -> usize {
        self.enabled_gems.len()
    }
    pub(super) fn footer_hint(&self) -> String {
        match self.template_step {
            ProjectTemplateStep::Template => "Choose a template to begin.",
            ProjectTemplateStep::Configure => {
                "Configure topology, renderer, pipeline, and platforms."
            }
            ProjectTemplateStep::Gems => "Enable the gems your project needs.",
        }
        .to_string()
    }
    pub(super) fn misc_title(&self) -> String {
        match self.active_screen {
            ManagerScreen::Samples => "Sample Projects",
            _ => "Learn",
        }
        .to_string()
    }
    pub(super) fn misc_sub(&self) -> String {
        match self.active_screen {
            ManagerScreen::Samples => "Browse ready-made scenes and workflow examples.",
            _ => "Open tutorials, reference docs, and engine notes.",
        }
        .to_string()
    }
    pub(super) const fn misc_icon() -> IconName {
        IconName::BookOpen
    }
    pub(super) fn sum_template(&self) -> String {
        self.selected_template.name().to_string()
    }
    pub(super) const fn sum_icon(&self) -> IconName {
        self.selected_template.icon()
    }

    // --- item lists ---
    pub(super) fn nav_items(&self) -> Vec<NavItem> {
        let mut items = ManagerScreen::ALL
            .iter()
            .map(|&screen| NavItem {
                icon: screen.icon(),
                label: screen.label().to_string(),
                active: screen == self.active_screen,
                handler: std::rc::Rc::new(move |this, _w, cx| {
                    this.active_screen = screen;
                    cx.notify();
                }),
            })
            .collect::<Vec<_>>();
        items.push(NavItem {
            icon: IconName::Cpu,
            label: "Asset Processor".to_string(),
            active: false,
            handler: std::rc::Rc::new(|this, _window, cx| {
                if cx.try_global::<EditorAttachSession>().is_some() {
                    if let Err(error) = open_or_focus_asset_processor_window(cx) {
                        tracing::error!(%error, "failed to open asset processor window from project manager");
                    }
                    return;
                }

                if let Some(project) =
                    sorted_recent_projects(&this.recent_projects, this.recent_sort)
                        .into_iter()
                        .next()
                {
                    request_open_asset_processor_after_attach(cx);
                    let action = project_open_action(project);
                    if !open_project_workflow_editor_session_from_action(&action, cx) {
                        cx.remove_global::<OpenAssetProcessorAfterAttach>();
                    }
                } else {
                    this.active_screen = ManagerScreen::Recent;
                    cx.default_global::<ConsoleState>().log_from_source(
                        LogLevel::Warn,
                        "project-manager",
                        "Open a project before launching Asset Processor.",
                    );
                    cx.notify();
                }
            }),
        });
        items
    }
    pub(super) fn recent_projects(&self) -> Vec<RecentItem> {
        // One clock read for the whole list so sibling rows bucket consistently.
        let now_unix_ms = crate::recent_projects::current_unix_ms();
        sorted_recent_projects(&self.recent_projects, self.recent_sort)
            .into_iter()
            .map(|p| {
                let action = project_open_action(p);
                RecentItem {
                    id: p.id.clone(),
                    icon: p.icon.clone(),
                    name: p.name.clone(),
                    path: p.path.clone(),
                    ver: p.engine_version.clone(),
                    opened: format_last_opened(now_unix_ms, p.last_opened_unix_ms),
                    pinned: p.pinned,
                    handler: std::rc::Rc::new(move |_this, window, cx| {
                        window.dispatch_action(Box::new(action.clone()), cx);
                    }),
                }
            })
            .collect()
    }
    pub(super) fn steps(&self) -> Vec<Step> {
        let current = self.template_step.number();
        ProjectTemplateStep::ALL
            .iter()
            .map(|&step| Step {
                num: step.number(),
                label: step.label().to_string(),
                done: step.number() < current,
                not_done: step.number() > current,
                handler: std::rc::Rc::new(move |this, _w, cx| {
                    this.template_step = step;
                    cx.notify();
                }),
            })
            .collect()
    }
    pub(super) fn templates(&self) -> Vec<Template> {
        let selected = self.selected_template;
        ProjectTemplateKind::ALL
            .iter()
            .map(|&kind| Template {
                selected: kind == selected,
                icon: kind.icon(),
                name: kind.name().to_string(),
                desc: kind.description().to_string(),
                handler: std::rc::Rc::new(move |this, _w, cx| {
                    this.select_template(kind, cx);
                    cx.notify();
                }),
            })
            .collect()
    }
    pub(super) fn renderer_opts() -> Vec<Opt> {
        ProjectRendererBackend::ALL
            .iter()
            .map(|&backend| Opt {
                icon: IconName::Palette,
                label: backend.label().to_string(),
                handler: std::rc::Rc::new(move |this, _w, cx| {
                    this.renderer_backend = backend;
                    cx.notify();
                }),
            })
            .collect()
    }
    pub(super) fn topology_opts(&self) -> Vec<Opt> {
        ProjectTopologyChoice::ALL
            .iter()
            .map(|&topology| Opt {
                icon: IconName::Network,
                label: format!(
                    "{}{} — {}",
                    if topology == self.topology {
                        "Selected: "
                    } else {
                        ""
                    },
                    topology.label(),
                    topology.description()
                ),
                handler: std::rc::Rc::new(move |this, _w, cx| {
                    this.topology = topology;
                    cx.notify();
                }),
            })
            .collect()
    }
    pub(super) fn pipeline_opts() -> Vec<Opt> {
        ProjectRenderPipeline::ALL
            .iter()
            .map(|&pipeline| Opt {
                icon: IconName::LayoutDashboard,
                label: pipeline.label().to_string(),
                handler: std::rc::Rc::new(move |this, _w, cx| {
                    this.render_pipeline = pipeline;
                    cx.notify();
                }),
            })
            .collect()
    }
    pub(super) fn color_opts() -> Vec<Opt> {
        ProjectColorSpace::ALL
            .iter()
            .map(|&color| Opt {
                icon: IconName::Palette,
                label: color.label().to_string(),
                handler: std::rc::Rc::new(move |this, _w, cx| {
                    this.color_space = color;
                    cx.notify();
                }),
            })
            .collect()
    }
    pub(super) fn platform_opts() -> Vec<Opt> {
        ProjectTargetPlatform::ALL
            .iter()
            .map(|&platform| Opt {
                icon: IconName::Network,
                label: platform.label().to_string(),
                handler: std::rc::Rc::new(move |this, _w, cx| {
                    if this.target_platforms.contains(&platform) {
                        this.target_platforms.remove(&platform);
                    } else {
                        this.target_platforms.insert(platform);
                    }
                    cx.notify();
                }),
            })
            .collect()
    }
    pub(super) fn gem_groups(&self) -> Vec<GemGroup> {
        let required_categories = self.selected_template.gem_categories();
        let query = self.gem_query.trim().to_ascii_lowercase();
        let mut groups =
            std::collections::BTreeMap::<String, Vec<az_editor_ui::EditorGemInfo>>::new();

        for gem in &self.gem_catalog {
            let display_description = gem.display_description();
            if !query.is_empty()
                && !gem.name.to_ascii_lowercase().contains(&query)
                && !gem.id.to_ascii_lowercase().contains(&query)
                && !display_description.to_ascii_lowercase().contains(&query)
            {
                continue;
            }
            groups
                .entry(if gem.category.is_empty() {
                    "Other".to_string()
                } else {
                    gem.category.clone()
                })
                .or_default()
                .push(gem.clone());
        }

        groups
            .into_iter()
            .map(|(category, mut gems)| {
                gems.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
                let items = gems
                    .into_iter()
                    .map(|gem| {
                        let id = gem.id.clone();
                        let display_name = gem.display_name();
                        let display_description = gem.display_description();
                        let locked = required_categories
                            .iter()
                            .any(|required| *required == category);
                        Gem {
                            name: display_name,
                            desc: if display_description.is_empty() {
                                "Engine feature gem".to_string()
                            } else {
                                display_description
                            },
                            ver: if gem.version.is_empty() {
                                "local".to_string()
                            } else {
                                gem.version
                            },
                            locked,
                            handler: std::rc::Rc::new(move |this, _w, cx| {
                                if !locked {
                                    this.toggle_project_gem(&id, cx);
                                }
                            }),
                        }
                    })
                    .collect::<Vec<_>>();
                GemGroup {
                    cat: category.clone(),
                    icon: gem_category_icon(&category),
                    count: items.len(),
                    items,
                }
            })
            .collect()
    }
    pub(super) fn sum_gems(&self) -> Vec<SumGem> {
        self.gem_catalog
            .iter()
            .filter(|gem| self.enabled_gems.contains(&gem.id))
            .map(|gem| SumGem {
                icon: gem_category_icon(&gem.category),
                name: gem.name.clone(),
            })
            .collect()
    }
    pub(super) fn sum_rows(&self) -> Vec<SumRow> {
        let platforms = self
            .target_platforms
            .iter()
            .map(|p| p.label())
            .collect::<Vec<_>>()
            .join(", ");
        vec![
            SumRow {
                label: "Topology".to_string(),
                value: self.topology.label().to_string(),
            },
            SumRow {
                label: "Renderer".to_string(),
                value: self.renderer_backend.label().to_string(),
            },
            SumRow {
                label: "Pipeline".to_string(),
                value: self.render_pipeline.label().to_string(),
            },
            SumRow {
                label: "Color".to_string(),
                value: self.color_space.label().to_string(),
            },
            SumRow {
                label: "Platforms".to_string(),
                value: platforms,
            },
            SumRow {
                label: "Gems".to_string(),
                value: format!("{} enabled", self.enabled_gems.len()),
            },
        ]
    }

    // --- direct button handlers (non-item on_click sites) ---
    pub(super) fn go_new(
        &mut self,
        _e: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_screen = ManagerScreen::New;
        self.template_step = ProjectTemplateStep::Template;
        cx.notify();
    }
    pub(super) fn go_recent(
        &mut self,
        _e: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_screen = ManagerScreen::Recent;
        cx.notify();
    }
    pub(super) fn on_back(
        &mut self,
        _e: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.template_step = self.template_step.previous();
        cx.notify();
    }
    pub(super) fn on_next(
        &mut self,
        _e: &gpui::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.template_step == ProjectTemplateStep::Gems {
            self.dispatch_template_create_project(window, cx);
        } else {
            self.template_step = self.template_step.next();
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE_MS: i64 = 60_000;
    const HOUR_MS: i64 = 60 * MINUTE_MS;
    const DAY_MS: i64 = 24 * HOUR_MS;
    /// 2020-01-01T00:00:00Z.
    const NEW_YEAR_2020_MS: i64 = 1_577_836_800_000;

    fn project(
        name: &str,
        engine_version: &str,
        last_opened_unix_ms: i64,
        pinned: bool,
    ) -> ProjectManagerRecentProject {
        ProjectManagerRecentProject {
            id: format!("local.{}", name.to_ascii_lowercase()),
            name: name.to_string(),
            path: std::env::temp_dir()
                .join(name)
                .to_string_lossy()
                .into_owned(),
            engine_version: engine_version.to_string(),
            last_opened_unix_ms,
            icon: IconName::FolderOpen,
            pinned,
        }
    }

    fn names(projects: &[ProjectManagerRecentProject], sort: ManagerRecentSort) -> Vec<&str> {
        sorted_recent_projects(projects, sort)
            .into_iter()
            .map(|project| project.name.as_str())
            .collect()
    }

    #[test]
    fn pinned_projects_sort_ahead_of_unpinned_under_every_key() {
        let projects = vec![
            project("Alder", "0.1.0", 300, false),
            project("Zeta", "0.9.0", 100, true),
        ];
        for sort in ManagerRecentSort::ALL {
            assert_eq!(
                names(&projects, sort),
                ["Zeta", "Alder"],
                "pinned must lead under {sort:?}"
            );
        }
    }

    #[test]
    fn last_opened_sorts_newest_first_and_sinks_undated_entries() {
        let projects = vec![
            project("Older", "0.1.0", 100, false),
            project("Legacy", "0.1.0", 0, false),
            project("Newest", "0.1.0", 300, false),
            project("Middle", "0.1.0", 200, false),
        ];
        assert_eq!(
            names(&projects, ManagerRecentSort::LastOpened),
            ["Newest", "Middle", "Older", "Legacy"]
        );
    }

    #[test]
    fn name_sorts_alphabetically() {
        let projects = vec![
            project("Cedar", "0.1.0", 300, false),
            project("Alder", "0.1.0", 100, false),
            project("Birch", "0.1.0", 200, false),
        ];
        assert_eq!(
            names(&projects, ManagerRecentSort::Name),
            ["Alder", "Birch", "Cedar"]
        );
    }

    #[test]
    fn engine_sorts_by_version_then_name() {
        let projects = vec![
            project("Zeta", "0.2.0", 300, false),
            project("Yew", "0.1.0", 100, false),
            project("Alder", "0.1.0", 200, false),
        ];
        assert_eq!(
            names(&projects, ManagerRecentSort::Engine),
            ["Alder", "Yew", "Zeta"]
        );
    }

    #[test]
    fn equal_keys_fall_back_to_name() {
        let projects = vec![
            project("Birch", "0.1.0", 500, false),
            project("Alder", "0.1.0", 500, false),
        ];
        assert_eq!(
            names(&projects, ManagerRecentSort::LastOpened),
            ["Alder", "Birch"]
        );
    }

    #[test]
    fn unknown_sort_preference_falls_back_to_last_opened() {
        // A preference persisted under a since-deleted key must not wedge the
        // list; the store's schema stays untouched.
        assert_eq!(
            ManagerRecentSort::from_preference_value("template"),
            ManagerRecentSort::LastOpened
        );
        assert_eq!(
            ManagerRecentSort::from_preference_value("gems"),
            ManagerRecentSort::LastOpened
        );
        assert_eq!(
            ManagerRecentSort::from_preference_value("engine"),
            ManagerRecentSort::Engine
        );
    }

    #[test]
    fn sort_cycle_visits_only_live_keys() {
        let mut sort = ManagerRecentSort::LastOpened;
        let mut visited = Vec::new();
        for _ in 0..ManagerRecentSort::ALL.len() {
            visited.push(sort);
            sort = sort.next();
        }
        assert_eq!(sort, ManagerRecentSort::LastOpened, "cycle must close");
        assert_eq!(visited, ManagerRecentSort::ALL);
    }

    #[test]
    fn last_opened_text_buckets_on_boundaries() {
        let now = 1_000 * DAY_MS;
        let text = |elapsed_ms: i64| format_last_opened(now, now - elapsed_ms);

        assert_eq!(text(0), "just now");
        assert_eq!(text(MINUTE_MS - 1), "just now");
        assert_eq!(text(MINUTE_MS), "1 min ago");
        assert_eq!(text(HOUR_MS - 1), "59 min ago");
        assert_eq!(text(HOUR_MS), "1 h ago");
        assert_eq!(text(DAY_MS - 1), "23 h ago");
        assert_eq!(text(DAY_MS), "yesterday");
        assert_eq!(text(2 * DAY_MS - 1), "yesterday");
        assert_eq!(text(2 * DAY_MS), "2 days ago");
        assert_eq!(text(30 * DAY_MS - 1), "29 days ago");
    }

    #[test]
    fn last_opened_text_goes_absolute_past_thirty_days() {
        assert_eq!(
            format_last_opened(NEW_YEAR_2020_MS + 30 * DAY_MS, NEW_YEAR_2020_MS),
            "2020-01-01"
        );
        assert_eq!(
            format_last_opened(NEW_YEAR_2020_MS + 400 * DAY_MS, NEW_YEAR_2020_MS),
            "2020-01-01"
        );
    }

    #[test]
    fn last_opened_text_handles_missing_and_skewed_stamps() {
        // Entries recorded before the timestamp field existed.
        assert_eq!(format_last_opened(NEW_YEAR_2020_MS, 0), "Unknown");
        assert_eq!(format_last_opened(NEW_YEAR_2020_MS, -5), "Unknown");
        // Clock skew: a future stamp must not read as decades ago.
        assert_eq!(
            format_last_opened(NEW_YEAR_2020_MS, NEW_YEAR_2020_MS + 5 * DAY_MS),
            "just now"
        );
    }

    #[test]
    fn civil_date_matches_known_days() {
        assert_eq!(civil_date_utc(0), "1970-01-01");
        assert_eq!(civil_date_utc(DAY_MS), "1970-01-02");
        assert_eq!(civil_date_utc(NEW_YEAR_2020_MS), "2020-01-01");
        // Leap day, and a stamp mid-day rather than on a day boundary.
        assert_eq!(civil_date_utc(1_582_934_400_000), "2020-02-29");
        assert_eq!(
            civil_date_utc(1_582_934_400_000 + HOUR_MS * 13),
            "2020-02-29"
        );
    }
}
