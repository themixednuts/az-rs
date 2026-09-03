//! In-process Bevy renderer for the `DirectComposition` editor viewport.
//!
//! A headless Bevy `App` (`WinitPlugin` disabled, no primary window) acquires,
//! renders, and presents its own wgpu composition swapchain. GPUI independently
//! renders the top premultiplied-alpha surface; DWM composes the sibling visuals.
//! Windows-only; always compiled into the editor.
#![cfg(target_os = "windows")]

mod camera;
mod dcomp_present;
pub(crate) mod gizmo;
mod mannequin_preview;
pub(crate) mod pick;

use camera::OrbitCameraController;
// Expansion drops `cfg(test)`-only names and adds unused ones; it does not compile.
#[allow(clippy::wildcard_imports)]
use dcomp_present::*;
// Expansion drops `cfg(test)`-only names and adds unused ones; it does not compile.
#[allow(clippy::wildcard_imports)]
use gizmo::*;
// Expansion drops `cfg(test)`-only names and adds unused ones; it does not compile.
#[allow(clippy::wildcard_imports)]
use mannequin_preview::*;
// Expansion drops `cfg(test)`-only names and adds unused ones; it does not compile.
#[allow(clippy::wildcard_imports)]
use pick::*;

pub use gizmo::MannequinPlaybackStatus;
pub use pick::EditorSceneObject;

use az_editor_ui::panels::{
    EditorBlendSpacePreview, EditorMannequinPreview, EditorViewportTelemetryData,
    ViewportCameraView, ViewportShadingMode, ViewportVisibilitySettings,
};
use bevy::animation::AnimationPlugin;
use bevy::animation::graph::{AnimationGraph, AnimationGraphHandle, AnimationNodeIndex};
use bevy::animation::{
    AnimatedBy, AnimationClip, AnimationPlayer, AnimationTargetId, RepeatAnimation,
};
use bevy::app::AppLabel;
use bevy::asset::{AssetPath, AssetPlugin, Assets, Handle, RenderAssetUsages, UnapprovedPathMode};
use bevy::camera::CameraPlugin;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::camera::{Camera, ManualTextureViewHandle, RenderTarget};
use bevy::camera::{CameraProjection, Projection};
use bevy::core_pipeline::CorePipelinePlugin;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::ecs::system::SystemState;
use bevy::gizmos::config::GizmoLineConfig;
use bevy::gizmos::retained::Gizmo;
use bevy::gizmos::{GizmoAsset, GizmoPlugin};
use bevy::gizmos_render::GizmoRenderPlugin;
use bevy::gltf::GltfAssetLabel;
use bevy::gltf::GltfPlugin;
use bevy::image::ImagePlugin;
use bevy::image::ImageSampler;
use bevy::light::LightPlugin;
use bevy::math::Affine3A;
use bevy::mesh::skinning::SkinnedMesh;
use bevy::mesh::{MeshPlugin, PrimitiveTopology, VertexAttributeValues};
use bevy::pbr::PbrPlugin;
use bevy::pbr::wireframe::{WireframeConfig, WireframePlugin};
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings, RayCastVisibility};
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::pipelined_rendering::{PipelinedRenderingPlugin, RenderExtractApp};
use bevy::render::render_resource::{
    Extent3d, Face, Texture, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
    TextureViewDescriptor,
};
use bevy::render::renderer::{
    RenderAdapter, RenderAdapterInfo, RenderDevice, RenderInstance, RenderQueue, WgpuWrapper,
};
use bevy::render::settings::{Backends, RenderCreation, RenderResources, WgpuSettings};
use bevy::render::texture::{ManualTextureView, ManualTextureViews};
use bevy::render::{Render, RenderApp, RenderPlugin, RenderSystems};
use bevy::scene::ScenePlugin;
use bevy::sprite_render::SpriteRenderPlugin;
use bevy::transform::TransformPlugin;
use bevy::transform::TransformSystems;
use bevy::window::{ExitCondition, WindowPlugin};
use bevy::world_serialization::WorldSerializationPlugin;
use bevy::world_serialization::{WorldAsset, WorldAssetRoot, WorldInstanceReady};
use std::{
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use windows::{
    Win32::{
        Graphics::{
            Direct3D12::{D3D12_FENCE_FLAG_NONE, ID3D12CommandQueue, ID3D12Device, ID3D12Fence},
            Dxgi::{
                DXGI_FRAME_STATISTICS, DXGI_FRAME_STATISTICS_MEDIA, IDXGISwapChain3,
                IDXGISwapChainMedia,
            },
        },
        System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
    },
    core::Interface,
};

const VIEWPORT_TEXTURE_VIEW: ManualTextureViewHandle = ManualTextureViewHandle(0x0AED_0001);
const VIEWPORT_FORMAT: TextureFormat = TextureFormat::Bgra8UnormSrgb;
const GIZMO_RENDER_LAYER: usize = 1;
const DIAGNOSTIC_RENDER_LAYER: usize = 31;
const GIZMO_SCREEN_PIXELS: f32 = 92.0;
const GRID_EXTENT_METERS: f32 = 20.0;
const GRID_STEP_METERS: f32 = 0.5;
/// Grid lines per axis measured out from the origin, i.e.
/// `GRID_EXTENT_METERS / GRID_STEP_METERS`. Kept as an integer so the grid
/// builder indexes lines without a float-to-integer narrowing.
const GRID_LINE_COUNT: i16 = 40;

const DIAGNOSTIC_BORDER: [u8; 4] = [255, 0, 255, 255];
const DIAGNOSTIC_CHECKER_DARK: [u8; 4] = [16, 32, 64, 255];
const DIAGNOSTIC_CHECKER_LIGHT: [u8; 4] = [48, 80, 112, 255];
const DIAGNOSTIC_BIT_ON: [u8; 4] = [255, 255, 0, 255];
const DIAGNOSTIC_BIT_OFF: [u8; 4] = [0, 255, 255, 255];

/// A viewport extent in device pixels as `f32`.
///
/// Surface and layout extents are display pixels, and `u16::MAX` (65,535) is
/// far above any dimension a swapchain or GPUI layout can report, so the
/// saturating fallback is unreachable and the widening stays lossless.
fn extent_pixels(extent: u32) -> f32 {
    f32::from(u16::try_from(extent.max(1)).unwrap_or(u16::MAX))
}

/// Round a telemetry or playhead reading into the `u32` the editor publishes.
// The clamp bounds the value to `0..=u32::MAX` before the narrowing, so neither
// the truncation nor the sign case this conversion warns about is reachable.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn clamped_u32(value: f64) -> u32 {
    value.clamp(0.0, f64::from(u32::MAX)).round() as u32
}

/// Perspective projection whose aspect is driven by the current GPUI layout,
/// not the temporarily retained physical composition surface.
#[derive(Clone, Debug)]
struct LayoutPerspectiveProjection(PerspectiveProjection);

impl LayoutPerspectiveProjection {
    fn new(width: u32, height: u32) -> Self {
        Self(PerspectiveProjection {
            aspect_ratio: extent_pixels(width) / extent_pixels(height),
            ..PerspectiveProjection::default()
        })
    }

    fn set_extent(&mut self, width: u32, height: u32) {
        self.0.aspect_ratio = extent_pixels(width) / extent_pixels(height);
    }
}

impl CameraProjection for LayoutPerspectiveProjection {
    fn get_clip_from_view(&self) -> Mat4 {
        self.0.get_clip_from_view()
    }

    fn get_clip_from_view_for_sub(&self, sub_view: &bevy::camera::SubCameraView) -> Mat4 {
        self.0.get_clip_from_view_for_sub(sub_view)
    }

    fn update(&mut self, _width: f32, _height: f32) {
        // The authoritative layout extent is applied by `set_layout_extent`.
    }

    fn far(&self) -> f32 {
        self.0.far()
    }

    fn get_frustum_corners(&self, z_near: f32, z_far: f32) -> [Vec3A; 8] {
        self.0.get_frustum_corners(z_near, z_far)
    }
}

struct DiagnosticViewportScene {
    image: Handle<Image>,
    sprite: Entity,
    frame_id: u32,
}

fn diagnostic_viewport_enabled() -> bool {
    std::env::var("AZOTH_EDITOR_VIEWPORT_DIAGNOSTIC").is_ok_and(|value| {
        matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn diagnostic_image(width: u32, height: u32, frame_id: u32) -> Image {
    let pixel_count = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .unwrap_or(4);
    let mut data = vec![0_u8; pixel_count];
    let mut canvas = DiagnosticCanvas {
        data: data.as_mut_slice(),
        width,
        height,
    };
    for y in 0..height {
        for x in 0..width {
            let color = if x == 0 || y == 0 || x + 1 == width || y + 1 == height {
                DIAGNOSTIC_BORDER
            } else if ((x / 32) + (y / 32)).is_multiple_of(2) {
                DIAGNOSTIC_CHECKER_DARK
            } else {
                DIAGNOSTIC_CHECKER_LIGHT
            };
            canvas.write_pixel(x, y, color);
        }
    }
    canvas.paint_metadata(frame_id);
    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image
}

/// The RGBA8 buffer one diagnostic frame is painted into, together with the
/// extent every write is bounds-checked against.
///
/// The buffer alone is meaningless: its row stride and row count come from
/// `width`/`height`, and every paint helper below needs all three. They are one
/// target, not three parallel arguments.
struct DiagnosticCanvas<'a> {
    data: &'a mut [u8],
    width: u32,
    height: u32,
}

impl DiagnosticCanvas<'_> {
    fn write_pixel(&mut self, x: u32, y: u32, color: [u8; 4]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let Some(index) =
            usize::try_from((u64::from(y) * u64::from(self.width) + u64::from(x)) * 4).ok()
        else {
            return;
        };
        if let Some(pixel) = self.data.get_mut(index..index.saturating_add(4)) {
            pixel.copy_from_slice(&color);
        }
    }

    fn fill_rect(
        &mut self,
        left: u32,
        top: u32,
        rect_width: u32,
        rect_height: u32,
        color: [u8; 4],
    ) {
        for y in top..top.saturating_add(rect_height).min(self.height) {
            for x in left..left.saturating_add(rect_width).min(self.width) {
                self.write_pixel(x, y, color);
            }
        }
    }

    fn paint_bits(&mut self, origin_x: u32, origin_y: u32, value: u32, bits: u32) {
        for bit in 0..bits {
            let color = if value & (1_u32 << bit) == 0 {
                DIAGNOSTIC_BIT_OFF
            } else {
                DIAGNOSTIC_BIT_ON
            };
            self.fill_rect(origin_x + bit * 4, origin_y, 3, 6, color);
        }
    }

    fn paint_metadata(&mut self, frame_id: u32) {
        const SYNC: [[u8; 4]; 4] = [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 255, 255],
        ];
        let metadata_x = self.width.saturating_sub(256) / 2;
        for (offset, color) in (0_u32..).step_by(8).zip(SYNC) {
            self.fill_rect(metadata_x + offset, 8, 6, 6, color);
        }
        let (width, height) = (self.width, self.height);
        self.paint_bits(metadata_x, 20, frame_id, 32);
        self.paint_bits(metadata_x, 30, width, 16);
        self.paint_bits(metadata_x, 40, height, 16);
    }
}

/// Failure to construct the editor's headless Bevy renderer before GPUI starts.
///
/// Keeping these cases typed is important because the D3D12 device must be
/// available before the Windows platform chooses its renderer. Collapsing a
/// missing resource to `None` loses the only useful startup diagnostic.
#[derive(Debug, thiserror::Error)]
pub enum EditorRenderInitError {
    #[error("initializing the D3D12 render resources failed: {0}")]
    GpuInitialization(String),
    #[error("Bevy RenderApp did not publish a RenderDevice after plugin initialization")]
    MissingRenderDevice,
    #[error("Bevy RenderApp did not publish a RenderQueue after plugin initialization")]
    MissingRenderQueue,
    #[error("Bevy RenderApp did not publish a RenderInstance after plugin initialization")]
    MissingRenderInstance,
    #[error("creating the DirectComposition wgpu surface failed: {0}")]
    CompositionSurface(String),
    #[error("creating the DirectComposition post-submit fence failed: {0}")]
    CompositionFence(String),
}

/// D3D12 resources created before GPUI initializes its independent native
/// D3D11 renderer. The bundle is moved exactly once into the Bevy producer.
pub struct EditorRenderBootstrap(RenderResources);

impl EditorRenderBootstrap {
    /// Create the D3D12 resources the Bevy producer later takes ownership of,
    /// before GPUI initializes its own native renderer.
    ///
    /// # Errors
    ///
    /// Returns [`EditorRenderInitError::GpuInitialization`] if no DX12 adapter
    /// is available, if the device and queue cannot be created from it, or if
    /// the initialization panics (the panic is caught and its payload becomes
    /// the error detail).
    #[tracing::instrument]
    pub fn initialize() -> Result<Self, EditorRenderInitError> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            futures::executor::block_on(async {
                let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
                descriptor.backends = wgpu::Backends::DX12;
                let instance = wgpu::Instance::new(descriptor);
                let adapter = instance
                    .request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference: wgpu::PowerPreference::HighPerformance,
                        force_fallback_adapter: false,
                        compatible_surface: None,
                    })
                    .await
                    .map_err(|error| {
                        EditorRenderInitError::GpuInitialization(format!(
                            "requesting the hardware DX12 adapter: {error}"
                        ))
                    })?;
                let adapter_info = adapter.get_info();
                let mut features = adapter.features();
                if adapter_info.device_type == wgpu::DeviceType::DiscreteGpu {
                    features.remove(wgpu::Features::MAPPABLE_PRIMARY_BUFFERS);
                }
                features |= wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
                let device_descriptor = wgpu::DeviceDescriptor {
                    label: Some("az-editor-dcomp"),
                    required_features: features,
                    required_limits: adapter.limits(),
                    // Matches Bevy's renderer initialization contract.
                    experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
                    memory_hints: wgpu::MemoryHints::Performance,
                    trace: wgpu::Trace::Off,
                };
                let (device, queue) =
                    adapter
                        .request_device(&device_descriptor)
                        .await
                        .map_err(|error| {
                            EditorRenderInitError::GpuInitialization(format!(
                                "creating the hardware DX12 device: {error}"
                            ))
                        })?;
                tracing::info!(
                    adapter = %adapter_info.name,
                    driver = %adapter_info.driver,
                    "initialized viewport D3D12 resources"
                );
                Ok(RenderResources(
                    RenderDevice::from(device),
                    RenderQueue(Arc::new(WgpuWrapper::new(queue))),
                    RenderAdapterInfo(WgpuWrapper::new(adapter_info)),
                    RenderAdapter(Arc::new(WgpuWrapper::new(adapter))),
                    RenderInstance(Arc::new(WgpuWrapper::new(instance))),
                ))
            })
        }))
        .map_err(|payload| {
            let detail = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| {
                    payload
                        .downcast_ref::<&'static str>()
                        .map(ToString::to_string)
                })
                .unwrap_or_else(|| "non-string GPU initialization panic".to_owned());
            EditorRenderInitError::GpuInitialization(detail)
        })?
        .map(Self)
    }
}

#[derive(Resource)]
struct RenderFrameStarted(std::time::Instant);

fn begin_render_frame(mut started: ResMut<RenderFrameStarted>) {
    started.0 = std::time::Instant::now();
}

