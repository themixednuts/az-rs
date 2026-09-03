//! Editor-facing gem catalog data.
//!
//! `az-editor-ui` only defines the data shape. The editor shell discovers the
//! workspace gems at runtime (from the engine manifest) and publishes this as a
//! GPUI global, so the catalog stays in sync with `gems/*` without recompiling
//! the editor.

use std::fmt::Write as _;

use gpui::Global;

/// One engine gem available to projects in this workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorGemInfo {
    /// Stable gem id from the engine manifest (e.g. `azoth.camera`).
    pub id: String,
    /// Human-readable gem name.
    pub name: String,
    /// Gem version.
    pub version: String,
    /// One-line description (sourced from the gem's Cargo manifest).
    pub description: String,
    /// Display category for grouping (derived; empty when unknown).
    pub category: String,
    /// Engine gems this gem depends on.
    pub dependencies: Vec<EditorGemDependencyInfo>,
    /// Lifecycle notice when the gem remains available only for migration.
    pub deprecation: Option<EditorGemDeprecationInfo>,
}

impl EditorGemInfo {
    #[must_use]
    pub const fn is_deprecated(&self) -> bool {
        self.deprecation.is_some()
    }

    #[must_use]
    pub fn display_name(&self) -> String {
        if self.is_deprecated() {
            format!("{} (Deprecated)", self.name)
        } else {
            self.name.clone()
        }
    }

    #[must_use]
    pub fn display_description(&self) -> String {
        let mut parts = Vec::new();
        if !self.description.trim().is_empty() {
            parts.push(self.description.trim().to_string());
        }
        if let Some(deprecation) = &self.deprecation {
            let mut notice = format!("Deprecated: {}", deprecation.message);
            if let Some(since) = &deprecation.since {
                let _ = write!(notice, " Since v{since}.");
            }
            if let Some(replacement) = &deprecation.replacement {
                let _ = write!(notice, " Use {}", replacement.name);
                if let Some(version) = &replacement.version {
                    let _ = write!(notice, " ({version})");
                }
                notice.push('.');
            }
            parts.push(notice);
        }
        parts.join(" ")
    }
}

/// Editor-facing lifecycle notice for a deprecated gem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorGemDeprecationInfo {
    pub message: String,
    pub since: Option<String>,
    pub replacement: Option<EditorGemReplacementInfo>,
}

/// Stable replacement pointer rendered by gem selection surfaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorGemReplacementInfo {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
}

/// One engine gem dependency published with a catalog entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorGemDependencyInfo {
    /// Stable gem id from the engine manifest.
    pub id: String,
    /// Human-readable gem name.
    pub name: String,
}

/// The set of engine gems discovered for the active engine workspace.
#[derive(Clone, Debug, Default)]
pub struct EditorGemCatalog {
    pub gems: Vec<EditorGemInfo>,
    /// Discovery error, if the engine manifest could not be resolved.
    pub status_error: Option<String>,
}

impl Global for EditorGemCatalog {}

/// The user's current gem selection in the project manager. Published by the
/// editor shell and read when a new project is created.
#[derive(Clone, Debug, Default)]
pub struct EditorGemSelection {
    /// Enabled gem ids.
    pub enabled: Vec<String>,
}

impl Global for EditorGemSelection {}

impl EditorGemSelection {
    #[must_use]
    pub const fn new(enabled: Vec<String>) -> Self {
        Self { enabled }
    }

    #[must_use]
    pub fn contains(&self, id: &str) -> bool {
        self.enabled.iter().any(|gem| gem == id)
    }
}

/// Whether the project has staged gem (or other primitive) changes that need a
/// rebuild to take effect. The workflow stages changes before rebuilding.
#[derive(Clone, Debug, Default)]
pub struct EditorGemRebuildState {
    pub rebuild_pending: bool,
}

impl Global for EditorGemRebuildState {}

impl EditorGemCatalog {
    #[must_use]
    pub const fn new(gems: Vec<EditorGemInfo>) -> Self {
        Self {
            gems,
            status_error: None,
        }
    }

    #[must_use]
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            gems: Vec::new(),
            status_error: Some(message.into()),
        }
    }
}

/// Capability id a provider gem declares (ADR 0026). Kept in sync with
/// `azoth gem new --capability provider` by convention.
pub const NEW_GEM_PROVIDER_CAPABILITY: &str = "provider";

/// Capability id a session-authority gem declares (ADR 0026). Kept in sync with
/// `azoth gem new --capability session-authority` by convention.
pub const NEW_GEM_SESSION_AUTHORITY_CAPABILITY: &str = "session-authority";

/// The capability template a new project gem is scaffolded with, mirroring the
/// `azoth gem new --capability <id>` templates. `None` keeps the generic gem
/// shape (code + assets + prefab scaffolding).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NewGemCapabilityChoice {
    /// Generic gem, no `--capability` flag.
    #[default]
    None,
    /// ADR 0026 auth provider (`--capability provider`).
    Provider,
    /// ADR 0026 session authority (`--capability session-authority`).
    SessionAuthority,
}

