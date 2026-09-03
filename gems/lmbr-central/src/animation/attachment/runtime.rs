use az_core::{AzEntityIndex, EntityId, crc::Crc32};
use bevy::{
    app::{AnimationSystems, PostUpdate},
    math::Affine3A,
    prelude::*,
    transform::{TransformSystems, helper::TransformHelper},
};
use smallvec::SmallVec;

use super::{AttachmentComponent, AttachmentConfiguration, AttachmentScaleSource};

/// Live state for an [`AttachmentComponent`].
///
/// The authored component remains the activation configuration. Requests mutate
/// this state, so changing a live attachment never rewrites the prefab data.
#[derive(Component, Debug, Clone)]
pub struct AttachmentRuntime {
    target_id: EntityId,
    target: AttachmentTarget,
    target_offset: Transform,
    scale_source: AttachmentScaleSource,
    update_tolerance: f32,
    target_entity: Option<Entity>,
    target_bone_entity: Option<Entity>,
    cached_target_transform: Option<Affine3A>,
    cached_bone_transform: Option<Affine3A>,
    cached_owner_transform: Option<Affine3A>,
    force_update: bool,
}

impl Default for AttachmentRuntime {
    fn default() -> Self {
        Self {
            target_id: EntityId::INVALID,
            target: AttachmentTarget::default(),
            target_offset: Transform::IDENTITY,
            scale_source: AttachmentScaleSource::WorldScale,
            update_tolerance: 0.0,
            target_entity: None,
            target_bone_entity: None,
            cached_target_transform: None,
            cached_bone_transform: None,
            cached_owner_transform: None,
            force_update: false,
        }
    }
}

impl AttachmentRuntime {
    #[inline]
    #[must_use]
    pub const fn is_attached(&self) -> bool {
        self.target_id.is_valid()
    }

    #[inline]
    #[must_use]
    pub const fn target_entity_id(&self) -> EntityId {
        self.target_id
    }

    #[inline]
    #[must_use]
    pub const fn target(&self) -> AttachmentTarget {
        self.target
    }

    #[inline]
    #[must_use]
    pub const fn offset(&self) -> Transform {
        self.target_offset
    }

    #[inline]
    #[must_use]
    pub const fn scale_source(&self) -> AttachmentScaleSource {
        self.scale_source
    }

    #[inline]
    #[must_use]
    pub const fn update_tolerance(&self) -> f32 {
        self.update_tolerance
    }

    const fn apply_authored_settings(&mut self, configuration: &AttachmentConfiguration) {
        self.scale_source = configuration.scale_source;
        self.update_tolerance = normalized_tolerance(configuration.update_tolerance);
    }

    const fn bind(
        &mut self,
        target_id: EntityId,
        target: AttachmentTarget,
        target_offset: Transform,
    ) {
        self.target_id = target_id;
        self.target = target;
        self.target_offset = target_offset;
        self.target_entity = None;
        self.target_bone_entity = None;
        self.cached_target_transform = None;
        self.cached_bone_transform = None;
        self.cached_owner_transform = None;
        self.force_update = true;
    }

    fn unbind(&mut self) -> Option<EntityId> {
        let target_id = self.is_attached().then_some(self.target_id)?;
        self.target_id = EntityId::INVALID;
        self.target_entity = None;
        self.target_bone_entity = None;
        self.cached_target_transform = None;
        self.cached_bone_transform = None;
        self.cached_owner_transform = None;
        self.force_update = false;
        Some(target_id)
    }
}

/// A skeletal attachment target represented by the two CRCs consumed by the
/// runtime attachment request.
///
/// A joint takes precedence. When it is absent, the named attachment is
/// resolved directly from the animated hierarchy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct AttachmentTarget {
    joint: Crc32,
    attachment: Crc32,
}

impl AttachmentTarget {
    #[inline]
    #[must_use]
    pub const fn new(joint: Crc32, attachment: Crc32) -> Self {
        Self { joint, attachment }
    }

    #[inline]
    #[must_use]
    pub const fn joint(joint: Crc32) -> Self {
        Self::new(joint, Crc32::ZERO)
    }