// Bevy systems take `Res<T>` by value; `&Res<T>` is not a SystemParam.
#[allow(clippy::needless_pass_by_value)]
fn finish_render_frame(started: Res<RenderFrameStarted>) {
    crate::perf::record_elapsed(crate::perf::FRAME_BEVY_RENDER, started.0);
}

/// Create the render-target texture the editor camera draws into and install it
/// as the manual texture view the camera targets. Shared by initial setup and
/// resize, so the viewport always matches the panel's device-pixel size.
fn create_viewport_target(
    render_device: &RenderDevice,
    world: &mut World,
    width: u32,
    height: u32,
) -> Texture {
    let texture = render_device.create_texture(&TextureDescriptor {
        label: Some("az-editor-inprocess-viewport"),
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: VIEWPORT_FORMAT,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&TextureViewDescriptor::default());
    world.resource_mut::<ManualTextureViews>().insert(
        VIEWPORT_TEXTURE_VIEW,
        ManualTextureView {
            texture_view: view,
            size: UVec2::new(width, height),
            view_format: VIEWPORT_FORMAT,
        },
    );
    texture
}

struct PreparedViewportScene {
    asset: az_scene::AzSceneAsset,
    identities: std::collections::BTreeMap<String, crate::gpu::AuthoredPreviewEntity>,
}

/// The engine floor the viewport preview lowers with, composed.
///
/// The preview used to call `az_prefab_builder::engine_lowerings()`, which
/// handed out the engine's adapters outside composition; that floor is gone and
/// its content is in the engine's `types` and `runtime` bundles, so the preview
/// composes them (asset-contract ticket 014, D6/F5). `runtime-host` is the role
/// because that is what the viewport is — a world the editor hosts, not a
/// pipeline stage — and it is the role `runtime_host` already composes with. A
/// `project-host` composer would additionally name the `builders` bundle, which
/// the preview never asks a question of.
///
/// Composed once for the process, because a composer owns its contributions for
/// lifecycle and reload and therefore outlives the registries it lends out.
fn viewport_registries() -> &'static az_gem_contract::Registries {
    static REGISTRIES: std::sync::OnceLock<&'static az_gem_contract::Registries> =
        std::sync::OnceLock::new();
    REGISTRIES.get_or_init(|| {
        let mut composer =
            az_gem_contract::Composer::new(az_gem_contract::GemTargetRole::RuntimeHost);
        for outcome in [
            composer.floor(az_engine_types::types_contribution()),
            composer.floor(az_engine_assets::assets_contribution()),
            composer.floor(az_engine_runtime::runtime_contribution()),
        ] {
            outcome.expect("the engine floor declares no host-capability floor");
        }
        composer
            .finalize()
            .expect("the engine composition is valid");
        Box::leak(Box::new(composer)).registries()
    })
}

fn prepare_viewport_scene(
    scene: &crate::gpu::ViewportScene,
    app_registry: &bevy::ecs::reflect::AppTypeRegistry,
) -> Result<Option<PreparedViewportScene>, String> {
    let encoded_sources = {
        let registry = app_registry.read();
        crate::gpu::encode_prefab_sources(scene, &registry)?
    };
    let Some(root) = encoded_sources.first() else {
        return Ok(None);
    };
    let sources = encoded_sources
        .iter()
        .map(|source| (source.source_path.as_str(), source.source.as_str()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let processed = az_prefab_builder::process_prefab_to_azscene(
        &root.source_path,
        &root.source,
        app_registry,
        // The preview composes no gem, so this is the engine floor alone — but
        // it is composed, attributed, and collision-checked like any other.
        &az_core::component::lowering::composed(viewport_registries()),
        |path| {
            sources
                .get(path)
                .map(|source| (*source).to_owned())
                .ok_or_else(|| format!("editor preview is missing nested Prefab `{path}`"))
        },
    )
    .map_err(|error| error.to_string())?;
    let asset = {
        let registry = app_registry.read();
        az_scene::read_scene_asset_from_reader(processed.bytes.as_slice(), &registry)
            .map_err(|error| error.to_string())?
    };
    let identities = crate::gpu::authored_preview_entities(scene)?;
    if let Some(metadata) = asset
        .metadata
        .entities
        .iter()
        .find(|metadata| !identities.contains_key(&metadata.source_alias))
    {
        return Err(format!(
            "AZSCENE entity `{}` has no authored preview identity",
            metadata.source_alias
        ));
    }
    Ok(Some(PreparedViewportScene { asset, identities }))
}

/// Marker for entities spawned by the project-host → editor-world content bridge
/// (one per authored object). Tagged separately from the placeholder primitives
/// so a scene update can despawn/replace only the bridged set, leaving the ground
/// + lights intact.
#[derive(Component, Debug)]
struct BridgedAuthoredEntity;

/// Despawn the previously bridged authored entities, releasing the retained
/// gizmo assets they owned so a scene update does not leak one per object.
fn despawn_bridged_entities(world: &mut World) {
    let stale: Vec<Entity> = {
        let mut query = world.query_filtered::<Entity, With<BridgedAuthoredEntity>>();
        query.iter(world).collect()
    };
    let stale_gizmo_handles = stale
        .iter()
        .filter_map(|entity| {
            world
                .get::<Gizmo>(*entity)
                .map(|gizmo| gizmo.handle.clone())
        })
        .collect::<Vec<_>>();
    for handle in stale_gizmo_handles {
        world
            .resource_mut::<Assets<GizmoAsset>>()
            .remove(handle.id());
    }
    for entity in stale {
        world.despawn(entity);
    }
}

/// Overlay editor-only decorations on the freshly materialized AZSCENE
/// entities: authored identity for picking, the layer-visibility filter, a
/// capability-driven gizmo request, and a placeholder mesh for empty entities.
fn decorate_bridged_entities(
    world: &mut World,
    app_registry: &bevy::ecs::reflect::AppTypeRegistry,
    prepared: &PreparedViewportScene,
    materialized: &[Entity],
    hidden: &std::collections::BTreeSet<String>,
    mesh: &Handle<Mesh>,
    material: &Handle<StandardMaterial>,
) {
    let registry = app_registry.read();
    for (entity, metadata) in materialized
        .iter()
        .copied()
        .zip(&prepared.asset.metadata.entities)
    {
        let identity = prepared
            .identities
            .get(&metadata.source_alias)
            .expect("prepared preview validated every AZSCENE source alias");
        debug_assert_eq!(identity.source_alias, metadata.source_alias);
        let id = format!("{}/{}", identity.document_id, identity.object_id);
        world.entity_mut(entity).insert((
            Name::new(identity.object_id.clone()),
            EditorSceneObject {
                id: id.clone(),
                document_id: Some(identity.document_id.clone()),
                object_id: Some(identity.object_id.clone()),
            },
            BridgedAuthoredEntity,
        ));
        let wants_component_gizmo = identity.component_type_paths.iter().any(|type_path| {
            registry
                .get_with_type_path(type_path)
                .and_then(|registration| registration.data::<az_core::ApplicabilityTypeData>())
                .is_some_and(|applicability| {
                    applicability
                        .provides
                        .iter()
                        .any(|capability| matches!(*capability, "azoth.camera" | "azoth.light"))
                })
        });
        if wants_component_gizmo {
            world.entity_mut(entity).insert(EditorComponentGizmoRequest);
        }
        if hidden.contains(&identity.document_id) || hidden.contains(&id) {
            world.entity_mut(entity).insert(Visibility::Hidden);
        }
        if world.get::<az_render::Mesh>(entity).is_none() {
            world.entity_mut(entity).insert((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                EditorPreviewPlaceholder::EmptyEntity,
            ));
        }
    }
}

/// Semantic viewport colors resolved by GPUI and applied on the Bevy thread.
/// Render code never invents UI colors.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportRenderTheme {
    pub background: [f32; 4],
    pub skybox: [f32; 4],
    pub grid_minor: [f32; 4],
    pub grid_major: [f32; 4],
}

/// The editor's in-process Bevy render app and composition surface owner.
///
/// Constructed and confined to the dedicated viewport-production thread. GPUI
/// receives only status/completion messages; no render texture crosses into its
/// D3D11 renderer. This parent module deliberately remains the orchestration
/// point; `DComp` presentation, picking, gizmos, mannequin preview, and camera
/// control live in sibling subsystem modules.
pub struct EditorRenderApp {
    app: App,
    /// Offscreen target used by headless constructors and renderer tests.
    texture: Texture,
    width: u32,
    height: u32,
    /// Current GPUI layout extent. It drives camera projection immediately and
    /// may differ from the bounded physical surface extent above.
    layout_width: u32,
    layout_height: u32,
    /// bevy render device, kept so the render-target texture can be recreated on
    /// viewport resize without re-fetching it from the render sub-app each time.
    render_device: RenderDevice,
    /// Presentable child-visual surface. Headless constructors leave this absent.
    composition_surface: Option<CompositionSurfaceOwner>,
    /// Cached mesh/material reused for every content-bridge entity, so a scene
    /// update does not leak a fresh asset per authored object.
    bridge_mesh: Handle<Mesh>,
    bridge_material: Handle<StandardMaterial>,
    /// Line-list bounds and unlit accent material used for selection.
    selection_bounds_mesh: Handle<Mesh>,
    selection_bounds_material: Handle<StandardMaterial>,
    selection_bounds_rgba: [f32; 4],
    /// Dim emissive back-face shell, visually subordinate to selection.
    hover_outline_material: Handle<StandardMaterial>,
    /// Translucent material used by the instant asset-placement preview.
    asset_ghost_material: Handle<StandardMaterial>,
    /// Drag preview currently following the cursor.
    active_asset_ghost: Option<AssetGhost>,
    /// Solid optimistic placements waiting for authored-scene reconciliation.
    optimistic_assets: Vec<AssetGhost>,
    /// Authored `(document_id, object_id)` whose bridged entity is highlighted.
    selected_authored: Option<(String, String)>,
    hovered_entity: Option<Entity>,
    hover_shell: Option<Entity>,
    /// Cached ray-cast state retains Bevy's local hit buffers between probes.
    mesh_ray_cast: SystemState<MeshRayCast<'static, 'static>>,
    /// Gizmo and outline-shell meshes excluded from scene hover/click picks.
    pick_exclusions: std::collections::HashSet<Entity>,
    camera: OrbitCameraController,
    mannequin_preview: EditorMannequinPreview,
    blend_space_preview: EditorBlendSpacePreview,
    mannequin_key: Option<MannequinPreviewKey>,
    /// Active gizmo mode (Select detaches the gizmo), transform space, and snap
    /// settings forwarded from `EditorSceneToolState` each frame.
    gizmo_mode: GizmoMode,
    gizmo_pivot: GizmoPivot,
    gizmo_space: GizmoSpace,
    gizmo_snap: GizmoSnap,
    /// Geometry currently spawned; re-spawned only when the mode changes.
    gizmo_spawned_mode: GizmoMode,
    /// In-flight axis drag, if any.
    gizmo_drag: Option<GizmoDrag>,
    /// Shared gizmo mesh handles (oriented per axis via child transforms).
    gizmo_shaft_mesh: Handle<Mesh>,
    gizmo_translate_tip_mesh: Handle<Mesh>,
    gizmo_scale_tip_mesh: Handle<Mesh>,
    gizmo_ring_mesh: Handle<Mesh>,
    /// Per-axis unlit materials (X/Y/Z), plus a highlight for the grabbed axis.
    gizmo_axis_materials: [Handle<StandardMaterial>; 3],
    gizmo_highlight_material: Handle<StandardMaterial>,
    grid_minor_material: Handle<StandardMaterial>,
    grid_major_material: Handle<StandardMaterial>,
    shading_mode: ViewportShadingMode,
    visibility: ViewportVisibilitySettings,
    render_theme: Option<ViewportRenderTheme>,
    diagnostic: Option<DiagnosticViewportScene>,
    next_pointer_sample: Option<PointerPresentSample>,
}

/// Retained materials for the two ground-grid draw submissions.
struct GroundGridMaterials {
    minor: Handle<StandardMaterial>,
    major: Handle<StandardMaterial>,
}

/// Retained meshes and materials the scene bridge, selection bounds, hover rim,
/// and asset-drop preview share across every authored object.
struct ViewportChromeAssets {
    bridge_mesh: Handle<Mesh>,
    bridge_material: Handle<StandardMaterial>,
    selection_bounds_mesh: Handle<Mesh>,
    selection_bounds_material: Handle<StandardMaterial>,
    hover_outline_material: Handle<StandardMaterial>,
    asset_ghost_material: Handle<StandardMaterial>,
}

/// Shared transform-handle meshes plus the per-axis and grabbed-axis materials.
struct TransformGizmoAssets {
    shaft_mesh: Handle<Mesh>,
    translate_tip_mesh: Handle<Mesh>,
    scale_tip_mesh: Handle<Mesh>,
    ring_mesh: Handle<Mesh>,
    axis_materials: [Handle<StandardMaterial>; 3],
    highlight_material: Handle<StandardMaterial>,
}

/// Asset source policy for the preview world: restrictive by default, rooted at
/// the project's processed products when the caller knows them.
fn viewport_asset_plugin(
    product_root: Option<PathBuf>,
    mannequin_preview: &EditorMannequinPreview,
) -> AssetPlugin {
    let mut asset_plugin = AssetPlugin {
        // Project preview assets live outside the editor crate's default
        // asset folder. Keep global policy restrictive and require the
        // mannequin bridge's explicit load override for direct paths.
        unapproved_path_mode: UnapprovedPathMode::Deny,
        // Cooked products are replaced by the live asset processor. Bevy
        // must observe those writes so renderer-owned asset handles update
        // without rebuilding authored entities.
        watch_for_changes_override: Some(true),
        ..default()
    };
    if let Some(product_root) = product_root {
        asset_plugin.file_path = product_root.to_string_lossy().into_owned();
    } else if let Some(project_asset_root) = mannequin_preview.project_asset_root.as_ref() {
        asset_plugin.file_path = project_asset_root.to_string_lossy().into_owned();
    }
    asset_plugin
}

/// Adopt the preinitialized D3D12 resources when the caller has them; otherwise
/// let Bevy create its own DX12 device.
fn viewport_render_creation(bootstrap: Option<EditorRenderBootstrap>) -> RenderCreation {
    bootstrap.map_or_else(
        || {
            RenderCreation::Automatic(Box::new(WgpuSettings {
                backends: Some(Backends::DX12),
                ..default()
            }))
        },
        |bootstrap| RenderCreation::Manual(bootstrap.0),
    )
}

/// The headless Bevy app the editor viewport renders through, with every plugin
/// and preview-facing system installed but not yet finished.
fn build_viewport_app(
    asset_plugin: AssetPlugin,
    render_creation: RenderCreation,
    diagnostic_enabled: bool,
) -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        TransformPlugin,
        WindowPlugin {
            primary_window: None,
            exit_condition: ExitCondition::DontExit,
            ..default()
        },
        asset_plugin,
        WorldSerializationPlugin,
        ScenePlugin,
        RenderPlugin {
            render_creation,
            ..default()
        },
        ImagePlugin::default(),
        MeshPlugin,
        CameraPlugin,
        LightPlugin,
        CorePipelinePlugin,
        GltfPlugin::default(),
        PbrPlugin::default(),
        AnimationPlugin,
    ));
    if diagnostic_enabled {
        // The diagnostic needs sprite extraction/rendering only. Installing
        // `SpritePlugin` would also install the optional picking backend,
        // which is intentionally absent from this headless producer.
        app.add_plugins(SpriteRenderPlugin);
    }
    app.add_plugins(WireframePlugin::default());
    app.add_plugins(FrameTimeDiagnosticsPlugin::new(32));
    app.add_plugins(az_render::AzothRenderPlugin);
    {
        let registry = app
            .world()
            .resource::<bevy::ecs::reflect::AppTypeRegistry>()
            .clone();
        let mut registry = registry.write();
        az_transform::register_transform_prefab_types(&mut registry);
        az_render::register_render_prefab_types(&mut registry);
        az_framework::register_framework_prefab_types(&mut registry);
    }
    app.add_plugins((GizmoPlugin, GizmoRenderPlugin));
    app.add_plugins(PipelinedRenderingPlugin);
    app.add_plugins(ExtractResourcePlugin::<PendingCompositionFrame>::default());
    app.add_systems(
        PostUpdate,
        (
            fit_selection_bounds_to_real_aabb,
            frame_pending_mannequin_camera.after(TransformSystems::Propagate),
        ),
    );
    app
}

