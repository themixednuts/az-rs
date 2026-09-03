//! Renderer-independent Cry character attachment visibility.
//!
//! Character render backends implement [`CharacterAttachmentRig`]. Procedural
//! clips retain only stable attachment/target handles and the visibility bits
//! needed to restore state; no renderer object or ECS borrow escapes the call.

use std::hash::Hash;

use az_core::crc::Crc32;
use bitflags::bitflags;

bitflags! {
    /// Visibility state captured by Cry's HideAttachment procedural clip.
    ///
    /// Bit values match Lumberyard's `EHideFlags` in
    /// `dev/Gems/CryLegacy/Code/Source/CryAction/Mannequin/ProceduralClipProps.cpp`.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
    pub struct AttachmentVisibility: u8 {
        const MAIN_PASS = 1 << 0;
        const SHADOW = 1 << 1;
        const RECURSION = 1 << 2;
    }
}

/// Minimal compatibility surface required by Cry attachment procedural clips.
///
/// IDs are backend-owned, cheap stable handles. The runtime never stores a
/// reference to an attachment or attached entity across animation updates.
pub trait CharacterAttachmentRig {
    type AttachmentId: Copy + Eq + Hash;
    type EntityTargetId: Copy + Eq + Hash;

    fn attachment_by_name_crc(&self, name: Crc32) -> Option<Self::AttachmentId>;

    fn visibility(&self, attachment: Self::AttachmentId) -> AttachmentVisibility;

    fn set_main_pass_hidden(&mut self, attachment: Self::AttachmentId, hidden: bool);

    fn set_shadow_hidden(&mut self, attachment: Self::AttachmentId, hidden: bool);

    fn set_recursion_hidden(&mut self, attachment: Self::AttachmentId, hidden: bool);

    /// Mirrors the shipping visibility notification emitted after mutating the
    /// attachment. The boolean is visibility, not hiddenness.
    fn attachment_visibility_changed(&mut self, attachment: Self::AttachmentId, visible: bool);

    /// Returns the entity target only when the attachment object is Cry type 4
    /// (`IAttachmentObject::eAttachment_Entity`).
    fn entity_attachment_target(
        &self,
        attachment: Self::AttachmentId,
    ) -> Option<Self::EntityTargetId>;

    fn set_entity_target_hidden(&mut self, target: Self::EntityTargetId, hidden: bool);
}

/// Attachment binding operations used by Cry's `AttachProp` clip.
///
/// `BindingId` identifies the concrete object installed by the backend, not
/// merely its source asset. That preserves Cry's exit rule: a clip only clears
/// the attachment when its own character or static-object binding is still
/// installed.
pub trait CharacterAttachmentBinding<A>: CharacterAttachmentRig {
    type BindingId: Copy + Eq + Hash;

    fn bind_attachment_asset(
        &mut self,
        attachment: Self::AttachmentId,
        asset: &A,
    ) -> Option<Self::BindingId>;

    fn attachment_binding(&self, attachment: Self::AttachmentId) -> Option<Self::BindingId>;

    fn clear_attachment_binding(&mut self, attachment: Self::AttachmentId);
}

/// Per-installed-clip state for `AttachProp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachPropState<AttachmentId, BindingId> {
    attachment: Option<AttachmentId>,
    binding: Option<BindingId>,
}

impl<AttachmentId, BindingId> Default for AttachPropState<AttachmentId, BindingId> {
    fn default() -> Self {
        Self {
            attachment: None,
            binding: None,
        }
    }
}

impl<AttachmentId, BindingId> AttachPropState<AttachmentId, BindingId>
where
    AttachmentId: Copy + Eq + Hash,
    BindingId: Copy + Eq + Hash,
{
    #[must_use]
    pub fn enter<A, R>(rig: &mut R, attachment_name: Crc32, asset: Option<&A>) -> Self
    where
        R: CharacterAttachmentBinding<A, AttachmentId = AttachmentId, BindingId = BindingId>,
    {
        let Some(asset) = asset else {
            return Self::default();
        };
        let Some(attachment) = rig.attachment_by_name_crc(attachment_name) else {
            return Self::default();
        };
        let binding = rig.bind_attachment_asset(attachment, asset);
        Self {
            attachment: binding.map(|_| attachment),
            binding,
        }
    }

    pub fn exit<A, R>(self, rig: &mut R)
    where
        R: CharacterAttachmentBinding<A, AttachmentId = AttachmentId, BindingId = BindingId>,
    {
        let (Some(attachment), Some(binding)) = (self.attachment, self.binding) else {
            return;
        };
        if rig.attachment_binding(attachment) == Some(binding) {
            rig.clear_attachment_binding(attachment);
        }
    }

    #[must_use]
    pub const fn attachment(&self) -> Option<AttachmentId> {
        self.attachment
    }

    #[must_use]
    pub const fn binding(&self) -> Option<BindingId> {
        self.binding
    }
}