    #[inline]
    #[must_use]
    pub const fn attachment(attachment: Crc32) -> Self {
        Self::new(Crc32::ZERO, attachment)
    }

    #[inline]
    #[must_use]
    pub fn joint_name(name: impl AsRef<str>) -> Self {
        Self::joint(Crc32::from_str_lower(name.as_ref()))
    }

    #[inline]
    #[must_use]
    pub fn attachment_name(name: impl AsRef<str>) -> Self {
        Self::attachment(Crc32::from_str_lower(name.as_ref()))
    }

    #[inline]
    #[must_use]
    pub const fn joint_crc(self) -> Crc32 {
        self.joint
    }

    #[inline]
    #[must_use]
    pub const fn attachment_crc(self) -> Crc32 {
        self.attachment
    }

    #[inline]
    #[must_use]
    const fn resolved_crc(self) -> Crc32 {
        if self.joint.value() != 0 {
            self.joint
        } else {
            self.attachment
        }
    }
}

impl From<Crc32> for AttachmentTarget {
    #[inline]
    fn from(value: Crc32) -> Self {
        Self::joint(value)
    }
}

impl From<&str> for AttachmentTarget {
    #[inline]
    fn from(value: &str) -> Self {
        Self::joint_name(value)
    }
}

impl From<String> for AttachmentTarget {
    #[inline]
    fn from(value: String) -> Self {
        Self::joint_name(value)
    }
}

/// A mutation addressed to one live attachment component.
#[derive(Debug, Clone, PartialEq, Message)]
pub struct AttachmentComponentRequest {
    pub entity: Entity,
    pub action: AttachmentComponentAction,
}

impl AttachmentComponentRequest {
    #[inline]
    #[must_use]
    pub const fn new(entity: Entity, action: AttachmentComponentAction) -> Self {
        Self { entity, action }
    }

    #[must_use]
    pub fn attach(
        entity: Entity,
        target_id: EntityId,
        target: impl Into<AttachmentTarget>,
        offset: Transform,
    ) -> Self {
        Self::new(
            entity,
            AttachmentComponentAction::Attach {
                target_id,
                target: target.into(),
                offset,
            },
        )
    }

    #[inline]
    #[must_use]
    pub const fn detach(entity: Entity) -> Self {
        Self::new(entity, AttachmentComponentAction::Detach)
    }

    #[inline]
    #[must_use]
    pub const fn set_attachment_offset(entity: Entity, offset: Transform) -> Self {
        Self::new(
            entity,
            AttachmentComponentAction::SetAttachmentOffset(offset),
        )
    }
}

/// Reflected runtime request surface for [`AttachmentComponent`].
#[derive(Debug, Clone, PartialEq)]
pub enum AttachmentComponentAction {
    Attach {
        target_id: EntityId,
        target: AttachmentTarget,
        offset: Transform,
    },
    Detach,
    SetAttachmentOffset(Transform),
}

/// Attachment lifecycle notification addressed by `target_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Message)]
pub enum AttachmentComponentNotification {
    Attached {
        target_id: EntityId,
        attachment_id: EntityId,
        attachment: Entity,
    },
    Detached {
        target_id: EntityId,
        attachment_id: EntityId,
        attachment: Entity,
    },
}

impl AttachmentComponentNotification {
    #[inline]
    #[must_use]
    pub const fn target_id(self) -> EntityId {
        match self {
            Self::Attached { target_id, .. } | Self::Detached { target_id, .. } => target_id,
        }
    }

    #[inline]
    #[must_use]
    pub const fn attachment_id(self) -> EntityId {
        match self {
            Self::Attached { attachment_id, .. } | Self::Detached { attachment_id, .. } => {
                attachment_id
            }
        }
    }

    #[inline]
    #[must_use]
    pub const fn attachment(self) -> Entity {
        match self {
            Self::Attached { attachment, .. } | Self::Detached { attachment, .. } => attachment,
        }
    }
}