/// Finish plugin initialization and take the renderer handles the producer owns
/// for the rest of the app's life.
fn open_render_resources(
    app: &mut App,
) -> Result<(RenderDevice, RenderQueue, RenderInstance), EditorRenderInitError> {
    // Force renderer init so the wgpu device exists, then build our texture.
    // Cleanup is deferred until the main-world camera and assets are ready;
    // pipelined cleanup moves the render sub-app to its worker thread.
    app.finish();
    let render_world = app.sub_app(RenderApp).world();
    let render_device = render_world
        .get_resource::<RenderDevice>()
        .ok_or(EditorRenderInitError::MissingRenderDevice)?
        .clone();
    let render_queue = render_world
        .get_resource::<RenderQueue>()
        .ok_or(EditorRenderInitError::MissingRenderQueue)?
        .clone();
    let render_instance = render_world
        .get_resource::<RenderInstance>()
        .ok_or(EditorRenderInitError::MissingRenderInstance)?
        .clone();
    Ok((render_device, render_queue, render_instance))
}

/// Install the post-submit fence and the pending-frame slot the composition
/// present path needs.
fn attach_composition_present(
    app: &mut App,
    render_device: &RenderDevice,
    render_queue: &RenderQueue,
) -> Result<(), EditorRenderInitError> {
    let present_fence = DcompPresentFence::new(render_device, render_queue)
        .map_err(EditorRenderInitError::CompositionFence)?;
    app.sub_app_mut(RenderApp).insert_resource(present_fence);
    app.world_mut()
        .insert_resource(PendingCompositionFrame(None));
    Ok(())
}

/// Bracket the render schedule with the producer's frame timing and place the
/// present after every command buffer that can reference the acquired texture.
fn install_render_frame_systems(app: &mut App) {
    app.sub_app_mut(RenderApp)
        .insert_resource(RenderFrameStarted(std::time::Instant::now()))
        .add_systems(
            Render,
            (
                begin_render_frame.before(RenderSystems::ExtractCommands),
                present_composition_frame
                    .in_set(RenderSystems::PostCleanup)
                    .after(RenderSystems::Render),
                finish_render_frame
                    .after(present_composition_frame)
                    .after(RenderSystems::PostCleanup),
            ),
        );
}

/// Spawn the opt-in diagnostic overlay: a checkerboard sprite carrying the
/// frame id and extent as readable bit patterns.
fn spawn_diagnostic_scene(app: &mut App, width: u32, height: u32) -> DiagnosticViewportScene {
    let image = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(diagnostic_image(width, height, 0));
    app.world_mut().spawn((
        Camera2d,
        Camera {
            order: 100,
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        RenderTarget::TextureView(VIEWPORT_TEXTURE_VIEW),
        RenderLayers::layer(DIAGNOSTIC_RENDER_LAYER),
        Msaa::Off,
    ));
    let mut sprite_component = Sprite::from_image(image.clone());
    sprite_component.custom_size = Some(Vec2::new(extent_pixels(width), extent_pixels(height)));
    let sprite = app
        .world_mut()
        .spawn((
            sprite_component,
            Transform::default(),
            NoFrustumCulling,
            RenderLayers::layer(DIAGNOSTIC_RENDER_LAYER),
        ))
        .id();
    DiagnosticViewportScene {
        image,
        sprite,
        frame_id: 0,
    }
}

/// Spawn the single editor camera into the manual texture view.
///
/// Gizmos share this camera on layer 1: a second `Camera3d` would execute a
/// complete PBR render graph every frame even while no gizmo is present.
fn spawn_viewport_camera(world: &mut World, width: u32, height: u32, position: Vec3, focus: Vec3) {
    world.spawn((
        Camera3d::default(),
        Projection::custom(LayoutPerspectiveProjection::new(width, height)),
        Msaa::Off,
        // The manual target is already the compositor's LDR BGRA format.
        // Avoid a full-target HDR tonemap/deband pass at 5K editor sizes.
        Tonemapping::None,
        DebandDither::Disabled,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::srgb(0.10, 0.12, 0.18)),
            ..default()
        },
        // bevy 0.19: the render target is its own component, not a Camera field.
        RenderTarget::TextureView(VIEWPORT_TEXTURE_VIEW),
        Transform::from_translation(position).looking_at(focus, Vec3::Y),
        // bevy 0.19: ambient light is a per-camera component (overrides the
        // global default) rather than a scene-wide resource.
        AmbientLight {
            color: Color::srgb(0.62, 0.68, 0.78),
            brightness: 220.0,
            ..default()
        },
        RenderLayers::layer(0).with(GIZMO_RENDER_LAYER),
    ));
}

/// Spawn the persistent ground grid as two merged draw submissions and return
/// the materials the theme recolors.
fn spawn_ground_grid(app: &mut App) -> GroundGridMaterials {
    let minor_mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(ground_grid_mesh(false));
    let major_mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(ground_grid_mesh(true));
    // Minor and major lines carry separate semantic theme tones, so each needs
    // its own material even though the two descriptors are identical here.
    let minor = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::NONE,
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            ..default()
        });
    let major = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::NONE,
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            ..default()
        });
    app.world_mut().spawn((
        Mesh3d(minor_mesh),
        MeshMaterial3d(minor.clone()),
        GroundGrid,
    ));
    app.world_mut().spawn((
        Mesh3d(major_mesh),
        MeshMaterial3d(major.clone()),
        GroundGrid,
    ));
    GroundGridMaterials { minor, major }
}

/// Create the retained bridge, selection, hover, and asset-ghost assets so a
/// scene update never allocates a fresh mesh or material per authored object.
fn create_chrome_assets(app: &mut App) -> ViewportChromeAssets {
    let bridge_mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(1.0, 1.0, 1.0));
    let bridge_material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgb(0.55, 0.58, 0.66),
            perceptual_roughness: 0.7,
            ..default()
        });
    let bounds_mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(selection_bounds_mesh());
    let bounds_material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            unlit: true,
            ..default()
        });
    let hover_outline_material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgba(0.22, 0.70, 0.92, 0.52),
            emissive: bevy::color::LinearRgba::rgb(0.08, 0.38, 0.62),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            cull_mode: Some(Face::Front),
            ..default()
        });
    let asset_ghost_material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgba(0.24, 0.72, 1.0, 0.42),
            emissive: bevy::color::LinearRgba::rgb(0.05, 0.28, 0.52),
            perceptual_roughness: 0.35,
            alpha_mode: AlphaMode::Blend,
            ..default()
        });
    ViewportChromeAssets {
        bridge_mesh,
        bridge_material,
        selection_bounds_mesh: bounds_mesh,
        selection_bounds_material: bounds_material,
        hover_outline_material,
        asset_ghost_material,
    }
}

/// Create the shared gizmo geometry: one shaft (long along local X), a
/// translation cone, a scale cube, and a rotation ring (torus around local Y).
/// Each is oriented per axis via child transforms, and every material is unlit
/// so the gizmo reads clearly regardless of scene lighting.
fn create_gizmo_assets(app: &mut App) -> TransformGizmoAssets {
    let shaft_mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(
            GIZMO_SHAFT_LEN,
            GIZMO_SHAFT_THICK,
            GIZMO_SHAFT_THICK,
        ));
    let translate_tip_mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(Cone {
        radius: GIZMO_TIP * 0.72,
        height: GIZMO_ARROW_HEIGHT,
    });
    let scale_tip_mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(GIZMO_TIP, GIZMO_TIP, GIZMO_TIP));
    let ring_mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(Torus {
        minor_radius: GIZMO_RING_MINOR,
        major_radius: GIZMO_RING_MAJOR,
    });
    let axis_materials = GizmoAxis::ALL.map(|axis| {
        app.world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color: axis.color(),
                unlit: true,
                ..default()
            })
    });
    let highlight_material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.92, 0.35),
            unlit: true,
            ..default()
        });
    TransformGizmoAssets {
        shaft_mesh,
        translate_tip_mesh,
        scale_tip_mesh,
        ring_mesh,
        axis_materials,
        highlight_material,
    }
}

#[derive(Debug)]
struct AssetGhost {
    interaction_id: u64,
    entity: Entity,
    position: Vec3,
    authored_object_id: Option<String>,
}

const fn rgba_color(rgba: [f32; 4]) -> Color {
    Color::srgba(rgba[0], rgba[1], rgba[2], rgba[3])
}

fn sync_camera_clear_color(
    world: &mut World,
    visibility: ViewportVisibilitySettings,
    theme: Option<ViewportRenderTheme>,
) {
    let Some(theme) = theme else {
        return;
    };
    let color = if visibility.skybox {
        theme.skybox
    } else {
        theme.background
    };
    let mut cameras = world.query_filtered::<&mut Camera, With<Camera3d>>();
    for mut camera in cameras.iter_mut(world) {
        camera.clear_color = ClearColorConfig::Custom(rgba_color(color));
    }
}

/// Robust central sample for UI telemetry. Capturing a native window can
/// briefly stall the producer thread; a bounded median keeps that one stall
/// from misrepresenting the otherwise real Bevy diagnostic history.
fn diagnostic_median(diagnostic: &bevy::diagnostic::Diagnostic) -> Option<f64> {
    let mut values = [0.0_f64; 32];
    let mut len = 0;
    for value in diagnostic.values().take(values.len()) {
        values[len] = *value;
        len += 1;
    }
    if len == 0 {
        return None;
    }
    values[..len].sort_unstable_by(f64::total_cmp);
    Some(if len % 2 == 0 {
        (values[len / 2 - 1] + values[len / 2]) * 0.5
    } else {
        values[len / 2]
    })
}

/// World scale for a one-unit handle to occupy `desired_pixels` vertically.
/// This is self-contained so the hand-rolled gizmo migration can remove it in
/// one change when Bevy's `TransformGizmoPlugin` becomes authoritative.
fn constant_screen_scale(
    projection: &Projection,
    camera_position: Vec3,
    origin: Vec3,
    viewport_height: u32,
    desired_pixels: f32,
) -> f32 {
    let height = extent_pixels(viewport_height);
    let world_per_pixel = match projection {
        Projection::Perspective(perspective) => {
            let distance = camera_position
                .distance(origin)
                .max(perspective.near.max(0.001));
            2.0 * distance * (perspective.fov * 0.5).tan() / height
        }
        Projection::Orthographic(orthographic) => orthographic.area.height().abs() / height,
        Projection::Custom(custom) => {
            let perspective = custom
                .get::<LayoutPerspectiveProjection>()
                .map_or_else(PerspectiveProjection::default, |layout| layout.0.clone());
            let distance = camera_position
                .distance(origin)
                .max(perspective.near.max(0.001));
            2.0 * distance * (perspective.fov * 0.5).tan() / height
        }
    };
    (world_per_pixel * desired_pixels).clamp(0.01, 10_000.0)
}

impl EditorRenderApp {
    /// Build a headless render app of the given size.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::new_with_product_root`] returns.
    pub fn new(width: u32, height: u32) -> Result<Self, EditorRenderInitError> {
        Self::new_with_product_root(width, height, None)
    }

    /// Build a headless render app whose default asset source is the project's
    /// processed product cache.
    ///
    /// # Errors
    ///
    /// Returns [`EditorRenderInitError::MissingRenderDevice`],
    /// [`EditorRenderInitError::MissingRenderQueue`] or
    /// [`EditorRenderInitError::MissingRenderInstance`] if Bevy's `RenderApp`
    /// does not publish the corresponding resource after plugin
    /// initialization.
    pub fn new_with_product_root(
        width: u32,
        height: u32,
        product_root: Option<PathBuf>,
    ) -> Result<Self, EditorRenderInitError> {
        Self::new_with_previews_and_product_root(
            width,
            height,
            EditorMannequinPreview::empty(),
            EditorBlendSpacePreview::empty(),
            product_root,
            None,
            None,
        )
    }

    /// Build against the renderer-owned `DirectComposition` child visual. This
    /// is called only after GPUI has created the window composition tree.
    ///
    /// # Errors
    ///
    /// Returns [`EditorRenderInitError::MissingRenderDevice`],
    /// [`EditorRenderInitError::MissingRenderQueue`] or
    /// [`EditorRenderInitError::MissingRenderInstance`] if Bevy's `RenderApp`
    /// does not publish the corresponding resource,
    /// [`EditorRenderInitError::CompositionSurface`] if the wgpu surface
    /// cannot be created against `target`, or
    /// [`EditorRenderInitError::CompositionFence`] if its post-submit fence
    /// cannot be created.
    pub fn new_dcomp_with_product_root(
        width: u32,
        height: u32,
        product_root: Option<PathBuf>,
        target: gpui_windows::ViewportVisualSurfaceTarget,
        bootstrap: EditorRenderBootstrap,
    ) -> Result<Self, EditorRenderInitError> {
        Self::new_with_previews_and_product_root(
            width,
            height,
            EditorMannequinPreview::empty(),
            EditorBlendSpacePreview::empty(),
            product_root,
            Some(target),
            Some(bootstrap),
        )
    }

    /// Build a headless render app with an initial mannequin preview selection.
    /// `EditorMannequinPreview::empty()` keeps the neutral primitives visible.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::new_with_product_root`] returns.
    pub fn new_with_mannequin_preview(
        width: u32,
        height: u32,
        mannequin_preview: EditorMannequinPreview,
    ) -> Result<Self, EditorRenderInitError> {
        Self::new_with_previews_and_product_root(
            width,
            height,
            mannequin_preview,
            EditorBlendSpacePreview::empty(),
            None,
            None,
            None,
        )
    }

    /// Build a headless render app with initial mannequin and blend-space
    /// preview selections.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::new_with_product_root`] returns.
    pub fn new_with_previews(
        width: u32,
        height: u32,
        mannequin_preview: EditorMannequinPreview,
        blend_space_preview: EditorBlendSpacePreview,
    ) -> Result<Self, EditorRenderInitError> {
        Self::new_with_previews_and_product_root(
            width,
            height,
            mannequin_preview,
            blend_space_preview,
            None,
            None,
            None,
        )
    }

