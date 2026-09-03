//! Resolves cooked motion assets into runtime [`BevyMotion`]s.
//!
//! `CryEngine` keeps one [`ModelAnimationHeader`] per animation in a character's
//! `CAnimationSet`, and that header's `m_nAssetType` is what selects between a
//! plain clip (`CAF_File`) and a parametric group (`LMG_File`)
//! (Lumberyard reference: `dev/Gems/CryLegacy/Code/Source/CryAnimation/ModelAnimationSet.h:23-28,91-98`).
//! Every consumer branches on it and then
//! indexes `m_nGlobalAnimId` into either `g_AnimationManager.m_arrGlobalCAF` or
//! `m_arrGlobalLMG` — `CSkeletonAnim::UpdateParameters`
//! (Lumberyard reference: `dev/Gems/CryLegacy/Code/Source/CryAnimation/SkeletonAnim_BlendMan.cpp:251`),
//! `CSkeletonAnim::AnimCallback` at line 713, and `StartAnimation`'s queue push
//! (Lumberyard reference: `dev/Gems/CryLegacy/Code/Source/CryAnimation/SkeletonAnim_Queue.cpp:475`)
//! all do it.
//!
//! This module is the ECS spelling of that header. One component names a motion
//! by catalog identity; the catalog's recorded [`AssetType`] plays the part of
//! `m_nAssetType`, and one resolution pipeline turns the entry into either a
//! direct or a parametric [`BevyMotion`]. The two kinds are deliberately not
//! separate paths: `CryEngine` resolves both through the same animation-set
//! entry and only the runtime clock and event window differ afterwards, which is
//! exactly how [`BevyMotion`] is shaped.
//!
//! The pipeline mirrors this crate's character-definition runtime
//! (`character_runtime.rs`: `CharacterDefinitionId` →
//! `PendingCharacterDefinition` → `CharacterInstance` / `CharacterInstanceError`)
//! because that is how every other cooked asset in this gem crosses from the
//! catalog into the ECS.
//!
//! # Hot reload
//!
//! Resolution runs once per request and is not re-run when a product changes on
//! disk. That matches `character_runtime.rs`, which also resolves once, and it
//! matches `CryEngine`'s granularity: a hot-loaded parametric group is rebuilt
//! by reloading the whole `CAnimationSet`, not by patching one entry — the
//! parameterizer only notices afterwards, and then only to log
//! (`ParametricSampler.cpp:68-72`, "Usually we can't start invalid `ParaGroups`.
//! So this must be the result of Hot-Loading"). Per-entry invalidation would be
//! new behaviour with no counterpart on either side, so it is deliberately left
//! out; removing and re-inserting [`CryMotionId`] re-runs the pipeline.
//!
//! [`ModelAnimationHeader`]: https://github.com/aws/lumberyard

use az_animation::{
    blend_space_asset::{BlendSpaceAsset, CombinedBlendSpaceAsset},
    ids,
};
use az_core::{AssetId, AssetType};
use az_framework::asset::{AssetCatalog, AssetRefLoadError};
use bevy::{
    animation::{AnimationClip, graph::AnimationMask},
    asset::{AssetServer, Assets, Handle},
    ecs::system::SystemParam,
    gltf::{Gltf, GltfNode},
    prelude::*,
};

use crate::{BevyMotion, BevyMotionSource, BevyParametricSource, MotionResolveError};

/// Selects one cooked motion for an ECS entity.
///
/// The identity is resolved through the [`AssetCatalog`], which is also the
/// authority on which of the three motion asset types it names. A direct motion
/// product is `CryEngine`'s `CAF_File`; a blend space or combined blend space is
/// its `LMG_File`.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CryMotionId {
    asset_id: AssetId,
    mask: AnimationMask,
}

impl CryMotionId {
    #[must_use]
    pub const fn new(asset_id: AssetId) -> Self {
        Self { asset_id, mask: 0 }
    }

    /// Restricts the resolved clips to one Bevy animation mask group.
    ///
    /// `CryEngine` carries the equivalent restriction on the queued animation
    /// rather than on the animation-set entry, but Bevy binds a mask per graph
    /// clip node, so it has to be known when the clip is bound.
    #[must_use]
    pub const fn with_mask(mut self, mask: AnimationMask) -> Self {
        self.mask = mask;
        self
    }

    #[must_use]
    pub const fn asset_id(self) -> AssetId {
        self.asset_id
    }

