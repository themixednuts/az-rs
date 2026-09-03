// Expansion drops `cfg(test)`-only names and adds unused ones; it does not compile.
#[allow(clippy::wildcard_imports)]
use super::*;

#[derive(Component, Debug)]
pub(super) struct MannequinPreviewEntity;

#[derive(Component, Debug)]
pub(super) struct MannequinPreviewRoot;

#[derive(Component, Debug, Clone)]
pub(super) struct MannequinPreviewPlayer {
    #[allow(dead_code)]
    clips: Vec<Handle<AnimationClip>>,
    pub(super) nodes: Vec<AnimationNodeIndex>,
    blend_space: bool,
}

#[cfg(test)]
impl MannequinPreviewPlayer {
    pub(super) fn clips(&self) -> &[Handle<AnimationClip>] {
        &self.clips
    }
}

#[derive(Component, Debug, Clone)]
pub(super) struct PendingMannequinAnimation {
    clips: Vec<Handle<AnimationClip>>,
    graph: Handle<AnimationGraph>,
    nodes: Vec<AnimationNodeIndex>,
    blend_space: bool,
    playing: bool,
    looping: bool,
    position_millis: u32,
}

#[derive(Component, Debug)]
pub(super) struct PendingMannequinCameraFrame;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MannequinPreviewKey {
    character_glb: PathBuf,
    character_asset_path: String,
    animation: MannequinAnimationKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum MannequinAnimationKey {
    SingleMotion {
        motion_glb: Option<PathBuf>,
        motion_asset_path: Option<String>,
    },
    BlendSpace {
        bspace_ron_path: String,
        motion_asset_paths: Vec<String>,
    },
}

impl MannequinPreviewKey {
    pub(super) fn from_previews(
        preview: &EditorMannequinPreview,
        blend_space: &EditorBlendSpacePreview,
    ) -> Option<Self> {
        let character_asset_path = preview.character_glb.as_ref()?.trim();
        if character_asset_path.is_empty() {
            return None;
        }
        let character_glb = preview.resolve_character_glb()?;
        let animation = if let (Some(bspace_ron_path), Some(document)) = (
            blend_space.bspace_ron_path.as_ref(),
            blend_space.document.as_ref(),
        ) {
            let motion_asset_paths = document
                .examples
                .iter()
                .filter_map(|example| {
                    let path = example.motion_path.trim();
                    (!path.is_empty()).then(|| path.to_owned())
                })
                .collect::<Vec<_>>();
            if motion_asset_paths.is_empty() {
                MannequinAnimationKey::SingleMotion {
                    motion_glb: preview.resolve_motion_glb(),
                    motion_asset_path: preview
                        .motion_glb
                        .as_ref()
                        .map(|path| path.trim())
                        .filter(|path| !path.is_empty())
                        .map(str::to_owned),
                }
            } else {
                MannequinAnimationKey::BlendSpace {
                    bspace_ron_path: bspace_ron_path.clone(),
                    motion_asset_paths,
                }
            }
        } else {
            MannequinAnimationKey::SingleMotion {
                motion_glb: preview.resolve_motion_glb(),
                motion_asset_path: preview
                    .motion_glb
                    .as_ref()
                    .map(|path| path.trim())
                    .filter(|path| !path.is_empty())
                    .map(str::to_owned),
            }
        };
        Some(Self {
            character_glb,
            character_asset_path: character_asset_path.to_owned(),
            animation,
        })
    }
}

/// Authored milliseconds as the seconds a bevy `AnimationPlayer` seeks with.
///
/// Whole seconds and the sub-second remainder are widened separately so both
/// halves come from a lossless `u16` conversion. Clips beyond ~18 hours would
/// saturate, which no preview animation reaches.
pub(super) fn preview_seek_seconds(position_millis: u32) -> f32 {
    let seconds = u16::try_from(position_millis / 1000).unwrap_or(u16::MAX);
    let remainder = u16::try_from(position_millis % 1000).unwrap_or(0);
    f32::from(remainder).mul_add(0.001, f32::from(seconds))
}

pub(super) fn load_absolute_asset<T: bevy::asset::Asset>(
    asset_server: &AssetServer,
    path: impl Into<AssetPath<'static>>,
) -> Handle<T> {
    asset_server
        .load_builder()
        .override_unapproved()
        .load(path.into())
}

pub(super) fn spawn_neutral_primitives(world: &mut World) {
    // Mesh + material handles (scope each resource borrow so they don't overlap).
    let ground_mesh = world
        .resource_mut::<Assets<Mesh>>()
        .add(Plane3d::default().mesh().size(40.0, 40.0));
    let cuboid_mesh = world
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(1.5, 1.5, 1.5));
    let sphere_mesh = world
        .resource_mut::<Assets<Mesh>>()
        .add(Sphere::new(0.9).mesh().uv(32, 18));

