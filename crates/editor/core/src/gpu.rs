//! Editor-owned GPU launch configuration.
//!
//! GPUI owns the editor window, while runtime-host owns project/game rendering.
//! This module keeps the editor's WGPU policy explicit so viewport frames can
//! use a GPU side-channel import path without coupling the editor process to
//! engine or project runtime crates.

use std::collections::{BTreeMap, BTreeSet};

use az_editor_ui::panels::EditorGpuStatus;
use az_prefab::{
    EntityAlias, InstanceAlias, OverrideOperation, PrefabAssetPath, PrefabCodec, PrefabDocument,
    PrefabEntity, PrefabInstance, PrefabTypeData, ReflectedPath, TypedOverrideAction,
    TypedOverrideTarget,
};
use az_proto_project::vnext::{
    PrefabOverrideOperation as WireOverrideOperation, PrefabSourceSnapshot, ReflectedPathSegment,
    ReflectedValueEncoding, ReflectedValueEnvelope, TypeRegistrySnapshot,
};
use az_proto_runtime::ViewportPixelFormat;
use bevy::reflect::TypeRegistry;
use gpui::BorrowAppContext;
use thiserror::Error;
use tracing::{error, info};

/// Static GPU configuration used when launching editor-owned rendering helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditorGpuLaunchConfig {
    pub backends: wgpu::Backends,
    pub power_preference: wgpu::PowerPreference,
    pub viewport_color_format: wgpu::TextureFormat,
    pub present_mode: wgpu::PresentMode,
}

impl EditorGpuLaunchConfig {
    #[must_use]
    pub fn instance_descriptor(self) -> wgpu::InstanceDescriptor {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = self.backends;
        descriptor
    }
}

impl Default for EditorGpuLaunchConfig {
    fn default() -> Self {
        Self {
            backends: wgpu::Backends::PRIMARY,
            power_preference: wgpu::PowerPreference::HighPerformance,
            viewport_color_format: wgpu::TextureFormat::Bgra8Unorm,
            present_mode: wgpu::PresentMode::AutoVsync,
        }
    }
}

#[must_use]
pub fn editor_gpu_instance(config: EditorGpuLaunchConfig) -> wgpu::Instance {
    wgpu::Instance::new(config.instance_descriptor())
}

#[must_use]
pub const fn editor_gpu_required_features(_config: EditorGpuLaunchConfig) -> wgpu::Features {
    wgpu::Features::empty()
}

#[must_use]
pub const fn editor_gpu_required_limits(_config: EditorGpuLaunchConfig) -> wgpu::Limits {
    wgpu::Limits::defaults()
}

#[must_use]
pub fn editor_gpu_device_descriptor(
    config: EditorGpuLaunchConfig,
) -> wgpu::DeviceDescriptor<'static> {
    wgpu::DeviceDescriptor {
        label: Some("az-editor-gpu"),
        required_features: editor_gpu_required_features(config),
        required_limits: editor_gpu_required_limits(config),
        experimental_features: wgpu::ExperimentalFeatures::default(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
    }
}

#[must_use]
pub const fn editor_gpu_request_adapter_options(
    config: EditorGpuLaunchConfig,
) -> wgpu::RequestAdapterOptions<'static, 'static> {
    wgpu::RequestAdapterOptions {
        power_preference: config.power_preference,
        force_fallback_adapter: false,
        compatible_surface: None,
    }
}

/// Request a WGPU adapter matching the editor's launch configuration.
///
/// # Errors
///
/// Returns [`wgpu::RequestAdapterError`] if no adapter satisfies the
/// configured backends and power preference (fallback adapters are not
/// accepted).
pub async fn request_editor_gpu_adapter(
    config: EditorGpuLaunchConfig,
) -> Result<wgpu::Adapter, wgpu::RequestAdapterError> {
    let instance = editor_gpu_instance(config);
    let options = editor_gpu_request_adapter_options(config);
    instance.request_adapter(&options).await
}

