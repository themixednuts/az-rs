//! Durable current session-supervisor lease.
//!
//! Each record pins the lease to a [`ProcessIdentity`], so a lease survives
//! only as long as the process that took it and never transfers to whatever
//! inherits its pid.

use std::fs::{self, OpenOptions};
use std::path::PathBuf;

use az_filesystem::{FileTransaction, FileTransactionError, FileWrite};
use az_proto_core::{ServiceDescriptor, ServiceRole};
use az_service_supervision::{ProcessIdentity, ServiceRecord};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::instrument;

pub const SESSION_SUPERVISOR_LEASE_FILE: &str = "supervisor-leases.toml";
pub const SESSION_SUPERVISOR_LEASE_SCHEMA_VERSION: u32 = 1;
pub const SESSION_SUPERVISOR_LEASE_HEARTBEAT_INTERVAL_MS: u64 = 5_000;
pub const SESSION_SUPERVISOR_LEASE_EXPIRY_MS: u64 = 20_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSupervisorLeaseRecord {
    pub process: ProcessIdentity,
    pub served_descriptor: ServiceRecord,
    pub heartbeat_unix_ms: u64,
}

impl SessionSupervisorLeaseRecord {
    #[must_use]
    pub fn descriptor(&self) -> ServiceDescriptor {
        self.served_descriptor.to_descriptor()
    }

