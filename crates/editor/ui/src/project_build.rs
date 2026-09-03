//! Data-only project build catalog and status state for editor UI surfaces.

use gpui::Global;

pub const HOST_TARGET_KEY: &str = "host";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditorProjectBuildCatalog {
    pub profiles: Vec<EditorBuildProfileData>,
    pub targets: Vec<EditorBuildTargetData>,
    pub default_build_targets: Vec<EditorBuildOutputTargetData>,
    pub diagnostics: Vec<String>,
}

impl Global for EditorProjectBuildCatalog {}

impl EditorProjectBuildCatalog {
    #[must_use]
    pub fn with_host_target(mut self) -> Self {
        if self.targets.is_empty() {
            self.targets.push(EditorBuildTargetData::host());
        }
        self
    }

    #[must_use]
    pub fn profile(&self, name: &str) -> Option<&EditorBuildProfileData> {
        self.profiles.iter().find(|profile| profile.name == name)
    }

    #[must_use]
    pub fn target(&self, key: &str) -> Option<&EditorBuildTargetData> {
        self.targets.iter().find(|target| target.key == key)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorBuildProfileData {
    pub name: String,
    pub asset_platform: String,
    pub cargo_profile: String,
    pub container: String,
    pub compression: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorBuildTargetData {
    pub key: String,
    pub label: String,
    pub icon: String,
    pub target_triple: Option<String>,
}

impl EditorBuildTargetData {
    #[must_use]
    pub fn host() -> Self {
        Self {
            key: HOST_TARGET_KEY.to_owned(),
            label: "Host".to_owned(),
            icon: "desktop_windows".to_owned(),
            target_triple: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorBuildOutputTargetData {
    pub owner_id: String,
    pub name: String,
    pub kind: String,
    pub role: String,
    pub default: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EditorProjectBuildPhase {
    #[default]
    Idle,
    Planning,
    Running,
    Planned,
    Succeeded,
    Failed,
}

impl EditorProjectBuildPhase {
    #[must_use]
    pub const fn is_busy(self) -> bool {
        matches!(self, Self::Planning | Self::Running)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditorProjectBuildState {
    pub selected_profile: Option<String>,
    pub selected_target_key: Option<String>,
    pub phase: EditorProjectBuildPhase,
    pub last_plan: Option<EditorProjectBuildPlanData>,
    pub progress: Option<EditorProjectBuildProgressData>,
    pub diagnostic: Option<String>,
}

impl Global for EditorProjectBuildState {}

impl EditorProjectBuildState {
    #[must_use]
    pub fn from_catalog(catalog: &EditorProjectBuildCatalog) -> Self {
        Self {
            selected_profile: catalog.profiles.first().map(|profile| profile.name.clone()),
            selected_target_key: catalog.targets.first().map(|target| target.key.clone()),
            phase: EditorProjectBuildPhase::Idle,
            last_plan: None,
            progress: None,
            diagnostic: catalog.diagnostics.first().cloned(),
        }
    }

    #[must_use]
    pub fn selected_profile<'a>(
        &self,
        catalog: &'a EditorProjectBuildCatalog,
    ) -> Option<&'a EditorBuildProfileData> {
        self.selected_profile
            .as_deref()
            .and_then(|name| catalog.profile(name))
            .or_else(|| catalog.profiles.first())
    }

    #[must_use]
    pub fn selected_target<'a>(
        &self,
        catalog: &'a EditorProjectBuildCatalog,
    ) -> Option<&'a EditorBuildTargetData> {
        self.selected_target_key
            .as_deref()
            .and_then(|key| catalog.target(key))
            .or_else(|| catalog.targets.first())
    }

    pub fn select_profile(&mut self, profile: &str, catalog: &EditorProjectBuildCatalog) -> bool {
        if catalog.profile(profile).is_none() {
            return false;
        }
        let changed = self.selected_profile.as_deref() != Some(profile);
        self.selected_profile = Some(profile.to_owned());
        changed
    }

    pub fn select_target(&mut self, target_key: &str, catalog: &EditorProjectBuildCatalog) -> bool {
        if catalog.target(target_key).is_none() {
            return false;
        }
        let changed = self.selected_target_key.as_deref() != Some(target_key);
        self.selected_target_key = Some(target_key.to_owned());
        changed
    }

    #[must_use]
    pub fn can_plan(&self, catalog: &EditorProjectBuildCatalog) -> bool {
        self.selected_profile(catalog).is_some() && self.selected_target(catalog).is_some()
    }

    #[must_use]
    pub fn can_execute(&self, catalog: &EditorProjectBuildCatalog) -> bool {
        self.can_plan(catalog) && !self.phase.is_busy()
    }

    pub fn mark_planning(&mut self) {
        self.phase = EditorProjectBuildPhase::Planning;
        self.progress = None;
        self.diagnostic = None;
    }

    pub fn mark_running(&mut self) {
        self.phase = EditorProjectBuildPhase::Running;
        self.progress = None;
        self.diagnostic = None;
    }

    pub fn mark_planned(&mut self, plan: EditorProjectBuildPlanData) {
        self.phase = EditorProjectBuildPhase::Planned;
        self.last_plan = Some(plan);
        self.progress = None;
        self.diagnostic = None;
    }

    pub fn mark_progress(&mut self, progress: EditorProjectBuildProgressData) -> bool {
        if self
            .progress
            .as_ref()
            .is_some_and(|current| progress.seq <= current.seq)
        {
            return false;
        }
        self.phase = EditorProjectBuildPhase::Running;
        self.progress = Some(progress);
        self.diagnostic = None;
        true
    }

    pub fn mark_succeeded(&mut self, plan: EditorProjectBuildPlanData) {
        self.phase = EditorProjectBuildPhase::Succeeded;
        self.last_plan = Some(plan);
        self.diagnostic = None;
    }

    pub fn mark_failed(&mut self, diagnostic: impl Into<String>) {
        self.phase = EditorProjectBuildPhase::Failed;
        self.diagnostic = Some(diagnostic.into());
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorProjectBuildProgressData {
    pub seq: u64,
    pub command_index: u32,
    pub command_count: u32,
    pub target_name: String,
    pub done_bp: u32,
    pub command_done: u64,
    pub command_total: Option<u64>,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorProjectBuildPlanData {
    pub profile: String,
    pub target_key: String,
    pub target_label: String,
    pub command_count: usize,
    pub commands: Vec<EditorProjectBuildCommandData>,
    pub package_profile: Option<EditorBuildProfileData>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorProjectBuildCommandData {
    pub owner_id: String,
    pub owner_root: String,
    pub target_name: String,
    pub program: String,
    pub cwd: String,
    pub args: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_state_selects_first_real_profile_and_host_target() {
        let catalog = EditorProjectBuildCatalog {
            profiles: vec![EditorBuildProfileData {
                name: "pc-dev".to_owned(),
                asset_platform: "pc".to_owned(),
                cargo_profile: "dev".to_owned(),
                container: "loose".to_owned(),
                compression: "none".to_owned(),
            }],
            targets: vec![EditorBuildTargetData::host()],
            default_build_targets: Vec::new(),
            diagnostics: Vec::new(),
        };

        let state = EditorProjectBuildState::from_catalog(&catalog);

        assert_eq!(state.selected_profile.as_deref(), Some("pc-dev"));
        assert_eq!(state.selected_target_key.as_deref(), Some(HOST_TARGET_KEY));
        assert!(state.can_plan(&catalog));
    }

    #[test]
    fn build_state_rejects_unknown_profile_or_target() {
        let catalog = EditorProjectBuildCatalog {
            profiles: Vec::new(),
            targets: vec![EditorBuildTargetData::host()],
            default_build_targets: Vec::new(),
            diagnostics: Vec::new(),
        };
        let mut state = EditorProjectBuildState::from_catalog(&catalog);

        assert!(!state.select_profile("shipping", &catalog));
        assert!(!state.select_target("x86_64-pc-windows-msvc", &catalog));
        assert!(!state.can_plan(&catalog));
    }

    #[test]
    fn build_state_blocks_execute_reentry_while_running() {
        let catalog = EditorProjectBuildCatalog {
            profiles: vec![EditorBuildProfileData {
                name: "pc-dev".to_owned(),
                asset_platform: "pc".to_owned(),
                cargo_profile: "debug".to_owned(),
                container: "loose".to_owned(),
                compression: "none".to_owned(),
            }],
            targets: vec![EditorBuildTargetData::host()],
            default_build_targets: Vec::new(),
            diagnostics: Vec::new(),
        };
        let mut state = EditorProjectBuildState::from_catalog(&catalog);

        assert!(state.can_execute(&catalog));
        state.mark_running();

        assert!(!state.can_execute(&catalog));
    }
}
