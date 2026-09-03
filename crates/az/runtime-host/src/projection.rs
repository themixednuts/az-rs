//! Composed runtime projections.
//!
//! Project and gem crates contribute runtime projections through their
//! contribution's registrar; the runtime-host process that composed them owns
//! the resulting set. Nothing is discovered at link time, so a host serves
//! exactly the projections its own composition registered — never whatever
//! happened to be linked beside it.

use std::path::Path;

use az_gem_contract::{Registries, RegistryEntry, Unconditional};
use az_proto_runtime::{
    RuntimeAssetPackageContainer, RuntimeAssetPackageRoot, RuntimeLaunchSnapshot, RuntimeRole,
    RuntimeState, RuntimeStatus, RuntimeViewportFrame,
};
use thiserror::Error;

/// Runtime-specific failure reported by a project/gem projection.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{reason}")]
pub struct RuntimeProjectionError {
    reason: String,
}

impl RuntimeProjectionError {
    #[must_use]
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl From<&str> for RuntimeProjectionError {
    fn from(reason: &str) -> Self {
        Self::new(reason)
    }
}

impl From<String> for RuntimeProjectionError {
    fn from(reason: String) -> Self {
        Self::new(reason)
    }
}

/// Context passed to a runtime projection during launch.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeProjectionLaunchContext<'a> {
    pub runtime_id: &'a str,
    pub role: RuntimeRole,
    pub side_channel_root: &'a Path,
    pub snapshot: &'a RuntimeLaunchSnapshot,
}

impl<'a> RuntimeProjectionLaunchContext<'a> {
    #[must_use]
    pub fn asset_package_roots(&self) -> &'a [RuntimeAssetPackageRoot] {
        &self.snapshot.asset_package_roots
    }

    #[must_use]
    pub fn primary_asset_package_root(&self) -> Option<&'a RuntimeAssetPackageRoot> {
        self.asset_package_roots().first()
    }

    #[must_use]
    pub fn asset_package_root(
        &self,
        profile: &str,
        asset_platform: &str,
    ) -> Option<&'a RuntimeAssetPackageRoot> {
        asset_package_root(self.asset_package_roots(), profile, asset_platform)
    }

    pub fn asset_package_roots_for_container(
        &self,
        container: RuntimeAssetPackageContainer,
    ) -> impl Iterator<Item = &'a RuntimeAssetPackageRoot> + 'a {
        asset_package_roots_for_container(self.asset_package_roots(), container)
    }
}

/// Context passed to a runtime projection during status refresh.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeProjectionStatusContext<'a> {
    pub runtime_id: &'a str,
    pub status: &'a RuntimeStatus,
    pub side_channel_root: &'a Path,
    pub snapshot: &'a RuntimeLaunchSnapshot,
}

impl<'a> RuntimeProjectionStatusContext<'a> {
    #[must_use]
    pub fn asset_package_roots(&self) -> &'a [RuntimeAssetPackageRoot] {
        &self.snapshot.asset_package_roots
    }

    #[must_use]
    pub fn primary_asset_package_root(&self) -> Option<&'a RuntimeAssetPackageRoot> {
        self.asset_package_roots().first()
    }

    #[must_use]
    pub fn asset_package_root(
        &self,
        profile: &str,
        asset_platform: &str,
    ) -> Option<&'a RuntimeAssetPackageRoot> {
        asset_package_root(self.asset_package_roots(), profile, asset_platform)
    }
}

/// Context passed to a runtime projection during stop.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeProjectionStopContext<'a> {
    pub runtime_id: &'a str,
    pub status: &'a RuntimeStatus,
    pub side_channel_root: &'a Path,
    pub snapshot: &'a RuntimeLaunchSnapshot,
    pub preserve: bool,
}

impl<'a> RuntimeProjectionStopContext<'a> {
    #[must_use]
    pub fn asset_package_roots(&self) -> &'a [RuntimeAssetPackageRoot] {
        &self.snapshot.asset_package_roots
    }

    #[must_use]
    pub fn primary_asset_package_root(&self) -> Option<&'a RuntimeAssetPackageRoot> {
        self.asset_package_roots().first()
    }
}

/// Context passed to a runtime projection when the editor asks for a viewport.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeProjectionViewportContext<'a> {
    pub runtime_id: &'a str,
    pub status: &'a RuntimeStatus,
    pub side_channel_root: &'a Path,
    pub snapshot: &'a RuntimeLaunchSnapshot,
}

impl<'a> RuntimeProjectionViewportContext<'a> {
    #[must_use]
    pub fn asset_package_roots(&self) -> &'a [RuntimeAssetPackageRoot] {
        &self.snapshot.asset_package_roots
    }

    #[must_use]
    pub fn primary_asset_package_root(&self) -> Option<&'a RuntimeAssetPackageRoot> {
        self.asset_package_roots().first()
    }
}