    #[must_use]
    pub const fn heartbeat_expired(&self, now_unix_ms: u64) -> bool {
        now_unix_ms.saturating_sub(self.heartbeat_unix_ms) > SESSION_SUPERVISOR_LEASE_EXPIRY_MS
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSupervisorLeaseState {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<SessionSupervisorLeaseRecord>,
}

impl Default for SessionSupervisorLeaseState {
    fn default() -> Self {
        Self {
            schema_version: SESSION_SUPERVISOR_LEASE_SCHEMA_VERSION,
            record: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum SessionSupervisorLeaseError {
    #[error("session-supervisor lease IO failed: {0}")]
    Io(#[from] std::io::Error),

    #[error("session-supervisor lease TOML decode failed: {0}")]
    Decode(#[from] toml::de::Error),

    #[error("session-supervisor lease TOML encode failed: {0}")]
    Encode(#[from] toml::ser::Error),

    #[error("session-supervisor lease transaction failed: {0}")]
    Transaction(#[from] FileTransactionError),

    #[error("session-supervisor lease schema {actual} is not supported; expected {expected}")]
    UnsupportedSchema { actual: u32, expected: u32 },

    #[error("session-supervisor lease descriptor has unsupported role {0:?}")]
    InvalidDescriptorRole(ServiceRole),

    #[error("session-supervisor lease for process {process_id} is not current")]
    MissingLease { process_id: u32 },
}

#[derive(Debug, Clone)]
pub struct SessionSupervisorLeaseStore {
    run_dir: PathBuf,
}

impl SessionSupervisorLeaseStore {
    #[must_use]
    pub fn new(run_dir: impl Into<PathBuf>) -> Self {
        Self {
            run_dir: run_dir.into(),
        }
    }

    #[must_use]
    pub fn path(&self) -> PathBuf {
        self.run_dir.join(SESSION_SUPERVISOR_LEASE_FILE)
    }

    /// Reads the current lease record under the store's exclusive file lock.
    ///
    /// # Errors
    ///
    /// Returns [`SessionSupervisorLeaseError::Io`] if the run directory cannot
    /// be created, the lock file cannot be opened or exclusively locked, or the
    /// lease file cannot be read; [`SessionSupervisorLeaseError::Transaction`]
    /// if a pending lease write cannot be recovered;
    /// [`SessionSupervisorLeaseError::Decode`] if the lease file is not valid
    /// TOML; and [`SessionSupervisorLeaseError::UnsupportedSchema`] if the
    /// recorded schema version is not
    /// [`SESSION_SUPERVISOR_LEASE_SCHEMA_VERSION`].
    #[instrument(skip_all, fields(path = %self.path().display()))]
    pub fn load(&self) -> Result<SessionSupervisorLeaseState, SessionSupervisorLeaseError> {
        self.with_lock(Self::read_unlocked)
    }

    #[instrument(
        skip_all,
        fields(
            path = %self.path().display(),
            run = %descriptor.run,
            process_id = process.process_id
        )
    )]
    /// Takes the lease for `process`, replacing whatever record is current.
    ///
    /// # Errors
    ///
    /// Returns [`SessionSupervisorLeaseError::InvalidDescriptorRole`] if
    /// `descriptor` is not a [`ServiceRole::SessionSupervisor`] descriptor or
    /// cannot be reduced to a [`ServiceRecord`], any error [`Self::load`]
    /// returns while reading the current record, and
    /// [`SessionSupervisorLeaseError::Encode`],
    /// [`SessionSupervisorLeaseError::Transaction`], or
    /// [`SessionSupervisorLeaseError::Io`] if the replacement record cannot be
    /// serialized or committed.
    pub fn acquire(
        &self,
        descriptor: &ServiceDescriptor,
        process: ProcessIdentity,
        now_unix_ms: u64,
    ) -> Result<SessionSupervisorLeaseRecord, SessionSupervisorLeaseError> {
        if descriptor.role != ServiceRole::SessionSupervisor {
            return Err(SessionSupervisorLeaseError::InvalidDescriptorRole(
                descriptor.role,
            ));
        }
        let served_descriptor = ServiceRecord::from_descriptor(descriptor).ok_or(
            SessionSupervisorLeaseError::InvalidDescriptorRole(descriptor.role),
        )?;
        self.with_lock(|store| {
            let mut state = store.read_unlocked()?;
            let record = SessionSupervisorLeaseRecord {
                process,
                served_descriptor,
                heartbeat_unix_ms: now_unix_ms,
            };
            state.record = Some(record.clone());
            store.write_unlocked(&state)?;
            Ok(record)
        })
    }

    /// Stamps a fresh heartbeat on the lease `process` already holds.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::load`] returns while reading the current
    /// record, [`SessionSupervisorLeaseError::MissingLease`] if no record is
    /// present or it belongs to a different process, and
    /// [`SessionSupervisorLeaseError::Encode`],
    /// [`SessionSupervisorLeaseError::Transaction`], or
    /// [`SessionSupervisorLeaseError::Io`] if the stamped record cannot be
    /// serialized or committed.
    pub fn renew(
        &self,
        process: ProcessIdentity,
        now_unix_ms: u64,
    ) -> Result<(), SessionSupervisorLeaseError> {
        self.with_lock(|store| {
            let mut state = store.read_unlocked()?;
            let record = state
                .record
                .as_mut()
                .filter(|record| record.process == process)
                .ok_or(SessionSupervisorLeaseError::MissingLease {
                    process_id: process.process_id,
                })?;
            record.heartbeat_unix_ms = now_unix_ms;
            store.write_unlocked(&state)?;
            Ok(())
        })
    }

    #[instrument(skip_all, fields(path = %self.path().display(), process_id = process.process_id))]
    /// Drops the current lease when `process` owns it, reporting whether a
    /// record was removed.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::load`] returns while reading the current
    /// record, and — only when the record is `process`'s and is therefore
    /// rewritten — [`SessionSupervisorLeaseError::Encode`],
    /// [`SessionSupervisorLeaseError::Transaction`], or
    /// [`SessionSupervisorLeaseError::Io`] if the cleared state cannot be
    /// serialized or committed.
    pub fn clear_if_process(
        &self,
        process: ProcessIdentity,
    ) -> Result<bool, SessionSupervisorLeaseError> {
        self.with_lock(|store| {
            let mut state = store.read_unlocked()?;
            let removed = state
                .record
                .as_ref()
                .is_some_and(|record| record.process == process);
            if removed {
                state.record = None;
                store.write_unlocked(&state)?;
            }
            Ok(removed)
        })
    }

    fn with_lock<T>(
        &self,
        operation: impl FnOnce(&Self) -> Result<T, SessionSupervisorLeaseError>,
    ) -> Result<T, SessionSupervisorLeaseError> {
        fs::create_dir_all(&self.run_dir)?;
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            // The lock file carries no content; it is only ever `flock`ed, so
            // an existing one must keep whatever bytes it already has.
            .truncate(false)
            .open(self.run_dir.join("supervisor-leases.lock"))?;
        lock.lock_exclusive()?;
        operation(self)
    }

    fn read_unlocked(&self) -> Result<SessionSupervisorLeaseState, SessionSupervisorLeaseError> {
        FileTransaction::new(self.transaction_root()).recover_pending()?;
        let path = self.path();
        if !path.exists() {
            return Ok(SessionSupervisorLeaseState::default());
        }
        let state: SessionSupervisorLeaseState = toml::from_str(&fs::read_to_string(path)?)?;
        if state.schema_version != SESSION_SUPERVISOR_LEASE_SCHEMA_VERSION {
            return Err(SessionSupervisorLeaseError::UnsupportedSchema {
                actual: state.schema_version,
                expected: SESSION_SUPERVISOR_LEASE_SCHEMA_VERSION,
            });
        }
        Ok(state)
    }

    fn write_unlocked(
        &self,
        state: &SessionSupervisorLeaseState,
    ) -> Result<(), SessionSupervisorLeaseError> {
        let text = toml::to_string_pretty(state)?;
        FileTransaction::new(self.transaction_root())
            .commit([FileWrite::new(self.path(), text.into_bytes())])?;
        Ok(())
    }

    fn transaction_root(&self) -> PathBuf {
        self.run_dir.join("supervisor-lease-transactions")
    }
}

#[cfg(test)]
mod tests {
    use az_proto_core::{Endpoint, EndpointKind, ServiceDescriptor, ServiceId};

    use super::*;

    fn descriptor(endpoint: Endpoint) -> ServiceDescriptor {
        ServiceDescriptor::new(
            ServiceId::new("azoth", "session-supervisor"),
            ServiceRole::SessionSupervisor,
            endpoint,
        )
        .with_run(uuid::Uuid::now_v7())
    }

    #[test]
    fn current_lease_preserves_explicit_tcp_endpoint() {
        let temp = tempfile::tempdir().unwrap();
        let store = SessionSupervisorLeaseStore::new(temp.path());
        let expected = descriptor(Endpoint::new(EndpointKind::Tcp, "127.0.0.1:43721"));
        let process = ProcessIdentity {
            process_id: 41,
            process_start_time: 72,
        };

        let record = store.acquire(&expected, process, 100).unwrap();
        let current = store.load().unwrap().record.unwrap();

        assert_eq!(record.descriptor(), expected);
        assert_eq!(current.descriptor(), expected);
        assert_eq!(current.process, process);
    }

    #[test]
    fn acquiring_a_lease_replaces_the_single_current_record() {
        let temp = tempfile::tempdir().unwrap();
        let store = SessionSupervisorLeaseStore::new(temp.path());
        let displaced = ProcessIdentity {
            process_id: 41,
            process_start_time: 72,
        };
        store
            .acquire(
                &descriptor(Endpoint::new(EndpointKind::Tcp, "127.0.0.1:40001")),
                displaced,
                100,
            )
            .unwrap();
        let current = ProcessIdentity {
            process_id: 42,
            process_start_time: 73,
        };
        let expected = descriptor(Endpoint::new(EndpointKind::Tcp, "127.0.0.1:40002"));
        store.acquire(&expected, current, 101).unwrap();

        let state = store.load().unwrap();
        let record = state.record.unwrap();
        assert_eq!(record.process, current);
        assert_eq!(record.descriptor(), expected);
        assert!(matches!(
            store.renew(displaced, 102),
            Err(SessionSupervisorLeaseError::MissingLease { process_id: 41 })
        ));
    }

    #[test]
    fn current_process_identity_is_repeatable() {
        let first = ProcessIdentity::current().unwrap();
        let second = ProcessIdentity::capture(first.process_id).unwrap().unwrap();
        assert_eq!(first, second);
    }
}
