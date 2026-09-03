//! Cap'n Proto protocol types for runtime-host services.

// Machine-generated Cap'n Proto output: written into OUT_DIR by build.rs and
// regenerated on every build, so it is not in git and has no per-site fix. The
// macro completes upstream's own `#![allow(clippy::all)]` for the pedantic and
// nursery groups this workspace denies.
az_proto_core::generated_schema!(pub mod runtime_capnp, "azoth/runtime_capnp.rs");

use std::{collections::BTreeSet, io::Cursor, path::Path};

use az_proto_core::{self as core, Capability, SideChannelHandle, StagingFileSideChannelError};
use capnp::{Error, message, serialize_packed};
use thiserror::Error as ThisError;
use uuid::Uuid;

pub use az_proto_core as core_proto;

pub const RUNTIME_HOST_NAMESPACE: &str = "azoth";
pub const RUNTIME_HOST_SERVICE_NAME: &str = "runtime-host";
pub const RUNTIME_HOST_AUDIENCE: &str = "runtime-host";
pub const RUNTIME_READ_PERMISSION: &str = "runtime.read";
pub const RUNTIME_CONTROL_PERMISSION: &str = "runtime.control";
/// Runtime launch snapshot schema version.
///
/// Pinned to 1 pre-release. ADR 0031 keeps IPC on lockstep, so both sides are
/// built from one tree and only *equality* of this value is load-bearing —
/// its absolute magnitude carries no compatibility information. Readers
/// reject a mismatched snapshot outright with no migration path, so shape
/// changes are answered by relaunching against a current snapshot.
pub const RUNTIME_LAUNCH_SNAPSHOT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RuntimeRole {
    EditorWorld,
    PlayPreview,
    ServerPreview,
    Validation,
    Thumbnail,
    Bake,
}

impl RuntimeRole {
    #[must_use]
    pub const fn to_capnp(self) -> runtime_capnp::RuntimeRole {
        match self {
            Self::EditorWorld => runtime_capnp::RuntimeRole::EditorWorld,
            Self::PlayPreview => runtime_capnp::RuntimeRole::PlayPreview,
            Self::ServerPreview => runtime_capnp::RuntimeRole::ServerPreview,
            Self::Validation => runtime_capnp::RuntimeRole::Validation,
            Self::Thumbnail => runtime_capnp::RuntimeRole::Thumbnail,
            Self::Bake => runtime_capnp::RuntimeRole::Bake,
        }
    }