/// Request the adapter and open the device and queue the editor renders with.
///
/// # Errors
///
/// Returns [`EditorGpuLaunchError::RequestAdapter`] if no adapter satisfies the
/// configuration, or [`EditorGpuLaunchError::RequestDevice`] if the adapter
/// cannot supply a device meeting the editor's requested limits and features.
pub async fn request_editor_gpu_device(
    config: EditorGpuLaunchConfig,
) -> Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue), EditorGpuLaunchError> {
    let adapter = request_editor_gpu_adapter(config).await?;
    let descriptor = editor_gpu_device_descriptor(config);
    let (device, queue) = adapter.request_device(&descriptor).await?;

    Ok((adapter, device, queue))
}

/// Bring up the editor's GPU state: adapter, device, queue and adapter report.
///
/// # Errors
///
/// Returns any error [`request_editor_gpu_device`] returns —
/// [`EditorGpuLaunchError::RequestAdapter`] if no adapter matches, or
/// [`EditorGpuLaunchError::RequestDevice`] if the device cannot be created.
pub async fn launch_editor_gpu(
    config: EditorGpuLaunchConfig,
) -> Result<EditorGpuReadyState, EditorGpuLaunchError> {
    let (adapter, device, queue) = request_editor_gpu_device(config).await?;
    let report = EditorGpuAdapterReport::from_adapter(&adapter);

    Ok(EditorGpuReadyState {
        report,
        device,
        queue,
    })
}