pub(in crate::animation) fn register_attachment_runtime(app: &mut App) {
    app.add_message::<AttachmentComponentRequest>()
        .add_message::<AttachmentComponentNotification>()
        .add_systems(
            PostUpdate,
            (
                sync_authored_attachments,
                route_attachment_component_requests,
                detach_removed_attachments,
                update_attachment_transforms,
            )
                .chain()
                .after(AnimationSystems)
                .before(TransformSystems::Propagate),
        );
}

fn sync_authored_attachments(
    mut attachments: Query<
        (
            Entity,
            Option<&EntityId>,
            &AttachmentComponent,
            &mut AttachmentRuntime,
        ),
        Changed<AttachmentComponent>,
    >,
    mut notifications: MessageWriter<AttachmentComponentNotification>,
) {
    for (owner, owner_id, attachment, mut runtime) in &mut attachments {
        if let Some(target_id) = runtime.unbind() {
            notifications.write(detached_notification(owner, owner_id, target_id));
        }

        let configuration = &attachment.configuration;
        runtime.apply_authored_settings(configuration);
        if configuration.attached_initially {
            attach(
                owner,
                owner_id,
                &mut runtime,
                configuration.target_id,
                AttachmentTarget::joint_name(&configuration.target_bone_name),
                configuration.target_offset,
                &mut notifications,
            );
        }
    }
}

fn route_attachment_component_requests(
    mut requests: MessageReader<AttachmentComponentRequest>,
    mut attachments: Query<(Option<&EntityId>, &mut AttachmentRuntime), With<AttachmentComponent>>,
    mut notifications: MessageWriter<AttachmentComponentNotification>,
) {
    for request in requests.read() {
        let Ok((owner_id, mut runtime)) = attachments.get_mut(request.entity) else {
            continue;
        };
        match &request.action {
            AttachmentComponentAction::Attach {
                target_id,
                target,
                offset,
            } => attach(
                request.entity,
                owner_id,
                &mut runtime,
                *target_id,
                *target,
                *offset,
                &mut notifications,
            ),
            AttachmentComponentAction::Detach => {
                if let Some(target_id) = runtime.unbind() {
                    notifications.write(detached_notification(request.entity, owner_id, target_id));
                }
            }
            AttachmentComponentAction::SetAttachmentOffset(offset) => {
                if runtime.is_attached() && runtime.target_offset != *offset {
                    runtime.target_offset = *offset;
                    runtime.force_update = true;
                }
            }
        }
    }
}

fn detach_removed_attachments(
    mut commands: Commands,
    mut removed: RemovedComponents<AttachmentComponent>,
    mut runtimes: Query<(Option<&EntityId>, &mut AttachmentRuntime)>,
    mut notifications: MessageWriter<AttachmentComponentNotification>,
) {
    for owner in removed.read() {
        let Ok((owner_id, mut runtime)) = runtimes.get_mut(owner) else {
            continue;
        };
        if let Some(target_id) = runtime.unbind() {
            notifications.write(detached_notification(owner, owner_id, target_id));
        }
        commands.entity(owner).remove::<AttachmentRuntime>();
    }
}

fn attach(
    owner: Entity,
    owner_id: Option<&EntityId>,
    runtime: &mut AttachmentRuntime,
    target_id: EntityId,
    target: AttachmentTarget,
    offset: Transform,
    notifications: &mut MessageWriter<AttachmentComponentNotification>,
) {
    if let Some(previous_target_id) = runtime.unbind() {
        notifications.write(detached_notification(owner, owner_id, previous_target_id));
    }
    if !target_id.is_valid()
        || owner_id
            .copied()
            .is_some_and(|owner_id| owner_id == target_id)
    {
        return;
    }

    runtime.bind(target_id, target, offset);
    notifications.write(attached_notification(owner, owner_id, target_id));
}

fn attached_notification(
    attachment: Entity,
    attachment_id: Option<&EntityId>,
    target_id: EntityId,
) -> AttachmentComponentNotification {
    AttachmentComponentNotification::Attached {
        target_id,
        attachment_id: attachment_id.copied().unwrap_or_default(),
        attachment,
    }
}

