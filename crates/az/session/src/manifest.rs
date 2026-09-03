use std::path::PathBuf;

use az_observability::STRUCTURED_LOG_EXTENSION;
use az_proto_core::{ServiceDescriptor, ServiceId, ServiceRole};
use az_service_supervision::{ServiceProcessKey, ServiceProcessRecord, ServiceRecord};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const SESSION_MANIFEST_FILE: &str = "session.toml";
pub const SESSIOND_OUTPUT_LOG_FILE: &str = "az-sessiond.log";
pub const SESSIOND_STRUCTURED_LOG_STEM: &str = "az-sessiond";
// TODO(task #39): no migration framework yet — pre-release, do NOT bump this
// gratuitously; prefer additive/in-place format changes. An incompatible
// on-disk manifest is currently a hard error (`SessionManager::list_sessions`),
// so a stale/wrong-version session must be removed, not skipped or migrated.
// Replace with real upgrade-on-read migration + forward/backward compat (also
// covers the project registry, asset db, and project/engine manifest versions).
pub const SESSION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SessionId(pub Uuid);

impl SessionId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SessionState {
    Preparing,
    Active,
    FailedPreserved,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionManifest {
    pub schema_version: u32,
    pub id: SessionId,
    pub project_id: String,
    pub slug: String,
    pub project_root: PathBuf,
    pub workspace_root: PathBuf,
    pub run_dir: PathBuf,
    pub state: SessionState,
    pub created_unix_ms: u128,
    pub updated_unix_ms: u128,
    #[serde(default)]
    pub services: Vec<ServiceRecord>,
    #[serde(default)]
    pub processes: Vec<ServiceProcessRecord>,
}

impl SessionManifest {
    #[must_use]
    pub const fn new(
        id: SessionId,
        project_id: String,
        slug: String,
        project_root: PathBuf,
        workspace_root: PathBuf,
        run_dir: PathBuf,
        now_unix_ms: u128,
    ) -> Self {
        Self {
            schema_version: SESSION_SCHEMA_VERSION,
            id,
            project_id,
            slug,
            project_root,
            workspace_root,
            run_dir,
            state: SessionState::Preparing,
            created_unix_ms: now_unix_ms,
            updated_unix_ms: now_unix_ms,
            services: Vec::new(),
            processes: Vec::new(),
        }
    }

    pub const fn activate(&mut self, now_unix_ms: u128) {
        self.state = SessionState::Active;
        self.updated_unix_ms = now_unix_ms;
    }

    pub const fn preserve_failure(&mut self, now_unix_ms: u128) {
        self.state = SessionState::FailedPreserved;
        self.updated_unix_ms = now_unix_ms;
    }

    #[must_use]
    pub fn manifest_path(&self) -> PathBuf {
        self.run_dir.join(SESSION_MANIFEST_FILE)
    }

    pub fn upsert_service_descriptor(
        &mut self,
        descriptor: &ServiceDescriptor,
        now_unix_ms: u128,
    ) -> Option<ServiceRecord> {
        let record = ServiceRecord::from_descriptor(descriptor)?;
        if let Some(existing) = self.services.iter_mut().find(|existing| {
            existing.namespace == record.namespace
                && existing.name == record.name
                && existing.role == record.role
        }) {
            *existing = record.clone();
        } else {
            self.services.push(record.clone());
        }
        self.updated_unix_ms = now_unix_ms;
        Some(record)
    }

    /// Attach a descriptor owned by another supervision scope without claiming
    /// its process lifecycle.
    pub fn attach_service_descriptor(
        &mut self,
        descriptor: &ServiceDescriptor,
        now_unix_ms: u128,
    ) -> Option<ServiceRecord> {
        let record = ServiceRecord::from_descriptor(descriptor)?;
        if let Some(existing) = self.services.iter_mut().find(|existing| {
            existing.namespace == record.namespace
                && existing.name == record.name
                && existing.role == record.role
        }) {
            *existing = record.clone();
        } else {
            self.services.push(record.clone());
        }
        self.updated_unix_ms = now_unix_ms;
        Some(record)
    }

    #[must_use]
    pub fn service_descriptor(
        &self,
        id: &ServiceId,
        role: ServiceRole,
    ) -> Option<ServiceDescriptor> {
        self.services
            .iter()
            .find(|record| record.matches(id, role))
            .map(ServiceRecord::to_descriptor)
    }

    pub fn upsert_process_record(&mut self, mut process: ServiceProcessRecord, now_unix_ms: u128) {
        let key = ServiceProcessKey::from_process(&process);
        if let Some(existing) = self
            .processes
            .iter_mut()
            .find(|existing| ServiceProcessKey::from_process(existing) == key)
        {
            process.previous_run = Some(existing.run);
            *existing = process;
        } else {
            self.processes.push(process);
        }
        self.updated_unix_ms = now_unix_ms;
    }

    #[must_use]
    pub fn current_service_process_index(&self, key: &ServiceProcessKey) -> Option<usize> {
        self.processes
            .iter()
            .position(|process| ServiceProcessKey::from_process(process) == *key)
    }

    pub fn service_process_mut(
        &mut self,
        key: &ServiceProcessKey,
    ) -> Option<&mut ServiceProcessRecord> {
        let index = self.current_service_process_index(key)?;
        self.processes.get_mut(index)
    }
}

#[must_use]
pub fn sessiond_output_log_path(manifest: &SessionManifest) -> PathBuf {
    manifest.run_dir.join(SESSIOND_OUTPUT_LOG_FILE)
}

#[must_use]
pub fn sessiond_structured_log_path(manifest: &SessionManifest) -> PathBuf {
    manifest.run_dir.join("logs").join(format!(
        "{SESSIOND_STRUCTURED_LOG_STEM}.{STRUCTURED_LOG_EXTENSION}"
    ))
}