    let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
    let ground_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.16, 0.18, 0.22),
        perceptual_roughness: 0.95,
        ..default()
    });
    let cuboid_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.25, 0.42, 0.82),
        perceptual_roughness: 0.5,
        ..default()
    });
    let sphere_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.82, 0.43, 0.26),
        perceptual_roughness: 0.35,
        metallic: 0.1,
        ..default()
    });
    // `materials` borrows `world`; NLL ends that borrow at its last use above,
    // so the spawns below can take `world` again.

    // Ground plane.
    world.spawn((
        Mesh3d(ground_mesh),
        MeshMaterial3d(ground_material),
        Transform::IDENTITY,
        EditorSceneObject::placeholder("scene/ground"),
        NeutralViewportEntity,
    ));
    // A cuboid and a sphere as representative editor content.
    world.spawn((
        Mesh3d(cuboid_mesh),
        MeshMaterial3d(cuboid_material),
        Transform::from_xyz(-1.6, 0.75, 0.0),
        EditorSceneObject::placeholder("scene/cuboid"),
        NeutralViewportEntity,
    ));
    world.spawn((
        Mesh3d(sphere_mesh),
        MeshMaterial3d(sphere_material),
        Transform::from_xyz(1.7, 0.9, -0.4),
        EditorSceneObject::placeholder("scene/sphere"),
        NeutralViewportEntity,
    ));
}

/// One retained triangle mesh for either the minor or major grid lines.
/// Thin cuboids are merged at startup so the visible grid adds two draws and
/// no per-frame geometry work.
pub(super) fn ground_grid_mesh(major: bool) -> Mesh {
    let thickness = if major { 0.022 } else { 0.008 };
    let mut merged: Option<Mesh> = None;
    for index in -GRID_LINE_COUNT..=GRID_LINE_COUNT {
        let is_major = index % 4 == 0;
        if is_major != major {
            continue;
        }
        let coordinate = f32::from(index) * GRID_STEP_METERS;
        for (size, translation) in [
            (
                Vec3::new(GRID_EXTENT_METERS * 2.0, 0.004, thickness),
                Vec3::new(0.0, 0.012, coordinate),
            ),
            (
                Vec3::new(thickness, 0.004, GRID_EXTENT_METERS * 2.0),
                Vec3::new(coordinate, 0.012, 0.0),
            ),
        ] {
            let line = Mesh::from(Cuboid::new(size.x, size.y, size.z))
                .transformed_by(Transform::from_translation(translation));
            if let Some(mesh) = merged.as_mut() {
                mesh.merge(&line)
                    .expect("grid cuboids have identical mesh attributes");
            } else {
                merged = Some(line);
            }
        }
    }
    merged.expect("ground grid always contains at least one line")
}

pub(super) fn clear_neutral_primitives(world: &mut World) {
    let stale: Vec<Entity> = {
        let mut query = world.query_filtered::<Entity, With<NeutralViewportEntity>>();
        query.iter(world).collect()
    };
    for entity in stale {
        let mut entity = world.entity_mut(entity);
        entity.despawn_related::<Children>();
        entity.despawn();
    }
}

pub(super) fn ensure_neutral_primitives(world: &mut World) {
    let has_neutral = {
        let mut query = world.query_filtered::<Entity, With<NeutralViewportEntity>>();
        query.iter(world).next().is_some()
    };
    if !has_neutral {
        spawn_neutral_primitives(world);
    }
}