fn detached_notification(
    attachment: Entity,
    attachment_id: Option<&EntityId>,
    target_id: EntityId,
) -> AttachmentComponentNotification {
    AttachmentComponentNotification::Detached {
        target_id,
        attachment_id: attachment_id.copied().unwrap_or_default(),
        attachment,
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "`Res` is an owned Bevy system parameter; a reference stops this satisfying `IntoSystem`"
)]
fn update_attachment_transforms(
    entity_index: Res<AzEntityIndex>,
    mut attachments: Query<(Entity, &mut AttachmentRuntime), With<AttachmentComponent>>,
    names: Query<&Name>,
    children: Query<&Children>,
    parents: Query<&ChildOf>,
    mut transforms: ParamSet<(TransformHelper, Query<&mut Transform>)>,
) {
    for (owner, mut runtime) in &mut attachments {
        if !runtime.is_attached() {
            continue;
        }

        let Some(target) = entity_index.resolve(runtime.target_id) else {
            runtime.target_entity = None;
            runtime.target_bone_entity = None;
            continue;
        };
        if target == owner
            || parents
                .iter_ancestors(target)
                .any(|ancestor| ancestor == owner)
        {
            continue;
        }

        if runtime.target_entity != Some(target) {
            runtime.target_entity = Some(target);
            runtime.target_bone_entity = None;
            runtime.cached_target_transform = None;
            runtime.cached_bone_transform = None;
            runtime.force_update = true;
        }

        let target_bone = resolve_target_bone(&mut runtime, target, &names, &children);
        let (target_transform, bone_transform, parent_transform) = {
            let helper = transforms.p0();
            let Ok(target_transform) = helper.compute_global_transform(target) else {
                continue;
            };
            let target_transform = target_transform.affine();
            let bone_transform = target_bone
                .and_then(|bone| helper.compute_global_transform(bone).ok())
                .map_or(Affine3A::IDENTITY, |bone_transform| {
                    target_transform.inverse() * bone_transform.affine()
                });
            let parent_transform = parents
                .get(owner)
                .ok()
                .and_then(|parent| helper.compute_global_transform(parent.parent()).ok())
                .map(|transform| transform.affine());
            (target_transform, bone_transform, parent_transform)
        };

        let tolerance = runtime.update_tolerance;
        let target_changed = runtime.force_update
            || transform_exceeds_tolerance(
                runtime.cached_target_transform.as_ref(),
                &target_transform,
                tolerance,
            );
        let bone_changed = runtime.force_update
            || transform_exceeds_tolerance(
                runtime.cached_bone_transform.as_ref(),
                &bone_transform,
                tolerance,
            );
        if target_changed {
            runtime.cached_target_transform = Some(target_transform);
        }
        if bone_changed {
            runtime.cached_bone_transform = Some(bone_transform);
        }
        runtime.force_update = false;
        if !target_changed && !bone_changed {
            continue;
        }

        let final_transform = compose_attachment_transform(
            target_transform,
            bone_transform,
            Affine3A::from_scale_rotation_translation(
                runtime.target_offset.scale,
                runtime.target_offset.rotation,
                runtime.target_offset.translation,
            ),
            runtime.scale_source,
        );
        if runtime.cached_owner_transform == Some(final_transform) {
            continue;
        }
        runtime.cached_owner_transform = Some(final_transform);

        let local_transform =
            parent_transform.map_or(final_transform, |parent| parent.inverse() * final_transform);
        let next_transform = Transform::from_matrix(Mat4::from(local_transform));
        let mut writable_transforms = transforms.p1();
        let Ok(mut transform) = writable_transforms.get_mut(owner) else {
            continue;
        };
        if *transform != next_transform {
            *transform = next_transform;
        }
    }
}

fn resolve_target_bone(
    runtime: &mut AttachmentRuntime,
    target: Entity,
    names: &Query<&Name>,
    children: &Query<&Children>,
) -> Option<Entity> {
    let target_crc = runtime.target.resolved_crc();
    if target_crc == Crc32::ZERO {
        runtime.target_bone_entity = None;
        return None;
    }
    if let Some(bone) = runtime.target_bone_entity
        && names
            .get(bone)
            .is_ok_and(|name| Crc32::from_str_lower(name.as_str()) == target_crc)
    {
        return Some(bone);
    }

    let mut pending = SmallVec::<[Entity; 64]>::new();
    pending.push(target);
    while let Some(candidate) = pending.pop() {
        if names
            .get(candidate)
            .is_ok_and(|name| Crc32::from_str_lower(name.as_str()) == target_crc)
        {
            runtime.target_bone_entity = Some(candidate);
            runtime.cached_bone_transform = None;
            return Some(candidate);
        }
        if let Ok(candidate_children) = children.get(candidate) {
            pending.extend(candidate_children.iter());
        }
    }

    runtime.target_bone_entity = None;
    None
}

