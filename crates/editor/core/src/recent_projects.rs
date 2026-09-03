//! Recently-opened project store.
//!
//! The editor shell records a project in this list the moment the user picks a
//! valid project folder — *before* the build/attach completes — so a slow,
//! failed, or cancelled open still leaves the project in the launcher's Recent
//! list. The store is a small JSON file owned by the editor shell (path policy
//! lives here, not in the data-only `az-editor-ui` crate).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use az_filesystem::AzothDataHome;
use serde::{Deserialize, Serialize};
use tracing::warn;

/// One recorded project-open entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentProjectEntry {
    /// Stable project id derived from the manifest.
    pub id: String,
    /// Display name from the manifest.
    pub name: String,
    /// Absolute project root path.
    pub path: String,
    /// Engine version recorded in the manifest, when known.
    #[serde(default)]
    pub engine_version: String,
    /// Unix milliseconds of the most recent open.
    #[serde(default)]
    pub last_opened_unix_ms: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RecentProjectsFile {
    #[serde(default)]
    entries: Vec<RecentProjectEntry>,
}

/// Resolve the recent-projects JSON path under the editor profile directory.
#[must_use]
pub fn recent_projects_path() -> PathBuf {
    AzothDataHome::resolve().recent_projects_path()
}

/// Load the recorded recent projects, most-recently-opened first. A missing or
/// unreadable file yields an empty list (non-fatal).
#[must_use]
pub fn load_recent_projects() -> Vec<RecentProjectEntry> {
    prepare_data_home();
    load_recent_projects_from(&recent_projects_path())
}

fn load_recent_projects_from(path: &Path) -> Vec<RecentProjectEntry> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut file: RecentProjectsFile = match serde_json::from_str(&text) {
        Ok(file) => file,
        Err(error) => {
            warn!(%error, path = %path.display(), "failed to parse recent projects file");
            return Vec::new();
        }
    };
    file.entries
        .sort_by_key(|entry| std::cmp::Reverse(entry.last_opened_unix_ms));
    file.entries
}

/// Record (or refresh) a project in the recent list, persisting immediately.
/// De-duplicates by absolute path; failures to persist are non-fatal.
pub fn record_recent_project(entry: RecentProjectEntry) {
    prepare_data_home();
    record_recent_project_at(&recent_projects_path(), entry);
}

fn prepare_data_home() {
    if let Err(error) = AzothDataHome::resolve().prepare() {
        warn!(%error, "failed to migrate recent-project preferences");
    }
}

fn record_recent_project_at(path: &Path, mut entry: RecentProjectEntry) {
    if entry.last_opened_unix_ms == 0 {
        entry.last_opened_unix_ms = current_unix_ms();
    }
    let mut file = RecentProjectsFile {
        entries: load_recent_projects_from(path),
    };
    file.entries
        .retain(|existing| !same_project(existing, &entry));
    file.entries.insert(0, entry);
    // Keep the list bounded; the launcher only shows a handful.
    file.entries.truncate(64);

    if let Some(parent) = path.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        warn!(%error, path = %parent.display(), "failed to create recent projects directory");
        return;
    }
    match serde_json::to_string_pretty(&file) {
        Ok(text) => {
            if let Err(error) = std::fs::write(path, text) {
                warn!(%error, path = %path.display(), "failed to write recent projects file");
            }
        }
        Err(error) => warn!(%error, "failed to serialize recent projects file"),
    }
}

fn same_project(left: &RecentProjectEntry, right: &RecentProjectEntry) -> bool {
    normalize_path(&left.path) == normalize_path(&right.path)
}

fn normalize_path(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

/// Wall-clock Unix milliseconds, saturating at `i64::MAX` and yielding 0 if the
/// clock predates the epoch. Shared with the launcher so recorded stamps and the
/// "last opened" text are read on the same scale.
pub(crate) fn current_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        // Saturating at `i64::MAX` is unreachable until the year 292'277'026'596.
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_dedupes_by_path_keeping_latest_first() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("recent-projects.json");
        let alpha = temp
            .path()
            .join("alpha")
            .to_string_lossy()
            .replace('\\', "/");
        let beta = temp
            .path()
            .join("beta")
            .to_string_lossy()
            .replace('\\', "/");
        let alpha_backslashes = alpha.replace('/', "\\");

        record_recent_project_at(
            &path,
            RecentProjectEntry {
                id: "local.alpha".to_string(),
                name: "Alpha".to_string(),
                path: alpha,
                engine_version: "0.1.0".to_string(),
                last_opened_unix_ms: 100,
            },
        );
        record_recent_project_at(
            &path,
            RecentProjectEntry {
                id: "local.beta".to_string(),
                name: "Beta".to_string(),
                path: beta,
                engine_version: "0.1.0".to_string(),
                last_opened_unix_ms: 200,
            },
        );
        // Re-open alpha (different slash style + newer time) must dedupe.
        record_recent_project_at(
            &path,
            RecentProjectEntry {
                id: "local.alpha".to_string(),
                name: "Alpha".to_string(),
                path: alpha_backslashes,
                engine_version: "0.1.0".to_string(),
                last_opened_unix_ms: 300,
            },
        );

        let entries = load_recent_projects_from(&path);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "local.alpha");
        assert_eq!(entries[0].last_opened_unix_ms, 300);
        assert_eq!(entries[1].id, "local.beta");
    }

    #[test]
    fn missing_file_loads_empty() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("does-not-exist.json");
        assert!(load_recent_projects_from(&path).is_empty());
    }
}
