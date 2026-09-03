//! Lock-generated native role-delivery preparation.
//!
//! This module verifies generated delivery data and artifact bytes. It never
//! opens a library or invokes contribution code; that belongs to
//! `az-gem-loader` after preflight succeeds.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use az_gem_contract::{
    GemTargetRole,
    native::{NativeBuildIdentity, NativeContributionExpectation},
};
use serde::Deserialize;
use thiserror::Error;

use crate::GENERATED_TARGETS_RELATIVE_ROOT;

/// Exact engine image identities a native role delivery must match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineBundleIdentity {
    pub engine: NativeBuildIdentity,
    pub rustc: NativeBuildIdentity,
}

/// Build authority for one lock-generated role delivery.
pub struct ProjectRoleDelivery;

impl ProjectRoleDelivery {
    /// Verify and stage one role's generated native delivery without loading
    /// project code.
    ///
    /// `prepared_root` is a caller-owned per-run directory. Each admitted
    /// image is copied there before its path becomes visible to the native
    /// loader, closing the verification-to-load race against project outputs.
    ///
    /// # Errors
    ///
    /// Refuses with [`RolePreflightRefusal::CanonicalizeProjectRoot`],
    /// [`RolePreflightRefusal::ReadManifest`], or
    /// [`RolePreflightRefusal::ParseManifest`] if `project_root` or the role's
    /// generated manifest cannot be resolved, read, or decoded;
    /// [`RolePreflightRefusal::RoleMismatch`] if the manifest names another
    /// role; [`RolePreflightRefusal::MissingNativeArtifacts`] if it carries no
    /// `native` section; [`RolePreflightRefusal::IncompleteNativeClosure`] if
    /// the native entries do not match the logical contributions in exact
    /// order; [`RolePreflightRefusal::InvalidIdentity`] if an engine, rustc,
    /// artifact, or descriptor digest is not 32 hex-encoded bytes; and
    /// [`RolePreflightRefusal::EngineMismatch`] if the manifest was built
    /// against an engine or rustc other than `engine`.
    ///
    /// Per artifact it further refuses with
    /// [`RolePreflightRefusal::InvalidArtifactPath`] or
    /// [`RolePreflightRefusal::ArtifactOutsideProjectRoot`] if the named path is
    /// not a normal relative path inside `project_root`,
    /// [`RolePreflightRefusal::ReadArtifact`] if the image cannot be read,
    /// [`RolePreflightRefusal::ArtifactDigestMismatch`] if its bytes hash to
    /// something other than the recorded digest, and
    /// [`RolePreflightRefusal::CreatePreparedDirectory`] or
    /// [`RolePreflightRefusal::StageArtifact`] if the verified bytes cannot be
    /// staged under `prepared_root`.
    pub fn prepare(
        project_root: impl AsRef<Path>,
        prepared_root: impl AsRef<Path>,
        role: GemTargetRole,
        engine: EngineBundleIdentity,
    ) -> Result<PreparedRoleDelivery, RolePreflightRefusal> {
        let project_root = std::fs::canonicalize(project_root.as_ref()).map_err(|source| {
            RolePreflightRefusal::CanonicalizeProjectRoot {
                path: project_root.as_ref().to_path_buf(),
                source,
            }
        })?;
        let prepared_root = prepared_root.as_ref();
        let manifest_path = project_root
            .join(GENERATED_TARGETS_RELATIVE_ROOT)
            .join(format!("{role}.manifest.json"));
        let bytes =
            std::fs::read(&manifest_path).map_err(|source| RolePreflightRefusal::ReadManifest {
                path: manifest_path.clone(),
                source,
            })?;
        let manifest = serde_json::from_slice::<NativeRoleManifest>(&bytes).map_err(|source| {
            RolePreflightRefusal::ParseManifest {
                path: manifest_path.clone(),
                source,
            }
        })?;
        if manifest.role != role {
            return Err(RolePreflightRefusal::RoleMismatch {
                path: manifest_path,
                expected: role,
                actual: manifest.role,
            });
        }
        let native =
            manifest
                .native
                .ok_or_else(|| RolePreflightRefusal::MissingNativeArtifacts {
                    path: manifest_path.clone(),
                    role,
                })?;
        let native_keys = native
            .contributions
            .iter()
            .map(|entry| LogicalManifestContribution {
                gem: entry.gem.clone(),
                contribution: entry.contribution.clone(),
                package: entry.package.clone(),
                entry: entry.entry.clone(),
            })
            .collect::<Vec<_>>();
        if native_keys != manifest.contributions {
            return Err(RolePreflightRefusal::IncompleteNativeClosure {
                path: manifest_path,
                logical: manifest.contributions.len(),
                native: native_keys.len(),
            });
        }
        let expected_engine = parse_identity(&manifest_path, "native.engine", &native.engine)?;
        let expected_rustc = parse_identity(&manifest_path, "native.rustc", &native.rustc)?;
        if expected_engine != engine.engine || expected_rustc != engine.rustc {
            return Err(RolePreflightRefusal::EngineMismatch {
                path: manifest_path,
                expected: Box::new(engine),
                actual: Box::new(EngineBundleIdentity {
                    engine: expected_engine,
                    rustc: expected_rustc,
                }),
            });
        }

        let contributions = native
            .contributions
            .into_iter()
            .enumerate()
            .map(|(index, entry)| {
                prepare_contribution(
                    &project_root,
                    prepared_root,
                    &manifest_path,
                    role,
                    index,
                    entry,
                    engine,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PreparedRoleDelivery {
            role,
            manifest_digest: NativeBuildIdentity(*blake3::hash(&bytes).as_bytes()),
            contributions,
        })
    }
}

/// A delivery verified by project build authority and consumable by a native
/// host bootstrap. Its fields stay private so callers cannot forge a prepared
/// artifact after choosing arbitrary paths.
#[derive(Debug)]
pub struct PreparedRoleDelivery {
    role: GemTargetRole,
    manifest_digest: NativeBuildIdentity,
    contributions: Vec<PreparedContributionArtifact>,
}

impl PreparedRoleDelivery {
    #[must_use]
    pub const fn role(&self) -> GemTargetRole {
        self.role
    }

    #[must_use]
    pub const fn manifest_digest(&self) -> NativeBuildIdentity {
        self.manifest_digest
    }

    /// Transfer the sealed prepared artifacts to the native loader.
    #[must_use]
    pub fn into_contributions(self) -> Vec<PreparedContributionArtifact> {
        self.contributions
    }

    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub const fn for_test(
        role: GemTargetRole,
        contributions: Vec<PreparedContributionArtifact>,
    ) -> Self {
        Self {
            role,
            manifest_digest: NativeBuildIdentity([0; 32]),
            contributions,
        }
    }
}

/// One byte-verified native image and its expected handshake values.
#[derive(Debug)]
pub struct PreparedContributionArtifact {
    path: PathBuf,
    expectation: NativeContributionExpectation,
}

impl PreparedContributionArtifact {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn expectation(&self) -> NativeContributionExpectation {
        self.expectation
    }

    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn for_test(path: impl Into<PathBuf>, expectation: NativeContributionExpectation) -> Self {
        Self {
            path: path.into(),
            expectation,
        }
    }
}

#[derive(Deserialize)]
struct NativeRoleManifest {
    role: GemTargetRole,
    #[serde(default)]
    contributions: Vec<LogicalManifestContribution>,
    native: Option<NativeManifest>,
}

#[derive(Deserialize, PartialEq, Eq)]
struct LogicalManifestContribution {
    gem: String,
    contribution: String,
    package: String,
    entry: String,
}

#[derive(Deserialize)]
struct NativeManifest {
    engine: String,
    rustc: String,
    contributions: Vec<NativeManifestContribution>,
}

#[derive(Deserialize)]
struct NativeManifestContribution {
    gem: String,
    contribution: String,
    package: String,
    entry: String,
    artifact: String,
    artifact_blake3: String,
    descriptor: String,
}

fn prepare_contribution(
    project_root: &Path,
    prepared_root: &Path,
    manifest_path: &Path,
    role: GemTargetRole,
    index: usize,
    entry: NativeManifestContribution,
    engine: EngineBundleIdentity,
) -> Result<PreparedContributionArtifact, RolePreflightRefusal> {
    let artifact_relative = Path::new(&entry.artifact);
    if !is_normal_relative_path(artifact_relative) {
        return Err(RolePreflightRefusal::InvalidArtifactPath {
            path: manifest_path.to_path_buf(),
            artifact: entry.artifact,
        });
    }
    let source_path = project_root.join(artifact_relative);
    let source_path = std::fs::canonicalize(&source_path).map_err(|source| {
        RolePreflightRefusal::ReadArtifact {
            path: source_path.clone(),
            source,
        }
    })?;
    if !source_path.starts_with(project_root) {
        return Err(RolePreflightRefusal::ArtifactOutsideProjectRoot {
            path: source_path,
            project_root: project_root.to_path_buf(),
        });
    }
    let bytes =
        std::fs::read(&source_path).map_err(|source| RolePreflightRefusal::ReadArtifact {
            path: source_path.clone(),
            source,
        })?;
    let expected_bytes = parse_identity(manifest_path, "artifact_blake3", &entry.artifact_blake3)?;
    let actual_bytes = NativeBuildIdentity(*blake3::hash(&bytes).as_bytes());
    if actual_bytes != expected_bytes {
        return Err(RolePreflightRefusal::ArtifactDigestMismatch {
            path: source_path,
            expected: expected_bytes,
            actual: actual_bytes,
        });
    }
    let file_name = artifact_relative
        .file_name()
        .expect("normal non-empty relative artifact path has a file name");
    let mut staged_file_name = OsString::from(index.to_string());
    staged_file_name.push("-");
    staged_file_name.push(file_name);
    let path = prepared_root.join(role.to_string()).join(staged_file_name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| {
            RolePreflightRefusal::CreatePreparedDirectory {
                path: parent.to_path_buf(),
                source,
            }
        })?;
    }
    let mut staged = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|source| RolePreflightRefusal::StageArtifact {
            path: path.clone(),
            source,
        })?;
    staged
        .write_all(&bytes)
        .and_then(|()| staged.sync_all())
        .map_err(|source| RolePreflightRefusal::StageArtifact {
            path: path.clone(),
            source,
        })?;
    Ok(PreparedContributionArtifact {
        path,
        expectation: NativeContributionExpectation {
            engine: engine.engine,
            rustc: engine.rustc,
            descriptor: parse_identity(manifest_path, "descriptor", &entry.descriptor)?,
        },
    })
}