#[inline]
const fn normalized_tolerance(tolerance: f32) -> f32 {
    if tolerance.is_finite() {
        tolerance.max(0.0)
    } else {
        0.0
    }
}

fn transform_exceeds_tolerance(
    previous: Option<&Affine3A>,
    current: &Affine3A,
    tolerance: f32,
) -> bool {
    let Some(previous) = previous else {
        return true;
    };
    previous
        .to_cols_array()
        .into_iter()
        .zip(current.to_cols_array())
        .any(|(previous, current)| {
            !previous.is_finite() || !current.is_finite() || (current - previous).abs() > tolerance
        })
}

/// Composes a target, target-node, and authored offset using the selected
/// attachment scale space.
#[must_use]
pub fn compose_attachment_transform(
    target_transform: Affine3A,
    bone_transform: Affine3A,
    offset: Affine3A,
    scale_source: AttachmentScaleSource,
) -> Affine3A {
    match scale_source {
        AttachmentScaleSource::WorldScale => {
            without_scale(target_transform * bone_transform) * offset
        }
        AttachmentScaleSource::TargetEntityScale => {
            target_transform * without_scale(bone_transform) * offset
        }
        AttachmentScaleSource::TargetBoneScale => target_transform * bone_transform * offset,
    }
}

fn without_scale(mut transform: Affine3A) -> Affine3A {
    transform.matrix3.x_axis /= transform.matrix3.x_axis.length();
    transform.matrix3.y_axis /= transform.matrix3.y_axis.length();
    transform.matrix3.z_axis /= transform.matrix3.z_axis.length();
    transform
}

#[cfg(test)]
mod tests {
    use super::*;

    fn affine(scale: Vec3, translation: Vec3) -> Affine3A {
        Affine3A::from_scale_rotation_translation(scale, Quat::IDENTITY, translation)
    }

    fn assert_affine_close(actual: Affine3A, expected: Affine3A) {
        for (actual, expected) in actual
            .to_cols_array()
            .into_iter()
            .zip(expected.to_cols_array())
        {
            assert!(
                (actual - expected).abs() <= 1.0e-5,
                "{actual} != {expected}"
            );
        }
    }

    #[test]
    fn scale_source_selects_the_native_composition_space() {
        let target = affine(Vec3::splat(2.0), Vec3::new(10.0, 0.0, 0.0));
        let bone = affine(Vec3::splat(3.0), Vec3::new(0.0, 4.0, 0.0));
        let offset = affine(Vec3::splat(5.0), Vec3::Z);

        assert_affine_close(
            compose_attachment_transform(target, bone, offset, AttachmentScaleSource::WorldScale),
            without_scale(target * bone) * offset,
        );
        assert_affine_close(
            compose_attachment_transform(
                target,
                bone,
                offset,
                AttachmentScaleSource::TargetEntityScale,
            ),
            target * without_scale(bone) * offset,
        );
        assert_affine_close(
            compose_attachment_transform(
                target,
                bone,
                offset,
                AttachmentScaleSource::TargetBoneScale,
            ),
            target * bone * offset,
        );
    }

    #[test]
    fn tolerance_accumulates_against_the_last_accepted_transform() {
        let previous = Affine3A::IDENTITY;
        let within = Affine3A::from_translation(Vec3::splat(0.09));
        let outside = Affine3A::from_translation(Vec3::new(0.11, 0.0, 0.0));

        assert!(!transform_exceeds_tolerance(Some(&previous), &within, 0.1));
        assert!(transform_exceeds_tolerance(Some(&previous), &outside, 0.1));
    }
}