fn asset_package_root<'a>(
    roots: &'a [RuntimeAssetPackageRoot],
    profile: &str,
    asset_platform: &str,
) -> Option<&'a RuntimeAssetPackageRoot> {
    roots.iter().find(|root| {
        root.profile == profile && root.asset_platform.eq_ignore_ascii_case(asset_platform)
    })
}

fn asset_package_roots_for_container(
    roots: &[RuntimeAssetPackageRoot],
    container: RuntimeAssetPackageContainer,
) -> impl Iterator<Item = &RuntimeAssetPackageRoot> + '_ {
    roots.iter().filter(move |root| root.container == container)
}

/// State change returned by a project/gem runtime projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeProjectionUpdate {
    pub state: RuntimeState,
    pub diagnostic: String,
    pub viewport_frame: Option<RuntimeViewportFrame>,
}

impl RuntimeProjectionUpdate {
    #[must_use]
    pub const fn new(state: RuntimeState) -> Self {
        Self {
            state,
            diagnostic: String::new(),
            viewport_frame: None,
        }
    }

    #[must_use]
    pub const fn running() -> Self {
        Self::new(RuntimeState::Running)
    }

    #[must_use]
    pub fn failed(reason: impl Into<String>) -> Self {
        Self::new(RuntimeState::Failed).with_diagnostic(reason)
    }

    #[must_use]
    pub fn stopped(preserve: bool) -> Self {
        let update = Self::new(RuntimeState::Stopped);
        if preserve {
            update.with_diagnostic("stopped and preserved")
        } else {
            update
        }
    }

    #[must_use]
    pub fn with_diagnostic(mut self, diagnostic: impl Into<String>) -> Self {
        self.diagnostic = diagnostic.into();
        self
    }

    #[must_use]
    pub fn with_viewport_frame(mut self, frame: RuntimeViewportFrame) -> Self {
        self.viewport_frame = Some(frame);
        self
    }
}

/// Project/gem-owned runtime projection implementation.
pub trait RuntimeProjection: Send {
    /// Bring this projection up for one runtime instance.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeProjectionError`] if the projection cannot start for
    /// the launch context it was given — the implementation decides what that
    /// means; the host records it as the instance's failure diagnostic.
    fn launch(
        &mut self,
        context: &RuntimeProjectionLaunchContext<'_>,
    ) -> Result<RuntimeProjectionUpdate, RuntimeProjectionError>;

    /// Report a state change since the last poll, if any.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeProjectionError`] if the projection cannot determine
    /// its state; the host records it as the instance's failure diagnostic.
    /// The default implementation, which reports no change, never fails.
    fn status(
        &mut self,
        _context: &RuntimeProjectionStatusContext<'_>,
    ) -> Result<Option<RuntimeProjectionUpdate>, RuntimeProjectionError> {
        Ok(None)
    }

    /// Tear this projection down, honouring `context.preserve`.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeProjectionError`] if the projection cannot stop
    /// cleanly; the host records it as the instance's failure diagnostic. The
    /// default implementation, which just reports stopped, never fails.
    fn stop(
        &mut self,
        context: &RuntimeProjectionStopContext<'_>,
    ) -> Result<RuntimeProjectionUpdate, RuntimeProjectionError> {
        Ok(RuntimeProjectionUpdate::stopped(context.preserve))
    }

    /// Produce the projection's current viewport frame, if it renders one.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeProjectionError`] if the frame cannot be produced; the
    /// host records it as the instance's failure diagnostic. The default
    /// implementation, which renders nothing, never fails.
    fn viewport_frame(
        &mut self,
        _context: &RuntimeProjectionViewportContext<'_>,
    ) -> Result<Option<RuntimeViewportFrame>, RuntimeProjectionError> {
        Ok(None)
    }
}

/// Factory for one statically registered runtime projection.
pub type RuntimeProjectionFactory = fn() -> Box<dyn RuntimeProjection>;

/// One runtime projection contributed to a runtime-host composition.
///
/// The name is the compose key: it is what a launch diagnostic attributes a
/// phase failure to, so two projections under one name would each be blamed
/// for the other's failures. Roles and launch profiles are not part of that
/// identity — they select which projections serve a *launch request*, which is
/// per-request policy, not composition identity.
#[derive(Clone, Copy)]
pub struct RuntimeProjectionRegistration {
    priority: i32,
    name: &'static str,
    roles: &'static [RuntimeRole],
    launch_profiles: &'static [&'static str],
    factory: RuntimeProjectionFactory,
}

