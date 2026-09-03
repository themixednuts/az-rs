//! Machine-local endpoint discovery and record persistence for the Azoth
//! user-level `azd` daemon.
//!
//! This is the filesystem/data-home side of daemon endpoint management. It is
//! deliberately kept out of the `az-proto-daemon` ABI crate so protocol wire
//! types stay free of `az-filesystem` and machine-local runtime concerns.

use std::path::{Path, PathBuf};

use az_filesystem::{
    AzothDataHome, FileTransaction, FileTransactionError, FileWrite, ProjectMachineKey,
};
use serde::{Deserialize, Serialize};
use thiserror::Error as ThisError;

use az_proto_core as core;

#[derive(Debug, ThisError)]
pub enum DaemonEndpointRecordError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to read azd endpoint record {path}: {source}")]
    EndpointRecordRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write azd endpoint record {path}: {source}")]
    EndpointRecordWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse azd endpoint record {path}: {source}")]
    EndpointRecordParse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("failed to encode azd endpoint record: {0}")]
    EndpointRecordEncode(#[from] toml::ser::Error),

    #[error("failed to recover azd endpoint record transactions under {path}: {source}")]
    EndpointRecordTransactionRecovery {
        path: PathBuf,
        #[source]
        source: FileTransactionError,
    },

    #[error("failed to commit azd endpoint record transaction for {path}: {source}")]
    EndpointRecordTransaction {
        path: PathBuf,
        #[source]
        source: FileTransactionError,
    },

    #[error("failed to read project manifest {path} for endpoint isolation: {source}")]
    ProjectManifestRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse project manifest {path} for endpoint isolation: {source}")]
    ProjectManifestParse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("project manifest {path} has no non-empty project id")]
    ProjectManifestId { path: PathBuf },

    #[error("machine-local Azoth data-home error: {0}")]
    DataHome(#[from] az_filesystem::DataHomeError),

    #[error("azd endpoint record {path} has unsupported endpoint kind `{kind}`")]
    EndpointRecordKind { path: PathBuf, kind: String },

    #[error(
        "{operation} cannot use endpoint kind {kind:?}; use platform IPC or explicit TCP debug endpoints"
    )]
    UnsupportedEndpointKind {
        operation: &'static str,
        kind: core::EndpointKind,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonEndpointRecord {
    pub endpoint: core::Endpoint,
    pub process_id: u32,
    pub protocol_version: core::ProtocolVersion,
}

impl DaemonEndpointRecord {
    /// Verifies the daemon build recorded beside this endpoint before a client
    /// opens the transport.
    ///
    /// # Errors
    ///
    /// Returns [`core::ProtocolVersionMismatch`] when the daemon that published
    /// the record must be restarted for the current lockstep protocol.
    pub fn require_protocol_version(
        &self,
        expected: core::ProtocolVersion,
    ) -> Result<(), core::ProtocolVersionMismatch> {
        self.protocol_version.require(expected)
    }
}

#[derive(Debug)]
pub struct DaemonEndpointRecordGuard {
    path: PathBuf,
}

impl DaemonEndpointRecordGuard {
    #[must_use]
    pub const fn new(path: PathBuf) -> Self {
        Self { path }
    }

    // Returning `&Path` from a `PathBuf` field relies on `Deref`, which is not
    // a `const` operation, so this accessor cannot be `const fn`.
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for DaemonEndpointRecordGuard {
    fn drop(&mut self) {
        let _ = remove_daemon_endpoint_record_at(&self.path);
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct DaemonEndpointRecordFile {
    kind: String,
    address: String,
    process_id: u32,
    #[serde(default)]
    protocol_major: u16,
    #[serde(default)]
    protocol_minor: u16,
    #[serde(default)]
    protocol_patch: u16,
}

#[derive(Debug, Deserialize)]
struct ProjectIdentityManifest {
    project: ProjectIdentity,
}

#[derive(Debug, Deserialize)]
struct ProjectIdentity {
    id: String,
}

#[must_use]
pub const fn default_daemon_endpoint_kind() -> core::EndpointKind {
    if cfg!(windows) {
        core::EndpointKind::WindowsNamedPipe
    } else {
        core::EndpointKind::UnixDomainSocket
    }
}

/// The default machine-global `azd` endpoint for `kind`.
///
/// # Errors
///
/// Returns [`DaemonEndpointRecordError`] if `kind` is not a public transport or
/// the machine-local runtime directory cannot be prepared.
pub fn default_daemon_endpoint(
    kind: core::EndpointKind,
) -> Result<core::Endpoint, DaemonEndpointRecordError> {
    validate_public_endpoint_kind(kind, "azd default endpoint")?;
    let address = match kind {
        core::EndpointKind::WindowsNamedPipe => r"\\.\pipe\azoth-daemon".to_string(),
        core::EndpointKind::UnixDomainSocket => {
            let dir = daemon_runtime_dir()?;
            std::fs::create_dir_all(&dir)?;
            dir.join("daemon.sock").to_string_lossy().into_owned()
        }
        core::EndpointKind::Tcp => "127.0.0.1:37612".to_string(),
        core::EndpointKind::InProcess => unreachable!("validated above"),
    };
    Ok(core::Endpoint::new(kind, address))
}

/// The machine-global `azd` endpoint record path, preparing the data home.
///
/// # Errors
///
/// Returns [`DaemonEndpointRecordError`] if the machine-local data home or its
/// runtime directory cannot be prepared.
pub fn daemon_endpoint_record_path() -> Result<PathBuf, DaemonEndpointRecordError> {
    let data_home = AzothDataHome::resolve();
    daemon_endpoint_record_path_in(&data_home)
}

/// The machine-global `azd` endpoint record path under an explicit data home.
///
/// This is the ownership-preserving entry point for embedded hosts and tests;
/// process entry points normally use [`daemon_endpoint_record_path`].
///
/// # Errors
///
/// Returns [`DaemonEndpointRecordError::DataHome`] if `data_home` cannot be
/// prepared, or [`DaemonEndpointRecordError::Io`] if its runtime directory
/// cannot be created.
pub fn daemon_endpoint_record_path_in(
    data_home: &AzothDataHome,
) -> Result<PathBuf, DaemonEndpointRecordError> {
    data_home.prepare()?;
    std::fs::create_dir_all(data_home.runtime_dir())?;
    Ok(data_home.daemon_endpoint_record_path())
}

/// A short, stable, IPC-name-safe key for one Lore repository instance.
///
/// Uses the compact Lore instance id from `.lore/instance` (or
/// `unlinked-<path-hash>` before Lore exists). Same instance → same key;
/// different instances stay isolated even when the display name matches.
#[must_use]
pub fn project_endpoint_key(project_root: &Path) -> String {
    ProjectMachineKey::derive("project", project_root)
        .hash_suffix()
        .to_string()
}

/// Convert a user-facing endpoint label into a stable IPC-name component.
///
/// Endpoint discovery owns this normalization so daemons and supervised
/// services cannot drift into subtly different pipe/socket names.
#[must_use]
pub fn endpoint_token(value: &str) -> String {
    let mut token = String::with_capacity(value.len());
    let mut last_was_separator = true;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            token.push(ch.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator {
            token.push('-');
            last_was_separator = true;
        }
    }

    while token.ends_with('-') {
        token.pop();
    }

    if token.is_empty() {
        "service".to_string()
    } else {
        token
    }
}

/// Endpoint for a service owned by one project instance.
///
/// Durable descriptors remain under the project data home, but Unix sockets
/// live in the short machine runtime directory so the generated observability
/// and generation suffixes stay within `sockaddr_un::sun_path`.
///
/// # Errors
///
/// Returns any error [`project_service_endpoint_in`] returns for the resolved
/// machine-local data home.
pub fn project_service_endpoint(
    kind: core::EndpointKind,
    project_root: &Path,
    service_name: &str,
) -> Result<core::Endpoint, DaemonEndpointRecordError> {
    let data_home = AzothDataHome::resolve();
    project_service_endpoint_in(&data_home, kind, project_root, service_name)
}

/// Project-service endpoint using an explicit machine-local data home.
///
/// This variant keeps embedded hosts and tests isolated while sharing the same
/// production endpoint layout.
///
/// # Errors
///
/// Returns [`DaemonEndpointRecordError::UnsupportedEndpointKind`] if `kind` is
/// [`core::EndpointKind::InProcess`], which has no addressable name,
/// [`DaemonEndpointRecordError::DataHome`] if `data_home` cannot be prepared,
/// or [`DaemonEndpointRecordError::Io`] if the runtime directory backing a Unix
/// socket cannot be created.
pub fn project_service_endpoint_in(
    data_home: &AzothDataHome,
    kind: core::EndpointKind,
    project_root: &Path,
    service_name: &str,
) -> Result<core::Endpoint, DaemonEndpointRecordError> {
    validate_public_endpoint_kind(kind, "project service endpoint")?;
    data_home.prepare()?;

    let project_key = project_endpoint_key(project_root);
    let service_token = endpoint_token(service_name);
    let address = match kind {
        core::EndpointKind::WindowsNamedPipe => {
            format!(r"\\.\pipe\azoth-{project_key}-{service_token}")
        }
        core::EndpointKind::UnixDomainSocket => {
            let runtime_dir = data_home.runtime_dir();
            std::fs::create_dir_all(&runtime_dir)?;
            runtime_dir
                .join(format!("svc-{project_key}-{service_token}.sock"))
                .to_string_lossy()
                .into_owned()
        }
        core::EndpointKind::Tcp => "127.0.0.1:0".to_string(),
        core::EndpointKind::InProcess => unreachable!("validated above"),
    };
    Ok(core::Endpoint::new(kind, address))
}

/// Per-project daemon endpoint, isolated by [`project_endpoint_key`].
///
/// - Windows: a path-keyed named pipe `\\.\pipe\azoth-daemon-<key>`.
/// - Unix: a path-keyed socket in the runtime dir (kept short to respect the
///   `sun_path` length limit).
/// - TCP: an ephemeral `127.0.0.1:0`; the bound port is published in the
///   per-project record after the server binds.
///
/// # Errors
///
/// Returns [`DaemonEndpointRecordError`] if `kind` is not a public transport or
/// the machine-local runtime directory cannot be prepared.
pub fn project_daemon_endpoint(
    kind: core::EndpointKind,
    project_root: &Path,
) -> Result<core::Endpoint, DaemonEndpointRecordError> {
    validate_public_endpoint_kind(kind, "azd project endpoint")?;
    let key = project_endpoint_key(project_root);
    let address = match kind {
        core::EndpointKind::WindowsNamedPipe => format!(r"\\.\pipe\azoth-daemon-{key}"),
        core::EndpointKind::UnixDomainSocket => {
            let dir = daemon_runtime_dir()?;
            std::fs::create_dir_all(&dir)?;
            dir.join(format!("azd-{key}.sock"))
                .to_string_lossy()
                .into_owned()
        }
        core::EndpointKind::Tcp => "127.0.0.1:0".to_string(),
        core::EndpointKind::InProcess => unreachable!("validated above"),
    };
    Ok(core::Endpoint::new(kind, address))
}

/// The per-project endpoint record path beneath the shared machine-local key.
///
/// # Errors
///
/// Returns [`DaemonEndpointRecordError`] if the project manifest cannot be read
/// for its id or the per-project endpoint directory cannot be prepared.
pub fn project_daemon_endpoint_record_path(
    project_root: &Path,
) -> Result<PathBuf, DaemonEndpointRecordError> {
    let data_home = AzothDataHome::resolve();
    project_daemon_endpoint_record_path_in(&data_home, project_root)
}

/// The per-project endpoint record path under an explicit data home.
///
/// # Errors
///
/// Returns [`DaemonEndpointRecordError::ProjectManifestRead`],
/// [`DaemonEndpointRecordError::ProjectManifestParse`], or
/// [`DaemonEndpointRecordError::ProjectManifestId`] if the project's manifest
/// cannot be read, parsed, or carries no non-empty project id;
/// [`DaemonEndpointRecordError::DataHome`] if `data_home` or the per-project
/// directory cannot be prepared; or
/// [`DaemonEndpointRecordError::EndpointRecordWrite`] if the endpoints
/// directory cannot be created.
pub fn project_daemon_endpoint_record_path_in(
    data_home: &AzothDataHome,
    project_root: &Path,
) -> Result<PathBuf, DaemonEndpointRecordError> {
    let project_id = project_id_for_endpoint(project_root)?;
    data_home.prepare()?;
    let paths = data_home.project(&project_id, project_root);
    paths.prepare()?;
    let dir = paths.endpoints_dir();
    std::fs::create_dir_all(&dir).map_err(|source| {
        DaemonEndpointRecordError::EndpointRecordWrite {
            path: dir.clone(),
            source,
        }
    })?;
    Ok(paths.daemon_endpoint_record_path())
}

/// Read the per-project endpoint record, if present.
///
/// # Errors
///
/// Returns [`DaemonEndpointRecordError`] if the record path cannot be resolved
/// or the record cannot be read or parsed.
pub fn read_project_daemon_endpoint_record(
    project_root: &Path,
) -> Result<Option<DaemonEndpointRecord>, DaemonEndpointRecordError> {
    let data_home = AzothDataHome::resolve();
    read_project_daemon_endpoint_record_in(&data_home, project_root)
}

/// Read the per-project endpoint record from an explicit data home.
///
/// # Errors
///
/// Returns any error [`project_daemon_endpoint_record_path_in`] returns while
/// resolving the record path, plus
/// [`DaemonEndpointRecordError::EndpointRecordRead`] or
/// [`DaemonEndpointRecordError::EndpointRecordParse`] if an existing record
/// cannot be read or is not valid TOML. A missing record is `Ok(None)`.
pub fn read_project_daemon_endpoint_record_in(
    data_home: &AzothDataHome,
    project_root: &Path,
) -> Result<Option<DaemonEndpointRecord>, DaemonEndpointRecordError> {
    read_daemon_endpoint_record_at(&project_daemon_endpoint_record_path_in(
        data_home,
        project_root,
    )?)
}

/// Write the per-project endpoint record and return a cleanup guard.
///
/// # Errors
///
/// Returns [`DaemonEndpointRecordError`] if the record path cannot be resolved
/// or the record cannot be written.
pub fn write_project_daemon_endpoint_record(
    project_root: &Path,
    endpoint: &core::Endpoint,
) -> Result<DaemonEndpointRecordGuard, DaemonEndpointRecordError> {
    let data_home = AzothDataHome::resolve();
    write_project_daemon_endpoint_record_in(&data_home, project_root, endpoint)
}

/// Write the per-project endpoint record beneath an explicit data home.
///
/// # Errors
///
/// Returns any error [`project_daemon_endpoint_record_path_in`] returns while
/// resolving the record path, plus
/// [`DaemonEndpointRecordError::EndpointRecordEncode`] if the record cannot be
/// encoded as TOML, or
/// [`DaemonEndpointRecordError::EndpointRecordTransactionRecovery`] /
/// [`DaemonEndpointRecordError::EndpointRecordTransaction`] if the atomic write
/// cannot recover prior transactions or commit.
pub fn write_project_daemon_endpoint_record_in(
    data_home: &AzothDataHome,
    project_root: &Path,
    endpoint: &core::Endpoint,
) -> Result<DaemonEndpointRecordGuard, DaemonEndpointRecordError> {
    let path = project_daemon_endpoint_record_path_in(data_home, project_root)?;
    write_daemon_endpoint_record_at(&path, endpoint)?;
    Ok(DaemonEndpointRecordGuard::new(path))
}

/// Remove the per-project endpoint record if it exists.
///
/// # Errors
///
/// Returns [`DaemonEndpointRecordError`] if the record path cannot be resolved
/// or the record cannot be removed.
pub fn remove_project_daemon_endpoint_record(
    project_root: &Path,
) -> Result<(), DaemonEndpointRecordError> {
    remove_daemon_endpoint_record_at(&project_daemon_endpoint_record_path(project_root)?)
}

/// Read the machine-global endpoint record, if present.
///
/// # Errors
///
/// Returns [`DaemonEndpointRecordError`] if the record path cannot be resolved
/// or the record cannot be read or parsed.
pub fn read_daemon_endpoint_record()
-> Result<Option<DaemonEndpointRecord>, DaemonEndpointRecordError> {
    read_daemon_endpoint_record_at(&daemon_endpoint_record_path()?)
}

/// Write the machine-global endpoint record and return a cleanup guard.
///
/// # Errors
///
/// Returns [`DaemonEndpointRecordError`] if the record path cannot be resolved
/// or the record cannot be written.
pub fn write_daemon_endpoint_record(
    endpoint: &core::Endpoint,
) -> Result<DaemonEndpointRecordGuard, DaemonEndpointRecordError> {
    let path = daemon_endpoint_record_path()?;
    write_daemon_endpoint_record_at(&path, endpoint)?;
    Ok(DaemonEndpointRecordGuard::new(path))
}

/// Remove the machine-global endpoint record if it exists.
///
/// # Errors
///
/// Returns [`DaemonEndpointRecordError`] if the record path cannot be resolved
/// or the record cannot be removed.
pub fn remove_daemon_endpoint_record() -> Result<(), DaemonEndpointRecordError> {
    remove_daemon_endpoint_record_at(&daemon_endpoint_record_path()?)
}

/// Read and decode an endpoint record from an explicit path.
///
/// # Errors
///
/// Returns [`DaemonEndpointRecordError`] if the record cannot be read, parsed,
/// or carries an unsupported endpoint kind.
pub fn read_daemon_endpoint_record_at(
    path: &Path,
) -> Result<Option<DaemonEndpointRecord>, DaemonEndpointRecordError> {
    recover_endpoint_record_transactions(path)?;
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(DaemonEndpointRecordError::EndpointRecordRead {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let record: DaemonEndpointRecordFile = toml::from_str(&contents).map_err(|source| {
        DaemonEndpointRecordError::EndpointRecordParse {
            path: path.to_path_buf(),
            source,
        }
    })?;
    let kind = parse_endpoint_kind(&record.kind).ok_or_else(|| {
        DaemonEndpointRecordError::EndpointRecordKind {
            path: path.to_path_buf(),
            kind: record.kind.clone(),
        }
    })?;
    Ok(Some(DaemonEndpointRecord {
        endpoint: core::Endpoint::new(kind, record.address),
        process_id: record.process_id,
        protocol_version: core::ProtocolVersion {
            major: record.protocol_major,
            minor: record.protocol_minor,
            patch: record.protocol_patch,
        },
    }))
}

/// Remove an endpoint record at an explicit path, ignoring a missing file.
///
/// # Errors
///
/// Returns [`DaemonEndpointRecordError`] if the file exists but cannot be
/// removed.
pub fn remove_daemon_endpoint_record_at(path: &Path) -> Result<(), DaemonEndpointRecordError> {
    recover_endpoint_record_transactions(path)?;
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(DaemonEndpointRecordError::EndpointRecordWrite {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Atomically write an endpoint record to an explicit path.
///
/// # Errors
///
/// Returns [`DaemonEndpointRecordError`] if `endpoint` uses a non-public kind or
/// the record cannot be encoded or written.
pub fn write_daemon_endpoint_record_at(
    path: &Path,
    endpoint: &core::Endpoint,
) -> Result<(), DaemonEndpointRecordError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| {
            DaemonEndpointRecordError::EndpointRecordWrite {
                path: parent.to_path_buf(),
                source,
            }
        })?;
    }

    let record = DaemonEndpointRecordFile {
        kind: endpoint_kind_arg(endpoint.kind)?.to_string(),
        address: endpoint.address.clone(),
        process_id: std::process::id(),
        protocol_major: core::PROTOCOL_MAJOR,
        protocol_minor: core::PROTOCOL_MINOR,
        protocol_patch: core::PROTOCOL_PATCH,
    };
    let contents = toml::to_string_pretty(&record)?;
    recover_endpoint_record_transactions(path)?;
    FileTransaction::new(endpoint_record_transaction_root(path))
        .commit([FileWrite::new(path, contents.into_bytes())])
        .map(|_| ())
        .map_err(
            |source| DaemonEndpointRecordError::EndpointRecordTransaction {
                path: path.to_path_buf(),
                source,
            },
        )
}

fn endpoint_record_transaction_root(path: &Path) -> PathBuf {
    // The machine-global endpoint and project registry records both sit
    // directly under the data-home runtime directory, so they intentionally
    // share its `control-record-transactions` root. Per-project endpoint
    // records derive a sibling root beneath their own endpoint directory.
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(
            || PathBuf::from("control-record-transactions"),
            |parent| parent.join("control-record-transactions"),
        )
}

fn recover_endpoint_record_transactions(path: &Path) -> Result<(), DaemonEndpointRecordError> {
    let transaction_root = endpoint_record_transaction_root(path);
    FileTransaction::new(&transaction_root)
        .recover_pending()
        .map(|_| ())
        .map_err(
            |source| DaemonEndpointRecordError::EndpointRecordTransactionRecovery {
                path: transaction_root,
                source,
            },
        )
}

#[must_use]
pub fn default_project_registry_path() -> PathBuf {
    AzothDataHome::resolve().daemon_project_registry_path()
}

fn endpoint_kind_arg(kind: core::EndpointKind) -> Result<&'static str, DaemonEndpointRecordError> {
    validate_public_endpoint_kind(kind, "azd endpoint record")?;
    Ok(match kind {
        core::EndpointKind::WindowsNamedPipe => "windows-named-pipe",
        core::EndpointKind::UnixDomainSocket => "unix-domain-socket",
        core::EndpointKind::Tcp => "tcp",
        core::EndpointKind::InProcess => unreachable!("validated above"),
    })
}

fn parse_endpoint_kind(kind: &str) -> Option<core::EndpointKind> {
    match kind {
        "windows-named-pipe" => Some(core::EndpointKind::WindowsNamedPipe),
        "unix-domain-socket" => Some(core::EndpointKind::UnixDomainSocket),
        "tcp" => Some(core::EndpointKind::Tcp),
        _ => None,
    }
}

const fn validate_public_endpoint_kind(
    kind: core::EndpointKind,
    operation: &'static str,
) -> Result<(), DaemonEndpointRecordError> {
    if matches!(kind, core::EndpointKind::InProcess) {
        return Err(DaemonEndpointRecordError::UnsupportedEndpointKind { operation, kind });
    }

    Ok(())
}

fn daemon_runtime_dir() -> Result<PathBuf, DaemonEndpointRecordError> {
    let data_home = AzothDataHome::resolve();
    data_home.prepare()?;
    Ok(data_home.runtime_dir())
}

fn project_id_for_endpoint(project_root: &Path) -> Result<String, DaemonEndpointRecordError> {
    let path = project_root.join("azoth.toml");
    let text = std::fs::read_to_string(&path).map_err(|source| {
        DaemonEndpointRecordError::ProjectManifestRead {
            path: path.clone(),
            source,
        }
    })?;
    let manifest: ProjectIdentityManifest = toml::from_str(&text).map_err(|source| {
        DaemonEndpointRecordError::ProjectManifestParse {
            path: path.clone(),
            source,
        }
    })?;
    let project_id = manifest.project.id.trim();
    if project_id.is_empty() {
        return Err(DaemonEndpointRecordError::ProjectManifestId { path });
    }
    Ok(project_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for the `remove_file`-then-`rename` window described
    /// in ADR 0031 Correction 5: a writer replacing the record in a loop must
    /// never leave a reader observing `NotFound`. Bounded to a fixed writer
    /// iteration count so the test terminates deterministically.
    #[test]
    fn concurrent_replace_never_exposes_a_missing_endpoint_record() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        const WRITER_ITERATIONS: usize = 500;

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("azd.endpoint.toml");
        let endpoint = core::Endpoint::new(core::EndpointKind::Tcp, "127.0.0.1:37612");

        // Seed the file so the reader always has something to observe from
        // the start of the race, isolating the assertion to the replace path.
        write_daemon_endpoint_record_at(&path, &endpoint).unwrap();

        let done = Arc::new(AtomicBool::new(false));

        let writer = {
            let path = path.clone();
            let done = Arc::clone(&done);
            std::thread::spawn(move || {
                for _ in 0..WRITER_ITERATIONS {
                    write_daemon_endpoint_record_at(&path, &endpoint).unwrap();
                }
                done.store(true, Ordering::SeqCst);
            })
        };

        let mut read_iterations = 0usize;
        loop {
            let finished = done.load(Ordering::SeqCst);
            match read_daemon_endpoint_record_at(&path) {
                Ok(Some(_)) => {}
                Ok(None) => panic!(
                    "reader observed a missing endpoint record during atomic replace \
                     (remove_file-then-rename regression)"
                ),
                Err(error) => {
                    panic!("reader hit an unexpected error during atomic replace: {error}")
                }
            }
            read_iterations += 1;
            if finished {
                break;
            }
        }

        writer.join().unwrap();
        assert!(
            read_iterations > 0,
            "reader loop should have observed the record at least once"
        );
    }

    #[test]
    fn daemon_endpoint_record_round_trips() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("azd.endpoint.toml");
        let endpoint = core::Endpoint::new(core::EndpointKind::Tcp, "127.0.0.1:37612");

        write_daemon_endpoint_record_at(&path, &endpoint).unwrap();

        let record = read_daemon_endpoint_record_at(&path).unwrap().unwrap();
        assert_eq!(record.endpoint, endpoint);
        assert_eq!(record.process_id, std::process::id());
        assert_eq!(record.protocol_version, core::ProtocolVersion::CURRENT);
    }

    fn stage_pending_endpoint_record_write(
        path: &Path,
        previous: &core::Endpoint,
        pending: &core::Endpoint,
    ) -> PathBuf {
        write_daemon_endpoint_record_at(path, previous).unwrap();
        let previous_contents = std::fs::read(path).unwrap();

        std::fs::remove_file(path).unwrap();
        std::fs::create_dir(path).unwrap();
        let error = write_daemon_endpoint_record_at(path, pending).unwrap_err();
        assert!(matches!(
            error,
            DaemonEndpointRecordError::EndpointRecordTransaction { .. }
        ));

        std::fs::remove_dir(path).unwrap();
        std::fs::write(path, previous_contents).unwrap();
        endpoint_record_transaction_root(path)
    }

    fn endpoint_transaction_entry_count(root: &Path) -> usize {
        std::fs::read_dir(root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
            .count()
    }

    #[test]
    fn daemon_endpoint_record_read_recovers_pending_file_transaction() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("azd.endpoint.toml");
        let previous = core::Endpoint::new(core::EndpointKind::Tcp, "127.0.0.1:37612");
        let pending = core::Endpoint::new(core::EndpointKind::Tcp, "127.0.0.1:37613");
        let transaction_root = stage_pending_endpoint_record_write(&path, &previous, &pending);

        let record = read_daemon_endpoint_record_at(&path).unwrap().unwrap();

        assert_eq!(record.endpoint, pending);
        assert_eq!(endpoint_transaction_entry_count(&transaction_root), 0);
    }

    #[test]
    fn daemon_endpoint_record_write_recovers_before_commit() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("azd.endpoint.toml");
        let previous = core::Endpoint::new(core::EndpointKind::Tcp, "127.0.0.1:37612");
        let pending = core::Endpoint::new(core::EndpointKind::Tcp, "127.0.0.1:37613");
        let final_endpoint = core::Endpoint::new(core::EndpointKind::Tcp, "127.0.0.1:37614");
        let transaction_root = stage_pending_endpoint_record_write(&path, &previous, &pending);

        write_daemon_endpoint_record_at(&path, &final_endpoint).unwrap();
        let record = read_daemon_endpoint_record_at(&path).unwrap().unwrap();

        assert_eq!(record.endpoint, final_endpoint);
        assert_eq!(endpoint_transaction_entry_count(&transaction_root), 0);
    }

    #[test]
    fn daemon_endpoint_record_remove_recovers_before_remove() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("azd.endpoint.toml");
        let previous = core::Endpoint::new(core::EndpointKind::Tcp, "127.0.0.1:37612");
        let pending = core::Endpoint::new(core::EndpointKind::Tcp, "127.0.0.1:37613");
        let transaction_root = stage_pending_endpoint_record_write(&path, &previous, &pending);

        remove_daemon_endpoint_record_at(&path).unwrap();

        assert!(!path.exists());
        assert_eq!(endpoint_transaction_entry_count(&transaction_root), 0);
    }

    #[test]
    fn legacy_daemon_endpoint_record_is_outdated_before_transport_connect() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("azd.endpoint.toml");
        std::fs::write(
            &path,
            r#"kind = "tcp"
address = "127.0.0.1:37612"
process_id = 42
"#,
        )
        .unwrap();

        let record = read_daemon_endpoint_record_at(&path).unwrap().unwrap();

        assert_eq!(
            record.protocol_version,
            core::ProtocolVersion {
                major: 0,
                minor: 0,
                patch: 0,
            }
        );
        assert!(
            record
                .require_protocol_version(core::ProtocolVersion::CURRENT)
                .is_err()
        );
    }

    #[test]
    fn daemon_endpoint_record_rejects_in_process_endpoint() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("azd.endpoint.toml");
        let endpoint = core::Endpoint::in_process("azd:test");

        let error = write_daemon_endpoint_record_at(&path, &endpoint).unwrap_err();

        assert!(matches!(
            error,
            DaemonEndpointRecordError::UnsupportedEndpointKind {
                operation: "azd endpoint record",
                kind: core::EndpointKind::InProcess
            }
        ));
        assert!(!path.exists());
    }

    #[test]
    fn default_daemon_endpoint_rejects_in_process_kind() {
        let error = default_daemon_endpoint(core::EndpointKind::InProcess).unwrap_err();

        assert!(matches!(
            error,
            DaemonEndpointRecordError::UnsupportedEndpointKind {
                operation: "azd default endpoint",
                kind: core::EndpointKind::InProcess
            }
        ));
    }

    #[test]
    fn project_endpoint_key_uses_shared_project_machine_key_suffix() {
        let temp = tempfile::tempdir().unwrap();
        let expected = ProjectMachineKey::derive("project", temp.path());

        assert_eq!(project_endpoint_key(temp.path()), expected.hash_suffix());
    }

    #[test]
    fn endpoint_tokens_are_shared_stable_ipc_names() {
        assert_eq!(endpoint_token(" Session Supervisor "), "session-supervisor");
        assert_eq!(endpoint_token("asset__worker"), "asset-worker");
        assert_eq!(endpoint_token(""), "service");
    }

    #[test]
    fn project_service_unix_endpoint_uses_short_machine_runtime_storage() {
        let temp = tempfile::tempdir().unwrap();
        let data_home = AzothDataHome::new(temp.path().join("azoth-home"));
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();

        let endpoint = project_service_endpoint_in(
            &data_home,
            core::EndpointKind::UnixDomainSocket,
            &project_root,
            "asset-processor",
        )
        .unwrap();
        let expected = data_home.runtime_dir().join(format!(
            "svc-{}-asset-processor.sock",
            project_endpoint_key(&project_root)
        ));

        assert_eq!(endpoint.kind, core::EndpointKind::UnixDomainSocket);
        assert_eq!(Path::new(&endpoint.address), expected);
        assert!(!Path::new(&endpoint.address).starts_with(data_home.projects_dir()));
    }

    #[test]
    fn project_service_pipe_is_project_instance_scoped() {
        let temp = tempfile::tempdir().unwrap();
        let data_home = AzothDataHome::new(temp.path().join("azoth-home"));
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();

        let endpoint = project_service_endpoint_in(
            &data_home,
            core::EndpointKind::WindowsNamedPipe,
            &project_root,
            "asset-worker",
        )
        .unwrap();

        assert_eq!(endpoint.kind, core::EndpointKind::WindowsNamedPipe);
        assert_eq!(
            endpoint.address,
            format!(
                r"\\.\pipe\azoth-{}-asset-worker",
                project_endpoint_key(&project_root)
            )
        );
    }

    #[test]
    fn daemon_endpoint_record_read_rejects_in_process_kind() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("azd.endpoint.toml");
        std::fs::write(
            &path,
            r#"kind = "in-process"
address = "azd:test"
process_id = 42
"#,
        )
        .unwrap();

        let error = read_daemon_endpoint_record_at(&path).unwrap_err();

        assert!(matches!(
            error,
            DaemonEndpointRecordError::EndpointRecordKind { kind, .. } if kind == "in-process"
        ));
    }

    #[test]
    fn missing_daemon_endpoint_record_is_none() {
        let temp = tempfile::tempdir().unwrap();

        assert_eq!(
            read_daemon_endpoint_record_at(&temp.path().join("missing.toml")).unwrap(),
            None
        );
    }

    #[test]
    fn daemon_endpoint_record_guard_removes_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("azd.endpoint.toml");
        let endpoint = core::Endpoint::new(core::EndpointKind::Tcp, "127.0.0.1:37612");
        write_daemon_endpoint_record_at(&path, &endpoint).unwrap();

        let guard = DaemonEndpointRecordGuard::new(path.clone());
        drop(guard);

        assert!(!path.exists());
    }
}