/// Per-installed-clip state for `HideAttachment`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HideAttachmentState<A, E> {
    attachment: Option<A>,
    entity_target: Option<E>,
    restored_visibility: AttachmentVisibility,
}

impl<A, E> Default for HideAttachmentState<A, E> {
    fn default() -> Self {
        Self {
            attachment: None,
            entity_target: None,
            restored_visibility: AttachmentVisibility::empty(),
        }
    }
}

impl<A, E> HideAttachmentState<A, E>
where
    A: Copy + Eq + Hash,
    E: Copy + Eq + Hash,
{
    /// Executes shipping `HideAttachment::OnEnter`.
    #[must_use]
    pub fn enter<R>(rig: &mut R, attachment_name: Crc32, force_visible_on_exit: bool) -> Self
    where
        R: CharacterAttachmentRig<AttachmentId = A, EntityTargetId = E>,
    {
        let Some(attachment) = rig.attachment_by_name_crc(attachment_name) else {
            return Self::default();
        };

        let restored_visibility = if force_visible_on_exit {
            AttachmentVisibility::empty()
        } else {
            rig.visibility(attachment)
        };

        rig.set_main_pass_hidden(attachment, true);
        rig.attachment_visibility_changed(attachment, false);

        let entity_target = rig.entity_attachment_target(attachment);
        if let Some(target) = entity_target {
            rig.set_entity_target_hidden(target, true);
        }

        Self {
            attachment: Some(attachment),
            entity_target,
            restored_visibility,
        }
    }

    /// Executes shipping `HideAttachment::OnExit` and consumes the installed
    /// state so it cannot accidentally restore twice.
    pub fn exit<R>(self, rig: &mut R)
    where
        R: CharacterAttachmentRig<AttachmentId = A, EntityTargetId = E>,
    {
        let Some(attachment) = self.attachment else {
            return;
        };

        rig.set_main_pass_hidden(
            attachment,
            self.restored_visibility
                .contains(AttachmentVisibility::MAIN_PASS),
        );
        rig.set_recursion_hidden(
            attachment,
            self.restored_visibility
                .contains(AttachmentVisibility::RECURSION),
        );
        rig.set_shadow_hidden(
            attachment,
            self.restored_visibility
                .contains(AttachmentVisibility::SHADOW),
        );
        rig.attachment_visibility_changed(
            attachment,
            !self
                .restored_visibility
                .contains(AttachmentVisibility::MAIN_PASS),
        );

        if let Some(target) = self.entity_target
            && rig.entity_attachment_target(attachment) == Some(target)
        {
            rig.set_entity_target_hidden(target, false);
        }
    }

    #[must_use]
    pub const fn attachment(&self) -> Option<A> {
        self.attachment
    }

    #[must_use]
    pub const fn restored_visibility(&self) -> AttachmentVisibility {
        self.restored_visibility
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Rig {
        visibility: AttachmentVisibility,
        target: Option<u32>,
        target_hidden: bool,
        notifications: Vec<bool>,
    }

    impl CharacterAttachmentRig for Rig {
        type AttachmentId = u32;
        type EntityTargetId = u32;

        fn attachment_by_name_crc(&self, name: Crc32) -> Option<Self::AttachmentId> {
            (name.value() == 42).then_some(7)
        }

        fn visibility(&self, _attachment: Self::AttachmentId) -> AttachmentVisibility {
            self.visibility
        }

        fn set_main_pass_hidden(&mut self, _attachment: Self::AttachmentId, hidden: bool) {
            self.visibility.set(AttachmentVisibility::MAIN_PASS, hidden);
        }

        fn set_shadow_hidden(&mut self, _attachment: Self::AttachmentId, hidden: bool) {
            self.visibility.set(AttachmentVisibility::SHADOW, hidden);
        }

        fn set_recursion_hidden(&mut self, _attachment: Self::AttachmentId, hidden: bool) {
            self.visibility.set(AttachmentVisibility::RECURSION, hidden);
        }

        fn attachment_visibility_changed(
            &mut self,
            _attachment: Self::AttachmentId,
            visible: bool,
        ) {
            self.notifications.push(visible);
        }

        fn entity_attachment_target(
            &self,
            _attachment: Self::AttachmentId,
        ) -> Option<Self::EntityTargetId> {
            self.target
        }

        fn set_entity_target_hidden(&mut self, _target: Self::EntityTargetId, hidden: bool) {
            self.target_hidden = hidden;
        }
    }

    #[test]
    fn hide_attachment_restores_all_captured_visibility_and_entity_target() {
        let original = AttachmentVisibility::SHADOW | AttachmentVisibility::RECURSION;
        let mut rig = Rig {
            visibility: original,
            target: Some(11),
            target_hidden: false,
            notifications: Vec::new(),
        };

        let state = HideAttachmentState::enter(&mut rig, Crc32::from_u32(42), false);
        assert!(rig.visibility.contains(AttachmentVisibility::MAIN_PASS));
        assert!(rig.target_hidden);
        assert_eq!(rig.notifications, [false]);

        state.exit(&mut rig);
        assert_eq!(rig.visibility, original);
        assert!(!rig.target_hidden);
        assert_eq!(rig.notifications, [false, true]);
    }

    #[test]
    fn force_visible_on_exit_discards_preexisting_hidden_flags() {
        let mut rig = Rig {
            visibility: AttachmentVisibility::all(),
            target: None,
            target_hidden: false,
            notifications: Vec::new(),
        };

        HideAttachmentState::enter(&mut rig, Crc32::from_u32(42), true).exit(&mut rig);

        assert!(rig.visibility.is_empty());
        assert_eq!(rig.notifications, [false, true]);
    }

    #[test]
    fn exit_does_not_unhide_a_replaced_entity_target() {
        let mut rig = Rig {
            visibility: AttachmentVisibility::empty(),
            target: Some(11),
            target_hidden: false,
            notifications: Vec::new(),
        };

        let state = HideAttachmentState::enter(&mut rig, Crc32::from_u32(42), false);
        rig.target = Some(12);
        state.exit(&mut rig);

        assert!(rig.target_hidden);
    }

    #[derive(Debug, Default)]
    struct BindingRig {
        current: Option<u32>,
        next: u32,
        clears: usize,
    }

    impl CharacterAttachmentRig for BindingRig {
        type AttachmentId = u32;
        type EntityTargetId = u32;

        fn attachment_by_name_crc(&self, name: Crc32) -> Option<Self::AttachmentId> {
            (name.value() == 42).then_some(7)
        }

        fn visibility(&self, _attachment: Self::AttachmentId) -> AttachmentVisibility {
            AttachmentVisibility::empty()
        }

        fn set_main_pass_hidden(&mut self, _attachment: Self::AttachmentId, _hidden: bool) {}

        fn set_shadow_hidden(&mut self, _attachment: Self::AttachmentId, _hidden: bool) {}

        fn set_recursion_hidden(&mut self, _attachment: Self::AttachmentId, _hidden: bool) {}

        fn attachment_visibility_changed(
            &mut self,
            _attachment: Self::AttachmentId,
            _visible: bool,
        ) {
        }

        fn entity_attachment_target(
            &self,
            _attachment: Self::AttachmentId,
        ) -> Option<Self::EntityTargetId> {
            None
        }

        fn set_entity_target_hidden(&mut self, _target: Self::EntityTargetId, _hidden: bool) {}
    }

    impl CharacterAttachmentBinding<&'static str> for BindingRig {
        type BindingId = u32;

        fn bind_attachment_asset(
            &mut self,
            _attachment: Self::AttachmentId,
            _asset: &&'static str,
        ) -> Option<Self::BindingId> {
            self.next += 1;
            self.current = Some(self.next);
            self.current
        }

        fn attachment_binding(&self, _attachment: Self::AttachmentId) -> Option<Self::BindingId> {
            self.current
        }

        fn clear_attachment_binding(&mut self, _attachment: Self::AttachmentId) {
            self.current = None;
            self.clears += 1;
        }
    }

    #[test]
    fn attach_prop_clears_only_the_binding_it_installed() {
        let mut rig = BindingRig::default();
        let state = AttachPropState::enter(&mut rig, Crc32::from_u32(42), Some(&"prop"));
        state.exit::<&'static str, _>(&mut rig);
        assert_eq!(rig.clears, 1);

        let state = AttachPropState::enter(&mut rig, Crc32::from_u32(42), Some(&"prop"));
        rig.current = Some(99);
        state.exit::<&'static str, _>(&mut rig);
        assert_eq!(rig.clears, 1);
        assert_eq!(rig.current, Some(99));
    }
}
