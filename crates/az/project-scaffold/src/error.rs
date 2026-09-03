use std::path::PathBuf;
use thiserror::Error;

pub type ScaffoldResult<T> = Result<T, ScaffoldError>;

#[derive(Debug, Error)]
pub enum ScaffoldError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    ProjectManifest(#[from] az_project::ProjectManifestError),

    #[error(transparent)]
    SourceControl(#[from] az_source_control::SourceControlError),

    #[error("Project already exists at path: {0}")]
    ProjectAlreadyExists(PathBuf),

    #[error("Invalid project name: {0}")]
    InvalidProjectName(String),

    #[error("Invalid cargo package name: {0}")]
    InvalidPackageName(String),

    #[error("could not read a cargo package name from {path}")]
    MissingCargoPackageName { path: PathBuf },

    #[error(
        "project engine workspace crate is not resolvable from the engine workspace manifest: {0}"
    )]
    UnsupportedEngineWorkspaceCrate(String),

    /// `azoth project init` refuses a pre-ADR-0025 layout instead of migrating
    /// it. The `crates/game` scaffold, its split service packages, and their
    /// generated entrypoints were retired with link-time registration; an
    /// Azoth project is a primary-gem project whose runtime, authoring, and
    /// builder role packages live under `gems/<slug>`. There is no dual path.
    #[error(
        "refusing to initialize `{path}`: {reason}. Azoth projects are primary-gem projects          (`[project].primary_gem` in azoth.toml, role packages under `gems/<slug>`); the          `crates/game` layout was retired with link-time registration. Start a new project          with `azoth project new` and move authored sources into its role packages."
    )]
    LegacyProjectLayout { path: PathBuf, reason: String },

    #[error("failed to parse config manifest {path}: {message}")]
    ConfigParse { path: PathBuf, message: String },

    #[error("command failed in {cwd}: {program} {args:?}; status={status:?}")]
    CommandFailed {
        program: String,
        args: Vec<String>,
        cwd: PathBuf,
        status: Option<i32>,
    },

    #[error("topology prune requires confirmation before removing authored packages: {packages:?}")]
    TopologyPruneConfirmationRequired { packages: Vec<String> },

    #[error("refusing to prune topology package at {path}: {reason}")]
    TopologyPruneUnsafe { path: PathBuf, reason: String },

    #[error("unknown gem capability `{requested}`; `azoth gem new --capability` supports: {valid}")]
    UnknownGemCapability { requested: String, valid: String },
}