#[derive(Debug, Error)]
pub enum EditorGpuLaunchError {
    #[error("failed to find an editor WGPU adapter: {0}")]
    RequestAdapter(#[from] wgpu::RequestAdapterError),
    #[error("failed to create an editor WGPU device: {0}")]
    RequestDevice(#[from] wgpu::RequestDeviceError),
    #[error("runtime viewport format {0:?} cannot be imported into an editor WGPU texture")]
    UnsupportedViewportFormat(ViewportPixelFormat),
    #[error("runtime viewport dimensions must be non-zero, got {width}x{height}")]
    InvalidViewportDimensions { width: u32, height: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorGpuAdapterReport {
    pub info: wgpu::AdapterInfo,
    pub features: wgpu::Features,
    pub limits: wgpu::Limits,
}

impl EditorGpuAdapterReport {
    #[must_use]
    pub fn from_adapter(adapter: &wgpu::Adapter) -> Self {
        Self {
            info: adapter.get_info(),
            features: adapter.features(),
            limits: adapter.limits(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct EditorGpuReadyState {
    pub report: EditorGpuAdapterReport,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

#[derive(Debug, Clone)]
pub enum EditorGpuLaunchStatus {
    NotRequested,
    Starting,
    /// Boxed: `EditorGpuReadyState` carries a full `wgpu::Limits` plus the
    /// adapter report, which would otherwise set the size of every launch
    /// status the editor stores, including the three empty ones.
    Ready(Box<EditorGpuReadyState>),
    Failed {
        diagnostic: String,
    },
}

impl EditorGpuLaunchStatus {
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        matches!(self, Self::Ready(_))
    }

    #[must_use]
    pub const fn ready(&self) -> Option<&EditorGpuReadyState> {
        match self {
            Self::Ready(ready) => Some(&**ready),
            Self::NotRequested | Self::Starting | Self::Failed { .. } => None,
        }
    }
}

/// GPUI global carrying editor GPU configuration.
#[derive(Debug, Clone)]
pub struct EditorGpuState {
    config: EditorGpuLaunchConfig,
    launch_status: EditorGpuLaunchStatus,
    viewport_texture: Option<EditorViewportGpuTexture>,
}

impl EditorGpuState {
    #[must_use]
    pub const fn new(config: EditorGpuLaunchConfig) -> Self {
        Self {
            config,
            launch_status: EditorGpuLaunchStatus::NotRequested,
            viewport_texture: None,
        }
    }

    #[must_use]
    pub const fn config(&self) -> EditorGpuLaunchConfig {
        self.config
    }

    #[must_use]
    pub const fn launch_status(&self) -> &EditorGpuLaunchStatus {
        &self.launch_status
    }

    #[must_use]
    pub const fn ready(&self) -> Option<&EditorGpuReadyState> {
        self.launch_status.ready()
    }

    #[must_use]
    pub const fn viewport_texture(&self) -> Option<&EditorViewportGpuTexture> {
        self.viewport_texture.as_ref()
    }

    pub fn mark_starting(&mut self) {
        self.launch_status = EditorGpuLaunchStatus::Starting;
        self.viewport_texture = None;
    }

    pub fn mark_ready(&mut self, ready: EditorGpuReadyState) {
        self.launch_status = EditorGpuLaunchStatus::Ready(Box::new(ready));
    }

    pub fn mark_failed(&mut self, diagnostic: impl Into<String>) {
        self.launch_status = EditorGpuLaunchStatus::Failed {
            diagnostic: diagnostic.into(),
        };
        self.viewport_texture = None;
    }

    pub fn store_viewport_texture(&mut self, texture: EditorViewportGpuTexture) {
        self.viewport_texture = Some(texture);
    }

    pub fn clear_viewport_texture(&mut self) {
        self.viewport_texture = None;
    }
}

impl Default for EditorGpuState {
    fn default() -> Self {
        Self::new(EditorGpuLaunchConfig::default())
    }
}

impl gpui::Global for EditorGpuState {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewportScene {
    pub registry: TypeRegistrySnapshot,
    pub sources: Vec<ViewportSourceSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewportSourceSnapshot {
    pub source_path: String,
    pub snapshot: PrefabSourceSnapshot,
}

impl Default for ViewportScene {
    fn default() -> Self {
        Self {
            registry: TypeRegistrySnapshot {
                schema_catalog_hash: Vec::new(),
                types: Vec::new(),
            },
            sources: Vec::new(),
        }
    }
}

impl ViewportScene {
    #[must_use]
    pub fn entity_count(&self) -> usize {
        self.sources
            .iter()
            .map(|source| source.snapshot.entities.len())
            .sum()
    }
}

#[derive(Debug)]
pub(crate) struct EncodedViewportSource {
    pub source_path: String,
    pub source: String,
}

#[derive(Debug, Clone)]
pub(crate) struct AuthoredPreviewEntity {
    pub source_alias: String,
    pub document_id: String,
    pub object_id: String,
    pub component_type_paths: Vec<String>,
}

pub(crate) fn encode_prefab_sources(
    scene: &ViewportScene,
    registry: &TypeRegistry,
) -> Result<Vec<EncodedViewportSource>, String> {
    let codec = PrefabCodec::new(registry).map_err(|error| error.to_string())?;
    scene
        .sources
        .iter()
        .map(|source| {
            let document = prefab_document_from_snapshot(&source.snapshot, &codec, registry)
                .map_err(|error| format!("rebuild `{}`: {error}", source.source_path))?;
            let source_text = codec
                .encode(&document)
                .map_err(|error| format!("encode `{}`: {error}", source.source_path))?;
            Ok(EncodedViewportSource {
                source_path: source.source_path.clone(),
                source: source_text,
            })
        })
        .collect()
}

pub(crate) fn authored_preview_entities(
    scene: &ViewportScene,
) -> Result<BTreeMap<String, AuthoredPreviewEntity>, String> {
    let Some(root) = scene.sources.first() else {
        return Ok(BTreeMap::new());
    };
    let sources = scene
        .sources
        .iter()
        .map(|source| (source.source_path.as_str(), &source.snapshot))
        .collect::<BTreeMap<_, _>>();
    let mut entities = BTreeMap::new();
    collect_authored_preview_entities(
        root.source_path.as_str(),
        "",
        &sources,
        &mut Vec::new(),
        &mut entities,
    )?;
    Ok(entities)
}

fn prefab_document_from_snapshot(
    snapshot: &PrefabSourceSnapshot,
    codec: &PrefabCodec<'_>,
    registry: &TypeRegistry,
) -> Result<PrefabDocument, String> {
    let type_versions = snapshot_type_versions(snapshot, registry)?;
    let parents = snapshot_parents(snapshot)?;
    let mut components = snapshot_components(snapshot, codec, registry)?;

    let entity_aliases = snapshot
        .entities
        .iter()
        .map(|entity| entity.alias.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(alias) = parents
        .keys()
        .chain(components.keys())
        .find(|alias| !entity_aliases.contains(alias.as_str()))
    {
        return Err(format!("snapshot data references missing entity `{alias}`"));
    }
    let entities = snapshot_entities(snapshot, &parents, &mut components)?;
    let instances = snapshot_instances(snapshot, codec)?;

    PrefabDocument::try_from_parts(
        snapshot.document_version,
        type_versions,
        entities,
        instances,
    )
    .map_err(|error| error.to_string())
}

/// Component schema versions rekeyed from wire type paths onto canonical Prefab
/// tags.
fn snapshot_type_versions(
    snapshot: &PrefabSourceSnapshot,
    registry: &TypeRegistry,
) -> Result<BTreeMap<String, u32>, String> {
    let mut type_versions = BTreeMap::new();
    for (type_path, version) in &snapshot.type_versions {
        let tag = canonical_prefab_tag(registry, type_path)?;
        if type_versions.insert(tag.to_owned(), *version).is_some() {
            return Err(format!("duplicate canonical component tag `{tag}`"));
        }
    }
    Ok(type_versions)
}

/// `child alias -> optional parent alias`, rejecting duplicate hierarchy edges.
fn snapshot_parents(
    snapshot: &PrefabSourceSnapshot,
) -> Result<BTreeMap<String, Option<String>>, String> {
    let mut parents = BTreeMap::new();
    for edge in &snapshot.hierarchy {
        if parents
            .insert(edge.child_alias.clone(), edge.parent_alias.clone())
            .is_some()
        {
            return Err(format!(
                "duplicate hierarchy record for `{}`",
                edge.child_alias
            ));
        }
    }
    Ok(parents)
}

/// `entity alias -> canonical tag -> decoded sparse value`.
fn snapshot_components(
    snapshot: &PrefabSourceSnapshot,
    codec: &PrefabCodec<'_>,
    registry: &TypeRegistry,
) -> Result<BTreeMap<String, BTreeMap<String, az_prefab::SparseValue>>, String> {
    let mut components = BTreeMap::<String, BTreeMap<String, az_prefab::SparseValue>>::new();
    for component in &snapshot.components {
        if component.type_path != component.sparse_value.type_path {
            return Err(format!(
                "component `{}` envelope carries type `{}`",
                component.type_path, component.sparse_value.type_path
            ));
        }
        let tag = canonical_prefab_tag(registry, &component.type_path)?;
        let value = decode_envelope(codec, &component.sparse_value)?;
        if components
            .entry(component.entity_alias.clone())
            .or_default()
            .insert(tag.to_owned(), value)
            .is_some()
        {
            return Err(format!(
                "entity `{}` contains duplicate component `{tag}`",
                component.entity_alias
            ));
        }
    }
    Ok(components)
}

/// Authored entities with their parent edge and component map attached; the
/// component map is drained so each entry lands on exactly one entity.
fn snapshot_entities(
    snapshot: &PrefabSourceSnapshot,
    parents: &BTreeMap<String, Option<String>>,
    components: &mut BTreeMap<String, BTreeMap<String, az_prefab::SparseValue>>,
) -> Result<Vec<(EntityAlias, PrefabEntity)>, String> {
    snapshot
        .entities
        .iter()
        .map(|entity| {
            let alias =
                EntityAlias::new(entity.alias.clone()).map_err(|error| error.to_string())?;
            let parent = parents
                .get(&entity.alias)
                .cloned()
                .flatten()
                .map(EntityAlias::new)
                .transpose()
                .map_err(|error| error.to_string())?;
            Ok((
                alias,
                PrefabEntity {
                    entity_id: None,
                    parent,
                    components: components.remove(&entity.alias).unwrap_or_default(),
                },
            ))
        })
        .collect()
}

/// Nested Prefab instances with their decoded override operations attached.
fn snapshot_instances(
    snapshot: &PrefabSourceSnapshot,
    codec: &PrefabCodec<'_>,
) -> Result<Vec<(InstanceAlias, PrefabInstance)>, String> {
    let mut overrides = BTreeMap::<String, Vec<OverrideOperation>>::new();
    for override_snapshot in &snapshot.overrides {
        let (instance_alias, operation) = convert_override(&override_snapshot.operation, codec)?;
        overrides.entry(instance_alias).or_default().push(operation);
    }
    let instances = snapshot
        .instances
        .iter()
        .map(|instance| {
            let alias =
                InstanceAlias::new(instance.alias.clone()).map_err(|error| error.to_string())?;
            let source = PrefabAssetPath::new(instance.source_asset.clone())
                .map_err(|error| error.to_string())?;
            let parent = instance
                .parent_entity_alias
                .clone()
                .map(EntityAlias::new)
                .transpose()
                .map_err(|error| error.to_string())?;
            Ok((
                alias,
                PrefabInstance {
                    source,
                    parent,
                    overrides: overrides.remove(&instance.alias).unwrap_or_default(),
                },
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    if let Some(instance_alias) = overrides.into_keys().next() {
        return Err(format!(
            "override references missing root instance `{instance_alias}`"
        ));
    }
    Ok(instances)
}

fn canonical_prefab_tag<'a>(
    registry: &'a TypeRegistry,
    type_path: &str,
) -> Result<&'a str, String> {
    registry
        .get_with_type_path(type_path)
        .and_then(|registration| registration.data::<PrefabTypeData>())
        .map(|prefab| prefab.tag)
        .ok_or_else(|| format!("type `{type_path}` is not a registered Prefab component"))
}

fn decode_envelope(
    codec: &PrefabCodec<'_>,
    envelope: &ReflectedValueEnvelope,
) -> Result<az_prefab::SparseValue, String> {
    if envelope.encoding != ReflectedValueEncoding::TypedRon {
        return Err(format!(
            "type `{}` uses unsupported preview encoding {:?}",
            envelope.type_path, envelope.encoding
        ));
    }
    codec
        .decode_sparse_value(&envelope.type_path, &envelope.payload)
        .map_err(|error| error.to_string())
}

fn convert_override(
    operation: &WireOverrideOperation,
    codec: &PrefabCodec<'_>,
) -> Result<(String, OverrideOperation), String> {
    let target = operation.target();
    let (root_instance, nested_instances) = target
        .instance_alias_chain
        .split_first()
        .ok_or_else(|| "override has no root instance alias".to_owned())?;
    let instance_chain = nested_instances
        .iter()
        .cloned()
        .map(InstanceAlias::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let fields = target
        .path
        .segments
        .iter()
        .map(|segment| match segment {
            ReflectedPathSegment::Field(field) => Ok(field.clone()),
            other => Err(format!(
                "Prefab override path contains unsupported segment `{other:?}`"
            )),
        })
        .collect::<Result<Vec<_>, String>>()?;
    let target = TypedOverrideTarget::new(
        instance_chain,
        EntityAlias::new(target.entity_alias.clone()).map_err(|error| error.to_string())?,
        target.path.component_type_path.clone(),
        ReflectedPath::new(fields).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let action = match operation {
        WireOverrideOperation::Set { value, .. } => {
            TypedOverrideAction::Set(decode_envelope(codec, value)?)
        }
        WireOverrideOperation::Clear { .. } => TypedOverrideAction::Clear,
        WireOverrideOperation::Insert { index, value, .. } => TypedOverrideAction::Insert {
            index: usize::try_from(*index).map_err(|error| error.to_string())?,
            value: decode_envelope(codec, value)?,
        },
        WireOverrideOperation::Remove { index, .. } => TypedOverrideAction::Remove {
            index: usize::try_from(*index).map_err(|error| error.to_string())?,
        },
        WireOverrideOperation::Move { from, to, .. } => TypedOverrideAction::Move {
            from: usize::try_from(*from).map_err(|error| error.to_string())?,
            to: usize::try_from(*to).map_err(|error| error.to_string())?,
        },
    };
    Ok((root_instance.clone(), OverrideOperation { target, action }))
}

fn collect_authored_preview_entities(
    source_path: &str,
    prefix: &str,
    sources: &BTreeMap<&str, &PrefabSourceSnapshot>,
    stack: &mut Vec<String>,
    entities: &mut BTreeMap<String, AuthoredPreviewEntity>,
) -> Result<(), String> {
    if stack.iter().any(|path| path == source_path) {
        stack.push(source_path.to_owned());
        return Err(format!(
            "Prefab preview source cycle: {}",
            stack.join(" -> ")
        ));
    }
    let snapshot = sources
        .get(source_path)
        .ok_or_else(|| format!("preview is missing nested Prefab `{source_path}`"))?;
    stack.push(source_path.to_owned());
    for entity in &snapshot.entities {
        let source_alias = join_source_alias(prefix, &entity.alias);
        let preview = AuthoredPreviewEntity {
            source_alias: source_alias.clone(),
            document_id: source_path.to_owned(),
            object_id: entity.alias.clone(),
            component_type_paths: snapshot
                .components
                .iter()
                .filter(|component| component.entity_alias == entity.alias)
                .map(|component| component.type_path.clone())
                .collect(),
        };
        if entities.insert(source_alias.clone(), preview).is_some() {
            return Err(format!(
                "duplicate flattened preview entity alias `{source_alias}`"
            ));
        }
    }
    for instance in &snapshot.instances {
        let nested_prefix = join_source_alias(prefix, &instance.alias);
        collect_authored_preview_entities(
            &instance.source_asset,
            &nested_prefix,
            sources,
            stack,
            entities,
        )?;
    }
    stack.pop();
    Ok(())
}

fn join_source_alias(prefix: &str, alias: &str) -> String {
    let alias = alias.replace('%', "%25").replace('/', "%2F");
    if prefix.is_empty() {
        alias
    } else {
        format!("{prefix}/{alias}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewportPickHit {
    pub id: String,
    pub document_id: Option<String>,
    pub object_id: Option<String>,
}

pub fn init_editor_gpu(cx: &mut gpui::App) {
    cx.set_global(EditorGpuState::default());
    cx.set_global(EditorGpuStatus::not_requested());
}

pub fn launch_editor_gpu_from_app(cx: &mut gpui::App) {
    let config = cx.global::<EditorGpuState>().config();
    cx.update_global::<EditorGpuState, _>(|state, _| state.mark_starting());
    cx.set_global(EditorGpuStatus::starting());
    cx.spawn(async move |cx| {
        let launch = launch_editor_gpu(config).await;

        match launch {
            Ok(ready) => {
                let info = ready.report.info.clone();
                let status = editor_gpu_status_from_ready(&ready);
                cx.update(move |cx| {
                    cx.update_global::<EditorGpuState, _>(|state, _| state.mark_ready(ready));
                    cx.set_global(status);
                });
                info!(
                    adapter = %info.name,
                    backend = ?info.backend,
                    device_type = ?info.device_type,
                    driver = %info.driver,
                    "editor WGPU device launched"
                );
            }
            Err(error) => {
                let diagnostic = error.to_string();
                error!(error = %diagnostic, "failed to launch editor WGPU device");
                cx.update(move |cx| {
                    cx.update_global::<EditorGpuState, _>(|state, _| {
                        state.mark_failed(diagnostic.clone());
                    });
                    cx.set_global(EditorGpuStatus::failed(diagnostic));
                });
            }
        }
    })
    .detach();
}

#[must_use]
pub fn editor_gpu_status_from_ready(ready: &EditorGpuReadyState) -> EditorGpuStatus {
    let info = &ready.report.info;
    EditorGpuStatus::ready(
        info.name.clone(),
        format!("{:?}", info.backend),
        format!("{:?}", info.device_type),
        info.driver.clone(),
    )
}

#[derive(Debug, Clone)]
pub struct EditorViewportGpuTexture {
    width: u32,
    height: u32,
    row_pitch: u32,
    format: wgpu::TextureFormat,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl EditorViewportGpuTexture {
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    #[must_use]
    pub const fn row_pitch(&self) -> u32 {
        self.row_pitch
    }

    #[must_use]
    pub const fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    #[must_use]
    pub const fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    #[must_use]
    pub const fn view(&self) -> &wgpu::TextureView {
        &self.view
    }
}

#[must_use]
pub const fn runtime_viewport_format_to_wgpu(
    format: ViewportPixelFormat,
) -> Option<wgpu::TextureFormat> {
    match format {
        ViewportPixelFormat::Unknown => None,
        ViewportPixelFormat::Bgra8Unorm => Some(wgpu::TextureFormat::Bgra8Unorm),
        ViewportPixelFormat::Rgba8Unorm => Some(wgpu::TextureFormat::Rgba8Unorm),
        ViewportPixelFormat::Rgba16Float => Some(wgpu::TextureFormat::Rgba16Float),
        ViewportPixelFormat::Depth32Float => Some(wgpu::TextureFormat::Depth32Float),
    }
}

/// Build the texture descriptor the editor imports a runtime viewport into.
///
/// # Errors
///
/// Returns [`EditorGpuLaunchError::InvalidViewportDimensions`] if `width` or
/// `height` is zero.
pub fn editor_viewport_texture_descriptor(
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> Result<wgpu::TextureDescriptor<'static>, EditorGpuLaunchError> {
    if width == 0 || height == 0 {
        return Err(EditorGpuLaunchError::InvalidViewportDimensions { width, height });
    }

    Ok(wgpu::TextureDescriptor {
        label: Some("az-editor-runtime-viewport"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}

/// Build the editor-side texture descriptor for a runtime viewport format.
///
/// # Errors
///
/// Returns [`EditorGpuLaunchError::UnsupportedViewportFormat`] if `format` has
/// no WGPU equivalent the editor can import, or
/// [`EditorGpuLaunchError::InvalidViewportDimensions`] if `width` or `height`
/// is zero.
pub fn runtime_viewport_texture_descriptor(
    width: u32,
    height: u32,
    format: ViewportPixelFormat,
) -> Result<wgpu::TextureDescriptor<'static>, EditorGpuLaunchError> {
    let Some(format) = runtime_viewport_format_to_wgpu(format) else {
        return Err(EditorGpuLaunchError::UnsupportedViewportFormat(format));
    };

    editor_viewport_texture_descriptor(width, height, format)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_gpu_launch_config_prefers_real_gpu_surface_interop() {
        let config = EditorGpuLaunchConfig::default();

        assert_eq!(config.backends, wgpu::Backends::PRIMARY);
        assert_eq!(
            config.power_preference,
            wgpu::PowerPreference::HighPerformance
        );
        assert_eq!(
            config.viewport_color_format,
            wgpu::TextureFormat::Bgra8Unorm
        );
        assert_eq!(config.present_mode, wgpu::PresentMode::AutoVsync);
        assert_eq!(config.instance_descriptor().backends, config.backends);
    }

    #[test]
    fn device_descriptor_requests_portable_editor_gpu_contract() {
        let config = EditorGpuLaunchConfig::default();
        let descriptor = editor_gpu_device_descriptor(config);

        assert_eq!(descriptor.label, Some("az-editor-gpu"));
        assert_eq!(descriptor.required_features, wgpu::Features::empty());
        assert_eq!(descriptor.required_limits, wgpu::Limits::defaults());
        assert!(matches!(
            descriptor.memory_hints,
            wgpu::MemoryHints::Performance
        ));
        assert!(matches!(descriptor.trace, wgpu::Trace::Off));
    }

    #[test]
    fn adapter_request_options_match_editor_gpu_launch_config() {
        let config = EditorGpuLaunchConfig::default();
        let options = editor_gpu_request_adapter_options(config);

        assert_eq!(options.power_preference, config.power_preference);
        assert!(!options.force_fallback_adapter);
        assert!(options.compatible_surface.is_none());
    }

    #[test]
    fn runtime_viewport_formats_map_to_wgpu_texture_formats() {
        assert_eq!(
            runtime_viewport_format_to_wgpu(ViewportPixelFormat::Unknown),
            None
        );
        assert_eq!(
            runtime_viewport_format_to_wgpu(ViewportPixelFormat::Bgra8Unorm),
            Some(wgpu::TextureFormat::Bgra8Unorm)
        );
        assert_eq!(
            runtime_viewport_format_to_wgpu(ViewportPixelFormat::Rgba8Unorm),
            Some(wgpu::TextureFormat::Rgba8Unorm)
        );
        assert_eq!(
            runtime_viewport_format_to_wgpu(ViewportPixelFormat::Rgba16Float),
            Some(wgpu::TextureFormat::Rgba16Float)
        );
        assert_eq!(
            runtime_viewport_format_to_wgpu(ViewportPixelFormat::Depth32Float),
            Some(wgpu::TextureFormat::Depth32Float)
        );
    }

    #[test]
    fn runtime_viewport_texture_descriptor_uses_importable_wgpu_policy() {
        let descriptor =
            runtime_viewport_texture_descriptor(1280, 720, ViewportPixelFormat::Bgra8Unorm)
                .unwrap();

        assert_eq!(descriptor.label, Some("az-editor-runtime-viewport"));
        assert_eq!(descriptor.size.width, 1280);
        assert_eq!(descriptor.size.height, 720);
        assert_eq!(descriptor.size.depth_or_array_layers, 1);
        assert_eq!(descriptor.mip_level_count, 1);
        assert_eq!(descriptor.sample_count, 1);
        assert_eq!(descriptor.dimension, wgpu::TextureDimension::D2);
        assert_eq!(descriptor.format, wgpu::TextureFormat::Bgra8Unorm);
        assert!(descriptor.usage.contains(wgpu::TextureUsages::COPY_DST));
        assert!(
            descriptor
                .usage
                .contains(wgpu::TextureUsages::TEXTURE_BINDING)
        );
        assert!(
            descriptor
                .usage
                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        );
        assert!(descriptor.view_formats.is_empty());
    }

    #[test]
    fn runtime_viewport_texture_descriptor_rejects_unknown_or_empty_frames() {
        assert!(matches!(
            runtime_viewport_texture_descriptor(1, 1, ViewportPixelFormat::Unknown).unwrap_err(),
            EditorGpuLaunchError::UnsupportedViewportFormat(ViewportPixelFormat::Unknown)
        ));
        assert!(matches!(
            runtime_viewport_texture_descriptor(0, 1, ViewportPixelFormat::Bgra8Unorm).unwrap_err(),
            EditorGpuLaunchError::InvalidViewportDimensions {
                width: 0,
                height: 1
            }
        ));
    }

    #[test]
    fn gpu_state_tracks_launch_lifecycle_without_project_runtime_types() {
        let mut state = EditorGpuState::default();
        assert!(matches!(
            state.launch_status(),
            EditorGpuLaunchStatus::NotRequested
        ));
        assert!(state.ready().is_none());

        state.mark_starting();
        assert!(matches!(
            state.launch_status(),
            EditorGpuLaunchStatus::Starting
        ));

        state.mark_failed("no adapter");
        assert!(matches!(
            state.launch_status(),
            EditorGpuLaunchStatus::Failed { diagnostic } if diagnostic == "no adapter"
        ));
    }

    #[test]
    #[ignore = "requests a real WGPU adapter/device from the local graphics driver"]
    fn editor_can_launch_real_wgpu_device() {
        let ready =
            futures::executor::block_on(launch_editor_gpu(EditorGpuLaunchConfig::default()))
                .unwrap();

        assert!(!ready.report.info.name.is_empty());
        assert!(
            ready.report.limits.max_texture_dimension_2d
                >= wgpu::Limits::defaults().max_texture_dimension_2d
        );
    }
}