    fn new_with_previews_and_product_root(
        width: u32,
        height: u32,
        mannequin_preview: EditorMannequinPreview,
        blend_space_preview: EditorBlendSpacePreview,
        product_root: Option<PathBuf>,
        composition_target: Option<gpui_windows::ViewportVisualSurfaceTarget>,
        render_bootstrap: Option<EditorRenderBootstrap>,
    ) -> Result<Self, EditorRenderInitError> {
        let width = width.max(1);
        let height = height.max(1);
        let diagnostic_enabled = diagnostic_viewport_enabled();
        let mut app = build_viewport_app(
            viewport_asset_plugin(product_root, &mannequin_preview),
            viewport_render_creation(render_bootstrap),
            diagnostic_enabled,
        );

        let (render_device, render_queue, render_instance) = open_render_resources(&mut app)?;
        let composition_surface = composition_target
            .map(|target| {
                CompositionSurfaceOwner::new(render_instance, &render_device, target, width, height)
            })
            .transpose()?;
        if composition_surface.is_some() {
            attach_composition_present(&mut app, &render_device, &render_queue)?;
        }
        install_render_frame_systems(&mut app);

        let texture = create_viewport_target(&render_device, app.world_mut(), width, height);
        let diagnostic =
            diagnostic_enabled.then(|| spawn_diagnostic_scene(&mut app, width, height));

        let camera_position = Vec3::new(6.0, 5.0, 11.0);
        let camera_focus = Vec3::new(0.0, 0.75, 0.0);
        spawn_viewport_camera(
            app.world_mut(),
            width,
            height,
            camera_position,
            camera_focus,
        );
        spawn_editor_scene(app.world_mut());
        let grid = spawn_ground_grid(&mut app);
        let chrome = create_chrome_assets(&mut app);
        let gizmo = create_gizmo_assets(&mut app);

        app.cleanup();
        let mesh_ray_cast = SystemState::<MeshRayCast>::new(app.world_mut());
        let mut renderer = Self {
            app,
            texture,
            width,
            height,
            layout_width: width,
            layout_height: height,
            render_device,
            composition_surface,
            bridge_mesh: chrome.bridge_mesh,
            bridge_material: chrome.bridge_material,
            selection_bounds_mesh: chrome.selection_bounds_mesh,
            selection_bounds_material: chrome.selection_bounds_material,
            selection_bounds_rgba: [0.0; 4],
            hover_outline_material: chrome.hover_outline_material,
            asset_ghost_material: chrome.asset_ghost_material,
            active_asset_ghost: None,
            optimistic_assets: Vec::new(),
            selected_authored: None,
            hovered_entity: None,
            hover_shell: None,
            mesh_ray_cast,
            pick_exclusions: std::collections::HashSet::with_capacity(32),
            camera: OrbitCameraController::from_pose(camera_position, camera_focus),
            mannequin_preview: EditorMannequinPreview::empty(),
            blend_space_preview: EditorBlendSpacePreview::empty(),
            mannequin_key: None,
            gizmo_mode: GizmoMode::None,
            gizmo_pivot: GizmoPivot::Pivot,
            gizmo_space: GizmoSpace::World,
            gizmo_snap: GizmoSnap::NONE,
            gizmo_spawned_mode: GizmoMode::None,
            gizmo_drag: None,
            gizmo_shaft_mesh: gizmo.shaft_mesh,
            gizmo_translate_tip_mesh: gizmo.translate_tip_mesh,
            gizmo_scale_tip_mesh: gizmo.scale_tip_mesh,
            gizmo_ring_mesh: gizmo.ring_mesh,
            gizmo_axis_materials: gizmo.axis_materials,
            gizmo_highlight_material: gizmo.highlight_material,
            grid_minor_material: grid.minor,
            grid_major_material: grid.major,
            shading_mode: ViewportShadingMode::Lit,
            visibility: ViewportVisibilitySettings::default(),
            render_theme: None,
            diagnostic,
            next_pointer_sample: None,
        };
        renderer.apply_previews(mannequin_preview, blend_space_preview);
        Ok(renderer)
    }

    /// Replace the physical render target after the bounded resize policy proves
    /// no composition surface texture is acquired.
    pub fn resize(&mut self, width: u32, height: u32) -> bool {
        let width = width.max(1);
        let height = height.max(1);
        if width == self.width && height == self.height {
            return false;
        }
        let started = std::time::Instant::now();
        let composition_configured = if let Some(surface) = &mut self.composition_surface {
            surface.configure(&self.render_device, width, height)
        } else {
            self.texture =
                create_viewport_target(&self.render_device, self.app.world_mut(), width, height);
            true
        };
        if !composition_configured {
            return false;
        }
        self.width = width;
        self.height = height;
        if let Some(diagnostic) = &mut self.diagnostic {
            diagnostic.frame_id = diagnostic.frame_id.wrapping_add(1);
            if let Some(mut image) = self
                .app
                .world_mut()
                .resource_mut::<Assets<Image>>()
                .get_mut(&diagnostic.image)
            {
                *image = diagnostic_image(width, height, diagnostic.frame_id);
            }
            if let Some(mut sprite) = self.app.world_mut().get_mut::<Sprite>(diagnostic.sprite) {
                sprite.custom_size = Some(Vec2::new(extent_pixels(width), extent_pixels(height)));
            }
        }
        crate::perf::record_elapsed(crate::perf::FRAME_BEVY_RESIZE, started);
        true
    }

    /// Apply the latest logical/device layout immediately, independently of a
    /// temporarily retained physical composition surface.
    pub fn set_layout_extent(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if self.layout_width == width && self.layout_height == height {
            return;
        }
        self.layout_width = width;
        self.layout_height = height;
        let world = self.app.world_mut();
        let mut projections = world.query_filtered::<&mut Projection, With<Camera3d>>();
        for mut projection in projections.iter_mut(world) {
            if let Projection::Custom(custom) = &mut *projection
                && let Some(layout) = custom.get_mut::<LayoutPerspectiveProjection>()
            {
                layout.set_extent(width, height);
            }
        }
    }

    #[must_use]
    pub const fn layout_size(&self) -> (u32, u32) {
        (self.layout_width, self.layout_height)
    }

    #[must_use]
    pub fn composition_surface_idle(&self) -> bool {
        self.composition_surface
            .as_ref()
            .is_none_or(CompositionSurfaceOwner::is_idle)
    }

    /// Wait for the producer's previously acquired image to reach a terminal
    /// present/discard state. Resize replacement calls this only after the
    /// bounded policy says a replacement is due, immediately before creating
    /// the next composition surface.
    pub fn wait_for_composition_surface_idle(&self) -> bool {
        self.composition_surface
            .as_ref()
            .is_none_or(CompositionSurfaceOwner::wait_for_previous)
    }

    pub fn set_pointer_present_sample(
        &mut self,
        interaction_id: u64,
        sampled_at: std::time::Instant,
    ) {
        let sample = PointerPresentSample {
            interaction_id,
            sampled_at,
        };
        if let Some(frame) = self
            .composition_surface
            .as_ref()
            .and_then(|surface| surface.previous_frame.as_ref())
            .filter(|frame| !frame.is_finished())
        {
            *frame
                .pointer_sample
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(sample);
        } else {
            self.next_pointer_sample = Some(sample);
        }
    }

