//! Project-instance service supervision state.
//!
//! The project host, asset processor, and worker pool have the same lifetime as
//! the project-local data paths they serve. Sessions attach to their published
//! descriptors; they never own these process records.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use az_filesystem::{
    FileTransaction, FileTransactionError, FileWrite, ProjectDataPaths, normalize,
};
use az_proto_core::{ServiceDescriptor, ServiceId, ServiceRole};
use az_service_supervision::{
    ServiceProcessError, ServiceProcessKey, ServiceProcessRecord, ServiceProcessState,
    ServiceRecord, SupervisedServiceRole,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROJECT_SERVICE_MANIFEST_FILE: &str = "manifest.toml";
pub const PROJECT_SERVICE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectServiceManifest {
    pub schema_version: u32,
    pub project_id: String,
    pub project_root: PathBuf,
    pub updated_unix_ms: u128,
    #[serde(default)]
    pub services: Vec<ServiceRecord>,
    #[serde(default)]
    pub processes: Vec<ServiceProcessRecord>,
}

impl ProjectServiceManifest {
    #[must_use]
    pub fn new(project_id: impl Into<String>, project_root: PathBuf, now_unix_ms: u128) -> Self {
        Self {
            schema_version: PROJECT_SERVICE_SCHEMA_VERSION,
            project_id: project_id.into(),
            project_root,
            updated_unix_ms: now_unix_ms,
            services: Vec::new(),
            processes: Vec::new(),
        }
    }

    /// Insert or replace the record for one service descriptor.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectServiceError::UnsupportedRole`] when `descriptor` names
    /// a role that has no supervised-service record form.
    pub fn upsert_service_descriptor(
        &mut self,
        descriptor: &ServiceDescriptor,
        now_unix_ms: u128,
    ) -> Result<ServiceRecord, ProjectServiceError> {
        let record = ServiceRecord::from_descriptor(descriptor).ok_or(
            ProjectServiceError::UnsupportedRole {
                role: descriptor.role,
            },
        )?;
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
        Ok(record)
    }

    pub fn upsert_process(&mut self, process: ServiceProcessRecord, now_unix_ms: u128) {
        let key = ServiceProcessKey::from_process(&process);
        if let Some(existing) = self
            .processes
            .iter_mut()
            .find(|existing| ServiceProcessKey::from_process(existing) == key)
        {
            let mut process = process;
            process.previous_run = Some(existing.run);
            *existing = process;
        } else {
            self.processes.push(process);
        }
        self.updated_unix_ms = now_unix_ms;
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

    #[must_use]
    pub fn current_process(&self, key: &ServiceProcessKey) -> Option<&ServiceProcessRecord> {
        self.processes
            .iter()
            .find(|process| ServiceProcessKey::from_process(process) == *key)
    }

    pub fn current_process_mut(
        &mut self,
        key: &ServiceProcessKey,
    ) -> Option<&mut ServiceProcessRecord> {
        self.processes
            .iter_mut()
            .find(|process| ServiceProcessKey::from_process(process) == *key)
    }

    #[must_use]
    pub fn running_descriptors(&self) -> Vec<ServiceDescriptor> {
        self.services
            .iter()
            .filter(|service| {
                let key = ServiceProcessKey::new(&service.name, service.role);
                self.current_process(&key)
                    .is_some_and(|process| process.state == ServiceProcessState::Running)
            })
            .map(ServiceRecord::to_descriptor)
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct ProjectServiceStore {
    paths: ProjectDataPaths,
    project_id: String,
    project_root: PathBuf,
}

impl ProjectServiceStore {
    /// Open the project's service store, creating its directory tree.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectServiceError::Io`] when the services directory cannot
    /// be created, or [`ProjectServiceError::Transaction`] when a prior
    /// interrupted manifest write cannot be recovered.
    pub fn new(
        paths: ProjectDataPaths,
        project_id: impl Into<String>,
        project_root: PathBuf,
    ) -> Result<Self, ProjectServiceError> {
        let store = Self {
            paths,
            project_id: project_id.into(),
            project_root,
        };
        fs::create_dir_all(store.root())?;
        FileTransaction::new(store.transactions_dir()).recover_pending()?;
        Ok(store)
    }

    #[must_use]
    pub fn root(&self) -> PathBuf {
        self.paths.services_dir()
    }

    #[must_use]
    pub fn logs_dir(&self) -> PathBuf {
        self.root().join("logs")
    }

    #[must_use]
    pub fn ready_dir(&self) -> PathBuf {
        self.root().join("ready")
    }

    #[must_use]
    pub fn grants_dir(&self) -> PathBuf {
        self.root().join("capability-grants")
    }

    #[must_use]
    pub fn side_channels_dir(&self) -> PathBuf {
        self.root().join("side-channels")
    }

    #[must_use]
    pub fn asset_db_path(&self) -> PathBuf {
        self.paths.asset_db_path()
    }

    #[must_use]
    pub fn asset_processing_staging_dir(&self) -> PathBuf {
        self.paths.asset_processing_staging_dir()
    }

    #[must_use]
    pub fn product_cache_dir(&self) -> PathBuf {
        self.paths.default_product_cache_dir()
    }

    #[must_use]
    pub fn manifest_path(&self) -> PathBuf {
        self.root().join(PROJECT_SERVICE_MANIFEST_FILE)
    }

    fn transactions_dir(&self) -> PathBuf {
        self.root().join("transactions")
    }

    /// Load the stored manifest, or synthesize an empty one when none exists.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectServiceError::Decode`] when the manifest file is not
    /// valid TOML, any error [`Self::validate`] returns when the stored
    /// manifest disagrees with this store's project identity, and
    /// [`ProjectServiceError::Io`] for any read failure other than a missing
    /// file (which yields a fresh manifest instead).
    pub fn load_or_create(
        &self,
        now_unix_ms: u128,
    ) -> Result<ProjectServiceManifest, ProjectServiceError> {
        match fs::read_to_string(self.manifest_path()) {
            Ok(text) => {
                let manifest: ProjectServiceManifest = toml::from_str(&text)?;
                self.validate(&manifest)?;
                Ok(manifest)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(ProjectServiceManifest::new(
                    self.project_id.clone(),
                    self.project_root.clone(),
                    now_unix_ms,
                ))
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Commit `manifest` together with `writes` as one atomic transaction.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::validate`] returns for `manifest`,
    /// [`ProjectServiceError::Io`] when the services or transactions directory
    /// cannot be created, [`ProjectServiceError::Encode`] when the manifest
    /// cannot be serialized as TOML, and [`ProjectServiceError::Transaction`]
    /// when pending transactions cannot be recovered or the commit fails.
    pub fn write_with_files(
        &self,
        manifest: &ProjectServiceManifest,
        mut writes: Vec<FileWrite>,
    ) -> Result<(), ProjectServiceError> {
        self.validate(manifest)?;
        fs::create_dir_all(self.root())?;
        fs::create_dir_all(self.transactions_dir())?;
        FileTransaction::new(self.transactions_dir()).recover_pending()?;
        writes.push(FileWrite::new(
            self.manifest_path(),
            toml::to_string_pretty(manifest)?.into_bytes(),
        ));
        FileTransaction::new(self.transactions_dir()).commit(writes)?;
        Ok(())
    }

    /// Commit `manifest` on its own.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::write_with_files`] returns.
    pub fn write(&self, manifest: &ProjectServiceManifest) -> Result<(), ProjectServiceError> {
        self.write_with_files(manifest, Vec::new())
    }

    /// Reject a manifest that does not belong to this store.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectServiceError::InvalidManifest`] when the schema
    /// version, project id, or project root disagrees with this store, when a
    /// service descriptor or process key is duplicated, when a project service
    /// carries a session-scoped capability, or when a service or process claims
    /// a role that projects do not own.
    fn validate(&self, manifest: &ProjectServiceManifest) -> Result<(), ProjectServiceError> {
        if manifest.schema_version != PROJECT_SERVICE_SCHEMA_VERSION {
            return Err(ProjectServiceError::InvalidManifest(format!(
                "schema version {} does not match {}",
                manifest.schema_version, PROJECT_SERVICE_SCHEMA_VERSION
            )));
        }
        if manifest.project_id != self.project_id {
            return Err(ProjectServiceError::InvalidManifest(format!(
                "project id `{}` does not match `{}`",
                manifest.project_id, self.project_id
            )));
        }
        if normalize(&manifest.project_root) != normalize(&self.project_root) {
            return Err(ProjectServiceError::InvalidManifest(format!(
                "project root `{}` does not match `{}`",
                manifest.project_root.display(),
                self.project_root.display()
            )));
        }
        let mut service_keys = BTreeSet::new();
        for service in &manifest.services {
            validate_project_role(service.role)?;
            if !service_keys.insert((
                service.namespace.as_str(),
                service.name.as_str(),
                service.role,
            )) {
                return Err(ProjectServiceError::InvalidManifest(format!(
                    "service descriptor `{}/{}` with role {:?} is duplicated",
                    service.namespace, service.name, service.role
                )));
            }
            if service
                .capabilities
                .iter()
                .any(|capability| capability.session.is_some())
            {
                return Err(ProjectServiceError::InvalidManifest(format!(
                    "project service `{}` contains a session-scoped capability",
                    service.name
                )));
            }
        }

        let mut process_keys = BTreeSet::new();
        for process in &manifest.processes {
            validate_project_role(process.role)?;
            if !process_keys.insert(ServiceProcessKey::from_process(process)) {
                return Err(ProjectServiceError::InvalidManifest(format!(
                    "project process `{}` role {:?} is duplicated",
                    process.service_name, process.role
                )));
            }
        }
        Ok(())
    }
}

fn validate_project_role(role: SupervisedServiceRole) -> Result<(), ProjectServiceError> {
    if matches!(
        role,
        SupervisedServiceRole::ProjectHost
            | SupervisedServiceRole::AssetProcessor
            | SupervisedServiceRole::Worker
    ) {
        Ok(())
    } else {
        Err(ProjectServiceError::InvalidManifest(format!(
            "session/global role {role:?} cannot be owned by project services"
        )))
    }
}

#[derive(Debug, Error)]
pub enum ProjectServiceError {
    #[error("project service IO failed: {0}")]
    Io(#[from] std::io::Error),

    #[error("project service manifest decode failed: {0}")]
    Decode(#[from] toml::de::Error),

    #[error("project service manifest encode failed: {0}")]
    Encode(#[from] toml::ser::Error),

    #[error("project service transaction failed: {0}")]
    Transaction(#[from] FileTransactionError),

    #[error("project service process operation failed: {0}")]
    Process(#[from] ServiceProcessError),

    #[error("project service manifest is invalid: {0}")]
    InvalidManifest(String),

    #[error("role {role:?} cannot be represented by a supervised service record")]
    UnsupportedRole { role: ServiceRole },
}