    #[must_use]
    pub const fn from_capnp(role: runtime_capnp::RuntimeRole) -> Self {
        match role {
            runtime_capnp::RuntimeRole::EditorWorld => Self::EditorWorld,
            runtime_capnp::RuntimeRole::PlayPreview => Self::PlayPreview,
            runtime_capnp::RuntimeRole::ServerPreview => Self::ServerPreview,
            runtime_capnp::RuntimeRole::Validation => Self::Validation,
            runtime_capnp::RuntimeRole::Thumbnail => Self::Thumbnail,
            runtime_capnp::RuntimeRole::Bake => Self::Bake,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeState {
    Stopped,
    Starting,
    Running,
    Failed,
}

impl RuntimeState {
    #[must_use]
    pub const fn to_capnp(self) -> runtime_capnp::RuntimeState {
        match self {
            Self::Stopped => runtime_capnp::RuntimeState::Stopped,
            Self::Starting => runtime_capnp::RuntimeState::Starting,
            Self::Running => runtime_capnp::RuntimeState::Running,
            Self::Failed => runtime_capnp::RuntimeState::Failed,
        }
    }

    #[must_use]
    pub const fn from_capnp(state: runtime_capnp::RuntimeState) -> Self {
        match state {
            runtime_capnp::RuntimeState::Stopped => Self::Stopped,
            runtime_capnp::RuntimeState::Starting => Self::Starting,
            runtime_capnp::RuntimeState::Running => Self::Running,
            runtime_capnp::RuntimeState::Failed => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RuntimeAssetPackageContainer {
    Loose,
    AzPack,
    Pak,
}

impl RuntimeAssetPackageContainer {
    #[must_use]
    pub const fn to_capnp(self) -> runtime_capnp::RuntimeAssetPackageContainer {
        match self {
            Self::Loose => runtime_capnp::RuntimeAssetPackageContainer::Loose,
            Self::AzPack => runtime_capnp::RuntimeAssetPackageContainer::Azpack,
            Self::Pak => runtime_capnp::RuntimeAssetPackageContainer::Pak,
        }
    }

    #[must_use]
    pub const fn from_capnp(container: runtime_capnp::RuntimeAssetPackageContainer) -> Self {
        match container {
            runtime_capnp::RuntimeAssetPackageContainer::Loose => Self::Loose,
            runtime_capnp::RuntimeAssetPackageContainer::Azpack => Self::AzPack,
            runtime_capnp::RuntimeAssetPackageContainer::Pak => Self::Pak,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Loose => "loose",
            Self::AzPack => "azpack",
            Self::Pak => "pak",
        }
    }

    #[must_use]
    pub const fn from_name(value: &str) -> Option<Self> {
        if value.eq_ignore_ascii_case(Self::Loose.as_str()) {
            Some(Self::Loose)
        } else if value.eq_ignore_ascii_case(Self::AzPack.as_str()) {
            Some(Self::AzPack)
        } else if value.eq_ignore_ascii_case(Self::Pak.as_str()) {
            Some(Self::Pak)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportPixelFormat {
    Unknown,
    Bgra8Unorm,
    Rgba8Unorm,
    Rgba16Float,
    Depth32Float,
}

impl ViewportPixelFormat {
    #[must_use]
    pub const fn to_capnp(self) -> runtime_capnp::ViewportPixelFormat {
        match self {
            Self::Unknown => runtime_capnp::ViewportPixelFormat::Unknown,
            Self::Bgra8Unorm => runtime_capnp::ViewportPixelFormat::Bgra8Unorm,
            Self::Rgba8Unorm => runtime_capnp::ViewportPixelFormat::Rgba8Unorm,
            Self::Rgba16Float => runtime_capnp::ViewportPixelFormat::Rgba16Float,
            Self::Depth32Float => runtime_capnp::ViewportPixelFormat::Depth32Float,
        }
    }

    #[must_use]
    pub const fn from_capnp(format: runtime_capnp::ViewportPixelFormat) -> Self {
        match format {
            runtime_capnp::ViewportPixelFormat::Unknown => Self::Unknown,
            runtime_capnp::ViewportPixelFormat::Bgra8Unorm => Self::Bgra8Unorm,
            runtime_capnp::ViewportPixelFormat::Rgba8Unorm => Self::Rgba8Unorm,
            runtime_capnp::ViewportPixelFormat::Rgba16Float => Self::Rgba16Float,
            runtime_capnp::ViewportPixelFormat::Depth32Float => Self::Depth32Float,
        }
    }

    #[must_use]
    pub const fn bytes_per_pixel(self) -> Option<u32> {
        match self {
            Self::Unknown => None,
            Self::Bgra8Unorm | Self::Rgba8Unorm | Self::Depth32Float => Some(4),
            Self::Rgba16Float => Some(8),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeLaunchSnapshot {
    pub schema_version: u32,
    pub project_id: String,
    pub session_id: Uuid,
    pub session_slug: String,
    pub role: RuntimeRole,
    pub project_root: String,
    pub workspace_path: String,
    pub authored_revision: u64,
    pub schema_catalog_hash: Vec<u8>,
    pub workspace_id: i64,
    pub include_unsaved_journal: bool,
    pub launch_profile: String,
    pub resolved_gems: Vec<RuntimeResolvedGem>,
    pub asset_source_roots: Vec<RuntimeAssetSourceRoot>,
    pub asset_package_roots: Vec<RuntimeAssetPackageRoot>,
}

impl RuntimeLaunchSnapshot {
    #[must_use]
    pub fn new(
        project_id: impl Into<String>,
        session_id: Uuid,
        session_slug: impl Into<String>,
        role: RuntimeRole,
        project_root: impl Into<String>,
        workspace_path: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: RUNTIME_LAUNCH_SNAPSHOT_SCHEMA_VERSION,
            project_id: project_id.into(),
            session_id,
            session_slug: session_slug.into(),
            role,
            project_root: project_root.into(),
            workspace_path: workspace_path.into(),
            authored_revision: 0,
            schema_catalog_hash: Vec::new(),
            workspace_id: 0,
            include_unsaved_journal: false,
            launch_profile: "editor".to_string(),
            resolved_gems: Vec::new(),
            asset_source_roots: Vec::new(),
            asset_package_roots: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAssetSourceRoot {
    pub workspace_root_id: i64,
    pub workspace_id: i64,
    pub root_id: i64,
    pub owner_id: String,
    pub source_root: String,
    pub display_name: String,
    pub portable_key: String,
    pub output_prefix: String,
    pub is_root: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAssetPackageRoot {
    pub profile: String,
    pub asset_platform: String,
    pub container: RuntimeAssetPackageContainer,
    pub mount_root: String,
    pub payload_path: String,
    pub catalog_path: String,
    pub release_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeResolvedGem {
    pub id: String,
    pub name: String,
    pub version: String,
    pub root: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRuntimeRequest {
    pub capability: Capability,
    pub runtime_id: String,
    pub role: RuntimeRole,
    pub snapshot: SideChannelHandle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStatusRequest {
    pub capability: Capability,
    pub runtime_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopRuntimeRequest {
    pub capability: Capability,
    pub runtime_id: String,
    pub preserve: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStatus {
    pub runtime_id: String,
    pub role: RuntimeRole,
    pub state: RuntimeState,
    pub project_id: String,
    pub session_slug: String,
    pub authored_revision: u64,
    pub diagnostic: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStatusResult {
    pub status: Option<RuntimeStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeViewportRequest {
    pub capability: Capability,
    pub runtime_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeViewportFrame {
    pub runtime_id: String,
    pub width: u32,
    pub height: u32,
    pub row_pitch: u32,
    pub format: ViewportPixelFormat,
    pub color: SideChannelHandle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeViewportResult {
    pub frame: Option<RuntimeViewportFrame>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeProjectionDescriptor {
    pub name: String,
    pub priority: i32,
    pub roles: Vec<RuntimeRole>,
    pub launch_profiles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeProjectionCatalogRequest {
    pub capability: Capability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeProjectionCatalogResult {
    pub projections: Vec<RuntimeProjectionDescriptor>,
}

#[derive(Debug, ThisError)]
pub enum RuntimeSnapshotSideChannelError {
    #[error("invalid runtime launch snapshot side-channel handle: {0}")]
    InvalidHandle(#[from] StagingFileSideChannelError),

    #[error("failed to decode runtime launch snapshot side-channel: {0}")]
    Decode(#[from] Error),

    #[error("unsupported runtime launch snapshot schema version {version}")]
    UnsupportedVersion { version: u32 },
}

fn write_runtime_launch_snapshot(
    snapshot: &RuntimeLaunchSnapshot,
    mut builder: runtime_capnp::runtime_launch_snapshot::Builder<'_>,
) -> Result<(), Error> {
    builder.set_schema_version(snapshot.schema_version);
    builder.set_project_id(&snapshot.project_id);
    core::SessionId::from(snapshot.session_id).to_capnp(builder.reborrow().init_session_id());
    builder.set_session_slug(&snapshot.session_slug);
    builder.set_role(snapshot.role.to_capnp());
    builder.set_project_root(&snapshot.project_root);
    builder.set_workspace_path(&snapshot.workspace_path);
    builder.set_authored_revision(snapshot.authored_revision);
    builder.set_schema_catalog_hash(&snapshot.schema_catalog_hash);
    builder.set_workspace_id(snapshot.workspace_id);
    builder.set_include_unsaved_journal(snapshot.include_unsaved_journal);
    builder.set_launch_profile(&snapshot.launch_profile);
    let mut gems = builder
        .reborrow()
        .init_resolved_gems(capnp_list_index(snapshot.resolved_gems.len())?);
    for (index, gem) in snapshot.resolved_gems.iter().enumerate() {
        let mut entry = gems.reborrow().get(capnp_list_index(index)?);
        (gem).to_capnp(entry.reborrow());
    }
    let mut asset_source_roots = builder
        .reborrow()
        .init_asset_source_roots(capnp_list_index(snapshot.asset_source_roots.len())?);
    for (index, source_root) in snapshot.asset_source_roots.iter().enumerate() {
        (source_root).to_capnp(asset_source_roots.reborrow().get(capnp_list_index(index)?));
    }
    let mut asset_package_roots = builder
        .reborrow()
        .init_asset_package_roots(capnp_list_index(snapshot.asset_package_roots.len())?);
    for (index, package_root) in snapshot.asset_package_roots.iter().enumerate() {
        (package_root).to_capnp(asset_package_roots.reborrow().get(capnp_list_index(index)?));
    }
    Ok(())
}

fn read_runtime_launch_snapshot(
    reader: runtime_capnp::runtime_launch_snapshot::Reader<'_>,
) -> Result<RuntimeLaunchSnapshot, Error> {
    let role = reader
        .get_role()
        .map(RuntimeRole::from_capnp)
        .map_err(|error| Error::failed(format!("unknown runtime role: {error:?}")))?;
    Ok(RuntimeLaunchSnapshot {
        schema_version: reader.get_schema_version(),
        project_id: reader.get_project_id()?.to_string()?,
        session_id: core::SessionId::from_capnp(reader.get_session_id()?)
            .map(core::SessionId::into_uuid)?,
        session_slug: reader.get_session_slug()?.to_string()?,
        role,
        project_root: reader.get_project_root()?.to_string()?,
        workspace_path: reader.get_workspace_path()?.to_string()?,
        authored_revision: reader.get_authored_revision(),
        schema_catalog_hash: reader.get_schema_catalog_hash()?.to_vec(),
        workspace_id: reader.get_workspace_id(),
        include_unsaved_journal: reader.get_include_unsaved_journal(),
        launch_profile: reader.get_launch_profile()?.to_string()?,
        resolved_gems: reader
            .get_resolved_gems()?
            .iter()
            .map(RuntimeResolvedGem::from_capnp)
            .collect::<Result<Vec<_>, Error>>()?,
        asset_source_roots: reader
            .get_asset_source_roots()?
            .iter()
            .map(RuntimeAssetSourceRoot::from_capnp)
            .collect::<Result<Vec<_>, Error>>()?,
        asset_package_roots: reader
            .get_asset_package_roots()?
            .iter()
            .map(RuntimeAssetPackageRoot::from_capnp)
            .collect::<Result<Vec<_>, Error>>()?,
    })
}

fn write_runtime_asset_source_root(
    source_root: &RuntimeAssetSourceRoot,
    mut builder: runtime_capnp::runtime_asset_source_root::Builder<'_>,
) {
    builder.set_workspace_root_id(source_root.workspace_root_id);
    builder.set_workspace_id(source_root.workspace_id);
    builder.set_root_id(source_root.root_id);
    builder.set_owner_id(&source_root.owner_id);
    builder.set_source_root(&source_root.source_root);
    builder.set_display_name(&source_root.display_name);
    builder.set_portable_key(&source_root.portable_key);
    builder.set_output_prefix(&source_root.output_prefix);
    builder.set_is_root(source_root.is_root);
}

fn read_runtime_asset_source_root(
    reader: runtime_capnp::runtime_asset_source_root::Reader<'_>,
) -> Result<RuntimeAssetSourceRoot, Error> {
    Ok(RuntimeAssetSourceRoot {
        workspace_root_id: reader.get_workspace_root_id(),
        workspace_id: reader.get_workspace_id(),
        root_id: reader.get_root_id(),
        owner_id: reader.get_owner_id()?.to_string()?,
        source_root: reader.get_source_root()?.to_string()?,
        display_name: reader.get_display_name()?.to_string()?,
        portable_key: reader.get_portable_key()?.to_string()?,
        output_prefix: reader.get_output_prefix()?.to_string()?,
        is_root: reader.get_is_root(),
    })
}

fn write_runtime_asset_package_root(
    package_root: &RuntimeAssetPackageRoot,
    mut builder: runtime_capnp::runtime_asset_package_root::Builder<'_>,
) {
    builder.set_profile(&package_root.profile);
    builder.set_asset_platform(&package_root.asset_platform);
    builder.set_container(package_root.container.to_capnp());
    builder.set_mount_root(&package_root.mount_root);
    builder.set_payload_path(&package_root.payload_path);
    builder.set_catalog_path(&package_root.catalog_path);
    builder.set_release_id(&package_root.release_id);
}

fn read_runtime_asset_package_root(
    reader: runtime_capnp::runtime_asset_package_root::Reader<'_>,
) -> Result<RuntimeAssetPackageRoot, Error> {
    let container = reader
        .get_container()
        .map(RuntimeAssetPackageContainer::from_capnp)
        .map_err(|error| Error::failed(format!("unknown runtime package container: {error:?}")))?;
    Ok(RuntimeAssetPackageRoot {
        profile: reader.get_profile()?.to_string()?,
        asset_platform: reader.get_asset_platform()?.to_string()?,
        container,
        mount_root: reader.get_mount_root()?.to_string()?,
        payload_path: reader.get_payload_path()?.to_string()?,
        catalog_path: reader.get_catalog_path()?.to_string()?,
        release_id: reader.get_release_id()?.to_string()?,
    })
}

pub const RUNTIME_PACKAGE_RELEASE_ID_HEX_LEN: usize = 64;
pub const RUNTIME_ASSET_CATALOG_FILE_NAME: &str = "assetcatalog.bin";

#[must_use]
pub fn is_runtime_package_release_id(value: &str) -> bool {
    value.len() == RUNTIME_PACKAGE_RELEASE_ID_HEX_LEN
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// # Errors
///
/// Returns a description of the first shape rule the root breaks: a catalog
/// path that does not end in the runtime asset catalog file name, a `Loose` or
/// `AzPack` root whose payload path is not its mount root, a `Pak` root whose
/// payload path is its mount root or does not end in `.pak`, or a catalog path
/// equal to the payload path.
pub fn validate_runtime_asset_package_root_shape(
    root: &RuntimeAssetPackageRoot,
) -> Result<(), String> {
    if !path_file_name_eq(&root.catalog_path, RUNTIME_ASSET_CATALOG_FILE_NAME) {
        return Err(format!(
            "catalog path `{}` must end with `{}`",
            root.catalog_path, RUNTIME_ASSET_CATALOG_FILE_NAME
        ));
    }

    match root.container {
        RuntimeAssetPackageContainer::Loose | RuntimeAssetPackageContainer::AzPack => {
            if !transport_paths_equal(&root.payload_path, &root.mount_root) {
                return Err(format!(
                    "container `{}` requires payload path `{}` to equal mount root `{}`",
                    root.container.as_str(),
                    root.payload_path,
                    root.mount_root
                ));
            }
        }
        RuntimeAssetPackageContainer::Pak => {
            if transport_paths_equal(&root.payload_path, &root.mount_root) {
                return Err(format!(
                    "container `pak` requires payload path `{}` to name a pak file, not the mount root",
                    root.payload_path
                ));
            }
            if !path_file_name_has_extension(&root.payload_path, "pak") {
                return Err(format!(
                    "container `pak` requires payload path `{}` to end with `.pak`",
                    root.payload_path
                ));
            }
        }
    }

    if transport_paths_equal(&root.catalog_path, &root.payload_path) {
        return Err(format!(
            "catalog path `{}` must be separate from payload path `{}`",
            root.catalog_path, root.payload_path
        ));
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, ThisError)]
pub enum RuntimeAssetPackageRootsError {
    #[error("asset package root profile cannot be empty")]
    EmptyProfile,
    #[error("asset package root `{profile}` platform cannot be empty")]
    EmptyAssetPlatform { profile: String },
    #[error("asset package root `{profile}` mount root cannot be empty")]
    EmptyMountRoot { profile: String },
    #[error("asset package root `{profile}` payload path cannot be empty")]
    EmptyPayloadPath { profile: String },
    #[error("asset package root `{profile}` catalog path cannot be empty")]
    EmptyCatalogPath { profile: String },
    #[error("asset package root `{profile}` has invalid container shape: {reason}")]
    InvalidContainerShape { profile: String, reason: String },
    #[error("asset package root `{profile}` release id must be 64 lowercase hex characters")]
    InvalidReleaseId { profile: String },
    #[error(
        "duplicate asset package root `{profile}` for platform `{asset_platform}` container `{container}` at `{mount_root}`"
    )]
    DuplicateMount {
        profile: String,
        asset_platform: String,
        container: &'static str,
        mount_root: String,
    },
    #[error("duplicate asset package catalog path `{catalog_path}`")]
    DuplicateCatalog { catalog_path: String },
}

/// # Errors
///
/// Returns [`RuntimeAssetPackageRootsError::EmptyProfile`],
/// [`RuntimeAssetPackageRootsError::EmptyAssetPlatform`],
/// [`RuntimeAssetPackageRootsError::EmptyMountRoot`],
/// [`RuntimeAssetPackageRootsError::EmptyPayloadPath`] or
/// [`RuntimeAssetPackageRootsError::EmptyCatalogPath`] for a root with a blank
/// required field, [`RuntimeAssetPackageRootsError::InvalidReleaseId`] for a
/// release id that is not 64 lowercase hex characters,
/// [`RuntimeAssetPackageRootsError::InvalidContainerShape`] wrapping any
/// [`validate_runtime_asset_package_root_shape`] failure, and
/// [`RuntimeAssetPackageRootsError::DuplicateMount`] or
/// [`RuntimeAssetPackageRootsError::DuplicateCatalog`] if two roots claim the
/// same mount or catalog path.
pub fn validate_runtime_asset_package_roots(
    package_roots: &[RuntimeAssetPackageRoot],
) -> Result<(), RuntimeAssetPackageRootsError> {
    let mut seen_mounts = BTreeSet::new();
    let mut seen_catalogs = BTreeSet::new();

    for root in package_roots {
        if root.profile.trim().is_empty() {
            return Err(RuntimeAssetPackageRootsError::EmptyProfile);
        }
        if root.asset_platform.trim().is_empty() {
            return Err(RuntimeAssetPackageRootsError::EmptyAssetPlatform {
                profile: root.profile.clone(),
            });
        }
        if root.mount_root.trim().is_empty() {
            return Err(RuntimeAssetPackageRootsError::EmptyMountRoot {
                profile: root.profile.clone(),
            });
        }
        if root.payload_path.trim().is_empty() {
            return Err(RuntimeAssetPackageRootsError::EmptyPayloadPath {
                profile: root.profile.clone(),
            });
        }
        if root.catalog_path.trim().is_empty() {
            return Err(RuntimeAssetPackageRootsError::EmptyCatalogPath {
                profile: root.profile.clone(),
            });
        }
        if let Err(reason) = validate_runtime_asset_package_root_shape(root) {
            return Err(RuntimeAssetPackageRootsError::InvalidContainerShape {
                profile: root.profile.clone(),
                reason,
            });
        }
        if !is_runtime_package_release_id(&root.release_id) {
            return Err(RuntimeAssetPackageRootsError::InvalidReleaseId {
                profile: root.profile.clone(),
            });
        }

        let mount_key = (
            root.profile.as_str(),
            root.asset_platform.as_str(),
            root.container,
            root.mount_root.as_str(),
        );
        if !seen_mounts.insert(mount_key) {
            return Err(RuntimeAssetPackageRootsError::DuplicateMount {
                profile: root.profile.clone(),
                asset_platform: root.asset_platform.clone(),
                container: root.container.as_str(),
                mount_root: root.mount_root.clone(),
            });
        }
        if !seen_catalogs.insert(root.catalog_path.as_str()) {
            return Err(RuntimeAssetPackageRootsError::DuplicateCatalog {
                catalog_path: root.catalog_path.clone(),
            });
        }
    }

    Ok(())
}

fn transport_paths_equal(left: &str, right: &str) -> bool {
    normalize_transport_path(left) == normalize_transport_path(right)
}

fn normalize_transport_path(path: &str) -> String {
    path.replace('\\', "/").trim_end_matches('/').to_string()
}

fn path_file_name_eq(path: &str, expected: &str) -> bool {
    path_file_name(path).is_some_and(|name| name.eq_ignore_ascii_case(expected))
}

fn path_file_name_has_extension(path: &str, extension: &str) -> bool {
    path_file_name(path)
        .and_then(|name| name.rsplit_once('.').map(|(_, ext)| ext))
        .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
}

fn path_file_name(path: &str) -> Option<&str> {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return None;
    }
    trimmed.rsplit(['/', '\\']).next()
}

fn write_runtime_resolved_gem(
    gem: &RuntimeResolvedGem,
    mut builder: runtime_capnp::runtime_resolved_gem::Builder<'_>,
) {
    builder.set_id(&gem.id);
    builder.set_name(&gem.name);
    builder.set_version(&gem.version);
    builder.set_root(&gem.root);
}

fn read_runtime_resolved_gem(
    reader: runtime_capnp::runtime_resolved_gem::Reader<'_>,
) -> Result<RuntimeResolvedGem, Error> {
    Ok(RuntimeResolvedGem {
        id: reader.get_id()?.to_string()?,
        name: reader.get_name()?.to_string()?,
        version: reader.get_version()?.to_string()?,
        root: reader.get_root()?.to_string()?,
    })
}

/// Serializes a runtime launch snapshot for a side-channel blob payload.
///
/// # Errors
///
/// Returns an error if the snapshot fails its pre-encode validation (schema
/// version mismatch or a blank required field), or if the message runs out of
/// space while writing the snapshot.
pub fn encode_runtime_launch_snapshot(snapshot: &RuntimeLaunchSnapshot) -> Result<Vec<u8>, Error> {
    validate_runtime_launch_snapshot_for_encode(snapshot)?;

    let mut message = message::Builder::new_default();
    snapshot
        .to_capnp(message.init_root::<runtime_capnp::runtime_launch_snapshot::Builder<'_>>())?;

    let mut bytes = Vec::new();
    serialize_packed::write_message(&mut bytes, &message)?;
    Ok(bytes)
}

fn validate_runtime_launch_snapshot_for_encode(
    snapshot: &RuntimeLaunchSnapshot,
) -> Result<(), Error> {
    if snapshot.schema_version != RUNTIME_LAUNCH_SNAPSHOT_SCHEMA_VERSION {
        return Err(invalid_runtime_launch_snapshot(format!(
            "schema version {} does not match {RUNTIME_LAUNCH_SNAPSHOT_SCHEMA_VERSION}",
            snapshot.schema_version
        )));
    }
    if snapshot.project_id.trim().is_empty() {
        return Err(invalid_runtime_launch_snapshot(
            "project id cannot be empty",
        ));
    }
    if snapshot.session_id == Uuid::nil() {
        return Err(invalid_runtime_launch_snapshot("session id cannot be nil"));
    }
    if snapshot.session_slug.trim().is_empty() {
        return Err(invalid_runtime_launch_snapshot(
            "session slug cannot be empty",
        ));
    }
    if snapshot.project_root.trim().is_empty() {
        return Err(invalid_runtime_launch_snapshot(
            "project root cannot be empty",
        ));
    }
    if snapshot.workspace_path.trim().is_empty() {
        return Err(invalid_runtime_launch_snapshot(
            "workspace path cannot be empty",
        ));
    }
    if snapshot.schema_catalog_hash.len() != 32 {
        return Err(invalid_runtime_launch_snapshot(format!(
            "schema catalog hash must be 32 bytes, got {}",
            snapshot.schema_catalog_hash.len()
        )));
    }
    if snapshot.workspace_id <= 0 {
        return Err(invalid_runtime_launch_snapshot(
            "workspace id must be positive",
        ));
    }
    if snapshot.asset_source_roots.is_empty() {
        return Err(invalid_runtime_launch_snapshot(
            "asset source roots cannot be empty",
        ));
    }

    validate_runtime_resolved_gems(&snapshot.resolved_gems)?;
    validate_runtime_asset_source_roots(
        snapshot.workspace_id,
        &snapshot.project_id,
        &snapshot.workspace_path,
        &snapshot.asset_source_roots,
    )?;
    validate_runtime_launch_snapshot_package_roots(&snapshot.asset_package_roots)
}

fn validate_runtime_asset_source_roots(
    workspace_id: i64,
    project_id: &str,
    workspace_path: &str,
    source_roots: &[RuntimeAssetSourceRoot],
) -> Result<(), Error> {
    let mut seen_source_root_ids = BTreeSet::new();
    let mut seen_portable_keys = BTreeSet::new();
    for root in source_roots {
        if root.workspace_id != workspace_id {
            return Err(invalid_runtime_launch_snapshot(format!(
                "asset source root `{}` belongs to workspace {}, expected {}",
                root.portable_key, root.workspace_id, workspace_id
            )));
        }
        if root.workspace_root_id <= 0 {
            return Err(invalid_runtime_launch_snapshot(format!(
                "asset source root `{}` must carry a positive workspace root id",
                root.portable_key
            )));
        }
        if root.root_id <= 0 {
            return Err(invalid_runtime_launch_snapshot(format!(
                "asset source root `{}` must carry a positive root id",
                root.portable_key
            )));
        }
        if root.owner_id.trim().is_empty() {
            return Err(invalid_runtime_launch_snapshot(
                "asset source root owner id cannot be empty",
            ));
        }
        if root.source_root.trim().is_empty() {
            return Err(invalid_runtime_launch_snapshot(format!(
                "asset source root `{}` physical root cannot be empty",
                root.portable_key
            )));
        }
        if root.portable_key.trim().is_empty() {
            return Err(invalid_runtime_launch_snapshot(
                "asset source root portable key cannot be empty",
            ));
        }
        if !seen_source_root_ids.insert(root.workspace_root_id) {
            return Err(invalid_runtime_launch_snapshot(format!(
                "workspace {workspace_id} contains duplicate source root id {}",
                root.workspace_root_id
            )));
        }
        if !seen_portable_keys.insert(root.portable_key.as_str()) {
            return Err(invalid_runtime_launch_snapshot(format!(
                "workspace {workspace_id} contains duplicate source root key `{}`",
                root.portable_key
            )));
        }
    }

    let project_assets_key = format!("project:{project_id}:assets");
    let Some(project_assets) = source_roots
        .iter()
        .find(|root| root.portable_key == project_assets_key)
    else {
        return Err(invalid_runtime_launch_snapshot(format!(
            "missing DB-owned project assets root `{project_assets_key}`"
        )));
    };
    if project_assets.owner_id != project_id {
        return Err(invalid_runtime_launch_snapshot(format!(
            "project assets root `{project_assets_key}` owner `{}` does not match project `{project_id}`",
            project_assets.owner_id
        )));
    }
    let project_assets_path = Path::new(&project_assets.source_root);
    let workspace_path = Path::new(workspace_path);
    if project_assets_path == workspace_path || !project_assets_path.starts_with(workspace_path) {
        return Err(invalid_runtime_launch_snapshot(format!(
            "project assets root `{project_assets_key}` path `{}` must be inside workspace `{}`",
            project_assets.source_root,
            workspace_path.display()
        )));
    }
    if !project_assets.is_root {
        return Err(invalid_runtime_launch_snapshot(format!(
            "project assets root `{project_assets_key}` must be marked as a root source"
        )));
    }
    if !project_assets.output_prefix.is_empty() {
        return Err(invalid_runtime_launch_snapshot(format!(
            "project assets root `{project_assets_key}` must use an empty output prefix"
        )));
    }

    Ok(())
}

fn validate_runtime_launch_snapshot_package_roots(
    package_roots: &[RuntimeAssetPackageRoot],
) -> Result<(), Error> {
    validate_runtime_asset_package_roots(package_roots)
        .map_err(|error| invalid_runtime_launch_snapshot(error.to_string()))
}

fn validate_runtime_resolved_gems(resolved_gems: &[RuntimeResolvedGem]) -> Result<(), Error> {
    let mut seen_ids = BTreeSet::new();
    let mut seen_roots = BTreeSet::new();

    for gem in resolved_gems {
        if gem.id.trim().is_empty() {
            return Err(invalid_runtime_launch_snapshot(
                "resolved gem id cannot be empty",
            ));
        }
        if gem.name.trim().is_empty() {
            return Err(invalid_runtime_launch_snapshot(format!(
                "resolved gem `{}` name cannot be empty",
                gem.id
            )));
        }
        if gem.version.trim().is_empty() {
            return Err(invalid_runtime_launch_snapshot(format!(
                "resolved gem `{}` version cannot be empty",
                gem.id
            )));
        }
        if gem.root.trim().is_empty() {
            return Err(invalid_runtime_launch_snapshot(format!(
                "resolved gem `{}` root cannot be empty",
                gem.id
            )));
        }
        if !seen_ids.insert(gem.id.as_str()) {
            return Err(invalid_runtime_launch_snapshot(format!(
                "resolved gems contain duplicate id `{}`",
                gem.id
            )));
        }
        if !seen_roots.insert(gem.root.as_str()) {
            return Err(invalid_runtime_launch_snapshot(format!(
                "resolved gems contain duplicate root `{}`",
                gem.root
            )));
        }
    }

    Ok(())
}

/// Narrows a `usize` list length or element index to the `u32` Cap'n Proto
/// uses for both.
///
/// The casts this replaces were lossy on 64-bit targets: an oversized
/// collection would have silently written a wrapped length and produced a
/// message that decodes to the wrong number of elements. Failing the write is
/// the honest outcome, and every caller is already in a `Result`.
///
/// # Errors
///
/// Returns an error if `value` does not fit in a `u32`.
fn capnp_list_index(value: usize) -> Result<u32, Error> {
    u32::try_from(value)
        .map_err(|_| Error::failed("Cap'n Proto list length exceeds u32 range".to_string()))
}

fn invalid_runtime_launch_snapshot(reason: impl Into<String>) -> Error {
    Error::failed(format!(
        "invalid runtime launch snapshot: {}",
        reason.into()
    ))
}

/// # Errors
///
/// Returns an error if the staging file behind `handle` cannot be read or fails
/// its hash verification, if the bytes are not a readable packed Cap'n Proto
/// message, if a field of the snapshot is absent from the message or is not
/// valid UTF-8, or [`RuntimeSnapshotSideChannelError::UnsupportedVersion`] if
/// the decoded snapshot declares a different schema version.
pub fn load_runtime_launch_snapshot_side_channel(
    handle: &SideChannelHandle,
) -> Result<RuntimeLaunchSnapshot, RuntimeSnapshotSideChannelError> {
    let file = core::read_verified_staging_file(handle)?;
    let message = serialize_packed::read_message(
        &mut Cursor::new(file.bytes),
        message::ReaderOptions::new(),
    )?;
    let snapshot = RuntimeLaunchSnapshot::from_capnp(
        message.get_root::<runtime_capnp::runtime_launch_snapshot::Reader<'_>>()?,
    )?;
    if snapshot.schema_version != RUNTIME_LAUNCH_SNAPSHOT_SCHEMA_VERSION {
        return Err(RuntimeSnapshotSideChannelError::UnsupportedVersion {
            version: snapshot.schema_version,
        });
    }
    Ok(snapshot)
}

fn validate_non_empty_text(kind: &str, field: &str, value: &str) -> Result<(), Error> {
    if value.trim().is_empty() {
        return Err(invalid_runtime_protocol_value(
            kind,
            format!("{field} cannot be empty"),
        ));
    }
    Ok(())
}

fn validate_runtime_id_for_transport(kind: &str, runtime_id: &str) -> Result<(), Error> {
    validate_non_empty_text(kind, "runtime id", runtime_id)
}

fn validate_launch_runtime_request_for_transport(
    request: &LaunchRuntimeRequest,
) -> Result<(), Error> {
    validate_runtime_control_capability_request_for_transport(
        "launch runtime request",
        &request.capability,
    )?;
    validate_runtime_id_for_transport("launch runtime request", &request.runtime_id)?;
    validate_bound_staging_handle("launch runtime request snapshot", &request.snapshot)
}

fn validate_runtime_status_request_for_transport(
    request: &RuntimeStatusRequest,
) -> Result<(), Error> {
    validate_runtime_read_capability_request_for_transport(
        "runtime status request",
        &request.capability,
    )?;
    validate_runtime_id_for_transport("runtime status request", &request.runtime_id)
}

fn validate_stop_runtime_request_for_transport(request: &StopRuntimeRequest) -> Result<(), Error> {
    validate_runtime_control_capability_request_for_transport(
        "stop runtime request",
        &request.capability,
    )?;
    validate_runtime_id_for_transport("stop runtime request", &request.runtime_id)
}

fn validate_runtime_viewport_request_for_transport(
    request: &RuntimeViewportRequest,
) -> Result<(), Error> {
    validate_runtime_read_capability_request_for_transport(
        "runtime viewport request",
        &request.capability,
    )?;
    validate_runtime_id_for_transport("runtime viewport request", &request.runtime_id)
}

fn validate_runtime_projection_catalog_request_for_transport(
    request: &RuntimeProjectionCatalogRequest,
) -> Result<(), Error> {
    validate_runtime_read_capability_request_for_transport(
        "runtime projection catalog request",
        &request.capability,
    )
}

fn validate_runtime_read_capability_request_for_transport(
    kind: &str,
    capability: &Capability,
) -> Result<(), Error> {
    validate_runtime_capability_request_for_transport(kind, capability, RUNTIME_READ_PERMISSION)
}

fn validate_runtime_control_capability_request_for_transport(
    kind: &str,
    capability: &Capability,
) -> Result<(), Error> {
    validate_runtime_capability_request_for_transport(kind, capability, RUNTIME_CONTROL_PERMISSION)
}

fn validate_runtime_capability_request_for_transport(
    kind: &str,
    capability: &Capability,
    required_permission: &'static str,
) -> Result<(), Error> {
    if capability.is_empty() {
        return Err(invalid_runtime_protocol_value(
            kind,
            "capability cannot be empty",
        ));
    }
    if capability.audience != RUNTIME_HOST_AUDIENCE {
        return Err(invalid_runtime_protocol_value(
            kind,
            format!(
                "capability audience must be `{RUNTIME_HOST_AUDIENCE}`, got `{}`",
                capability.audience
            ),
        ));
    }
    if !matches!(
        capability.role,
        core::ServiceRole::Editor
            | core::ServiceRole::SessionSupervisor
            | core::ServiceRole::ProjectHost
            | core::ServiceRole::RuntimeHost
    ) {
        return Err(invalid_runtime_protocol_value(
            kind,
            format!("capability role {:?} is not allowed", capability.role),
        ));
    }
    let has_required = capability.has_permissions(&[required_permission]);
    let has_control = capability.has_permissions(&[RUNTIME_CONTROL_PERMISSION]);
    if !(has_required || (required_permission == RUNTIME_READ_PERMISSION && has_control)) {
        return Err(invalid_runtime_protocol_value(
            kind,
            format!("capability missing `{required_permission}`"),
        ));
    }
    if capability.session.is_none() {
        return Err(invalid_runtime_protocol_value(
            kind,
            format!("`{required_permission}` capability must be session-scoped"),
        ));
    }
    capability
        .validate_lifetime()
        .map_err(|error| invalid_runtime_protocol_value(kind, error.to_string()))?;
    Ok(())
}

fn validate_runtime_status_for_transport(status: &RuntimeStatus) -> Result<(), Error> {
    validate_runtime_id_for_transport("runtime status", &status.runtime_id)?;
    validate_non_empty_text("runtime status", "project id", &status.project_id)?;
    validate_non_empty_text("runtime status", "session slug", &status.session_slug)
}

fn validate_runtime_viewport_frame_for_transport(
    frame: &RuntimeViewportFrame,
) -> Result<(), Error> {
    validate_runtime_id_for_transport("runtime viewport frame", &frame.runtime_id)?;
    if frame.width == 0 || frame.height == 0 {
        return Err(invalid_runtime_protocol_value(
            "runtime viewport frame",
            format!(
                "viewport dimensions must be non-zero, got {}x{}",
                frame.width, frame.height
            ),
        ));
    }
    let bytes_per_pixel = frame.format.bytes_per_pixel().ok_or_else(|| {
        invalid_runtime_protocol_value(
            "runtime viewport frame",
            "viewport pixel format cannot be unknown",
        )
    })?;
    let min_row_pitch = frame.width.checked_mul(bytes_per_pixel).ok_or_else(|| {
        invalid_runtime_protocol_value(
            "runtime viewport frame",
            format!(
                "viewport row pitch overflow for {} pixels at {} bytes per pixel",
                frame.width, bytes_per_pixel
            ),
        )
    })?;
    if frame.row_pitch < min_row_pitch {
        return Err(invalid_runtime_protocol_value(
            "runtime viewport frame",
            format!(
                "viewport row pitch {} is smaller than minimum {}",
                frame.row_pitch, min_row_pitch
            ),
        ));
    }
    let min_byte_length = u64::from(frame.row_pitch)
        .checked_mul(u64::from(frame.height))
        .ok_or_else(|| {
            invalid_runtime_protocol_value(
                "runtime viewport frame",
                format!(
                    "viewport byte length overflow for row pitch {} and height {}",
                    frame.row_pitch, frame.height
                ),
            )
        })?;
    if frame.color.byte_length < min_byte_length {
        return Err(invalid_runtime_protocol_value(
            "runtime viewport frame",
            format!(
                "viewport side-channel byte length {} is smaller than minimum {}",
                frame.color.byte_length, min_byte_length
            ),
        ));
    }
    validate_gpu_surface_side_channel_handle("runtime viewport frame color", &frame.color)?;
    if frame.color.capability.is_none() {
        return Err(invalid_runtime_protocol_value(
            "runtime viewport frame",
            "viewport color handle must be bound to the accepted runtime.read capability",
        ));
    }
    Ok(())
}

fn validate_runtime_projection_descriptor_for_transport(
    descriptor: &RuntimeProjectionDescriptor,
) -> Result<(), Error> {
    validate_non_empty_text("runtime projection descriptor", "name", &descriptor.name)?;
    if descriptor.roles.is_empty() {
        return Err(invalid_runtime_protocol_value(
            "runtime projection descriptor",
            format!("projection `{}` roles cannot be empty", descriptor.name),
        ));
    }
    let mut seen_roles = BTreeSet::new();
    for role in &descriptor.roles {
        if !seen_roles.insert(*role) {
            return Err(invalid_runtime_protocol_value(
                "runtime projection descriptor",
                format!(
                    "projection `{}` contains duplicate role {role:?}",
                    descriptor.name
                ),
            ));
        }
    }
    if descriptor.launch_profiles.is_empty() {
        return Err(invalid_runtime_protocol_value(
            "runtime projection descriptor",
            format!(
                "projection `{}` launch profiles cannot be empty",
                descriptor.name
            ),
        ));
    }
    let mut seen_profiles = BTreeSet::new();
    for profile in &descriptor.launch_profiles {
        if profile.trim().is_empty() {
            return Err(invalid_runtime_protocol_value(
                "runtime projection descriptor",
                format!(
                    "projection `{}` launch profile cannot be empty",
                    descriptor.name
                ),
            ));
        }
        if !seen_profiles.insert(profile.as_str()) {
            return Err(invalid_runtime_protocol_value(
                "runtime projection descriptor",
                format!(
                    "projection `{}` contains duplicate launch profile `{profile}`",
                    descriptor.name
                ),
            ));
        }
    }
    Ok(())
}

fn validate_runtime_projection_catalog_result_for_transport(
    result: &RuntimeProjectionCatalogResult,
) -> Result<(), Error> {
    let mut seen_names = BTreeSet::new();
    for projection in &result.projections {
        validate_runtime_projection_descriptor_for_transport(projection)?;
        if !seen_names.insert(projection.name.as_str()) {
            return Err(invalid_runtime_protocol_value(
                "runtime projection catalog result",
                format!("duplicate projection `{}`", projection.name),
            ));
        }
    }
    Ok(())
}

fn validate_bound_staging_handle(kind: &str, handle: &SideChannelHandle) -> Result<(), Error> {
    validate_non_empty_text(kind, "platform", &handle.platform)?;
    if handle.byte_length == 0 {
        return Err(invalid_runtime_protocol_value(
            kind,
            "byte length must be positive",
        ));
    }
    if handle.capability.is_none() {
        return Err(invalid_runtime_protocol_value(
            kind,
            "side-channel handle must be bound to a brokered capability",
        ));
    }
    core::validated_staging_file_path(handle)
        .map_err(|error| invalid_runtime_protocol_value(kind, error.to_string()))?;
    Ok(())
}

fn validate_gpu_surface_side_channel_handle(
    kind: &str,
    handle: &SideChannelHandle,
) -> Result<(), Error> {
    validate_non_empty_text(kind, "locator", &handle.locator)?;
    validate_non_empty_text(kind, "platform", &handle.platform)?;
    match handle.kind {
        core::SideChannelKind::GpuSurface => Ok(()),
        side_channel => Err(invalid_runtime_protocol_value(
            kind,
            format!(
                "runtime viewport frames must use gpuSurface side channels, got {side_channel:?}"
            ),
        )),
    }
}

fn invalid_runtime_protocol_value(kind: &str, reason: impl Into<String>) -> Error {
    Error::failed(format!("invalid {kind}: {}", reason.into()))
}

fn write_launch_runtime_request(
    request: &LaunchRuntimeRequest,
    mut builder: runtime_capnp::launch_runtime_request::Builder<'_>,
) -> Result<(), Error> {
    validate_launch_runtime_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_runtime_id(&request.runtime_id);
    builder.set_role(request.role.to_capnp());
    core::SideChannelHandle::to_capnp(&request.snapshot, builder.init_snapshot())?;
    Ok(())
}

fn read_launch_runtime_request(
    reader: runtime_capnp::launch_runtime_request::Reader<'_>,
) -> Result<LaunchRuntimeRequest, Error> {
    let role = reader
        .get_role()
        .map(RuntimeRole::from_capnp)
        .map_err(|error| Error::failed(format!("unknown runtime role: {error:?}")))?;
    let request = LaunchRuntimeRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        runtime_id: reader.get_runtime_id()?.to_string()?,
        role,
        snapshot: core::SideChannelHandle::from_capnp(reader.get_snapshot()?)?,
    };
    validate_launch_runtime_request_for_transport(&request)?;
    Ok(request)
}

fn write_runtime_status_request(
    request: &RuntimeStatusRequest,
    mut builder: runtime_capnp::runtime_status_request::Builder<'_>,
) -> Result<(), Error> {
    validate_runtime_status_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_runtime_id(&request.runtime_id);
    Ok(())
}

fn read_runtime_status_request(
    reader: runtime_capnp::runtime_status_request::Reader<'_>,
) -> Result<RuntimeStatusRequest, Error> {
    let request = RuntimeStatusRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        runtime_id: reader.get_runtime_id()?.to_string()?,
    };
    validate_runtime_status_request_for_transport(&request)?;
    Ok(request)
}

fn write_stop_runtime_request(
    request: &StopRuntimeRequest,
    mut builder: runtime_capnp::stop_runtime_request::Builder<'_>,
) -> Result<(), Error> {
    validate_stop_runtime_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_runtime_id(&request.runtime_id);
    builder.set_preserve(request.preserve);
    Ok(())
}

fn read_stop_runtime_request(
    reader: runtime_capnp::stop_runtime_request::Reader<'_>,
) -> Result<StopRuntimeRequest, Error> {
    let request = StopRuntimeRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        runtime_id: reader.get_runtime_id()?.to_string()?,
        preserve: reader.get_preserve(),
    };
    validate_stop_runtime_request_for_transport(&request)?;
    Ok(request)
}

fn write_runtime_status(
    status: &RuntimeStatus,
    mut builder: runtime_capnp::runtime_status::Builder<'_>,
) -> Result<(), Error> {
    validate_runtime_status_for_transport(status)?;
    builder.set_runtime_id(&status.runtime_id);
    builder.set_role(status.role.to_capnp());
    builder.set_state(status.state.to_capnp());
    builder.set_project_id(&status.project_id);
    builder.set_session_slug(&status.session_slug);
    builder.set_authored_revision(status.authored_revision);
    builder.set_diagnostic(&status.diagnostic);
    Ok(())
}

fn read_runtime_status(
    reader: runtime_capnp::runtime_status::Reader<'_>,
) -> Result<RuntimeStatus, Error> {
    let role = reader
        .get_role()
        .map(RuntimeRole::from_capnp)
        .map_err(|error| Error::failed(format!("unknown runtime role: {error:?}")))?;
    let state = reader
        .get_state()
        .map(RuntimeState::from_capnp)
        .map_err(|error| Error::failed(format!("unknown runtime state: {error:?}")))?;
    let status = RuntimeStatus {
        runtime_id: reader.get_runtime_id()?.to_string()?,
        role,
        state,
        project_id: reader.get_project_id()?.to_string()?,
        session_slug: reader.get_session_slug()?.to_string()?,
        authored_revision: reader.get_authored_revision(),
        diagnostic: reader.get_diagnostic()?.to_string()?,
    };
    validate_runtime_status_for_transport(&status)?;
    Ok(status)
}

fn write_runtime_status_result(
    result: &RuntimeStatusResult,
    mut builder: runtime_capnp::runtime_status_result::Builder<'_>,
) -> Result<(), Error> {
    match &result.status {
        Some(status) => (status).to_capnp(builder.init_status())?,
        None => builder.set_none(()),
    }
    Ok(())
}

fn read_runtime_status_result(
    reader: runtime_capnp::runtime_status_result::Reader<'_>,
) -> Result<RuntimeStatusResult, Error> {
    let status = match reader.which()? {
        runtime_capnp::runtime_status_result::Which::None(()) => None,
        runtime_capnp::runtime_status_result::Which::Status(status) => {
            Some(RuntimeStatus::from_capnp(status?)?)
        }
    };
    Ok(RuntimeStatusResult { status })
}

fn write_runtime_viewport_request(
    request: &RuntimeViewportRequest,
    mut builder: runtime_capnp::runtime_viewport_request::Builder<'_>,
) -> Result<(), Error> {
    validate_runtime_viewport_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_runtime_id(&request.runtime_id);
    Ok(())
}

fn read_runtime_viewport_request(
    reader: runtime_capnp::runtime_viewport_request::Reader<'_>,
) -> Result<RuntimeViewportRequest, Error> {
    let request = RuntimeViewportRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        runtime_id: reader.get_runtime_id()?.to_string()?,
    };
    validate_runtime_viewport_request_for_transport(&request)?;
    Ok(request)
}

fn write_runtime_viewport_frame(
    frame: &RuntimeViewportFrame,
    mut builder: runtime_capnp::runtime_viewport_frame::Builder<'_>,
) -> Result<(), Error> {
    validate_runtime_viewport_frame_for_transport(frame)?;
    builder.set_runtime_id(&frame.runtime_id);
    builder.set_width(frame.width);
    builder.set_height(frame.height);
    builder.set_row_pitch(frame.row_pitch);
    builder.set_format(frame.format.to_capnp());
    core::SideChannelHandle::to_capnp(&frame.color, builder.init_color())?;
    Ok(())
}

fn read_runtime_viewport_frame(
    reader: runtime_capnp::runtime_viewport_frame::Reader<'_>,
) -> Result<RuntimeViewportFrame, Error> {
    let format = reader
        .get_format()
        .map(ViewportPixelFormat::from_capnp)
        .map_err(|error| Error::failed(format!("unknown viewport pixel format: {error:?}")))?;
    let frame = RuntimeViewportFrame {
        runtime_id: reader.get_runtime_id()?.to_string()?,
        width: reader.get_width(),
        height: reader.get_height(),
        row_pitch: reader.get_row_pitch(),
        format,
        color: core::SideChannelHandle::from_capnp(reader.get_color()?)?,
    };
    validate_runtime_viewport_frame_for_transport(&frame)?;
    Ok(frame)
}

fn write_runtime_viewport_result(
    result: &RuntimeViewportResult,
    mut builder: runtime_capnp::runtime_viewport_result::Builder<'_>,
) -> Result<(), Error> {
    match &result.frame {
        Some(frame) => (frame).to_capnp(builder.init_frame())?,
        None => builder.set_none(()),
    }
    Ok(())
}

fn read_runtime_viewport_result(
    reader: runtime_capnp::runtime_viewport_result::Reader<'_>,
) -> Result<RuntimeViewportResult, Error> {
    let frame = match reader.which()? {
        runtime_capnp::runtime_viewport_result::Which::None(()) => None,
        runtime_capnp::runtime_viewport_result::Which::Frame(frame) => {
            Some(RuntimeViewportFrame::from_capnp(frame?)?)
        }
    };
    Ok(RuntimeViewportResult { frame })
}

fn write_runtime_projection_descriptor(
    descriptor: &RuntimeProjectionDescriptor,
    mut builder: runtime_capnp::runtime_projection_descriptor::Builder<'_>,
) -> Result<(), Error> {
    validate_runtime_projection_descriptor_for_transport(descriptor)?;
    builder.set_name(&descriptor.name);
    builder.set_priority(descriptor.priority);

    let mut roles = builder
        .reborrow()
        .init_roles(capnp_list_index(descriptor.roles.len())?);
    for (index, role) in descriptor.roles.iter().enumerate() {
        roles.set(capnp_list_index(index)?, role.to_capnp());
    }

    let mut profiles = builder
        .reborrow()
        .init_launch_profiles(capnp_list_index(descriptor.launch_profiles.len())?);
    for (index, profile) in descriptor.launch_profiles.iter().enumerate() {
        profiles.set(capnp_list_index(index)?, profile);
    }
    Ok(())
}

fn read_runtime_projection_descriptor(
    reader: runtime_capnp::runtime_projection_descriptor::Reader<'_>,
) -> Result<RuntimeProjectionDescriptor, Error> {
    let roles = reader
        .get_roles()?
        .iter()
        .map(|role| {
            role.map(RuntimeRole::from_capnp)
                .map_err(|error| Error::failed(format!("unknown runtime role: {error:?}")))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let launch_profiles = reader
        .get_launch_profiles()?
        .iter()
        .map(|profile| profile.and_then(|profile| Ok(profile.to_string()?)))
        .collect::<Result<Vec<_>, Error>>()?;
    let descriptor = RuntimeProjectionDescriptor {
        name: reader.get_name()?.to_string()?,
        priority: reader.get_priority(),
        roles,
        launch_profiles,
    };
    validate_runtime_projection_descriptor_for_transport(&descriptor)?;
    Ok(descriptor)
}

fn write_runtime_projection_catalog_request(
    request: &RuntimeProjectionCatalogRequest,
    mut builder: runtime_capnp::runtime_projection_catalog_request::Builder<'_>,
) -> Result<(), Error> {
    validate_runtime_projection_catalog_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    Ok(())
}

fn read_runtime_projection_catalog_request(
    reader: runtime_capnp::runtime_projection_catalog_request::Reader<'_>,
) -> Result<RuntimeProjectionCatalogRequest, Error> {
    let request = RuntimeProjectionCatalogRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
    };
    validate_runtime_projection_catalog_request_for_transport(&request)?;
    Ok(request)
}

fn write_runtime_projection_catalog_result(
    result: &RuntimeProjectionCatalogResult,
    mut builder: runtime_capnp::runtime_projection_catalog_result::Builder<'_>,
) -> Result<(), Error> {
    validate_runtime_projection_catalog_result_for_transport(result)?;
    let mut projections = builder
        .reborrow()
        .init_projections(capnp_list_index(result.projections.len())?);
    for (index, projection) in result.projections.iter().enumerate() {
        (projection).to_capnp(projections.reborrow().get(capnp_list_index(index)?))?;
    }
    Ok(())
}

fn read_runtime_projection_catalog_result(
    reader: runtime_capnp::runtime_projection_catalog_result::Reader<'_>,
) -> Result<RuntimeProjectionCatalogResult, Error> {
    let projections = reader
        .get_projections()?
        .iter()
        .map(read_runtime_projection_descriptor)
        .collect::<Result<Vec<_>, Error>>()?;
    let result = RuntimeProjectionCatalogResult { projections };
    validate_runtime_projection_catalog_result_for_transport(&result)?;
    Ok(result)
}

impl RuntimeLaunchSnapshot {
    /// Writes this snapshot into its generated Cap'n Proto builder.
    ///
    /// # Errors
    ///
    /// Returns a protocol error when a nested side-channel handle is invalid.
    pub fn to_capnp(
        &self,
        builder: runtime_capnp::runtime_launch_snapshot::Builder<'_>,
    ) -> Result<(), Error> {
        write_runtime_launch_snapshot(self, builder)
    }

    /// Decodes a runtime launch snapshot from Cap'n Proto.
    ///
    /// # Errors
    ///
    /// Returns a protocol error when the payload is malformed.
    pub fn from_capnp(
        reader: runtime_capnp::runtime_launch_snapshot::Reader<'_>,
    ) -> Result<Self, Error> {
        read_runtime_launch_snapshot(reader)
    }
}

impl RuntimeAssetSourceRoot {
    pub fn to_capnp(&self, builder: runtime_capnp::runtime_asset_source_root::Builder<'_>) {
        write_runtime_asset_source_root(self, builder);
    }

    /// # Errors
    ///
    /// Returns an error if a field of the runtime asset source root is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: runtime_capnp::runtime_asset_source_root::Reader<'_>,
    ) -> Result<Self, Error> {
        read_runtime_asset_source_root(reader)
    }
}

impl RuntimeAssetPackageRoot {
    pub fn to_capnp(&self, builder: runtime_capnp::runtime_asset_package_root::Builder<'_>) {
        write_runtime_asset_package_root(self, builder);
    }

    /// # Errors
    ///
    /// Returns an error if a field of the runtime asset package root is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: runtime_capnp::runtime_asset_package_root::Reader<'_>,
    ) -> Result<Self, Error> {
        read_runtime_asset_package_root(reader)
    }
}

impl RuntimeResolvedGem {
    pub fn to_capnp(&self, builder: runtime_capnp::runtime_resolved_gem::Builder<'_>) {
        write_runtime_resolved_gem(self, builder);
    }

    /// # Errors
    ///
    /// Returns an error if a field of the runtime resolved gem is absent from the message or is not
    /// valid UTF-8.
    pub fn from_capnp(
        reader: runtime_capnp::runtime_resolved_gem::Reader<'_>,
    ) -> Result<Self, Error> {
        read_runtime_resolved_gem(reader)
    }
}

impl LaunchRuntimeRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the launch runtime request.
    pub fn to_capnp(
        &self,
        builder: runtime_capnp::launch_runtime_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_launch_runtime_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the launch runtime request is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: runtime_capnp::launch_runtime_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_launch_runtime_request(reader)
    }
}

impl RuntimeStatusRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the runtime status request.
    pub fn to_capnp(
        &self,
        builder: runtime_capnp::runtime_status_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_runtime_status_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the runtime status request is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: runtime_capnp::runtime_status_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_runtime_status_request(reader)
    }
}

impl StopRuntimeRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the stop runtime request.
    pub fn to_capnp(
        &self,
        builder: runtime_capnp::stop_runtime_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_stop_runtime_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the stop runtime request is absent from the message or is not
    /// valid UTF-8.
    pub fn from_capnp(
        reader: runtime_capnp::stop_runtime_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_stop_runtime_request(reader)
    }
}

impl RuntimeStatus {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the runtime status.
    pub fn to_capnp(
        &self,
        builder: runtime_capnp::runtime_status::Builder<'_>,
    ) -> Result<(), Error> {
        write_runtime_status(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the runtime status is absent from the message or is not valid
    /// UTF-8.
    pub fn from_capnp(reader: runtime_capnp::runtime_status::Reader<'_>) -> Result<Self, Error> {
        read_runtime_status(reader)
    }
}

impl RuntimeStatusResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the runtime status result.
    pub fn to_capnp(
        &self,
        builder: runtime_capnp::runtime_status_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_runtime_status_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the runtime status result is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: runtime_capnp::runtime_status_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_runtime_status_result(reader)
    }
}

impl RuntimeViewportRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the runtime viewport
    /// request.
    pub fn to_capnp(
        &self,
        builder: runtime_capnp::runtime_viewport_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_runtime_viewport_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the runtime viewport request is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: runtime_capnp::runtime_viewport_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_runtime_viewport_request(reader)
    }
}

impl RuntimeViewportFrame {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the runtime viewport frame.
    pub fn to_capnp(
        &self,
        builder: runtime_capnp::runtime_viewport_frame::Builder<'_>,
    ) -> Result<(), Error> {
        write_runtime_viewport_frame(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the runtime viewport frame is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: runtime_capnp::runtime_viewport_frame::Reader<'_>,
    ) -> Result<Self, Error> {
        read_runtime_viewport_frame(reader)
    }
}

impl RuntimeViewportResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the runtime viewport result.
    pub fn to_capnp(
        &self,
        builder: runtime_capnp::runtime_viewport_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_runtime_viewport_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the runtime viewport result is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: runtime_capnp::runtime_viewport_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_runtime_viewport_result(reader)
    }
}

impl RuntimeProjectionDescriptor {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the runtime projection
    /// descriptor.
    pub fn to_capnp(
        &self,
        builder: runtime_capnp::runtime_projection_descriptor::Builder<'_>,
    ) -> Result<(), Error> {
        write_runtime_projection_descriptor(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the runtime projection descriptor is absent from the message
    /// or is not valid UTF-8.
    pub fn from_capnp(
        reader: runtime_capnp::runtime_projection_descriptor::Reader<'_>,
    ) -> Result<Self, Error> {
        read_runtime_projection_descriptor(reader)
    }
}

impl RuntimeProjectionCatalogRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the runtime projection
    /// catalog request.
    pub fn to_capnp(
        &self,
        builder: runtime_capnp::runtime_projection_catalog_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_runtime_projection_catalog_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the runtime projection catalog request is absent from the
    /// message or is not valid UTF-8.
    pub fn from_capnp(
        reader: runtime_capnp::runtime_projection_catalog_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_runtime_projection_catalog_request(reader)
    }
}

impl RuntimeProjectionCatalogResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the runtime projection
    /// catalog result.
    pub fn to_capnp(
        &self,
        builder: runtime_capnp::runtime_projection_catalog_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_runtime_projection_catalog_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the runtime projection catalog result is absent from the
    /// message or is not valid UTF-8.
    pub fn from_capnp(
        reader: runtime_capnp::runtime_projection_catalog_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_runtime_projection_catalog_result(reader)
    }
}

#[cfg(test)]
mod tests {
    use az_proto_core::{Capability, ServiceId, ServiceRole, SideChannelHandle, SideChannelKind};
    use capnp::message;

    use super::*;

    fn test_session_id() -> Uuid {
        Uuid::from_bytes([0x77; 16])
    }

    fn control_capability() -> Capability {
        Capability::new(
            ServiceId::new("azoth", "session-supervisor"),
            ServiceRole::SessionSupervisor,
        )
        .with_session(test_session_id())
        .with_audience(RUNTIME_HOST_AUDIENCE)
        .with_permissions([RUNTIME_CONTROL_PERMISSION])
    }

    fn read_capability() -> Capability {
        Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_session(test_session_id())
            .with_audience(RUNTIME_HOST_AUDIENCE)
            .with_permissions([RUNTIME_READ_PERMISSION])
    }

    fn snapshot() -> RuntimeLaunchSnapshot {
        let mut snapshot = RuntimeLaunchSnapshot::new(
            "local.runtime",
            Uuid::from_bytes([0x77; 16]),
            "lighting",
            RuntimeRole::EditorWorld,
            "projects/runtime",
            "projects/runtime/.azoth/workspaces/lighting",
        );
        snapshot.authored_revision = 42;
        snapshot.schema_catalog_hash = vec![0xab; 32];
        snapshot.workspace_id = 7;
        snapshot.include_unsaved_journal = true;
        snapshot.asset_source_roots.push(RuntimeAssetSourceRoot {
            workspace_root_id: 11,
            workspace_id: 7,
            root_id: 33,
            owner_id: "local.runtime".to_string(),
            source_root: "projects/runtime/.azoth/workspaces/lighting/assets".to_string(),
            display_name: "Project Assets".to_string(),
            portable_key: "project:local.runtime:assets".to_string(),
            output_prefix: String::new(),
            is_root: true,
        });
        snapshot.resolved_gems.push(RuntimeResolvedGem {
            id: "azoth.physics".to_string(),
            name: "Physics".to_string(),
            version: "0.1.0".to_string(),
            root: "projects/runtime/gems/physics".to_string(),
        });
        snapshot.asset_source_roots.push(RuntimeAssetSourceRoot {
            workspace_root_id: 12,
            workspace_id: 7,
            root_id: 34,
            owner_id: "azoth.physics".to_string(),
            source_root: "projects/runtime/gems/physics/assets".to_string(),
            display_name: "Physics Assets".to_string(),
            portable_key: "gem:azoth.physics:assets".to_string(),
            output_prefix: "gems/azoth.physics".to_string(),
            is_root: true,
        });
        snapshot.asset_package_roots.push(RuntimeAssetPackageRoot {
            profile: "pc-dev".to_string(),
            asset_platform: "pc".to_string(),
            container: RuntimeAssetPackageContainer::AzPack,
            mount_root: "projects/runtime/target/azoth/packages/pc-dev/lighting/azpack".to_string(),
            payload_path: "projects/runtime/target/azoth/packages/pc-dev/lighting/azpack"
                .to_string(),
            catalog_path:
                "projects/runtime/target/azoth/packages/pc-dev/lighting/azpack/assetcatalog.bin"
                    .to_string(),
            release_id: "0".repeat(RUNTIME_PACKAGE_RELEASE_ID_HEX_LEN),
        });
        snapshot
    }

    fn valid_status() -> RuntimeStatus {
        RuntimeStatus {
            runtime_id: "editor-world".to_string(),
            role: RuntimeRole::EditorWorld,
            state: RuntimeState::Running,
            project_id: "local.runtime".to_string(),
            session_slug: "lighting".to_string(),
            authored_revision: 42,
            diagnostic: String::new(),
        }
    }

    fn valid_frame() -> RuntimeViewportFrame {
        RuntimeViewportFrame {
            runtime_id: "editor-world".to_string(),
            width: 1920,
            height: 1080,
            row_pitch: 7680,
            format: ViewportPixelFormat::Bgra8Unorm,
            color: SideChannelHandle {
                kind: SideChannelKind::GpuSurface,
                capability: Some(read_capability()),
                locator: "dxgi://adapter/0/texture/42".to_string(),
                byte_length: 1920 * 1080 * 4,
                content_hash: Vec::new(),
                platform: "windows".to_string(),
            },
        }
    }

    fn valid_projection_catalog() -> RuntimeProjectionCatalogResult {
        RuntimeProjectionCatalogResult {
            projections: vec![RuntimeProjectionDescriptor {
                name: "az.test.editor-world".to_string(),
                priority: 25,
                roles: vec![RuntimeRole::EditorWorld, RuntimeRole::PlayPreview],
                launch_profiles: vec!["editor".to_string(), "play".to_string()],
            }],
        }
    }

    #[test]
    fn runtime_launch_snapshot_side_channel_validates_hash_and_decodes() {
        let expected = snapshot();
        let bytes = encode_runtime_launch_snapshot(&expected).unwrap();
        let hash = blake3::hash(&bytes).as_bytes().to_vec();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("runtime-launch.capnp.packed");
        std::fs::write(&path, &bytes).unwrap();
        let handle = SideChannelHandle::staging_file(
            path.to_string_lossy(),
            bytes.len() as u64,
            hash,
            std::env::consts::OS,
        );

        let actual = load_runtime_launch_snapshot_side_channel(&handle).unwrap();
        assert_eq!(actual, expected);

        let mut corrupted = handle;
        corrupted.content_hash = vec![0; 32];
        assert!(matches!(
            load_runtime_launch_snapshot_side_channel(&corrupted),
            Err(RuntimeSnapshotSideChannelError::InvalidHandle(
                StagingFileSideChannelError::HashMismatch { .. }
            ))
        ));
    }

    #[test]
    fn runtime_package_root_shape_tracks_container_backend() {
        let mut snapshot = snapshot();
        let azpack = snapshot.asset_package_roots.remove(0);
        validate_runtime_asset_package_root_shape(&azpack).unwrap();

        let mut pak = azpack.clone();
        pak.container = RuntimeAssetPackageContainer::Pak;
        pak.mount_root = "projects/runtime/target/azoth/packages/pc-dev/lighting".to_string();
        pak.payload_path =
            "projects/runtime/target/azoth/packages/pc-dev/lighting/pc-dev.pak".to_string();
        pak.catalog_path =
            "projects/runtime/target/azoth/packages/pc-dev/lighting/assetcatalog.bin".to_string();
        validate_runtime_asset_package_root_shape(&pak).unwrap();

        let mut bad_pak = pak.clone();
        bad_pak.payload_path = bad_pak.mount_root.clone();
        let error = validate_runtime_asset_package_root_shape(&bad_pak).unwrap_err();
        assert!(error.contains("pak file"), "{error}");

        let mut bad_azpack = azpack;
        bad_azpack.payload_path.push_str("/package.azpack.index");
        let error = validate_runtime_asset_package_root_shape(&bad_azpack).unwrap_err();
        assert!(error.contains("payload path"), "{error}");

        let mut bad_catalog = pak;
        bad_catalog.catalog_path =
            "projects/runtime/target/azoth/packages/pc-dev/lighting/catalog.ron".to_string();
        let error = validate_runtime_asset_package_root_shape(&bad_catalog).unwrap_err();
        assert!(error.contains("assetcatalog.bin"), "{error}");
    }

    #[test]
    fn runtime_package_roots_validator_rejects_duplicate_catalogs() {
        let snapshot = snapshot();
        let mut duplicate = snapshot.asset_package_roots[0].clone();
        duplicate.profile = "pc-release".to_string();
        duplicate.mount_root.push_str("-release");
        duplicate.payload_path.push_str("-release");

        let error = validate_runtime_asset_package_roots(&[
            snapshot.asset_package_roots[0].clone(),
            duplicate,
        ])
        .unwrap_err();

        assert!(matches!(
            error,
            RuntimeAssetPackageRootsError::DuplicateCatalog { .. }
        ));
        assert!(error.to_string().contains("assetcatalog.bin"), "{error}");
    }

    #[test]
    fn runtime_launch_snapshot_encoding_rejects_missing_db_source_roots() {
        let mut snapshot = RuntimeLaunchSnapshot::new(
            "local.runtime",
            Uuid::from_bytes([0x77; 16]),
            "lighting",
            RuntimeRole::EditorWorld,
            "projects/runtime",
            "projects/runtime/.azoth/workspaces/lighting",
        );
        snapshot.authored_revision = 42;
        snapshot.schema_catalog_hash = vec![0xab; 32];
        snapshot.workspace_id = 7;

        let error = encode_runtime_launch_snapshot(&snapshot).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("asset source roots cannot be empty"),
            "{error}"
        );
    }

    #[test]
    fn runtime_launch_snapshot_encoding_requires_project_assets_root() {
        let mut snapshot = RuntimeLaunchSnapshot::new(
            "local.runtime",
            Uuid::from_bytes([0x77; 16]),
            "lighting",
            RuntimeRole::EditorWorld,
            "projects/runtime",
            "projects/runtime/.azoth/workspaces/lighting",
        );
        snapshot.authored_revision = 42;
        snapshot.schema_catalog_hash = vec![0xab; 32];
        snapshot.workspace_id = 7;
        snapshot.asset_source_roots.push(RuntimeAssetSourceRoot {
            workspace_root_id: 12,
            workspace_id: 7,
            root_id: 34,
            owner_id: "azoth.physics".to_string(),
            source_root: "projects/runtime/gems/physics/assets".to_string(),
            display_name: "Physics Assets".to_string(),
            portable_key: "gem:azoth.physics:assets".to_string(),
            output_prefix: "gems/azoth.physics".to_string(),
            is_root: true,
        });

        let error = encode_runtime_launch_snapshot(&snapshot).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("missing DB-owned project assets root"),
            "{error}"
        );
    }

    #[test]
    fn runtime_launch_snapshot_encoding_rejects_malformed_asset_source_roots() {
        let mut duplicate_id = snapshot();
        let mut duplicated = duplicate_id.asset_source_roots[0].clone();
        duplicated.portable_key = "project:local.runtime:assets".to_string();
        duplicated.source_root = "projects/runtime/assets".to_string();
        duplicate_id.asset_source_roots.push(duplicated);
        let error = encode_runtime_launch_snapshot(&duplicate_id).unwrap_err();
        assert!(
            error.to_string().contains("duplicate source root id"),
            "{error}"
        );

        let mut duplicate_key = snapshot();
        duplicate_key.asset_source_roots[1].portable_key =
            duplicate_key.asset_source_roots[0].portable_key.clone();
        duplicate_key.asset_source_roots[1].workspace_root_id += 100;
        duplicate_key.asset_source_roots[1].root_id += 100;
        let error = encode_runtime_launch_snapshot(&duplicate_key).unwrap_err();
        assert!(
            error.to_string().contains("duplicate source root key"),
            "{error}"
        );

        let mut wrong_owner = snapshot();
        wrong_owner.asset_source_roots[0].owner_id = "azoth.physics".to_string();
        let error = encode_runtime_launch_snapshot(&wrong_owner).unwrap_err();
        assert!(
            error.to_string().contains("owner")
                && error.to_string().contains("azoth.physics")
                && error.to_string().contains("local.runtime"),
            "{error}"
        );
    }

    #[test]
    fn runtime_launch_snapshot_encoding_rejects_project_assets_root_shape_mismatch() {
        let mut not_root = snapshot();
        not_root.asset_source_roots[0].is_root = false;
        let error = encode_runtime_launch_snapshot(&not_root).unwrap_err();
        assert!(
            error.to_string().contains("marked as a root source"),
            "{error}"
        );

        let mut prefixed = snapshot();
        prefixed.asset_source_roots[0].output_prefix = "prefabs".to_string();
        let error = encode_runtime_launch_snapshot(&prefixed).unwrap_err();
        assert!(error.to_string().contains("empty output prefix"), "{error}");
    }

    #[test]
    fn runtime_launch_snapshot_encoding_rejects_duplicate_resolved_gems() {
        let mut duplicate_id_snapshot = snapshot();
        let mut duplicate = duplicate_id_snapshot.resolved_gems[0].clone();
        duplicate.name = "Physics Copy".to_string();
        duplicate.root = "projects/runtime/gems/physics-copy".to_string();
        duplicate_id_snapshot.resolved_gems.push(duplicate);

        let error = encode_runtime_launch_snapshot(&duplicate_id_snapshot).unwrap_err();

        assert!(
            error.to_string().contains("duplicate id `azoth.physics`"),
            "{error}"
        );

        let mut duplicate_root_snapshot = snapshot();
        let mut duplicate = duplicate_root_snapshot.resolved_gems[0].clone();
        duplicate.id = "azoth.physics.copy".to_string();
        duplicate.name = "Physics Copy".to_string();
        duplicate_root_snapshot.resolved_gems.push(duplicate);

        let error = encode_runtime_launch_snapshot(&duplicate_root_snapshot).unwrap_err();

        assert!(error.to_string().contains("duplicate root"), "{error}");
    }

    #[test]
    fn launch_request_round_trips_snapshot_handle() {
        let temp = tempfile::tempdir().unwrap();
        let handle = SideChannelHandle::staging_file(
            temp.path()
                .join("runtime-launch.capnp.packed")
                .to_string_lossy(),
            128,
            vec![0xcd; 32],
            "windows",
        )
        .with_capability(control_capability());
        let expected = LaunchRuntimeRequest {
            capability: control_capability(),
            runtime_id: "editor-world".to_string(),
            role: RuntimeRole::EditorWorld,
            snapshot: handle,
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<runtime_capnp::launch_runtime_request::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<runtime_capnp::launch_runtime_request::Reader<'_>>()
            .unwrap();
        assert_eq!(LaunchRuntimeRequest::from_capnp(reader).unwrap(), expected);
    }

    #[test]
    fn launch_request_encode_rejects_unbound_snapshot_handle() {
        let temp = tempfile::tempdir().unwrap();
        let handle = SideChannelHandle::staging_file(
            temp.path()
                .join("runtime-launch.capnp.packed")
                .to_string_lossy(),
            128,
            vec![0xcd; 32],
            "windows",
        );
        let request = LaunchRuntimeRequest {
            capability: control_capability(),
            runtime_id: "editor-world".to_string(),
            role: RuntimeRole::EditorWorld,
            snapshot: handle,
        };
        let mut message = message::Builder::new_default();

        let error = (request)
            .to_capnp(message.init_root::<runtime_capnp::launch_runtime_request::Builder<'_>>())
            .unwrap_err();

        assert!(error.to_string().contains("bound to a brokered capability"));
    }

    #[test]
    fn launch_request_encode_rejects_read_capability() {
        let temp = tempfile::tempdir().unwrap();
        let handle = SideChannelHandle::staging_file(
            temp.path()
                .join("runtime-launch.capnp.packed")
                .to_string_lossy(),
            128,
            vec![0xcd; 32],
            "windows",
        )
        .with_capability(control_capability());
        let request = LaunchRuntimeRequest {
            capability: read_capability(),
            runtime_id: "editor-world".to_string(),
            role: RuntimeRole::EditorWorld,
            snapshot: handle,
        };
        let mut message = message::Builder::new_default();

        let error = (request)
            .to_capnp(message.init_root::<runtime_capnp::launch_runtime_request::Builder<'_>>())
            .unwrap_err();

        assert!(error.to_string().contains(RUNTIME_CONTROL_PERMISSION));
    }

    #[test]
    fn runtime_status_request_accepts_control_capability_for_read() {
        let expected = RuntimeStatusRequest {
            capability: control_capability(),
            runtime_id: "editor-world".to_string(),
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<runtime_capnp::runtime_status_request::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<runtime_capnp::runtime_status_request::Reader<'_>>()
            .unwrap();
        assert_eq!(RuntimeStatusRequest::from_capnp(reader).unwrap(), expected);
    }

    #[test]
    fn runtime_status_request_encode_rejects_wrong_audience() {
        let request = RuntimeStatusRequest {
            capability: read_capability().with_audience("project-host"),
            runtime_id: "editor-world".to_string(),
        };
        let mut message = message::Builder::new_default();

        let error = (request)
            .to_capnp(message.init_root::<runtime_capnp::runtime_status_request::Builder<'_>>())
            .unwrap_err();

        assert!(error.to_string().contains(RUNTIME_HOST_AUDIENCE));
    }

    #[test]
    fn status_result_round_trips_optional_status() {
        let expected = RuntimeStatusResult {
            status: Some(valid_status()),
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<runtime_capnp::runtime_status_result::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<runtime_capnp::runtime_status_result::Reader<'_>>()
            .unwrap();
        assert_eq!(RuntimeStatusResult::from_capnp(reader).unwrap(), expected);
    }

    #[test]
    fn viewport_result_round_trips_side_channel_frame() {
        let expected = RuntimeViewportResult {
            frame: Some(valid_frame()),
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<runtime_capnp::runtime_viewport_result::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<runtime_capnp::runtime_viewport_result::Reader<'_>>()
            .unwrap();
        assert_eq!(RuntimeViewportResult::from_capnp(reader).unwrap(), expected);
    }

    #[test]
    fn viewport_result_encode_rejects_unbound_color_handle() {
        let mut frame = valid_frame();
        frame.color.capability = None;
        let result = RuntimeViewportResult { frame: Some(frame) };
        let mut message = message::Builder::new_default();

        let error = (result)
            .to_capnp(message.init_root::<runtime_capnp::runtime_viewport_result::Builder<'_>>())
            .unwrap_err();

        assert!(error.to_string().contains("runtime.read capability"));
    }

    #[test]
    fn viewport_result_encode_rejects_non_gpu_surface_handles() {
        let mut frame = valid_frame();
        frame.color.kind = SideChannelKind::SharedMemory;
        frame.color.locator = "shared-memory://viewport/42".to_string();
        let result = RuntimeViewportResult { frame: Some(frame) };
        let mut message = message::Builder::new_default();

        let error = (result)
            .to_capnp(message.init_root::<runtime_capnp::runtime_viewport_result::Builder<'_>>())
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("runtime viewport frames must use gpuSurface side channels")
        );
    }

    #[test]
    fn runtime_projection_catalog_round_trips_projection_metadata() {
        let expected_request = RuntimeProjectionCatalogRequest {
            capability: read_capability(),
        };
        let mut request_message = message::Builder::new_default();
        (expected_request)
            .to_capnp(
                request_message
                    .init_root::<runtime_capnp::runtime_projection_catalog_request::Builder<'_>>(),
            )
            .unwrap();

        let request_reader = request_message
            .get_root_as_reader::<runtime_capnp::runtime_projection_catalog_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            RuntimeProjectionCatalogRequest::from_capnp(request_reader).unwrap(),
            expected_request
        );

        let expected_result = valid_projection_catalog();
        let mut result_message = message::Builder::new_default();
        (expected_result)
            .to_capnp(
                result_message
                    .init_root::<runtime_capnp::runtime_projection_catalog_result::Builder<'_>>(),
            )
            .unwrap();

        let result_reader = result_message
            .get_root_as_reader::<runtime_capnp::runtime_projection_catalog_result::Reader<'_>>()
            .unwrap();
        assert_eq!(
            RuntimeProjectionCatalogResult::from_capnp(result_reader).unwrap(),
            expected_result
        );
    }

    #[test]
    fn runtime_projection_catalog_request_encode_rejects_unscoped_capability() {
        let mut capability = read_capability();
        capability.session = None;
        let request = RuntimeProjectionCatalogRequest { capability };
        let mut message = message::Builder::new_default();

        let error = (request)
            .to_capnp(
                message
                    .init_root::<runtime_capnp::runtime_projection_catalog_request::Builder<'_>>(),
            )
            .unwrap_err();

        assert!(error.to_string().contains("session-scoped"), "{error}");
    }

    #[test]
    fn runtime_projection_catalog_encode_rejects_duplicate_projection_names() {
        let mut catalog = valid_projection_catalog();
        catalog.projections.push(catalog.projections[0].clone());
        let mut message = message::Builder::new_default();

        let error = (catalog)
            .to_capnp(
                message
                    .init_root::<runtime_capnp::runtime_projection_catalog_result::Builder<'_>>(),
            )
            .unwrap_err();

        assert!(error.to_string().contains("duplicate projection"));
    }
}