fn is_normal_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn parse_identity(
    path: &Path,
    field: &'static str,
    value: &str,
) -> Result<NativeBuildIdentity, RolePreflightRefusal> {
    let value = value.strip_prefix("blake3:").unwrap_or(value);
    let bytes = hex::decode(value).map_err(|_| RolePreflightRefusal::InvalidIdentity {
        path: path.to_path_buf(),
        field,
        value: value.to_string(),
    })?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| RolePreflightRefusal::InvalidIdentity {
            path: path.to_path_buf(),
            field,
            value: value.to_string(),
        })?;
    Ok(NativeBuildIdentity(bytes))
}

/// Pre-spawn native role-delivery refusal.
#[derive(Debug, Error)]
pub enum RolePreflightRefusal {
    #[error("canonicalize project root `{path}`")]
    CanonicalizeProjectRoot {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("read native role manifest `{path}`")]
    ReadManifest {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parse native role manifest `{path}`")]
    ParseManifest {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("native role manifest `{path}` declares `{actual}`, expected `{expected}`")]
    RoleMismatch {
        path: PathBuf,
        expected: GemTargetRole,
        actual: GemTargetRole,
    },
    #[error("native role manifest `{path}` has no prepared artifacts for `{role}`")]
    MissingNativeArtifacts { path: PathBuf, role: GemTargetRole },
    #[error(
        "native role manifest `{path}` covers {native} of {logical} logical contributions in exact manifest order"
    )]
    IncompleteNativeClosure {
        path: PathBuf,
        logical: usize,
        native: usize,
    },
    #[error("native role manifest `{path}` has invalid {field} identity `{value}`")]
    InvalidIdentity {
        path: PathBuf,
        field: &'static str,
        value: String,
    },
    #[error("native role manifest `{path}` names a non-relative artifact `{artifact}`")]
    InvalidArtifactPath { path: PathBuf, artifact: String },
    #[error("prepared native contribution `{path}` escapes project root `{project_root}`")]
    ArtifactOutsideProjectRoot {
        path: PathBuf,
        project_root: PathBuf,
    },
    #[error("native role manifest `{path}` does not match the selected engine bundle")]
    EngineMismatch {
        path: PathBuf,
        // Each `EngineBundleIdentity` is two 32-byte digests; inline they made
        // this variant 160 bytes and set the size of every
        // `Result<_, RolePreflightRefusal>` in the module
        // (`clippy::result_large_err`).
        expected: Box<EngineBundleIdentity>,
        actual: Box<EngineBundleIdentity>,
    },
    #[error("read prepared native contribution `{path}`")]
    ReadArtifact {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("prepared native contribution `{path}` does not match its manifest digest")]
    ArtifactDigestMismatch {
        path: PathBuf,
        expected: NativeBuildIdentity,
        actual: NativeBuildIdentity,
    },
    #[error("create prepared native delivery directory `{path}`")]
    CreatePreparedDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("stage verified native contribution `{path}`")]
    StageArtifact {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENGINE: NativeBuildIdentity = NativeBuildIdentity([1; 32]);
    const RUSTC: NativeBuildIdentity = NativeBuildIdentity([2; 32]);

    #[test]
    fn delivery_rejects_current_schema_eight_manifest_until_native_artifacts_are_generated() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = temp
            .path()
            .join(".azoth/targets/runtime-host.manifest.json");
        std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        std::fs::write(&manifest, r#"{"role":"runtime-host"}"#).unwrap();

        let error = ProjectRoleDelivery::prepare(
            temp.path(),
            temp.path().join("prepared"),
            GemTargetRole::RuntimeHost,
            EngineBundleIdentity {
                engine: ENGINE,
                rustc: RUSTC,
            },
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RolePreflightRefusal::MissingNativeArtifacts { .. }
        ));
    }

    #[test]
    fn delivery_accepts_a_byte_verified_native_artifact_without_loading_it() {
        let temp = tempfile::tempdir().unwrap();
        let artifact = temp.path().join("dynamic/runtime-host.dll");
        std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        std::fs::write(&artifact, b"native contribution fixture").unwrap();
        let manifest = temp
            .path()
            .join(".azoth/targets/runtime-host.manifest.json");
        std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        let artifact_digest = blake3::hash(b"native contribution fixture").to_hex();
        let engine = hex::encode(ENGINE.0);
        let rustc = hex::encode(RUSTC.0);
        let descriptor = hex::encode([3; 32]);
        std::fs::write(
            &manifest,
            format!(
                r#"{{"role":"runtime-host","contributions":[{{"gem":"acme.runtime","contribution":"runtime","package":"acme-runtime","entry":"runtime_contribution"}}],"native":{{"engine":"{engine}","rustc":"{rustc}","contributions":[{{"gem":"acme.runtime","contribution":"runtime","package":"acme-runtime","entry":"runtime_contribution","artifact":"dynamic/runtime-host.dll","artifact_blake3":"{artifact_digest}","descriptor":"{descriptor}"}}]}}}}"#
            ),
        )
        .unwrap();

        let delivery = ProjectRoleDelivery::prepare(
            temp.path(),
            temp.path().join("prepared"),
            GemTargetRole::RuntimeHost,
            EngineBundleIdentity {
                engine: ENGINE,
                rustc: RUSTC,
            },
        )
        .unwrap();

        assert_eq!(delivery.role(), GemTargetRole::RuntimeHost);
        let contributions = delivery.into_contributions();
        assert_eq!(contributions.len(), 1);
        assert_ne!(contributions[0].path(), artifact);
        assert_eq!(
            std::fs::read(contributions[0].path()).unwrap(),
            b"native contribution fixture"
        );
    }

    #[test]
    fn delivery_refuses_an_artifact_path_that_can_escape_the_project() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = temp
            .path()
            .join(".azoth/targets/runtime-host.manifest.json");
        std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        let engine = hex::encode(ENGINE.0);
        let rustc = hex::encode(RUSTC.0);
        let descriptor = hex::encode([3; 32]);
        std::fs::write(
            &manifest,
            format!(
                r#"{{"role":"runtime-host","contributions":[{{"gem":"acme.runtime","contribution":"runtime","package":"acme-runtime","entry":"runtime_contribution"}}],"native":{{"engine":"{engine}","rustc":"{rustc}","contributions":[{{"gem":"acme.runtime","contribution":"runtime","package":"acme-runtime","entry":"runtime_contribution","artifact":"../outside.dll","artifact_blake3":"{}","descriptor":"{descriptor}"}}]}}}}"#,
                hex::encode([0; 32]),
            ),
        )
        .unwrap();

        let error = ProjectRoleDelivery::prepare(
            temp.path(),
            temp.path().join("prepared"),
            GemTargetRole::RuntimeHost,
            EngineBundleIdentity {
                engine: ENGINE,
                rustc: RUSTC,
            },
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RolePreflightRefusal::InvalidArtifactPath { .. }
        ));
    }

    #[test]
    fn delivery_refuses_a_partial_native_projection_before_reading_artifacts() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = temp
            .path()
            .join(".azoth/targets/runtime-host.manifest.json");
        std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        let engine = hex::encode(ENGINE.0);
        let rustc = hex::encode(RUSTC.0);
        std::fs::write(
            &manifest,
            format!(
                r#"{{"role":"runtime-host","contributions":[{{"gem":"acme.runtime","contribution":"runtime","package":"acme-runtime","entry":"runtime_contribution"}},{{"gem":"acme.assets","contribution":"assets","package":"acme-assets","entry":"assets_contribution"}}],"native":{{"engine":"{engine}","rustc":"{rustc}","contributions":[{{"gem":"acme.runtime","contribution":"runtime","package":"acme-runtime","entry":"runtime_contribution","artifact":"missing.dll","artifact_blake3":"{}","descriptor":"{}"}}]}}}}"#,
                hex::encode([0; 32]),
                hex::encode([0; 32]),
            ),
        )
        .unwrap();

        let error = ProjectRoleDelivery::prepare(
            temp.path(),
            temp.path().join("prepared"),
            GemTargetRole::RuntimeHost,
            EngineBundleIdentity {
                engine: ENGINE,
                rustc: RUSTC,
            },
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RolePreflightRefusal::IncompleteNativeClosure {
                logical: 2,
                native: 1,
                ..
            }
        ));
    }
}