    #[must_use]
    pub const fn mask(self) -> AnimationMask {
        self.mask
    }
}

impl From<AssetId> for CryMotionId {
    fn from(asset_id: AssetId) -> Self {
        Self::new(asset_id)
    }
}

impl AsRef<AssetId> for CryMotionId {
    fn as_ref(&self) -> &AssetId {
        &self.asset_id
    }
}

/// A motion that has been classified but whose products are still loading.
///
/// A blend space needs two loads to become playable — its own compiled product,
/// then one motion product per example — so the pending state is a small state
/// machine rather than a single handle. Direct motions skip straight to
/// [`Self::Direct`].
#[derive(Component, Debug, Clone)]
enum PendingCryMotion {
    /// `CAF_File`: one motion product, waiting on its glTF, nodes and clip.
    Direct(BevyMotionSource),
    /// `LMG_File`: waiting on the compiled blend space itself.
    BlendSpace {
        handle: Handle<BlendSpaceAsset>,
        mask: AnimationMask,
    },
    /// `LMG_File` with sub-spaces: waiting on the compiled combined blend space.
    CombinedBlendSpace {
        handle: Handle<CombinedBlendSpaceAsset>,
        mask: AnimationMask,
    },
    /// Either blend-space kind, now waiting on every example's motion product.
    Parametric(BevyParametricSource),
}

/// A resolved motion, ready to be queued onto a [`CryAnimationPlayer`].
///
/// [`CryAnimationPlayer`]: crate::CryAnimationPlayer
#[derive(Component, Debug, Clone, PartialEq)]
pub struct CryMotion(BevyMotion);

impl CryMotion {
    #[must_use]
    pub const fn motion(&self) -> &BevyMotion {
        &self.0
    }

    #[must_use]
    pub fn into_motion(self) -> BevyMotion {
        self.0
    }
}

impl AsRef<BevyMotion> for CryMotion {
    fn as_ref(&self) -> &BevyMotion {
        &self.0
    }
}

/// Terminal resolution failure for one requested motion.
///
/// Every variant means the request will never succeed as authored, so the
/// pipeline stops retrying and leaves the reason on the entity. Transient
/// "still loading" is not represented here — it is simply the absence of both
/// [`CryMotion`] and `CryMotionError`.
#[derive(Component, Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CryMotionError {
    #[error("motion asset: {0}")]
    Catalog(AssetRefLoadError),
    #[error("catalog asset {asset_id} has type {asset_type}, which is not a motion asset")]
    UnsupportedAssetType {
        asset_id: AssetId,
        asset_type: AssetType,
    },
    #[error("blend space {asset_id} cannot resolve animation-set alias `{alias}`")]
    UnresolvableExample {
        asset_id: AssetId,
        alias: az_core::name::AzNameCaseInsensitive,
    },
    #[error("motion {asset_id}: {source}")]
    Product {
        asset_id: AssetId,
        #[source]
        source: MotionResolveError,
    },
}

/// The asset stores the second stage reads.
///
/// Grouped into one [`SystemParam`] so the system keeps a readable argument
/// list, and held as `Option` for the same reason `character_runtime` does: a
/// headless host that never installed the glTF plugins should idle, not panic.
#[derive(SystemParam)]
struct MotionAssetStores<'w> {
    blend_spaces: Option<Res<'w, Assets<BlendSpaceAsset>>>,
    combined_blend_spaces: Option<Res<'w, Assets<CombinedBlendSpaceAsset>>>,
    gltfs: Option<Res<'w, Assets<Gltf>>>,
    nodes: Option<Res<'w, Assets<GltfNode>>>,
    clips: Option<Res<'w, Assets<AnimationClip>>>,
}

#[expect(
    clippy::redundant_pub_crate,
    reason = "lib.rs re-exports this module with `pub use`, so `pub(crate)` is what actually keeps the registration hook out of the crate API"
)]
pub(crate) fn register_motion_runtime(app: &mut App) {
    app.add_systems(
        Update,
        (resolve_cry_motion_sources, build_resolved_cry_motions).chain(),
    );
}