impl NewGemCapabilityChoice {
    /// The choices offered in the New Gem dialog, in display order.
    pub const ALL: [Self; 3] = [Self::None, Self::Provider, Self::SessionAuthority];

    /// Human display name (no raw capability ids in the UI).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Provider => "Auth Provider",
            Self::SessionAuthority => "Session Authority",
        }
    }

    /// One-line description shown beneath each option.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::None => "Standard gem with code, assets, and prefab scaffolding.",
            Self::Provider => "Supplies credentials through the auth provider contract (ADR 0026).",
            Self::SessionAuthority => "Owns session lifecycle as the session authority (ADR 0026).",
        }
    }

    /// The `azoth gem new --capability` id, or `None` for the generic template.
    #[must_use]
    pub const fn capability_id(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Provider => Some(NEW_GEM_PROVIDER_CAPABILITY),
            Self::SessionAuthority => Some(NEW_GEM_SESSION_AUTHORITY_CAPABILITY),
        }
    }
}

/// Whether `name` is a valid gem name. Mirrors the scaffold's
/// `is_valid_project_name` so the dialog can disable the Create button before
/// the authoritative check runs in the editor shell.
#[must_use]
pub fn is_valid_gem_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && !name.starts_with(|c: char| c.is_ascii_digit())
}

/// The gem id derived from `name` when the id field is left blank.
///
/// Mirrors `az_project::project_id_from_name` for an accurate placeholder. The
/// editor shell computes the authoritative id; this only drives the display
/// hint.
#[must_use]
pub fn derived_gem_id(name: &str) -> String {
    let mut id = String::new();
    let mut last_was_separator = false;
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            id.push(ch.to_ascii_lowercase());
            last_was_separator = false;
        } else if (ch == '_' || ch == '-' || ch.is_ascii_whitespace()) && !last_was_separator {
            id.push('_');
            last_was_separator = true;
        }
    }
    while id.ends_with('_') {
        id.pop();
    }
    if id.is_empty() {
        id.push_str("gem");
    }
    id
}

/// Inline feedback for the New Gem dialog.
///
/// Published by the editor shell after a gem-creation attempt and read by the
/// Gems panel to surface success or the scaffold error (already-exists, invalid
/// name, …) next to the form.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditorGemCreationStatus {
    /// `Some(Ok(message))` on success, `Some(Err(message))` on failure.
    pub outcome: Option<Result<String, String>>,
}

impl Global for EditorGemCreationStatus {}

impl EditorGemCreationStatus {
    #[must_use]
    pub const fn success(message: String) -> Self {
        Self {
            outcome: Some(Ok(message)),
        }
    }

    #[must_use]
    pub const fn error(message: String) -> Self {
        Self {
            outcome: Some(Err(message)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EditorGemDeprecationInfo, EditorGemInfo, EditorGemReplacementInfo, NewGemCapabilityChoice,
        derived_gem_id, is_valid_gem_name,
    };

    #[test]
    fn deprecated_gem_display_names_replacement() {
        let gem = EditorGemInfo {
            id: "azoth.old_render".to_string(),
            name: "Old Render".to_string(),
            version: "0.4.0".to_string(),
            description: "Legacy renderer.".to_string(),
            category: "Rendering".to_string(),
            dependencies: Vec::new(),
            deprecation: Some(EditorGemDeprecationInfo {
                message: "Use the frame-graph renderer for new projects.".to_string(),
                since: Some("0.4.0".to_string()),
                replacement: Some(EditorGemReplacementInfo {
                    id: "azoth.frame_graph".to_string(),
                    name: "Frame Graph".to_string(),
                    version: Some("^0.5.0".to_string()),
                }),
            }),
        };

        assert_eq!(gem.display_name(), "Old Render (Deprecated)");
        assert_eq!(
            gem.display_description(),
            "Legacy renderer. Deprecated: Use the frame-graph renderer for new projects. Since v0.4.0. Use Frame Graph (^0.5.0)."
        );
    }

    #[test]
    fn gem_name_validation_mirrors_scaffold_rules() {
        assert!(is_valid_gem_name("my_gem"));
        assert!(is_valid_gem_name("Combat-System"));
        assert!(!is_valid_gem_name(""));
        assert!(!is_valid_gem_name("1abc"), "must not start with a digit");
        assert!(!is_valid_gem_name("has space"));
        assert!(!is_valid_gem_name("has.dot"));
    }

    #[test]
    fn derived_gem_id_matches_scaffold_shape() {
        assert_eq!(derived_gem_id("My Cool Gem"), "my_cool_gem");
        assert_eq!(derived_gem_id("combat-system"), "combat_system");
        assert_eq!(derived_gem_id("gem__"), "gem");
    }

    #[test]
    fn capability_choice_maps_to_scaffold_ids() {
        assert_eq!(NewGemCapabilityChoice::None.capability_id(), None);
        assert_eq!(
            NewGemCapabilityChoice::Provider.capability_id(),
            Some("provider")
        );
        assert_eq!(
            NewGemCapabilityChoice::SessionAuthority.capability_id(),
            Some("session-authority")
        );
    }
}