impl RuntimeProjectionRegistration {
    /// Register a projection with default priority.
    #[must_use]
    pub const fn new(
        name: &'static str,
        roles: &'static [RuntimeRole],
        launch_profiles: &'static [&'static str],
        factory: RuntimeProjectionFactory,
    ) -> Self {
        Self {
            priority: 0,
            name,
            roles,
            launch_profiles,
            factory,
        }
    }

    /// Override dispatch precedence. Higher priority projections are matched
    /// first, which keeps project-specific projections ahead of broad
    /// fallbacks.
    ///
    /// This is real domain precedence, not a stand-in for link order: a launch
    /// runs its matching projections in this order and the first one to claim
    /// the viewport wins, so the order is the answer. Ties fall to composition
    /// order — the resolved lock closure — and nothing else.
    #[must_use]
    pub const fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    #[must_use]
    pub const fn priority(&self) -> i32 {
        self.priority
    }

    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    #[must_use]
    pub const fn roles(&self) -> &'static [RuntimeRole] {
        self.roles
    }

    #[must_use]
    pub const fn launch_profiles(&self) -> &'static [&'static str] {
        self.launch_profiles
    }

    #[must_use]
    pub fn matches(&self, role: RuntimeRole, launch_profile: &str) -> bool {
        let role_matches = self.roles.is_empty() || self.roles.contains(&role);
        let profile_matches =
            self.launch_profiles.is_empty() || self.launch_profiles.contains(&launch_profile);
        role_matches && profile_matches
    }

    #[must_use]
    pub fn projection(&self) -> Box<dyn RuntimeProjection> {
        (self.factory)()
    }
}

impl RegistryEntry for RuntimeProjectionRegistration {
    type Key = &'static str;
    type Requires = Unconditional;

    fn registry_name() -> &'static str {
        "runtime-projection"
    }

    fn key(&self) -> &'static str {
        self.name
    }
}

/// Projections this host composed, in dispatch order.
///
/// Precedence descending, then composition order. The sort is stable, so the
/// second key is the resolved closure order the composition already carries —
/// there is no third tiebreak and no re-sort anywhere downstream.
#[must_use]
pub fn projections(registries: &Registries) -> Vec<&RuntimeProjectionRegistration> {
    let mut projections = registries
        .get::<RuntimeProjectionRegistration>()
        .map(|registry| registry.entries().collect::<Vec<_>>())
        .unwrap_or_default();
    projections.sort_by_key(|projection| std::cmp::Reverse(projection.priority()));
    projections
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn launch_context_exposes_service_provided_package_roots() {
        let mut snapshot = RuntimeLaunchSnapshot::new(
            "local.runtime",
            uuid::Uuid::from_bytes([0x42; 16]),
            "lighting",
            RuntimeRole::EditorWorld,
            "projects/runtime",
            "projects/runtime/.azoth/workspaces/lighting",
        );
        snapshot.asset_package_roots = vec![
            package_root(
                "pc-dev",
                "pc",
                RuntimeAssetPackageContainer::Loose,
                "projects/runtime/target/azoth/packages/pc-dev/lighting/loose",
                "projects/runtime/target/azoth/packages/pc-dev/lighting/loose",
                "projects/runtime/target/azoth/packages/pc-dev/lighting/loose/assetcatalog.bin",
            ),
            package_root(
                "pc-release",
                "pc",
                RuntimeAssetPackageContainer::Pak,
                "projects/runtime/target/azoth/packages/pc-release/lighting",
                "projects/runtime/target/azoth/packages/pc-release/lighting/pc-release.pak",
                "projects/runtime/target/azoth/packages/pc-release/lighting/assetcatalog.bin",
            ),
        ];
        let context = RuntimeProjectionLaunchContext {
            runtime_id: "editor-world",
            role: RuntimeRole::EditorWorld,
            side_channel_root: Path::new("projects/runtime/.azoth/session/side-channels"),
            snapshot: &snapshot,
        };

        assert_eq!(context.asset_package_roots().len(), 2);
        assert_eq!(
            context.primary_asset_package_root().unwrap().profile,
            "pc-dev"
        );
        assert_eq!(
            context
                .asset_package_root("pc-release", "PC")
                .unwrap()
                .payload_path,
            "projects/runtime/target/azoth/packages/pc-release/lighting/pc-release.pak"
        );
        assert_eq!(
            context
                .asset_package_roots_for_container(RuntimeAssetPackageContainer::Pak)
                .count(),
            1
        );
    }

    fn package_root(
        profile: &str,
        asset_platform: &str,
        container: RuntimeAssetPackageContainer,
        mount_root: &str,
        payload_path: &str,
        catalog_path: &str,
    ) -> RuntimeAssetPackageRoot {
        RuntimeAssetPackageRoot {
            profile: profile.to_string(),
            asset_platform: asset_platform.to_string(),
            container,
            mount_root: mount_root.to_string(),
            payload_path: payload_path.to_string(),
            catalog_path: catalog_path.to_string(),
            release_id: "ab".repeat(32),
        }
    }
}