/// Classifies each requested motion the way `ModelAnimationHeader::m_nAssetType`
/// does, and starts the load for whichever product kind it turned out to be.
#[expect(
    clippy::type_complexity,
    reason = "a Bevy query filter tuple has no shorter spelling"
)]
fn resolve_cry_motion_sources(
    mut commands: Commands,
    catalog: Option<Res<AssetCatalog>>,
    asset_server: Option<Res<AssetServer>>,
    requests: Query<
        (Entity, &CryMotionId),
        (
            Without<PendingCryMotion>,
            Without<CryMotion>,
            Without<CryMotionError>,
        ),
    >,
) {
    let (Some(catalog), Some(asset_server)) = (catalog, asset_server) else {
        return;
    };

    for (entity, request) in &requests {
        let asset_id = request.asset_id();
        let mask = request.mask();
        let Some(entry) = catalog.get_by_id(asset_id) else {
            commands
                .entity(entity)
                .insert(CryMotionError::Catalog(AssetRefLoadError::Missing {
                    asset_id,
                }));
            continue;
        };

        let asset_type = entry.asset_type();
        let pending = if asset_type == ids::ANIMATION {
            // A motion product carries exactly one animation, so the direct
            // path always binds animation zero — the same assumption the
            // blend-space example wiring makes.
            PendingCryMotion::Direct(BevyMotionSource::load(
                &asset_server,
                entry.relative_path().to_path_buf(),
                0,
                mask,
            ))
        } else if asset_type == ids::BLEND_SPACE {
            PendingCryMotion::BlendSpace {
                handle: asset_server.load(entry.relative_path().to_path_buf()),
                mask,
            }
        } else if asset_type == ids::COMBINED_BLEND_SPACE {
            PendingCryMotion::CombinedBlendSpace {
                handle: asset_server.load(entry.relative_path().to_path_buf()),
                mask,
            }
        } else {
            commands
                .entity(entity)
                .insert(CryMotionError::UnsupportedAssetType {
                    asset_id,
                    asset_type,
                });
            continue;
        };
        commands.entity(entity).insert(pending);
    }
}

/// Advances every pending motion one stage per frame, ending in either a
/// [`CryMotion`] or a [`CryMotionError`].
#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value"
)]
fn build_resolved_cry_motions(
    mut commands: Commands,
    asset_server: Option<Res<AssetServer>>,
    stores: MotionAssetStores,
    pending: Query<(Entity, &CryMotionId, &PendingCryMotion)>,
) {
    let Some(asset_server) = asset_server else {
        return;
    };

    for (entity, request, pending) in &pending {
        let asset_id = request.asset_id();
        match pending {
            PendingCryMotion::BlendSpace { handle, mask } => {
                let Some(assets) = stores.blend_spaces.as_deref() else {
                    continue;
                };
                let Some(asset) = assets.get(handle) else {
                    continue;
                };
                match BevyParametricSource::from_blend_space(asset_id, asset, &asset_server, *mask)
                {
                    Some(source) => {
                        commands
                            .entity(entity)
                            .insert(PendingCryMotion::Parametric(source));
                    }
                    None => fail(
                        &mut commands,
                        entity,
                        CryMotionError::UnresolvableExample {
                            asset_id,
                            alias: unresolvable_example_alias(&asset.motions),
                        },
                    ),
                }
            }
            PendingCryMotion::CombinedBlendSpace { handle, mask } => {
                let Some(assets) = stores.combined_blend_spaces.as_deref() else {
                    continue;
                };
                let Some(asset) = assets.get(handle) else {
                    continue;
                };
                match BevyParametricSource::from_combined_blend_space(
                    asset_id,
                    asset,
                    &asset_server,
                    *mask,
                ) {
                    Some(source) => {
                        commands
                            .entity(entity)
                            .insert(PendingCryMotion::Parametric(source));
                    }
                    None => fail(
                        &mut commands,
                        entity,
                        CryMotionError::UnresolvableExample {
                            asset_id,
                            alias: unresolvable_example_alias(&asset.motions),
                        },
                    ),
                }
            }
            PendingCryMotion::Direct(source) => {
                let Some((gltfs, nodes, clips)) = stores.gltf_stores() else {
                    continue;
                };
                finish(
                    &mut commands,
                    entity,
                    asset_id,
                    source.resolve(gltfs, nodes, clips),
                );
            }
            PendingCryMotion::Parametric(source) => {
                let Some((gltfs, nodes, clips)) = stores.gltf_stores() else {
                    continue;
                };
                finish(
                    &mut commands,
                    entity,
                    asset_id,
                    source.resolve(gltfs, nodes, clips),
                );
            }
        }
    }
}