/// Populate the in-process editor world with a lit ground + a few primitives so
/// the viewport shows a real 3D scene when no authored mannequin is selected.
pub(super) fn spawn_editor_scene(world: &mut World) {
    // Ambient fill is set per-camera (see the camera spawn above) in bevy 0.19.
    spawn_neutral_primitives(world);

    // One shadow-free key light is enough for authoring. A second point light
    // forced per-view clustered-light preparation even in the neutral scene.
    world.spawn((
        DirectionalLight {
            illuminance: 9_500.0,
            // The editor viewport prioritizes 120 Hz interaction. Shipping
            // shadow products are previewed in dedicated render tooling; this
            // neutral authoring light does not allocate or render shadow maps.
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(8.0, 14.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

pub(super) fn clear_mannequin_preview(world: &mut World) {
    let stale: Vec<Entity> = {
        let mut query = world.query_filtered::<Entity, With<MannequinPreviewEntity>>();
        query.iter(world).collect()
    };
    for entity in stale {
        if world.get_entity(entity).is_ok() {
            let mut entity = world.entity_mut(entity);
            entity.despawn_related::<Children>();
            entity.despawn();
        }
    }
}

pub(super) fn spawn_mannequin_preview(
    world: &mut World,
    key: &MannequinPreviewKey,
    preview: &EditorMannequinPreview,
    blend_space: &EditorBlendSpacePreview,
) {
    let asset_server = world.resource::<AssetServer>().clone();
    let scene_path = GltfAssetLabel::Scene(0).from_asset(key.character_glb.clone());
    let scene: Handle<WorldAsset> = load_absolute_asset(&asset_server, scene_path);

    let root = world
        .spawn((
            WorldAssetRoot(scene),
            Transform::IDENTITY,
            Visibility::default(),
            EditorSceneObject {
                id: format!("mannequin/{}", key.character_glb.display()),
                document_id: None,
                object_id: None,
            },
            MannequinPreviewRoot,
            MannequinPreviewEntity,
            PendingMannequinCameraFrame,
        ))
        .observe(play_mannequin_animation_on_ready)
        .id();

    if let Some(pending) =
        pending_mannequin_animation(world, &asset_server, &key.animation, preview, blend_space)
    {
        world.entity_mut(root).insert(pending);
    }
}

pub(super) fn apply_mannequin_playback_state(world: &mut World, preview: &EditorMannequinPreview) {
    let mut query = world.query_filtered::<
        (&mut AnimationPlayer, &MannequinPreviewPlayer),
        With<MannequinPreviewPlayer>,
    >();
    for (mut player, preview_player) in query.iter_mut(world) {
        for node in &preview_player.nodes {
            if let Some(animation) = player.animation_mut(*node) {
                animation.set_repeat(if preview.looping {
                    RepeatAnimation::Forever
                } else {
                    RepeatAnimation::Never
                });
                animation.seek_to(preview_seek_seconds(preview.position_millis));
                if preview.playing {
                    animation.resume();
                } else {
                    animation.pause();
                }
            }
        }
        if preview.playing {
            player.resume_all();
        } else {
            player.pause_all();
        }
    }

    let mut pending =
        world.query_filtered::<&mut PendingMannequinAnimation, With<MannequinPreviewRoot>>();
    for mut pending in pending.iter_mut(world) {
        pending.playing = preview.playing;
        pending.looping = preview.looping;
        pending.position_millis = preview.position_millis;
    }
}

pub(super) fn pending_mannequin_animation(
    world: &mut World,
    asset_server: &AssetServer,
    animation: &MannequinAnimationKey,
    preview: &EditorMannequinPreview,
    blend_space: &EditorBlendSpacePreview,
) -> Option<PendingMannequinAnimation> {
    match animation {
        MannequinAnimationKey::SingleMotion { motion_glb, .. } => {
            let motion_glb = motion_glb.as_ref()?;
            let clip_path = GltfAssetLabel::Animation(0).from_asset(motion_glb.clone());
            let clip: Handle<AnimationClip> = load_absolute_asset(asset_server, clip_path);
            let (graph, node) = AnimationGraph::from_clip(clip.clone());
            let graph = world.resource_mut::<Assets<AnimationGraph>>().add(graph);
            Some(PendingMannequinAnimation {
                clips: vec![clip],
                graph,
                nodes: vec![node],
                blend_space: false,
                playing: preview.playing,
                looping: preview.looping,
                position_millis: preview.position_millis,
            })
        }
        MannequinAnimationKey::BlendSpace {
            motion_asset_paths, ..
        } => {
            let document = blend_space.document.as_ref()?;
            let weights = document.weight_values_for_params(&blend_space.param_values);
            let mut graph = AnimationGraph::new();
            let blend = graph.add_blend(1.0, graph.root);
            let mut clips = Vec::new();
            let mut nodes = Vec::new();
            for (index, motion_glb) in motion_asset_paths.iter().enumerate() {
                let source_path = preview
                    .project_asset_root
                    .as_ref()
                    .map_or_else(|| PathBuf::from(motion_glb), |root| root.join(motion_glb));
                let clip_path = GltfAssetLabel::Animation(0).from_asset(source_path);
                let clip: Handle<AnimationClip> = load_absolute_asset(asset_server, clip_path);
                let weight = weights.get(index).copied().unwrap_or_default();
                let node = graph.add_clip(clip.clone(), weight, blend);
                clips.push(clip);
                nodes.push(node);
            }
            let graph = world.resource_mut::<Assets<AnimationGraph>>().add(graph);
            Some(PendingMannequinAnimation {
                clips,
                graph,
                nodes,
                blend_space: true,
                playing: preview.playing,
                looping: preview.looping,
                position_millis: preview.position_millis,
            })
        }
    }
}

pub(super) fn apply_mannequin_blend_space_weights(
    world: &mut World,
    blend_space: &EditorBlendSpacePreview,
) {
    let Some(document) = blend_space.document.as_ref() else {
        return;
    };
    let weights = document.weight_values_for_params(&blend_space.param_values);
    let graph_updates = {
        let mut players = world.query::<(&AnimationGraphHandle, &MannequinPreviewPlayer)>();
        players
            .iter(world)
            .filter(|(_, player)| player.blend_space)
            .map(|(graph, player)| (graph.0.clone(), player.nodes.clone()))
            .collect::<Vec<_>>()
    };
    let mut graphs = world.resource_mut::<Assets<AnimationGraph>>();
    for (graph_handle, nodes) in graph_updates {
        let Some(mut graph) = graphs.get_mut(&graph_handle) else {
            continue;
        };
        for (node, weight) in nodes.into_iter().zip(weights.iter().copied()) {
            if let Some(graph_node) = graph.get_mut(node) {
                graph_node.weight = weight;
            }
        }
    }
}

// Bevy observers receive their trigger by value; `&On<E>` is not a SystemParam.
#[allow(clippy::needless_pass_by_value)]
pub(super) fn play_mannequin_animation_on_ready(
    trigger: On<WorldInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    names: Query<&Name>,
    pending: Query<&PendingMannequinAnimation>,
    skinned_meshes: Query<&SkinnedMesh>,
    mut players: Query<&mut AnimationPlayer>,
) {
    let Ok(pending) = pending.get(trigger.entity) else {
        return;
    };
    let Some(animation_root) = mannequin_animation_root(trigger.entity, &children, &skinned_meshes)
    else {
        return;
    };

    install_mannequin_animation_targets(
        &mut commands,
        animation_root,
        animation_root,
        &children,
        &names,
        &mut Vec::new(),
    );

    if let Ok(mut player) = players.get_mut(animation_root) {
        player.stop_all();
        play_pending_nodes(&mut player, pending);
        commands.entity(animation_root).insert((
            AnimationGraphHandle(pending.graph.clone()),
            MannequinPreviewEntity,
            MannequinPreviewPlayer {
                clips: pending.clips.clone(),
                nodes: pending.nodes.clone(),
                blend_space: pending.blend_space,
            },
        ));
    } else {
        let mut player = AnimationPlayer::default();
        play_pending_nodes(&mut player, pending);
        commands.entity(animation_root).insert((
            player,
            AnimationGraphHandle(pending.graph.clone()),
            MannequinPreviewEntity,
            MannequinPreviewPlayer {
                clips: pending.clips.clone(),
                nodes: pending.nodes.clone(),
                blend_space: pending.blend_space,
            },
        ));
    }

    commands
        .entity(trigger.entity)
        .remove::<PendingMannequinAnimation>();
}

pub(super) fn play_pending_nodes(
    player: &mut AnimationPlayer,
    pending: &PendingMannequinAnimation,
) {
    for node in &pending.nodes {
        let animation = player.play(*node);
        animation.set_repeat(if pending.looping {
            RepeatAnimation::Forever
        } else {
            RepeatAnimation::Never
        });
        animation.seek_to(preview_seek_seconds(pending.position_millis));
        if pending.playing {
            animation.resume();
        } else {
            animation.pause();
        }
    }
}

pub(super) fn mannequin_animation_root(
    root: Entity,
    children: &Query<&Children>,
    skinned_meshes: &Query<&SkinnedMesh>,
) -> Option<Entity> {
    let skinned_mesh = children
        .iter_descendants(root)
        .filter_map(|entity| skinned_meshes.get(entity).ok())
        .max_by_key(|skinned_mesh| skinned_mesh.joints.len())?;

    children
        .iter_descendants(root)
        .find(|entity| skinned_mesh.joints.contains(entity))
}

pub(super) fn install_mannequin_animation_targets(
    commands: &mut Commands,
    entity: Entity,
    player: Entity,
    children: &Query<&Children>,
    names: &Query<&Name>,
    path: &mut Vec<Name>,
) {
    let Ok(name) = names.get(entity) else {
        return;
    };
    path.push(name.clone());
    commands.entity(entity).insert((
        AnimationTargetId::from_names(path.iter()),
        AnimatedBy(player),
        MannequinPreviewEntity,
    ));

    let child_entities: Vec<Entity> = children
        .get(entity)
        .map(|children| children.iter().collect())
        .unwrap_or_default();
    for child in child_entities {
        install_mannequin_animation_targets(commands, child, player, children, names, path);
    }
    path.pop();
}

pub(super) fn frame_pending_mannequin_camera(
    mut commands: Commands,
    roots: Query<
        Entity,
        (
            With<MannequinPreviewRoot>,
            With<PendingMannequinCameraFrame>,
        ),
    >,
    children: Query<&Children>,
    skinned_meshes: Query<&SkinnedMesh>,
    globals: Query<&GlobalTransform>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
) {
    for root in &roots {
        let Some((min, max)) = mannequin_joint_bounds(root, &children, &skinned_meshes, &globals)
        else {
            continue;
        };

        let center = (min + max) * 0.5;
        let extent = (max - min).max(Vec3::splat(0.25));
        let height = extent.y.max(1.0);
        let radius = extent.length().max(1.0) * 0.5;
        let focus = center + Vec3::Y * height * 0.08;
        let position = focus + Vec3::new(0.0, height * 0.18, radius.max(1.0) * 3.2);

        for mut transform in &mut cameras {
            *transform = Transform::from_translation(position).looking_at(focus, Vec3::Y);
        }
        commands
            .entity(root)
            .remove::<PendingMannequinCameraFrame>();
    }
}

pub(super) fn mannequin_joint_bounds(
    root: Entity,
    children: &Query<&Children>,
    skinned_meshes: &Query<&SkinnedMesh>,
    globals: &Query<&GlobalTransform>,
) -> Option<(Vec3, Vec3)> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut found = false;

    for entity in children.iter_descendants(root) {
        let Ok(skinned_mesh) = skinned_meshes.get(entity) else {
            continue;
        };
        for joint in &skinned_mesh.joints {
            let Ok(transform) = globals.get(*joint) else {
                continue;
            };
            let point = transform.translation();
            min = min.min(point);
            max = max.max(point);
            found = true;
        }
    }

    found.then_some((min, max))
}