    /// Acquire exactly one child-visual surface texture and install its view
    /// before Bevy extraction. The surface's DX12 latency object performs the
    /// blocking pace; the producer never observes GPUI's swapchain handle here.
    pub fn acquire_composition_frame(&mut self) -> bool {
        let Some(surface) = &mut self.composition_surface else {
            return true;
        };
        surface.retire_committed_surfaces();
        // wgpu permits only one outstanding image per Surface.  The render app
        // is pipelined, so `render_current_world` returns as soon as extraction
        // hands the prior frame to the worker; wait for that worker's terminal
        // present/discard before asking DXGI for the next image.  Once retired,
        // `get_current_texture` below performs the actual swapchain pacing via
        // this composition surface's frame-latency waitable object.
        if !surface.wait_for_previous() {
            tracing::error!(
                "timed out waiting for the prior DirectComposition frame before acquisition"
            );
            return false;
        }
        if surface.needs_reconfigure {
            if !surface.configure(&self.render_device, self.width, self.height) {
                return false;
            }
            surface.needs_reconfigure = false;
        }

        let (surface_texture, suboptimal) = match surface.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => (texture, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => (texture, true),
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return false;
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                surface.needs_reconfigure = true;
                return false;
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                if let Err(error) = surface.recreate(&self.render_device) {
                    tracing::error!(%error, "lost DirectComposition surface could not be recreated");
                }
                return false;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                tracing::error!("wgpu validation rejected DirectComposition surface acquisition");
                return false;
            }
        };
        surface.needs_reconfigure |= suboptimal;

        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let frame = Arc::new(AcquiredCompositionFrame::new(
            surface_texture,
            (self.width, self.height),
            surface.target.clone(),
            surface.display_telemetry.clone(),
            self.next_pointer_sample.take(),
        ));
        self.app
            .world_mut()
            .resource_mut::<ManualTextureViews>()
            .insert(
                VIEWPORT_TEXTURE_VIEW,
                ManualTextureView {
                    texture_view: view.into(),
                    size: UVec2::new(self.width, self.height),
                    view_format: VIEWPORT_FORMAT,
                },
            );
        self.app
            .world_mut()
            .insert_resource(PendingCompositionFrame(Some(frame.clone())));
        surface.previous_frame = Some(frame);
        gpui_windows::record_dcomp_texture_acquired((self.width, self.height));
        true
    }

    /// Apply the editor-published mannequin preview selection. A selected
    /// character replaces the neutral viewport primitives with the authored glTF
    /// scene and optional glTF animation; explicit empty selection restores the
    /// neutral viewport.
    pub fn apply_mannequin_preview(&mut self, preview: EditorMannequinPreview) {
        self.apply_previews(preview, self.blend_space_preview.clone());
    }

    /// Apply the editor-published blend-space preview state. Parameter-only
    /// changes update graph node weights in-place.
    pub fn apply_blend_space_preview(&mut self, preview: EditorBlendSpacePreview) {
        self.apply_previews(self.mannequin_preview.clone(), preview);
    }

    fn apply_previews(
        &mut self,
        preview: EditorMannequinPreview,
        blend_space_preview: EditorBlendSpacePreview,
    ) {
        let next_key = MannequinPreviewKey::from_previews(&preview, &blend_space_preview);
        if next_key.is_none() {
            let world = self.app.world_mut();
            clear_mannequin_preview(world);
            ensure_neutral_primitives(world);
            self.mannequin_key = None;
            self.mannequin_preview = preview;
            self.blend_space_preview = blend_space_preview;
            return;
        }

        if self.mannequin_key.as_ref() == next_key.as_ref() {
            if self.mannequin_preview.playing != preview.playing
                || self.mannequin_preview.looping != preview.looping
                || self.mannequin_preview.position_millis != preview.position_millis
            {
                apply_mannequin_playback_state(self.app.world_mut(), &preview);
            }
            if self.blend_space_preview.param_values != blend_space_preview.param_values {
                apply_mannequin_blend_space_weights(self.app.world_mut(), &blend_space_preview);
            }
            self.mannequin_preview = preview;
            self.blend_space_preview = blend_space_preview;
            return;
        }

        let key = next_key.expect("checked is_some above");
        let world = self.app.world_mut();
        clear_mannequin_preview(world);
        clear_neutral_primitives(world);
        spawn_mannequin_preview(world, &key, &preview, &blend_space_preview);
        self.mannequin_key = Some(key);
        self.mannequin_preview = preview;
        self.blend_space_preview = blend_space_preview;
    }

    /// Read the mannequin preview's live playback position back from the bevy
    /// `AnimationPlayer` (max seek time across the preview's graph nodes) so
    /// the editor timeline playhead can advance while playing. `finished` is
    /// true once every preview animation has completed (non-looping playback).
    /// `None` until the glTF animation player has spawned.
    pub fn mannequin_playback_status(&mut self) -> Option<MannequinPlaybackStatus> {
        let world = self.app.world_mut();
        let mut players = world.query::<(&AnimationPlayer, &MannequinPreviewPlayer)>();
        let mut seek_seconds: Option<f32> = None;
        let mut finished = true;
        for (player, preview_player) in players.iter(world) {
            for node in &preview_player.nodes {
                let Some(animation) = player.animation(*node) else {
                    continue;
                };
                let seek = animation.seek_time();
                seek_seconds = Some(seek_seconds.map_or(seek, |current| current.max(seek)));
                finished &= animation.is_finished();
            }
        }
        seek_seconds.map(|seconds| MannequinPlaybackStatus {
            position_millis: if seconds.is_finite() && seconds > 0.0 {
                clamped_u32(f64::from(seconds) * 1000.0)
            } else {
                0
            },
            finished,
        })
    }

    /// Process and materialize the latest typed Prefab snapshots, then add only
    /// editor identity, picking, visibility, and explicit proxy decorations.
    ///
    /// # Panics
    ///
    /// Panics if the Bevy world has no `AppTypeRegistry` resource, or if a
    /// prepared preview references an AZSCENE source alias the preview
    /// validation pass did not record.
    pub fn apply_scene(
        &mut self,
        scene: &crate::gpu::ViewportScene,
        hidden: &std::collections::BTreeSet<String>,
    ) {
        let app_registry = self
            .app
            .world()
            .resource::<bevy::ecs::reflect::AppTypeRegistry>()
            .clone();
        let prepared = match prepare_viewport_scene(scene, &app_registry) {
            Ok(prepared) => prepared,
            Err(error) => {
                tracing::warn!(%error, "editor preview AZSCENE cook failed");
                return;
            }
        };
        let reconciled_object_ids = prepared
            .as_ref()
            .map(|prepared| {
                prepared
                    .identities
                    .values()
                    .map(|identity| identity.object_id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mesh = self.bridge_mesh.clone();
        let material = self.bridge_material.clone();
        let world = self.app.world_mut();
        let materialized = if let Some(prepared) = prepared.as_ref() {
            let registry = app_registry.read();
            match prepared.asset.materialize(world, &registry) {
                Ok(instance) => instance.entities,
                Err(error) => {
                    tracing::warn!(%error, "editor preview AZSCENE materialization failed");
                    return;
                }
            }
        } else {
            Vec::new()
        };
        self.hovered_entity = None;
        self.hover_shell = None;
        self.pick_exclusions
            .retain(|entity| world.get::<GizmoAxisPart>(*entity).is_some());

        despawn_bridged_entities(world);
        if let Some(prepared) = prepared.as_ref() {
            decorate_bridged_entities(
                world,
                &app_registry,
                prepared,
                &materialized,
                hidden,
                &mesh,
                &material,
            );
        }
        install_editor_component_gizmos(world, &materialized);

        self.retire_reconciled_ghosts(&reconciled_object_ids);
        self.sync_selection_bounds();
    }

    /// Drop the optimistic placement ghosts whose authored object has now
    /// arrived through the scene bridge, so the swap has no visible frame.
    fn retire_reconciled_ghosts(&mut self, reconciled_object_ids: &[String]) {
        let mut index = 0;
        while index < self.optimistic_assets.len() {
            let reconciled = self.optimistic_assets[index]
                .authored_object_id
                .as_ref()
                .is_some_and(|object_id| reconciled_object_ids.contains(object_id));
            if reconciled {
                let ghost = self.optimistic_assets.swap_remove(index);
                self.pick_exclusions.remove(&ghost.entity);
                let _ = self.app.world_mut().despawn(ghost.entity);
                crate::perf::input_trace(
                    crate::perf::InputTraceKind::AuthoredEntityReconciled,
                    ghost.interaction_id,
                    None,
                    Some((ghost.position.x, ghost.position.y, ghost.position.z)),
                    0,
                );
                crate::perf::request_summary_dump();
            } else {
                index += 1;
            }
        }
    }

    /// Highlight the bridged entity for the given authored
    /// `(document_id, object_id)` selection (viewport pick or outliner
    /// selection), restoring the shared bridge material on everything else.
    pub fn set_selected_authored(&mut self, selection: Option<(&str, &str)>) -> bool {
        if self
            .selected_authored
            .as_ref()
            .map(|(document_id, object_id)| (document_id.as_str(), object_id.as_str()))
            == selection
        {
            return false;
        }
        self.selected_authored = selection
            .map(|(document_id, object_id)| (document_id.to_owned(), object_id.to_owned()));
        let bridge_material = self.bridge_material.clone();
        let world = self.app.world_mut();
        let mut query = world
            .query_filtered::<&mut MeshMaterial3d<StandardMaterial>, With<BridgedAuthoredEntity>>();
        for mut material in query.iter_mut(world) {
            material.0 = bridge_material.clone();
        }
        self.sync_selection_bounds();
        self.remove_hover_shell_if_selected();
        true
    }

    fn sync_selection_bounds(&mut self) {
        let selected = self.selected_authored.clone();
        let mesh = self.selection_bounds_mesh.clone();
        let material = self.selection_bounds_material.clone();
        let show_bounds = self.visibility.bounds || self.visibility.bounding_boxes;
        let world = self.app.world_mut();
        let stale = {
            let mut query = world.query_filtered::<Entity, With<SelectionBounds>>();
            query.iter(world).collect::<Vec<_>>()
        };
        for entity in stale {
            world.despawn(entity);
        }
        let Some((document_id, object_id)) = selected else {
            self.refresh_pick_exclusions();
            return;
        };
        let selected_entity = {
            let mut query =
                world.query_filtered::<(Entity, &EditorSceneObject), With<BridgedAuthoredEntity>>();
            query.iter(world).find_map(|(entity, object)| {
                (object.document_id.as_deref() == Some(document_id.as_str())
                    && object.object_id.as_deref() == Some(object_id.as_str()))
                .then_some(entity)
            })
        };
        if let Some(selected_entity) = selected_entity {
            let bounds_transform = world
                .get::<Aabb>(selected_entity)
                .map(selection_bounds_transform)
                .unwrap_or_default();
            world.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                bounds_transform,
                if show_bounds {
                    Visibility::Visible
                } else {
                    Visibility::Hidden
                },
                RenderLayers::layer(GIZMO_RENDER_LAYER),
                SelectionBounds,
                ChildOf(selected_entity),
            ));
        }
        self.refresh_pick_exclusions();
    }

    fn remove_hover_shell_if_selected(&mut self) {
        let Some(hovered) = self.hovered_entity else {
            return;
        };
        let selected = self.selected_authored.as_ref();
        let is_selected = self
            .app
            .world()
            .get::<EditorSceneObject>(hovered)
            .is_some_and(|object| {
                matches!(
                    (selected, &object.document_id, &object.object_id),
                    (Some((doc, obj)), Some(document_id), Some(object_id))
                        if doc == document_id && obj == object_id
                )
            });
        if is_selected {
            self.set_hovered_entity(None);
        }
    }

    /// Update the dim hover rim from a normalized pointer coordinate.
    pub fn set_hovered_at(&mut self, coord: Vec2) {
        let entity = self.pick_entity(coord);
        self.set_hovered_entity(entity);
    }

    /// Remove the hover rim when the pointer leaves the viewport.
    pub fn clear_hover(&mut self) {
        self.set_hovered_entity(None);
    }

    fn set_hovered_entity(&mut self, entity: Option<Entity>) {
        if self.hovered_entity == entity {
            return;
        }
        let world = self.app.world_mut();
        if let Some(shell) = self.hover_shell.take() {
            self.pick_exclusions.remove(&shell);
            if world.get_entity(shell).is_ok() {
                world.despawn(shell);
            }
        }
        self.hovered_entity = None;

        let Some(entity) = entity else {
            return;
        };
        if world.get::<BridgedAuthoredEntity>(entity).is_none() {
            return;
        }
        let object = world.get::<EditorSceneObject>(entity);
        let is_selected = object.is_some_and(|object| {
            matches!(
                (&self.selected_authored, &object.document_id, &object.object_id),
                (Some((doc, obj)), Some(document_id), Some(object_id))
                    if doc == document_id && obj == object_id
            )
        });
        if is_selected {
            return;
        }
        let shell = world
            .spawn((
                Mesh3d(self.bridge_mesh.clone()),
                MeshMaterial3d(self.hover_outline_material.clone()),
                Transform::from_scale(Vec3::splat(1.045)),
                HoverOutlineShell,
                ChildOf(entity),
            ))
            .id();
        self.pick_exclusions.insert(shell);
        self.hovered_entity = Some(entity);
        self.hover_shell = Some(shell);
    }

    /// Orbit the editor camera by normalized viewport-fraction mouse deltas.
    pub fn camera_orbit(&mut self, dx: f32, dy: f32) {
        self.camera.yaw = (dx * std::f32::consts::TAU).mul_add(-0.6, self.camera.yaw);
        self.camera.pitch = (dy * std::f32::consts::PI)
            .mul_add(0.6, self.camera.pitch)
            .clamp(-1.5, 1.5);
        self.sync_camera_transform();
    }

    /// Pan the camera focus in the view plane by normalized viewport-fraction
    /// mouse deltas (content follows the mouse).
    pub fn camera_pan(&mut self, dx: f32, dy: f32) {
        let scale = self.camera.distance * self.camera.speed * 0.25;
        let right = self.camera.right();
        let up = self.camera.up();
        self.camera.focus -= right * (dx * scale);
        self.camera.focus += up * (dy * scale);
        self.sync_camera_transform();
    }

    /// Dolly toward (positive steps) or away from the camera focus.
    pub fn camera_dolly(&mut self, steps: f32) {
        let factor = steps.mul_add(-0.1, 1.0).clamp(0.5, 2.0);
        self.camera.distance = (self.camera.distance * factor).clamp(0.5, 500.0);
        self.sync_camera_transform();
    }

    /// Apply one of the editor's named camera poses around the current focus.
    pub fn set_camera_view(&mut self, view: ViewportCameraView) {
        match view {
            ViewportCameraView::Perspective => {
                self.camera.yaw = std::f32::consts::FRAC_PI_4;
                self.camera.pitch = 0.48;
                self.camera.distance = 12.0;
            }
            ViewportCameraView::Top => {
                self.camera.yaw = 0.0;
                // Avoid the exact look-at/up singularity while remaining
                // visually top-down.
                self.camera.pitch = 1.5;
            }
            ViewportCameraView::Front => {
                self.camera.yaw = 0.0;
                self.camera.pitch = 0.0;
            }
            ViewportCameraView::Side => {
                self.camera.yaw = std::f32::consts::FRAC_PI_2;
                self.camera.pitch = 0.0;
            }
            ViewportCameraView::Game => {
                self.camera.focus.y = self.camera.focus.y.max(1.0);
                self.camera.yaw = std::f32::consts::PI;
                self.camera.pitch = 0.24;
                self.camera.distance = 8.0;
            }
        }
        self.sync_camera_transform();
    }

    /// Switch scene materials between lit, unlit, and global wireframe modes.
    /// This runs only on a pill selection, never in the per-frame hot path.
    pub fn set_shading_mode(&mut self, mode: ViewportShadingMode) {
        if self.shading_mode == mode {
            return;
        }
        self.shading_mode = mode;
        let world = self.app.world_mut();
        {
            let mut wireframe = world.resource_mut::<WireframeConfig>();
            wireframe.global = mode == ViewportShadingMode::Wireframe;
            wireframe.default_color = Color::srgb(0.24, 0.72, 1.0);
            wireframe.default_line_width = 1.35;
        }
        let utility_materials = [
            self.selection_bounds_material.clone(),
            self.hover_outline_material.clone(),
            self.asset_ghost_material.clone(),
            self.gizmo_highlight_material.clone(),
            self.gizmo_axis_materials[0].clone(),
            self.gizmo_axis_materials[1].clone(),
            self.gizmo_axis_materials[2].clone(),
        ];
        let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
        for (_, material) in materials.iter_mut() {
            material.unlit = mode != ViewportShadingMode::Lit;
        }
        for handle in utility_materials {
            if let Some(mut material) = materials.get_mut(&handle) {
                material.unlit = true;
            }
        }
    }

    /// Apply semantic viewport visibility without respawning retained assets.
    pub fn set_visibility(&mut self, settings: ViewportVisibilitySettings) {
        if self.visibility == settings {
            return;
        }
        self.visibility = settings;
        let world = self.app.world_mut();
        let mut grid = world.query_filtered::<&mut Visibility, With<GroundGrid>>();
        for mut visibility in grid.iter_mut(world) {
            *visibility = if settings.grid {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
        }
        let show_bounds = settings.bounds || settings.bounding_boxes;
        let mut bounds = world.query_filtered::<&mut Visibility, With<SelectionBounds>>();
        for mut visibility in bounds.iter_mut(world) {
            *visibility = if show_bounds {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
        }
        sync_camera_clear_color(world, settings, self.render_theme);
    }

    /// Apply GPUI-resolved semantic colors to retained Bevy viewport chrome.
    pub fn set_render_theme(&mut self, theme: ViewportRenderTheme) {
        if self.render_theme == Some(theme) {
            return;
        }
        self.render_theme = Some(theme);
        let world = self.app.world_mut();
        {
            let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
            if let Some(mut material) = materials.get_mut(&self.grid_minor_material) {
                material.base_color = rgba_color(theme.grid_minor);
            }
            if let Some(mut material) = materials.get_mut(&self.grid_major_material) {
                material.base_color = rgba_color(theme.grid_major);
            }
        }
        sync_camera_clear_color(world, self.visibility, Some(theme));
    }

    /// Real main-world diagnostics sampled by the producer at a bounded cadence.
    /// Draws are visible mesh submissions (before renderer batching); triangle
    /// and vertex counts are the submitted mesh geometry, including instances.
    pub fn telemetry(&mut self) -> EditorViewportTelemetryData {
        let (fps, frame_time_us) = {
            let diagnostics = self.app.world().resource::<DiagnosticsStore>();
            let fps = diagnostics
                .get(&FrameTimeDiagnosticsPlugin::FPS)
                .and_then(diagnostic_median)
                .map(clamped_u32);
            let frame_time_us = diagnostics
                .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
                .and_then(diagnostic_median)
                .map(|value| clamped_u32(value * 1_000.0));
            (fps, frame_time_us)
        };

        let (draw_calls, triangles, vertices) =
            self.app
                .world_mut()
                .resource_scope(|world, meshes: Mut<Assets<Mesh>>| {
                    let mut draws = 0_u64;
                    let mut triangles = 0_u64;
                    let mut vertices = 0_u64;
                    let mut query =
                        world
                            .query::<(&Mesh3d, Option<&Visibility>, Option<&InheritedVisibility>)>(
                            );
                    for (mesh_handle, visibility, inherited) in query.iter(world) {
                        if visibility.is_some_and(|visibility| *visibility == Visibility::Hidden)
                            || inherited.is_some_and(|visibility| !visibility.get())
                        {
                            continue;
                        }
                        let Some(mesh) = meshes.get(&mesh_handle.0) else {
                            continue;
                        };
                        draws += 1;
                        if let Some((vertex_count, triangle_count)) = mesh_geometry_counts(mesh) {
                            vertices += vertex_count;
                            triangles += triangle_count;
                        }
                    }
                    (Some(draws), Some(triangles), Some(vertices))
                });

        EditorViewportTelemetryData {
            fps,
            frame_time_us,
            draw_calls,
            triangles,
            vertices,
            gpu_memory_bytes: None,
        }
    }

    /// Frame the selected authored entity using the aggregate bounds supplied
    /// by its resolved cooked mesh product.
    pub fn frame_selected(&mut self) -> bool {
        let Some((document_id, object_id)) = self.selected_authored.clone() else {
            return false;
        };
        let framed = {
            let world = self.app.world_mut();
            let mut query = world.query::<(&EditorSceneObject, &GlobalTransform, &Aabb)>();
            query.iter(world).find_map(|(object, transform, bounds)| {
                (object.document_id.as_deref() == Some(document_id.as_str())
                    && object.object_id.as_deref() == Some(object_id.as_str()))
                .then(|| {
                    let center = transform.transform_point(Vec3::from(bounds.center));
                    let half_extents = Vec3::from(bounds.half_extents);
                    let affine = transform.affine();
                    let scaled_half_extents = Vec3::new(
                        affine.matrix3.x_axis.length() * half_extents.x,
                        affine.matrix3.y_axis.length() * half_extents.y,
                        affine.matrix3.z_axis.length() * half_extents.z,
                    );
                    (center, scaled_half_extents.length())
                })
            })
        };
        let Some((center, radius)) = framed else {
            return false;
        };
        self.camera.focus = center;
        self.camera.distance = (radius * 2.75).clamp(0.5, 500.0);
        self.sync_camera_transform();
        true
    }

    /// Current camera pose as `(yaw, pitch, distance, speed)` for the panel's
    /// live badges and orientation triad.
    pub const fn camera_pose(&self) -> (f32, f32, f32, f32) {
        (
            self.camera.yaw,
            self.camera.pitch,
            self.camera.distance,
            self.camera.speed,
        )
    }

    fn sync_camera_transform(&mut self) {
        let camera = self.camera;
        let world = self.app.world_mut();
        let mut query =
            world.query_filtered::<(&mut Transform, &mut GlobalTransform), With<Camera3d>>();
        for (mut transform, mut global_transform) in query.iter_mut(world) {
            let updated =
                Transform::from_translation(camera.position()).looking_at(camera.focus, Vec3::Y);
            *transform = updated;
            *global_transform = GlobalTransform::from(updated);
        }
    }

    pub const fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Run the main-world schedule so camera and scene global transforms are
    /// current before input raycasts.
    pub fn update_main_world(&mut self) {
        if let Some(diagnostic) = &mut self.diagnostic {
            diagnostic.frame_id = diagnostic.frame_id.wrapping_add(1);
            if let Some(mut image) = self
                .app
                .world_mut()
                .resource_mut::<Assets<Image>>()
                .get_mut(&diagnostic.image)
                && let Some(data) = image.data.as_mut()
            {
                DiagnosticCanvas {
                    data: data.as_mut_slice(),
                    width: self.width,
                    height: self.height,
                }
                .paint_metadata(diagnostic.frame_id);
            }
        }
        // Keep main-world work and extraction explicit for diagnostics. The
        // render schedule itself runs on Bevy's pipelined render thread and is
        // timed by the systems installed around the Render schedule above.
        let sub_apps = self.app.sub_apps_mut();
        debug_assert_eq!(sub_apps.sub_apps.len(), 1);

        let started = std::time::Instant::now();
        sub_apps.main.run_default_schedule();
        crate::perf::record_elapsed(crate::perf::FRAME_BEVY_MAIN, started);
    }

    /// Extract the already-updated main world and release the pipelined render
    /// schedule. Input-visible material/ghost changes made between these two
    /// halves therefore appear in this same frame.
    ///
    /// # Panics
    ///
    /// Panics if `PipelinedRenderingPlugin` did not install a
    /// `RenderExtractApp` sub-app.
    pub fn render_current_world(&mut self) {
        let sub_apps = self.app.sub_apps_mut();
        debug_assert_eq!(sub_apps.sub_apps.len(), 1);
        let render_extract_app = sub_apps
            .sub_apps
            .get_mut(&RenderExtractApp.intern())
            .expect("PipelinedRenderingPlugin installed a RenderExtractApp sub-app");
        let started = std::time::Instant::now();
        render_extract_app.extract(sub_apps.main.world_mut());
        crate::perf::record_elapsed(crate::perf::FRAME_BEVY_EXTRACT, started);
        render_extract_app.update();
        sub_apps.main.world_mut().clear_trackers();
    }

    /// Advance one complete frame. Kept for focused renderer tests; the live
    /// viewport host uses the split methods to resolve input between them.
    pub fn tick(&mut self) {
        self.update_main_world();
        self.render_current_world();
    }

    /// Spawn or move the immediate placeholder bounds ghost. The placeholder
    /// is always available in the first frame; product-specific scene meshes
    /// may replace it later without delaying interaction feedback.
    pub fn update_asset_ghost(&mut self, interaction_id: u64, position: Vec3) {
        let centered = position;
        if let Some(ghost) = self.active_asset_ghost.as_mut()
            && ghost.interaction_id == interaction_id
        {
            ghost.position = centered;
            if let Some(mut transform) = self.app.world_mut().get_mut::<Transform>(ghost.entity) {
                transform.translation = centered;
            }
            return;
        }
        self.clear_asset_ghost();
        let entity = self
            .app
            .world_mut()
            .spawn((
                Mesh3d(self.bridge_mesh.clone()),
                MeshMaterial3d(self.asset_ghost_material.clone()),
                Transform::from_translation(centered),
                Visibility::default(),
            ))
            .id();
        self.pick_exclusions.insert(entity);
        self.active_asset_ghost = Some(AssetGhost {
            interaction_id,
            entity,
            position: centered,
            authored_object_id: None,
        });
    }

    pub fn clear_asset_ghost(&mut self) {
        if let Some(ghost) = self.active_asset_ghost.take() {
            self.pick_exclusions.remove(&ghost.entity);
            let _ = self.app.world_mut().despawn(ghost.entity);
        }
    }

    /// Convert the active ghost into a solid local placement without changing
    /// its entity or transform, avoiding any drop-time flicker.
    pub fn solidify_asset_ghost(&mut self, interaction_id: u64) -> Option<Vec3> {
        let ghost = self.active_asset_ghost.take()?;
        if ghost.interaction_id != interaction_id {
            self.active_asset_ghost = Some(ghost);
            return None;
        }
        if let Some(mut material) = self
            .app
            .world_mut()
            .get_mut::<MeshMaterial3d<StandardMaterial>>(ghost.entity)
        {
            material.0 = self.bridge_material.clone();
        }
        let position = ghost.position;
        self.optimistic_assets.push(ghost);
        Some(position)
    }

    /// Associate an optimistic placement with the authoritative object returned
    /// by project-host. It remains visible until that object enters the scene
    /// bridge, where `apply_scene` swaps it seamlessly in one world update.
    pub fn mark_asset_authored(&mut self, interaction_id: u64, authored_object_id: String) {
        if let Some(asset) = self
            .optimistic_assets
            .iter_mut()
            .find(|asset| asset.interaction_id == interaction_id)
        {
            asset.authored_object_id = Some(authored_object_id);
        }
    }

    /// The camera ray through a normalized `[0,1]` viewport coordinate (origin
    /// top-left). `None` if there is no camera yet or the coordinate can't be
    /// unprojected.
    fn camera_ray(&mut self, coord: Vec2) -> Option<Ray3d> {
        let pixel = coord * Vec2::new(extent_pixels(self.width), extent_pixels(self.height));
        let world = self.app.world_mut();
        let mut camera_query =
            world.query_filtered::<(&Camera, &GlobalTransform), With<Camera3d>>();
        let (camera, camera_transform) = camera_query.iter(world).next()?;
        camera.viewport_to_world(camera_transform, pixel).ok()
    }

    /// Update the gizmo mode / transform space / snap config from the editor's
    /// scene tool state and re-sync the gizmo geometry to the current selection.
    /// Called every pump frame.
    pub(crate) fn update_gizmo(
        &mut self,
        mode: GizmoMode,
        pivot: GizmoPivot,
        space: GizmoSpace,
        snap: GizmoSnap,
    ) {
        self.gizmo_mode = mode;
        self.gizmo_pivot = pivot;
        self.gizmo_space = space;
        self.gizmo_snap = snap;
        self.sync_gizmo();
    }

    /// Keep selection bounds on the editor theme's accent color. The viewport
    /// host forwards this from `cx.theme()` so renderer chrome never embeds a
    /// parallel color constant.
    // The comparison below is change detection against the exact array stored
    // by the previous call, so an epsilon would drop real theme changes.
    #[allow(clippy::float_cmp)]
    pub fn update_selection_bounds_color(&mut self, rgba: [f32; 4]) {
        if self.selection_bounds_rgba == rgba {
            return;
        }
        self.selection_bounds_rgba = rgba;
        let color = Color::srgba(rgba[0], rgba[1], rgba[2], rgba[3]);
        if let Some(mut material) = self
            .app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .get_mut(&self.selection_bounds_material)
        {
            material.base_color = color;
            material.emissive = bevy::color::LinearRgba::from(color);
        }
    }

    /// Whether an axis drag is currently in progress.
    #[must_use]
    pub const fn is_gizmo_dragging(&self) -> bool {
        self.gizmo_drag.is_some()
    }

    /// The Bevy entity plus local and propagated world transforms of the
    /// currently selected bridged authored object.
    fn selected_bevy_entity(&mut self) -> Option<(Entity, Transform, GlobalTransform, Vec3)> {
        let (document_id, object_id) = self.selected_authored.clone()?;
        let world = self.app.world_mut();
        let mut query = world.query_filtered::<(
            Entity,
            &EditorSceneObject,
            &Transform,
            &GlobalTransform,
            Option<&Aabb>,
        ), With<BridgedAuthoredEntity>>();
        for (entity, object, transform, global, aabb) in query.iter(world) {
            if object.document_id.as_deref() == Some(document_id.as_str())
                && object.object_id.as_deref() == Some(object_id.as_str())
            {
                let bounds_center = aabb.map_or_else(
                    || global.translation(),
                    |aabb| global.transform_point(Vec3::from(aabb.center)),
                );
                return Some((entity, *transform, *global, bounds_center));
            }
        }
        None
    }

    /// Screen-facing gizmo scale derived from the active camera projection.
    fn gizmo_scale(&mut self, origin: Vec3) -> f32 {
        let projection = {
            let world = self.app.world_mut();
            let mut query = world.query_filtered::<&Projection, With<Camera3d>>();
            query.iter(world).next().cloned()
        };
        projection.map_or(1.0, |projection| {
            constant_screen_scale(
                &projection,
                self.camera.position(),
                origin,
                self.layout_height,
                GIZMO_SCREEN_PIXELS,
            )
        })
    }

    /// Despawn the gizmo entities, if any.
    fn despawn_gizmo(&mut self) {
        {
            let world = self.app.world_mut();
            let roots: Vec<Entity> = {
                let mut query = world.query_filtered::<Entity, With<GizmoRoot>>();
                query.iter(world).collect()
            };
            for root in roots {
                let mut entity = world.entity_mut(root);
                entity.despawn_related::<Children>();
                entity.despawn();
            }
        }
        self.gizmo_spawned_mode = GizmoMode::None;
        self.refresh_pick_exclusions();
    }

    fn refresh_pick_exclusions(&mut self) {
        self.pick_exclusions
            .retain(|entity| self.hover_shell == Some(*entity));
        let world = self.app.world_mut();
        let exclusions = &mut self.pick_exclusions;
        let mut query = world.query_filtered::<Entity, With<GizmoAxisPart>>();
        for entity in query.iter(world) {
            exclusions.insert(entity);
        }
        let mut selection_query = world.query_filtered::<Entity, With<SelectionBounds>>();
        for entity in selection_query.iter(world) {
            exclusions.insert(entity);
        }
    }

    /// Spawn/reposition the gizmo for the current mode + selection. Re-spawns
    /// geometry only when the mode changes; otherwise just moves the root.
    fn sync_gizmo(&mut self) {
        if self.gizmo_mode == GizmoMode::None {
            if self.gizmo_spawned_mode != GizmoMode::None {
                self.despawn_gizmo();
            }
            return;
        }
        let Some((_, _, global, bounds_center)) = self.selected_bevy_entity() else {
            if self.gizmo_spawned_mode != GizmoMode::None {
                self.despawn_gizmo();
            }
            return;
        };
        let translation = match self.gizmo_pivot {
            GizmoPivot::Pivot => global.translation(),
            GizmoPivot::Center => bounds_center,
        };
        let rotation = global.rotation();

        let space_rotation = match self.gizmo_space {
            GizmoSpace::World => Quat::IDENTITY,
            GizmoSpace::Local => rotation,
        };
        let scale = self.gizmo_scale(translation);
        let root_transform = Transform {
            translation,
            rotation: space_rotation,
            scale: Vec3::splat(scale),
        };

        if self.gizmo_spawned_mode == self.gizmo_mode {
            // Geometry is current; just move the root.
            let world = self.app.world_mut();
            let root = {
                let mut query = world.query_filtered::<Entity, With<GizmoRoot>>();
                query.iter(world).next()
            };
            if let Some(root) = root {
                if let Some(mut transform) = world.get_mut::<Transform>(root) {
                    *transform = root_transform;
                }
                return;
            }
        }

        // Mode changed (or no root exists): re-spawn.
        self.despawn_gizmo();
        let mode = self.gizmo_mode;
        let shaft = self.gizmo_shaft_mesh.clone();
        let translate_tip = self.gizmo_translate_tip_mesh.clone();
        let scale_tip = self.gizmo_scale_tip_mesh.clone();
        let ring = self.gizmo_ring_mesh.clone();
        let axis_materials = self.gizmo_axis_materials.clone();
        let world = self.app.world_mut();
        let root = world
            .spawn((GizmoRoot, root_transform, Visibility::default()))
            .id();
        for (index, axis) in GizmoAxis::ALL.into_iter().enumerate() {
            let material = axis_materials[index].clone();
            spawn_gizmo_axis(
                world,
                root,
                mode,
                axis,
                &shaft,
                &translate_tip,
                &scale_tip,
                &ring,
                material,
            );
        }
        self.gizmo_spawned_mode = mode;
        self.refresh_pick_exclusions();
    }

    /// Begin an axis drag if the pointer grabbed a gizmo handle. Returns `true`
    /// when a drag started (the caller must then suppress the pick).
    pub fn begin_gizmo_drag(&mut self, coord: Vec2) -> bool {
        if self.gizmo_mode == GizmoMode::None {
            return false;
        }
        let Some((document_id, object_id)) = self.selected_authored.clone() else {
            return false;
        };
        let Some((entity, local, global, bounds_center)) = self.selected_bevy_entity() else {
            return false;
        };
        let translation = match self.gizmo_pivot {
            GizmoPivot::Pivot => global.translation(),
            GizmoPivot::Center => bounds_center,
        };
        let rotation = global.rotation();
        let Some(ray) = self.camera_ray(coord) else {
            return false;
        };

        // Hit-test only the gizmo handles.
        let world = self.app.world_mut();
        let part_by_entity: std::collections::HashMap<Entity, (GizmoMode, GizmoAxis)> = {
            let mut query = world.query::<(Entity, &GizmoAxisPart)>();
            query
                .iter(world)
                .map(|(entity, part)| (entity, (part.mode, part.axis)))
                .collect()
        };
        if part_by_entity.is_empty() {
            return false;
        }
        let filter = |entity: Entity| part_by_entity.contains_key(&entity);
        let settings = MeshRayCastSettings::default()
            .with_visibility(RayCastVisibility::Any)
            .with_filter(&filter);
        let hit_entity = {
            let Ok(mut ray_cast) = self.mesh_ray_cast.get_mut(world) else {
                return false;
            };
            ray_cast
                .cast_ray(ray, &settings)
                .first()
                .map(|(entity, _hit)| *entity)
        };
        let Some((drag_mode, axis)) =
            hit_entity.and_then(|entity| part_by_entity.get(&entity).copied())
        else {
            return false;
        };

        let space_rotation = match self.gizmo_space {
            GizmoSpace::World => Quat::IDENTITY,
            GizmoSpace::Local => rotation,
        };
        let axis_dir = (space_rotation * axis.unit()).normalize_or_zero();
        if axis_dir.length_squared() < 1e-8 {
            return false;
        }
        let view_dir = (self.camera.focus - self.camera.position()).normalize_or_zero();
        let start_offset =
            axis_drag_offset(ray.origin, *ray.direction, translation, axis_dir, view_dir)
                .unwrap_or(0.0);
        let parent_global = self
            .app
            .world()
            .get::<ChildOf>(entity)
            .and_then(|child_of| self.app.world().get::<GlobalTransform>(child_of.0))
            .copied()
            .unwrap_or(GlobalTransform::IDENTITY);

        self.highlight_gizmo_axis(Some((drag_mode, axis)));
        self.gizmo_drag = Some(GizmoDrag {
            entity,
            document_id,
            object_id,
            mode: drag_mode,
            axis,
            axis_dir,
            origin: translation,
            parent_global,
            start_translation: local.translation,
            start_global_rotation: rotation,
            start_scale: local.scale,
            start_offset,
            start_ray_origin: ray.origin,
            start_ray_dir: *ray.direction,
        });
        true
    }

    /// Advance the active axis drag, updating the selected entity's bevy
    /// transform live. No-op when no drag is active.
    pub fn update_gizmo_drag(&mut self, coord: Vec2) {
        let Some(drag) = self.gizmo_drag.clone() else {
            return;
        };
        let Some(ray) = self.camera_ray(coord) else {
            return;
        };
        let view_dir = (self.camera.focus - self.camera.position()).normalize_or_zero();
        match drag.mode {
            GizmoMode::Translate => {
                let Some(offset) = axis_drag_offset(
                    ray.origin,
                    *ray.direction,
                    drag.origin,
                    drag.axis_dir,
                    view_dir,
                ) else {
                    return;
                };
                let mut delta = offset - drag.start_offset;
                if let Some(step) = self.gizmo_snap.translate_step_meters {
                    delta = snap_scalar(delta, step);
                }
                let new_translation = local_translation_after_world_delta(
                    drag.start_translation,
                    drag.parent_global,
                    drag.axis_dir * delta,
                );
                self.set_entity_translation(drag.entity, new_translation);
            }
            GizmoMode::Rotate => {
                let Some(mut angle) = rotation_drag_angle(
                    drag.start_ray_origin,
                    drag.start_ray_dir,
                    ray.origin,
                    *ray.direction,
                    drag.origin,
                    drag.axis_dir,
                ) else {
                    return;
                };
                if let Some(step_degrees) = self.gizmo_snap.rotate_step_degrees {
                    angle = snap_scalar(angle, step_degrees.to_radians());
                }
                let global_rotation =
                    Quat::from_axis_angle(drag.axis_dir, angle) * drag.start_global_rotation;
                let new_rotation = local_rotation_from_global(drag.parent_global, global_rotation);
                self.set_entity_rotation(drag.entity, new_rotation);
            }
            GizmoMode::Scale => {
                let Some(offset) = axis_drag_offset(
                    ray.origin,
                    *ray.direction,
                    drag.origin,
                    drag.axis_dir,
                    view_dir,
                ) else {
                    return;
                };
                let delta = offset - drag.start_offset;
                let factor = delta.mul_add(GIZMO_SCALE_SENSITIVITY, 1.0).max(0.01);
                let mut new_scale = drag.start_scale;
                match drag.axis {
                    GizmoAxis::X => new_scale.x = drag.start_scale.x * factor,
                    GizmoAxis::Y => new_scale.y = drag.start_scale.y * factor,
                    GizmoAxis::Z => new_scale.z = drag.start_scale.z * factor,
                }
                self.set_entity_scale(drag.entity, new_scale);
            }
            GizmoMode::None | GizmoMode::Universal => {}
        }
        // Keep the gizmo root glued to the (now moved) selection.
        self.sync_gizmo();
    }

    /// Current selected-entity translation after this tick's gizmo update.
    #[must_use]
    pub fn gizmo_drag_rendered_translation(&self) -> Option<Vec3> {
        let drag = self.gizmo_drag.as_ref()?;
        self.app
            .world()
            .get::<Transform>(drag.entity)
            .map(|transform| transform.translation)
    }

    /// End the active drag and return the changed authored transform channel.
    /// No-op when no drag is active or the selection disappeared during it.
    pub(crate) fn end_gizmo_drag(&mut self) -> Option<GizmoCommit> {
        let drag = self.gizmo_drag.take()?;
        self.highlight_gizmo_axis(None);
        let transform = *self.app.world().get::<Transform>(drag.entity)?;
        let value = gizmo_commit_value(drag.mode, transform)?;
        tracing::info!(
            target: "az_editor::viewport",
            event = "gizmo_drag.committed",
            mode = ?drag.mode,
            value = ?value,
            translate_step_meters = ?self.gizmo_snap.translate_step_meters,
            rotate_step_degrees = ?self.gizmo_snap.rotate_step_degrees,
            "viewport gizmo state"
        );
        Some(GizmoCommit {
            document_id: drag.document_id,
            object_id: drag.object_id,
            value,
        })
    }

    fn set_entity_translation(&mut self, entity: Entity, translation: Vec3) {
        if let Some(mut transform) = self.app.world_mut().get_mut::<Transform>(entity) {
            transform.translation = translation;
        }
    }

    fn set_entity_rotation(&mut self, entity: Entity, rotation: Quat) {
        if let Some(mut transform) = self.app.world_mut().get_mut::<Transform>(entity) {
            transform.rotation = rotation;
        }
    }

    fn set_entity_scale(&mut self, entity: Entity, scale: Vec3) {
        if let Some(mut transform) = self.app.world_mut().get_mut::<Transform>(entity) {
            transform.scale = scale;
        }
    }

    /// Tint the grabbed axis handle with the highlight material and restore the
    /// others to their axis colors.
    fn highlight_gizmo_axis(&mut self, active: Option<(GizmoMode, GizmoAxis)>) {
        let axis_materials = self.gizmo_axis_materials.clone();
        let highlight = self.gizmo_highlight_material.clone();
        let world = self.app.world_mut();
        let mut query = world.query::<(&GizmoAxisPart, &mut MeshMaterial3d<StandardMaterial>)>();
        for (part, mut material) in query.iter_mut(world) {
            let axis_index = match part.axis {
                GizmoAxis::X => 0,
                GizmoAxis::Y => 1,
                GizmoAxis::Z => 2,
            };
            material.0 = if active == Some((part.mode, part.axis)) {
                highlight.clone()
            } else {
                axis_materials[axis_index].clone()
            };
        }
    }

    /// Pick the nearest [`EditorSceneObject`] under a normalized `[0,1]` viewport
    /// coordinate by casting the camera ray against scene mesh **triangles** via
    /// bevy's accelerated [`MeshRayCast`] for local, in-process, pixel-accurate
    /// picking with no runtime round-trip. Returns the hit (id +
    /// optional authored reference), or `None` on a miss.
    ///
    /// Runs on the render thread between ticks, so it has exclusive `&mut World`
    /// access and never races the renderer.
    pub fn pick(&mut self, coord: Vec2) -> Option<crate::gpu::ViewportPickHit> {
        let hit_entity = self.pick_entity(coord)?;
        let world = self.app.world();
        let object = world.get::<EditorSceneObject>(hit_entity)?;
        Some(crate::gpu::ViewportPickHit {
            id: object.id.clone(),
            document_id: object.document_id.clone(),
            object_id: object.object_id.clone(),
        })
    }

    /// Capture the current pickable scene into a read-only acceleration used
    /// by latency-critical pointer-down selection. This allocates only when
    /// scene geometry, camera pose, or viewport size changes; ordinary frames
    /// reuse the published snapshot.
    pub(crate) fn pick_snapshot(&mut self) -> Option<EditorPickSnapshot> {
        let ray_origin = self.camera_ray(Vec2::ZERO)?.origin;
        let ray_directions = [
            *self.camera_ray(Vec2::ZERO)?.direction,
            *self.camera_ray(Vec2::new(1.0, 0.0))?.direction,
            *self.camera_ray(Vec2::new(0.0, 1.0))?.direction,
            *self.camera_ray(Vec2::ONE)?.direction,
        ];
        let exclusions = &self.pick_exclusions;
        let world = self.app.world_mut();
        let mut query = world.query::<(
            Entity,
            &EditorSceneObject,
            &GlobalTransform,
            Option<&Aabb>,
            &Mesh3d,
        )>();
        let mut candidates = query
            .iter(world)
            .filter(|(_, object, _, aabb, _)| {
                aabb.is_some() && !object.id.starts_with("selection/")
            })
            .filter_map(|(entity, object, global, aabb, mesh)| {
                if exclusions.contains(&entity) {
                    return None;
                }
                let aabb = aabb?;
                Some((
                    Some(crate::gpu::ViewportPickHit {
                        id: object.id.clone(),
                        document_id: object.document_id.clone(),
                        object_id: object.object_id.clone(),
                    }),
                    false,
                    global.affine().inverse(),
                    Vec3::from(aabb.center),
                    Vec3::from(aabb.half_extents),
                    mesh.0.clone(),
                ))
            })
            .collect::<Vec<_>>();
        let mut gizmo_query = world
            .query_filtered::<(&GlobalTransform, Option<&Aabb>, &Mesh3d), With<GizmoAxisPart>>();
        candidates.extend(gizmo_query.iter(world).filter_map(|(global, aabb, mesh)| {
            let aabb = aabb?;
            Some((
                None,
                true,
                global.affine().inverse(),
                Vec3::from(aabb.center),
                Vec3::from(aabb.half_extents),
                mesh.0.clone(),
            ))
        }));
        let meshes = world.resource::<Assets<Mesh>>();
        let objects = candidates
            .into_iter()
            .filter_map(
                |(hit, gizmo, local_from_world, center, half_extents, mesh)| {
                    let triangles = mesh_triangles(meshes.get(&mesh)?);
                    (!triangles.is_empty()).then_some(EditorPickProxy::new(
                        hit,
                        gizmo,
                        local_from_world,
                        center,
                        half_extents,
                        triangles,
                    ))
                },
            )
            .collect();
        Some(EditorPickSnapshot::new(ray_origin, ray_directions, objects))
    }

    fn pick_entity(&mut self, coord: Vec2) -> Option<Entity> {
        let ray = self.camera_ray(coord)?;
        let exclusions = &self.pick_exclusions;
        let filter = |entity: Entity| !exclusions.contains(&entity);
        let settings = MeshRayCastSettings::default()
            .with_visibility(RayCastVisibility::Any)
            .with_filter(&filter);
        let world = self.app.world_mut();
        let Ok(mut ray_cast) = self.mesh_ray_cast.get_mut(world) else {
            return None;
        };
        let hit = ray_cast
            .cast_ray(ray, &settings)
            .first()
            .map(|(entity, _hit)| *entity);
        // `ray_cast` borrows `world`; NLL ends that borrow at its last use above,
        // so `world` is free again for the owner lookup below.
        hit.map(|entity| {
            world
                .get::<az_render::RenderMeshChild>(entity)
                .map_or(entity, |child| child.owner)
        })
    }

    /// Project a normalized `[0,1]` viewport coordinate onto the ground plane
    /// (y = 0) via the camera ray, returning the world hit point. `None` if there
    /// is no camera or the ray is parallel to / points away from the ground.
    pub fn pick_ground(&mut self, coord: Vec2) -> Option<Vec3> {
        let ray = self.camera_ray(coord)?;
        let direction_y = ray.direction.y;
        if direction_y.abs() < f32::EPSILON {
            return None;
        }
        let distance = -ray.origin.y / direction_y;
        if distance <= 0.0 {
            return None;
        }
        let hit = ray.origin + *ray.direction * distance;
        Some(Vec3::new(hit.x, 0.0, hit.z))
    }

    /// Resolve a normalized `[0,1]` viewport drop coordinate to the world
    /// ground plane. Authored persistence owns entity creation; the scene bridge
    /// displays the result after project-host reconciliation.
    pub fn dropped_asset_position(&mut self, coord: Vec2) -> Option<Vec3> {
        self.pick_ground(coord)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_editor_ui::panels::{DEFAULT_MANNEQUIN_CHARACTER_GLB, DEFAULT_MANNEQUIN_MOTION_GLB};
    use bevy::time::TimeUpdateStrategy;
    use std::fs;
    use std::time::Duration;

    /// Joint count of the generated synthetic rig that any checkout loads.
    const EXPECTED_SYNTHETIC_JOINTS: usize = 2;

    #[test]
    fn camera_and_light_gizmos_are_editor_requested_layer_one_decorations() {
        let mut world = World::new();
        world.insert_resource(Assets::<GizmoAsset>::default());
        let projection = Projection::Perspective(PerspectiveProjection {
            fov: 70.0_f32.to_radians(),
            near: 0.1,
            far: 100.0,
            ..default()
        });
        let runtime_entity = world.spawn((Camera3d::default(), projection.clone())).id();
        let editor_entity = world
            .spawn((Camera3d::default(), projection, EditorComponentGizmoRequest))
            .id();

        install_editor_component_gizmos(&mut world, &[runtime_entity, editor_entity]);

        assert!(world.get::<Gizmo>(runtime_entity).is_none());
        assert!(world.get::<EditorComponentGizmo>(runtime_entity).is_none());
        let editor_gizmo = world.get::<Gizmo>(editor_entity).expect("editor gizmo");
        let handle = editor_gizmo.handle.clone();
        assert!(
            world
                .get::<RenderLayers>(editor_entity)
                .unwrap()
                .intersects(&RenderLayers::layer(GIZMO_RENDER_LAYER))
        );
        let assets = world.resource::<Assets<GizmoAsset>>();
        let asset = assets.get(&handle).expect("retained gizmo asset");
        assert!(
            !asset.buffer().list_positions.is_empty() || !asset.buffer().strip_positions.is_empty()
        );

        for asset in [
            directional_light_component_gizmo(),
            point_light_component_gizmo(12.0),
            spot_light_component_gizmo(12.0, 15.0_f32.to_radians(), 30.0_f32.to_radians()),
        ] {
            assert!(
                !asset.buffer().list_positions.is_empty()
                    || !asset.buffer().strip_positions.is_empty()
            );
        }
    }

    #[test]
    fn pick_snapshot_skips_render_only_mesh_vertex_data() {
        let mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        );

        assert!(mesh_triangles(&mesh).is_empty());
    }

    #[test]
    fn viewport_telemetry_skips_render_only_mesh_vertex_data() {
        let mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        );

        assert_eq!(mesh_geometry_counts(&mesh), None);
    }

    fn wait_for_skin_ready(renderer: &mut EditorRenderApp, expected_joints: usize) {
        for _ in 0..600 {
            renderer.tick();
            if largest_skinned_joint_count(renderer.app.world_mut()) == Some(expected_joints)
                && mannequin_animation_player_ready(renderer.app.world_mut())
            {
                for _ in 0..10 {
                    renderer.tick();
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        panic!("timed out waiting for mannequin character skin and glTF animation player to load");
    }

    /// Writes a minimal animated skinned character so the mannequin preview
    /// machinery (skin import → animation player → joint motion) is testable
    /// on any checkout: two named joints, one triangle bound to the animated
    /// joint, identity inverse-bind matrices.
    fn write_synthetic_character_glb(asset_root: &std::path::Path, asset_path: &str) {
        let position: Vec<f32> = vec![-0.5, -0.5, 0.0, 0.5, -0.5, 0.0, 0.0, 0.5, 0.0];
        let joints: Vec<u8> = vec![1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0];
        let weights: Vec<f32> = vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        let indices: Vec<u16> = vec![0, 1, 2];
        let identity: Vec<f32> = vec![
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0,
            0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];

        let mut bin: Vec<u8> = Vec::new();
        let push_bytes = |bin: &mut Vec<u8>, bytes: &[u8]| {
            while !bin.len().is_multiple_of(4) {
                bin.push(0);
            }
            let offset = u32::try_from(bin.len()).expect("fixture buffer fits u32");
            bin.extend_from_slice(bytes);
            (
                offset,
                u32::try_from(bytes.len()).expect("fixture chunk fits u32"),
            )
        };

        let mut position_bytes = Vec::new();
        for value in &position {
            position_bytes.extend_from_slice(&value.to_le_bytes());
        }
        let positions_view = push_bytes(&mut bin, &position_bytes);

        let joints_view = push_bytes(&mut bin, &joints);

        let mut weight_bytes = Vec::new();
        for value in &weights {
            weight_bytes.extend_from_slice(&value.to_le_bytes());
        }
        let weights_view = push_bytes(&mut bin, &weight_bytes);

        let mut index_bytes = Vec::new();
        for index in &indices {
            index_bytes.extend_from_slice(&index.to_le_bytes());
        }
        let indices_view = push_bytes(&mut bin, &index_bytes);

        let mut ibm_bytes = Vec::new();
        for value in &identity {
            ibm_bytes.extend_from_slice(&value.to_le_bytes());
        }
        let ibm_view = push_bytes(&mut bin, &ibm_bytes);

        let json = serde_json::json!({
            "asset": { "version": "2.0", "generator": "az-editor synthetic fixture" },
            "scene": 0,
            "scenes": [{ "nodes": [0, 1] }],
            "nodes": [
                { "name": "synthetic-mesh", "mesh": 0, "skin": 0 },
                { "name": "joint-root", "children": [2] },
                { "name": "joint-swing" }
            ],
            "skins": [{
                "name": "synthetic-skin",
                "inverseBindMatrices": 4,
                "joints": [1, 2]
            }],
            "meshes": [{
                "primitives": [{
                    "attributes": { "POSITION": 0, "JOINTS_0": 1, "WEIGHTS_0": 2 },
                    "indices": 3
                }]
            }],
            "accessors": [
                { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                  "min": [-0.5, -0.5, 0.0], "max": [0.5, 0.5, 0.0] },
                { "bufferView": 1, "componentType": 5121, "count": 3, "type": "VEC4" },
                { "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC4" },
                { "bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR" },
                { "bufferView": 4, "componentType": 5126, "count": 2, "type": "MAT4" }
            ],
            "bufferViews": [
                { "buffer": 0, "byteOffset": positions_view.0, "byteLength": positions_view.1, "target": 34962 },
                { "buffer": 0, "byteOffset": joints_view.0, "byteLength": joints_view.1, "target": 34962 },
                { "buffer": 0, "byteOffset": weights_view.0, "byteLength": weights_view.1, "target": 34962 },
                { "buffer": 0, "byteOffset": indices_view.0, "byteLength": indices_view.1, "target": 34963 },
                { "buffer": 0, "byteOffset": ibm_view.0, "byteLength": ibm_view.1 }
            ],
            "buffers": [{ "byteLength": u32::try_from(bin.len()).expect("fixture buffer fits u32") }]
        });

        write_glb_file(asset_root, asset_path, &json, &bin);
    }

    /// Writes an animation-only `.anim.glb`: one rotation channel targeting
    /// the synthetic rig's named joint. The ramp spans ten minutes so a test
    /// window of seconds always lands on moving poses instead of a clamped
    /// final keyframe.
    fn write_synthetic_motion_glb(asset_root: &std::path::Path, asset_path: &str) {
        let half_turn_z = 0.5_f32.sqrt();
        let mut bin: Vec<u8> = Vec::new();
        let push_bytes = |bin: &mut Vec<u8>, bytes: &[u8]| {
            while !bin.len().is_multiple_of(4) {
                bin.push(0);
            }
            let offset = u32::try_from(bin.len()).expect("fixture buffer fits u32");
            bin.extend_from_slice(bytes);
            (
                offset,
                u32::try_from(bytes.len()).expect("fixture chunk fits u32"),
            )
        };

        let mut input_bytes = Vec::new();
        for value in [0.0_f32, 600.0] {
            input_bytes.extend_from_slice(&value.to_le_bytes());
        }
        let input_view = push_bytes(&mut bin, &input_bytes);

        let mut output_bytes = Vec::new();
        for quat in [
            [0.0_f32, 0.0, 0.0, 1.0],
            [0.0, 0.0, half_turn_z, half_turn_z],
        ] {
            for value in quat {
                output_bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        let output_view = push_bytes(&mut bin, &output_bytes);

        let json = serde_json::json!({
            "asset": { "version": "2.0", "generator": "az-editor synthetic fixture" },
            "scene": 0,
            "scenes": [{ "nodes": [0] }],
            "nodes": [
                { "name": "joint-root", "children": [1] },
                { "name": "joint-swing" }
            ],
            "accessors": [
                { "bufferView": 0, "componentType": 5126, "count": 2, "type": "SCALAR",
                  "min": [0.0], "max": [600.0] },
                { "bufferView": 1, "componentType": 5126, "count": 2, "type": "VEC4" }
            ],
            "bufferViews": [
                { "buffer": 0, "byteOffset": input_view.0, "byteLength": input_view.1 },
                { "buffer": 0, "byteOffset": output_view.0, "byteLength": output_view.1 }
            ],
            "buffers": [{ "byteLength": u32::try_from(bin.len()).expect("fixture buffer fits u32") }],
            "animations": [{
                "channels": [{ "sampler": 0, "target": { "node": 1, "path": "rotation" } }],
                "samplers": [{ "input": 0, "output": 1, "interpolation": "LINEAR" }]
            }]
        });

        write_glb_file(asset_root, asset_path, &json, &bin);
    }

    fn write_glb_file(
        asset_root: &std::path::Path,
        asset_path: &str,
        json: &serde_json::Value,
        bin: &[u8],
    ) {
        let mut json_bytes = serde_json::to_vec(json).expect("serialize glTF json");
        while !json_bytes.len().is_multiple_of(4) {
            json_bytes.push(0x20);
        }
        let mut bin_bytes = bin.to_vec();
        while !bin_bytes.len().is_multiple_of(4) {
            bin_bytes.push(0);
        }
        let total_bytes = 12 + 8 + json_bytes.len() + 8 + bin_bytes.len();
        let total_length = u32::try_from(total_bytes).expect("fixture glb fits u32");

        let mut out = Vec::with_capacity(total_bytes);
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2_u32.to_le_bytes());
        out.extend_from_slice(&total_length.to_le_bytes());
        out.extend_from_slice(
            &u32::try_from(json_bytes.len())
                .expect("fixture json chunk fits u32")
                .to_le_bytes(),
        );
        out.extend_from_slice(&0x4E_4F_53_4A_u32.to_le_bytes()); // JSON chunk
        out.extend_from_slice(&json_bytes);
        out.extend_from_slice(
            &u32::try_from(bin_bytes.len())
                .expect("fixture bin chunk fits u32")
                .to_le_bytes(),
        );
        out.extend_from_slice(&0x00_4E_49_42_u32.to_le_bytes()); // BIN chunk
        out.extend_from_slice(&bin_bytes);

        let path = asset_root.join(asset_path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, out).unwrap();
    }

    #[test]
    fn editor_render_mannequin_preview_loads_synthetic_animated_skin_and_animates_joint() {
        let temp = tempfile::tempdir().unwrap();
        let asset_root = temp.path().join("assets");
        write_synthetic_character_glb(&asset_root, DEFAULT_MANNEQUIN_CHARACTER_GLB);
        write_synthetic_motion_glb(&asset_root, DEFAULT_MANNEQUIN_MOTION_GLB);

        let preview = EditorMannequinPreview::default_for_project_asset_root(asset_root);
        let mut renderer = EditorRenderApp::new_with_mannequin_preview(320, 240, preview)
            .expect("create in-process editor render app");
        renderer
            .app
            .world_mut()
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                16,
            )));

        wait_for_skin_ready(&mut renderer, EXPECTED_SYNTHETIC_JOINTS);

        let before = mannequin_joint_snapshots(renderer.app.world_mut());
        assert_eq!(before.len(), EXPECTED_SYNTHETIC_JOINTS);

        for _ in 0..90 {
            renderer.tick();
        }

        let after = mannequin_joint_snapshots(renderer.app.world_mut());
        assert_eq!(after.len(), EXPECTED_SYNTHETIC_JOINTS);

        let moved_joint = before
            .iter()
            .zip(after.iter())
            .filter(|(left, right)| left.entity == right.entity)
            .map(|(left, right)| matrix_delta(left.matrix, right.matrix))
            .any(|delta| delta > 0.0001);
        assert!(
            moved_joint,
            "expected the synthetic rig's animated joint to move after ticks"
        );
    }

    fn largest_skinned_joint_count(world: &mut World) -> Option<usize> {
        let mut skinned_meshes = world.query::<&SkinnedMesh>();
        skinned_meshes
            .iter(world)
            .map(|skinned_mesh| skinned_mesh.joints.len())
            .max()
    }

    fn mannequin_animation_player_ready(world: &mut World) -> bool {
        let playing_clips: Vec<Handle<AnimationClip>> = {
            let mut players = world.query::<(
                &AnimationPlayer,
                &AnimationGraphHandle,
                &MannequinPreviewPlayer,
            )>();
            players
                .iter(world)
                .filter(|(player, _, _)| {
                    player
                        .playing_animations()
                        .any(|(_, animation)| !animation.is_paused())
                })
                .flat_map(|(_, _, preview_player)| preview_player.clips().to_vec())
                .collect()
        };
        let clips = world.resource::<Assets<AnimationClip>>();
        playing_clips.iter().any(|clip| clips.get(clip).is_some())
    }

    #[derive(Clone, Debug)]
    struct JointSnapshot {
        entity: Entity,
        matrix: [f32; 16],
    }

    fn mannequin_joint_snapshots(world: &mut World) -> Vec<JointSnapshot> {
        let joints = {
            let mut skinned_meshes = world.query::<&SkinnedMesh>();
            skinned_meshes
                .iter(world)
                .max_by_key(|skinned_mesh| skinned_mesh.joints.len())
                .map(|skinned_mesh| skinned_mesh.joints.clone())
                .unwrap_or_default()
        };

        joints
            .into_iter()
            .filter_map(|entity| {
                let transform = world.get::<GlobalTransform>(entity)?;
                Some(JointSnapshot {
                    entity,
                    matrix: transform.to_matrix().to_cols_array(),
                })
            })
            .collect()
    }

    fn matrix_delta(left: [f32; 16], right: [f32; 16]) -> f32 {
        left.into_iter()
            .zip(right)
            .map(|(left, right)| (left - right).abs())
            .fold(0.0, f32::max)
    }

    #[test]
    fn snap_scalar_quantizes_to_step_and_passes_through_when_disabled() {
        assert!((snap_scalar(0.62, 0.5) - 0.5).abs() < 1e-6);
        assert!((snap_scalar(0.80, 0.5) - 1.0).abs() < 1e-6);
        assert!((snap_scalar(-0.20, 0.5) - 0.0).abs() < 1e-6);
        assert!((snap_scalar(-0.30, 0.5) - (-0.5)).abs() < 1e-6);
        // Disabled snapping (non-positive step) is a pass-through.
        assert!((snap_scalar(0.62, 0.0) - 0.62).abs() < 1e-6);
        assert!((snap_scalar(0.62, -1.0) - 0.62).abs() < 1e-6);
        let snapped_angle = snap_scalar(23.0_f32.to_radians(), 15.0_f32.to_radians());
        assert!((snapped_angle.to_degrees() - 30.0).abs() < 1e-5);
    }

    #[test]
    fn constant_screen_scale_preserves_perspective_pixel_size() {
        let projection = Projection::Perspective(PerspectiveProjection {
            fov: 60.0_f32.to_radians(),
            ..default()
        });
        let viewport_height = 1_080_u32;
        let desired_pixels = 92.0;
        for distance in [2.0, 20.0, 200.0] {
            let scale = constant_screen_scale(
                &projection,
                Vec3::new(0.0, 0.0, distance),
                Vec3::ZERO,
                viewport_height,
                desired_pixels,
            );
            let projected_pixels = scale * extent_pixels(viewport_height)
                / (2.0 * distance * (30.0_f32.to_radians()).tan());
            assert!((projected_pixels - desired_pixels).abs() < 0.001);
        }
    }

    #[test]
    fn constant_screen_scale_uses_orthographic_area() {
        let mut orthographic = OrthographicProjection::default_3d();
        orthographic.area = Rect::from_corners(Vec2::new(-8.0, -4.0), Vec2::new(8.0, 4.0));
        let projection = Projection::Orthographic(orthographic);
        let scale = constant_screen_scale(
            &projection,
            Vec3::new(0.0, 0.0, 10.0),
            Vec3::ZERO,
            800,
            100.0,
        );
        assert!((scale - 1.0).abs() < 1e-6);
    }

    #[test]
    fn gizmo_commit_value_returns_the_edited_local_channel_for_every_mode() {
        let transform = Transform {
            translation: Vec3::new(1.0, 2.0, 3.0),
            rotation: Quat::from_rotation_y(0.75),
            scale: Vec3::new(2.0, 3.0, 4.0),
        };

        assert_eq!(
            gizmo_commit_value(GizmoMode::Translate, transform),
            Some(GizmoCommitValue::Position(transform.translation))
        );
        assert_eq!(
            gizmo_commit_value(GizmoMode::Rotate, transform),
            Some(GizmoCommitValue::Rotation(transform.rotation))
        );
        assert_eq!(
            gizmo_commit_value(GizmoMode::Scale, transform),
            Some(GizmoCommitValue::Scale(transform.scale))
        );
        assert_eq!(gizmo_commit_value(GizmoMode::None, transform), None);
    }

    #[test]
    fn world_gizmo_deltas_convert_back_to_parent_relative_trs() {
        let parent = GlobalTransform::from(Transform {
            translation: Vec3::new(10.0, 0.0, -2.0),
            rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            scale: Vec3::splat(2.0),
        });
        let start_local = Vec3::new(1.0, 2.0, 3.0);
        let world_delta = parent.affine().transform_vector3(Vec3::X * 4.0);
        assert!(
            local_translation_after_world_delta(start_local, parent, world_delta)
                .abs_diff_eq(start_local + Vec3::X * 4.0, 1e-5)
        );

        let local_rotation = Quat::from_rotation_x(0.4);
        let global_rotation = parent.rotation() * local_rotation;
        assert!(
            local_rotation_from_global(parent, global_rotation).abs_diff_eq(local_rotation, 1e-5)
        );
    }

    #[test]
    fn axis_drag_plane_normal_is_perpendicular_to_axis_and_faces_view() {
        let axis = Vec3::X;
        let view = Vec3::new(0.2, -0.5, -1.0).normalize();
        let normal = axis_drag_plane_normal(axis, view).expect("plane normal");
        // Perpendicular to the constrained axis.
        assert!(normal.dot(axis).abs() < 1e-5);
        // Faces the viewer (positive projection onto the view direction).
        assert!(normal.dot(view) > 0.0);
        // Degenerate when the view is parallel to the axis.
        assert!(axis_drag_plane_normal(Vec3::X, Vec3::X).is_none());
    }

    #[test]
    fn ray_plane_intersection_hits_and_rejects_parallel_rays() {
        // Ray straight down onto the ground plane (y = 0).
        let hit =
            ray_plane_intersection(Vec3::new(1.0, 4.0, -2.0), Vec3::NEG_Y, Vec3::ZERO, Vec3::Y)
                .expect("hit");
        assert!((hit - Vec3::new(1.0, 0.0, -2.0)).length() < 1e-5);
        // Ray parallel to the plane never intersects.
        assert!(
            ray_plane_intersection(Vec3::new(0.0, 4.0, 0.0), Vec3::X, Vec3::ZERO, Vec3::Y)
                .is_none()
        );
    }

    #[test]
    fn axis_constrained_delta_recovers_world_offset_between_two_rays() {
        // Camera looking down -Z at a gizmo on the X axis; two vertical rays a
        // known distance apart along X must yield exactly that delta.
        let axis_origin = Vec3::ZERO;
        let axis_dir = Vec3::X;
        let view_dir = Vec3::NEG_Z;
        let ray_dir = Vec3::NEG_Z;
        let start = axis_drag_offset(
            Vec3::new(2.0, 0.0, 5.0),
            ray_dir,
            axis_origin,
            axis_dir,
            view_dir,
        )
        .expect("start offset");
        let end = axis_drag_offset(
            Vec3::new(4.5, 0.0, 5.0),
            ray_dir,
            axis_origin,
            axis_dir,
            view_dir,
        )
        .expect("end offset");
        assert!((start - 2.0).abs() < 1e-4);
        assert!((end - 4.5).abs() < 1e-4);
        assert!(((end - start) - 2.5).abs() < 1e-4);
    }

    #[test]
    fn rotation_drag_angle_measures_signed_turn_in_plane() {
        // Rotation about +Y; two rays hitting the XZ plane at +X then +Z.
        // Going +X -> +Z about +Y is a -90 degree (clockwise) signed turn.
        let center = Vec3::ZERO;
        let normal = Vec3::Y;
        let angle = rotation_drag_angle(
            Vec3::new(2.0, 5.0, 0.0),
            Vec3::NEG_Y,
            Vec3::new(0.0, 5.0, 2.0),
            Vec3::NEG_Y,
            center,
            normal,
        )
        .expect("angle");
        assert!((angle + std::f32::consts::FRAC_PI_2).abs() < 1e-4);
    }
}