impl MotionAssetStores<'_> {
    fn gltf_stores(&self) -> Option<(&Assets<Gltf>, &Assets<GltfNode>, &Assets<AnimationClip>)> {
        Some((
            self.gltfs.as_deref()?,
            self.nodes.as_deref()?,
            self.clips.as_deref()?,
        ))
    }
}

/// Installs a resolved motion, or records why it can never resolve. A
/// [`MotionResolveError::Pending`] leaves the entity untouched so the next
/// frame tries again.
fn finish(
    commands: &mut Commands,
    entity: Entity,
    asset_id: AssetId,
    resolved: Result<BevyMotion, MotionResolveError>,
) {
    match resolved {
        Ok(motion) => {
            commands
                .entity(entity)
                .insert(CryMotion(motion.with_asset_id(asset_id)))
                .remove::<PendingCryMotion>();
        }
        Err(error) if error.is_pending() => {}
        Err(source) => fail(
            commands,
            entity,
            CryMotionError::Product { asset_id, source },
        ),
    }
}

fn fail(commands: &mut Commands, entity: Entity, error: CryMotionError) {
    commands
        .entity(entity)
        .insert(error)
        .remove::<PendingCryMotion>();
}

fn unresolvable_example_alias(
    motions: &[az_animation::blend_space_asset::BlendSpaceMotion],
) -> az_core::name::AzNameCaseInsensitive {
    motions
        .iter()
        .find(|motion| crate::motion_product_path(&motion.animation).is_none())
        .map(|motion| motion.animation.alias.clone())
        .expect("parametric source construction failed only for an unresolvable example")
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_animation::{
        animation_set::{AnimationMotionRef, AnimationRef},
        blend_space::{
            BlendSpaceDimension, DirectDeltaMotion, MotionParameterId, ParametricBlendSpace,
            ParametricBlendSpaceDescription, VirtualExample,
        },
        blend_space_asset::BlendSpaceMotion,
        controller_target::CONTROLLER_TARGET_SPACE,
    };
    use az_asset::{
        ASSET_CATALOG_FILE_NAME, AssetCatalog as AssetCatalogFile,
        AssetCatalogEntry as AssetCatalogFileEntry, write_asset_catalog,
    };
    use bevy::{
        animation::{
            AnimationTargetId, animated_field,
            prelude::{AnimatableCurve, AnimatableKeyframeCurve},
        },
        app::TaskPoolPlugin,
        asset::{AssetApp, AssetPlugin},
        math::Vec3,
        transform::components::Transform,
    };
    use tempfile::TempDir;

    use crate::controller_animation_target_root_id;

    const DIRECT_PATH: &str = "animations/player/idle.motion.glb";
    const BLEND_SPACE_PATH: &str = "animations/player/locomotion.blend-space.bin";
    const COMBINED_PATH: &str = "animations/player/locomotion.combined-blend-space.bin";
    const FIRST_EXAMPLE_PATH: &str = "animations/player/idle.motion.glb";
    const SECOND_EXAMPLE_PATH: &str = "animations/player/run.motion.glb";

    fn asset_id(byte: u8) -> AssetId {
        AssetId::new(uuid::Uuid::from_bytes([byte; 16]), 0)
    }

    fn catalog_entry(id: u8, asset_type: AssetType, path: &str) -> AssetCatalogFileEntry {
        AssetCatalogFileEntry::new(
            asset_id(id),
            uuid::Uuid::from(asset_type),
            "az.test.motion",
            1,
            path,
            Some(1),
            0,
            [0u8; 32],
        )
    }

    /// A real on-disk catalog, because [`AssetCatalog`] is the thing under test:
    /// the recorded asset type is what stands in for
    /// `ModelAnimationHeader::m_nAssetType`.
    fn motion_app(entries: Vec<AssetCatalogFileEntry>) -> (TempDir, App) {
        let temp = tempfile::tempdir().expect("temp asset root");
        let file = AssetCatalogFile::new(entries).expect("catalog");
        let mut out =
            std::fs::File::create(temp.path().join(ASSET_CATALOG_FILE_NAME)).expect("catalog file");
        write_asset_catalog(&file, &mut out).expect("write catalog");
        drop(out);
        let catalog = AssetCatalog::open_native(temp.path()).expect("open catalog");

        let mut app = App::new();
        app.add_plugins((
            TaskPoolPlugin::default(),
            AssetPlugin {
                file_path: temp.path().to_string_lossy().into_owned(),
                ..AssetPlugin::default()
            },
        ))
        .init_asset::<Gltf>()
        .init_asset::<GltfNode>()
        .init_asset::<AnimationClip>()
        .init_asset::<BlendSpaceAsset>()
        .init_asset::<CombinedBlendSpaceAsset>()
        .insert_resource(catalog);
        register_motion_runtime(&mut app);
        (temp, app)
    }

    /// A one-node, one-animation GLB with the extras the animation builder
    /// writes: one second at 30 Hz, no segment events, and a root translation
    /// track that travels two units along glTF +Y.
    fn motion_glb() -> Vec<u8> {
        let mut bin = Vec::new();
        for time in [0.0f32, 1.0] {
            bin.extend_from_slice(&time.to_le_bytes());
        }
        for value in [[0.0f32, 0.0, 0.0], [0.0, 2.0, 0.0]] {
            for component in value {
                bin.extend_from_slice(&component.to_le_bytes());
            }
        }
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
"nodes":[{{"name":"root"}}],"buffers":[{{"byteLength":{}}}],
"bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":8}},
{{"buffer":0,"byteOffset":8,"byteLength":24}}],
"accessors":[{{"bufferView":0,"componentType":5126,"count":2,"type":"SCALAR","min":[0.0],"max":[1.0]}},
{{"bufferView":1,"componentType":5126,"count":2,"type":"VEC3"}}],
"animations":[{{"name":"motion","samplers":[{{"input":0,"output":1,"interpolation":"LINEAR"}}],
"channels":[{{"sampler":0,"target":{{"node":0,"path":"translation"}}}}],
"extras":{{"cryDuration":1.0,"crySampleRate":30.0,"cryEvents":[],
"azothAnimationTargetSpace":"{CONTROLLER_TARGET_SPACE}"}}}}]}}"#,
            bin.len()
        );
        let mut json = json.into_bytes();
        while json.len() % 4 != 0 {
            json.push(b' ');
        }

        let mut glb = Vec::with_capacity(28 + json.len() + bin.len());
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2u32.to_le_bytes());
        let total = u32::try_from(12 + 8 + json.len() + 8 + bin.len())
            .expect("synthetic glb is far smaller than 4 GiB");
        let json_length =
            u32::try_from(json.len()).expect("synthetic glb JSON chunk is far smaller than 4 GiB");
        let bin_length =
            u32::try_from(bin.len()).expect("synthetic glb BIN chunk is far smaller than 4 GiB");
        glb.extend_from_slice(&total.to_le_bytes());
        glb.extend_from_slice(&json_length.to_le_bytes());
        glb.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
        glb.extend_from_slice(&json);
        glb.extend_from_slice(&bin_length.to_le_bytes());
        glb.extend_from_slice(&0x004E_4942u32.to_le_bytes());
        glb.extend_from_slice(&bin);
        glb
    }

    /// Publishes the three assets Bevy's glTF loader would have produced for
    /// `handle`, so the resolution stage runs against a real glTF document
    /// without waiting on asynchronous file loads.
    fn publish_motion_product(app: &mut App, handle: &Handle<Gltf>) {
        let root_target = AnimationTargetId::from_name(&Name::new("root"));
        let mut clip = AnimationClip::default();
        clip.add_curve_to_target(
            root_target,
            AnimatableCurve::new(
                animated_field!(Transform::translation),
                AnimatableKeyframeCurve::new([(0.0, Vec3::ZERO), (1.0, Vec3::new(0.0, 2.0, 0.0))])
                    .unwrap(),
            ),
        );
        let clip = app
            .world_mut()
            .resource_mut::<Assets<AnimationClip>>()
            .add(clip);
        let node = app
            .world_mut()
            .resource_mut::<Assets<GltfNode>>()
            .add(GltfNode {
                index: 0,
                name: "root".to_owned(),
                children: Vec::new(),
                mesh: None,
                skin: None,
                transform: Transform::IDENTITY,
                is_animation_root: true,
                extras: None,
            });
        let source = gltf::Gltf::from_slice(&motion_glb()).expect("synthetic glb parses");
        app.world_mut()
            .resource_mut::<Assets<Gltf>>()
            .insert(handle, gltf_asset(vec![node], vec![clip], Some(source)))
            .expect("published motion product");
    }

    fn gltf_asset(
        nodes: Vec<Handle<GltfNode>>,
        animations: Vec<Handle<AnimationClip>>,
        source: Option<gltf::Gltf>,
    ) -> Gltf {
        Gltf {
            scenes: Vec::new(),
            named_scenes: bevy::platform::collections::HashMap::default(),
            meshes: Vec::new(),
            named_meshes: bevy::platform::collections::HashMap::default(),
            materials: Vec::new(),
            named_materials: bevy::platform::collections::HashMap::default(),
            nodes,
            named_nodes: bevy::platform::collections::HashMap::default(),
            skins: Vec::new(),
            named_skins: bevy::platform::collections::HashMap::default(),
            default_scene: None,
            animations,
            named_animations: bevy::platform::collections::HashMap::default(),
            source,
        }
    }

    fn blend_space_asset(first: Option<&str>, second: Option<&str>) -> BlendSpaceAsset {
        let sampler = ParametricBlendSpace::try_from(ParametricBlendSpaceDescription {
            dimensions: vec![BlendSpaceDimension {
                parameter: MotionParameterId::TravelSpeed,
                min: 0.0,
                max: 1.0,
                cells: 2,
                locked: false,
            }],
            additional_extraction: Vec::new(),
            example_count: 2,
            example_parameters: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
            pseudo_examples: Vec::new(),
            faces: Vec::new(),
            virtual_examples: vec![
                VirtualExample {
                    indices: [0, 1].into_iter().collect(),
                    weights: [1.0, 0.0].into_iter().collect(),
                },
                VirtualExample {
                    indices: [0, 1].into_iter().collect(),
                    weights: [0.0, 1.0].into_iter().collect(),
                },
            ],
            threshold: None,
            idle_to_move: false,
        })
        .unwrap();
        BlendSpaceAsset {
            motions: vec![
                BlendSpaceMotion {
                    animation: AnimationRef::new(
                        "first",
                        AnimationMotionRef::new(
                            asset_id(0x10),
                            first.map(|path| path.replace(".motion.glb", ".anim.glb")),
                        ),
                    ),
                    playback_scale: 2.0,
                    direct_delta_motion: DirectDeltaMotion::from_dimensions([(true, 3.0)]),
                },
                BlendSpaceMotion {
                    animation: AnimationRef::new(
                        "second",
                        AnimationMotionRef::new(
                            asset_id(0x11),
                            second.map(|path| path.replace(".motion.glb", ".anim.glb")),
                        ),
                    ),
                    playback_scale: 0.5,
                    direct_delta_motion: DirectDeltaMotion::default(),
                },
            ],
            timewarp_group: Some("Locomotion".to_owned()),
            sampler,
        }
    }

    fn pending_of(app: &App, entity: Entity) -> PendingCryMotion {
        app.world()
            .entity(entity)
            .get::<PendingCryMotion>()
            .expect("motion is still pending")
            .clone()
    }

    /// `CryEngine` dispatches on `ModelAnimationHeader::m_nAssetType`
    /// (`ModelAnimationSet.h:23-28`); the catalog's asset type is this
    /// runtime's copy of that field, and it has to steer all three product
    /// kinds through one request component.
    #[test]
    fn catalog_asset_type_selects_the_direct_or_parametric_stage() {
        let (_temp, mut app) = motion_app(vec![
            catalog_entry(1, ids::ANIMATION, DIRECT_PATH),
            catalog_entry(2, ids::BLEND_SPACE, BLEND_SPACE_PATH),
            catalog_entry(3, ids::COMBINED_BLEND_SPACE, COMBINED_PATH),
        ]);
        let direct = app.world_mut().spawn(CryMotionId::new(asset_id(1))).id();
        let blend_space = app.world_mut().spawn(CryMotionId::new(asset_id(2))).id();
        let combined = app.world_mut().spawn(CryMotionId::new(asset_id(3))).id();

        app.update();

        assert!(matches!(
            pending_of(&app, direct),
            PendingCryMotion::Direct(_)
        ));
        assert!(matches!(
            pending_of(&app, blend_space),
            PendingCryMotion::BlendSpace { .. }
        ));
        assert!(matches!(
            pending_of(&app, combined),
            PendingCryMotion::CombinedBlendSpace { .. }
        ));
    }

    #[test]
    fn unknown_and_foreign_catalog_entries_fail_without_retrying() {
        let (_temp, mut app) = motion_app(vec![catalog_entry(
            4,
            az_animation::ids::CHARACTER_DEFINITION,
            "characters/player.character.bin",
        )]);
        let missing = app.world_mut().spawn(CryMotionId::new(asset_id(9))).id();
        let foreign = app.world_mut().spawn(CryMotionId::new(asset_id(4))).id();

        app.update();

        assert_eq!(
            app.world().entity(missing).get::<CryMotionError>(),
            Some(&CryMotionError::Catalog(AssetRefLoadError::Missing {
                asset_id: asset_id(9)
            }))
        );
        assert_eq!(
            app.world().entity(foreign).get::<CryMotionError>(),
            Some(&CryMotionError::UnsupportedAssetType {
                asset_id: asset_id(4),
                asset_type: az_animation::ids::CHARACTER_DEFINITION,
            })
        );
        assert!(
            app.world()
                .entity(missing)
                .get::<PendingCryMotion>()
                .is_none()
        );
        assert!(
            app.world()
                .entity(foreign)
                .get::<PendingCryMotion>()
                .is_none()
        );
    }

    /// The direct path is `CryEngine`'s `CAF_File` entry: one clip, its own
    /// timing, and a queue-ready state whose single weight is already pinned.
    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "the duration, sample rate and playback scale are copied verbatim out of the \
                  cooked product, so the assertion is that they arrive bit-for-bit unchanged"
    )]
    fn direct_motion_resolves_into_a_one_clip_runtime_motion() {
        let (_temp, mut app) = motion_app(vec![catalog_entry(1, ids::ANIMATION, DIRECT_PATH)]);
        let entity = app.world_mut().spawn(CryMotionId::new(asset_id(1))).id();
        app.update();
        let PendingCryMotion::Direct(source) = pending_of(&app, entity) else {
            panic!("direct motion did not take the direct stage");
        };
        publish_motion_product(&mut app, source.as_ref());

        app.update();

        let motion = app
            .world()
            .entity(entity)
            .get::<CryMotion>()
            .expect("direct motion resolved")
            .motion()
            .clone();
        let clip = motion.direct_clip().expect("direct sampler");
        assert_eq!(motion.asset_id(), Some(asset_id(1)));
        assert_eq!(clip.duration(), 1.0);
        assert_eq!(clip.root_target(), controller_animation_target_root_id());
        assert_eq!(clip.timing().segment_count(), 1);
        assert_eq!(clip.timing().sample_rate(), 30.0);
        let segment = clip.timing().segment(0).expect("one segment");
        assert!((segment.travel_distance - 2.0).abs() < 0.0001);
        assert!((segment.mean_travel_speed - 2.0).abs() < 0.0001);
        // A direct entry has exactly one sample and CryEngine never runs a
        // parameterizer over it, so its queue state starts fully weighted.
        assert_eq!(
            motion
                .initial_state()
                .weights()
                .active()
                .collect::<Vec<_>>(),
            vec![(0, 1.0)]
        );
        assert!(
            app.world()
                .entity(entity)
                .get::<PendingCryMotion>()
                .is_none()
        );
    }

    /// The parametric path is the same entry with `LMG_File` set: two stages,
    /// one shared example resolver, and the per-example metadata `CryEngine`
    /// keeps on `m_arrParameter[i]` landing on the right clip.
    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "the per-example playback scales are copied verbatim out of the compiled blend \
                  space, so the assertion is that they arrive bit-for-bit unchanged"
    )]
    fn blend_space_resolves_through_its_examples_into_one_parametric_motion() {
        let (_temp, mut app) =
            motion_app(vec![catalog_entry(2, ids::BLEND_SPACE, BLEND_SPACE_PATH)]);
        let entity = app.world_mut().spawn(CryMotionId::new(asset_id(2))).id();
        app.update();
        let PendingCryMotion::BlendSpace { handle, .. } = pending_of(&app, entity) else {
            panic!("blend space did not take the blend-space stage");
        };
        app.world_mut()
            .resource_mut::<Assets<BlendSpaceAsset>>()
            .insert(
                &handle,
                blend_space_asset(Some(FIRST_EXAMPLE_PATH), Some(SECOND_EXAMPLE_PATH)),
            )
            .expect("published blend space");

        app.update();

        let PendingCryMotion::Parametric(source) = pending_of(&app, entity) else {
            panic!("blend space did not advance to its examples");
        };
        let examples = source.sources().cloned().collect::<Vec<_>>();
        assert_eq!(examples.len(), 2);
        for example in &examples {
            publish_motion_product(&mut app, example);
        }

        app.update();

        let motion = app
            .world()
            .entity(entity)
            .get::<CryMotion>()
            .expect("parametric motion resolved")
            .motion()
            .clone();
        assert!(motion.direct_clip().is_none());
        assert_eq!(motion.clips().len(), 2);
        assert_eq!(motion.asset_id(), Some(asset_id(2)));
        assert_eq!(motion.timewarp_group(), Some("Locomotion"));
        // `<Example PlaybackScale>` is per example and positional
        // (`GlobalAnimationHeaderLMG.cpp:583`).
        assert_eq!(motion.clips()[0].timing().playback_scale(), 2.0);
        assert_eq!(motion.clips()[1].timing().playback_scale(), 0.5);
        assert_eq!(
            motion.clips()[0].direct_delta_motion(),
            DirectDeltaMotion::from_dimensions([(true, 3.0)])
        );
        assert!(
            app.world()
                .entity(entity)
                .get::<PendingCryMotion>()
                .is_none()
        );
    }

    /// The sampler indexes clips positionally, so a hole would silently shift
    /// every later example onto the wrong clip. That is terminal, not a retry.
    #[test]
    fn blend_space_with_an_unresolvable_example_fails_without_retrying() {
        let (_temp, mut app) =
            motion_app(vec![catalog_entry(2, ids::BLEND_SPACE, BLEND_SPACE_PATH)]);
        let entity = app.world_mut().spawn(CryMotionId::new(asset_id(2))).id();
        app.update();
        let PendingCryMotion::BlendSpace { handle, .. } = pending_of(&app, entity) else {
            panic!("blend space did not take the blend-space stage");
        };
        app.world_mut()
            .resource_mut::<Assets<BlendSpaceAsset>>()
            .insert(&handle, blend_space_asset(None, Some(SECOND_EXAMPLE_PATH)))
            .expect("published blend space");

        app.update();

        assert_eq!(
            app.world().entity(entity).get::<CryMotionError>(),
            Some(&CryMotionError::UnresolvableExample {
                asset_id: asset_id(2),
                alias: az_core::name::AzNameCaseInsensitive::new("first"),
            })
        );
        assert!(
            app.world()
                .entity(entity)
                .get::<PendingCryMotion>()
                .is_none()
        );
    }

    /// A malformed product must not be retried forever: once the glTF is in
    /// `Assets` and still cannot yield a motion, the reason is terminal.
    #[test]
    fn a_malformed_motion_product_stops_retrying() {
        let (_temp, mut app) = motion_app(vec![catalog_entry(1, ids::ANIMATION, DIRECT_PATH)]);
        let entity = app.world_mut().spawn(CryMotionId::new(asset_id(1))).id();
        app.update();
        let PendingCryMotion::Direct(source) = pending_of(&app, entity) else {
            panic!("direct motion did not take the direct stage");
        };
        let clip = app
            .world_mut()
            .resource_mut::<Assets<AnimationClip>>()
            .add(AnimationClip::default());
        app.world_mut()
            .resource_mut::<Assets<Gltf>>()
            .insert(source.as_ref(), gltf_asset(Vec::new(), vec![clip], None))
            .expect("published motion product");

        app.update();

        assert_eq!(
            app.world().entity(entity).get::<CryMotionError>(),
            Some(&CryMotionError::Product {
                asset_id: asset_id(1),
                source: MotionResolveError::MissingGltfDocument,
            })
        );
        assert!(
            app.world()
                .entity(entity)
                .get::<PendingCryMotion>()
                .is_none()
        );
    }

    /// A motion whose products have not arrived yet keeps waiting instead of
    /// failing, which is what separates `Pending` from every other resolve
    /// error.
    #[test]
    fn an_unloaded_motion_product_keeps_waiting() {
        let (_temp, mut app) = motion_app(vec![catalog_entry(1, ids::ANIMATION, DIRECT_PATH)]);
        let entity = app.world_mut().spawn(CryMotionId::new(asset_id(1))).id();

        app.update();
        app.update();

        assert!(app.world().entity(entity).get::<CryMotion>().is_none());
        assert!(app.world().entity(entity).get::<CryMotionError>().is_none());
        assert!(matches!(
            pending_of(&app, entity),
            PendingCryMotion::Direct(_)
        ));
    }
}
