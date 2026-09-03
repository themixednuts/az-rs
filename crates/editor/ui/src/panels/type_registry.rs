//! Reflected type registry shared by editor panels.

use az_proto_project::vnext::TypeRegistrySnapshot;
use gpui::Global;

/// Authoritative project-host registry snapshot for the attached session.
#[derive(Debug, Clone)]
pub struct EditorTypeRegistry {
    pub snapshot: TypeRegistrySnapshot,
}

impl EditorTypeRegistry {
    #[must_use]
    pub const fn new(snapshot: TypeRegistrySnapshot) -> Self {
        Self { snapshot }
    }
}

impl Global for EditorTypeRegistry {}
