use std::fmt;

use crate::capability::{CapSet, HostCapability};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum GemTargetRole {
    Game,
    P2p,
    Client,
    Server,
    Unified,
    HeadlessServer,
    ProjectHost,
    AssetProcessor,
    AssetWorker,
    RuntimeHost,
    Tool,
    NamedService,
}

impl GemTargetRole {
    /// Host capabilities this role implies (ADR 0041).
    #[must_use]
    pub const fn provided_caps(self) -> CapSet {
        match self {
            Self::Client => CapSet::EMPTY
                .with(HostCapability::HostsWorld)
                .with(HostCapability::HasClient)
                .with(HostCapability::OwnsFixedClock),
            Self::Server | Self::HeadlessServer => CapSet::EMPTY
                .with(HostCapability::HostsWorld)
                .with(HostCapability::HasAuthority)
                .with(HostCapability::OwnsFixedClock),
            Self::Unified | Self::Game | Self::P2p => CapSet::EMPTY
                .with(HostCapability::HostsWorld)
                .with(HostCapability::HasClient)
                .with(HostCapability::HasAuthority)
                .with(HostCapability::OwnsFixedClock),
            Self::ProjectHost
            | Self::AssetProcessor
            | Self::AssetWorker
            | Self::RuntimeHost
            | Self::Tool
            | Self::NamedService => CapSet::EMPTY,
        }
    }

    /// Whether this role's composition owns a Bevy `App`.
    #[must_use]
    pub const fn hosts_world(self) -> bool {
        self.provided_caps().has(HostCapability::HostsWorld)
    }
}

impl fmt::Display for GemTargetRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Game => "game",
            Self::P2p => "p2p",
            Self::Client => "client",
            Self::Server => "server",
            Self::Unified => "unified",
            Self::HeadlessServer => "headless-server",
            Self::ProjectHost => "project-host",
            Self::AssetProcessor => "asset-processor",
            Self::AssetWorker => "asset-worker",
            Self::RuntimeHost => "runtime-host",
            Self::Tool => "tool",
            Self::NamedService => "named-service",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_has_fixed_clock_without_authority() {
        let caps = GemTargetRole::Client.provided_caps();
        assert!(caps.has(HostCapability::OwnsFixedClock));
        assert!(!caps.has(HostCapability::HasAuthority));
        assert!(GemTargetRole::Client.hosts_world());
    }

    #[test]
    fn unified_is_the_conjunction() {
        let unified = GemTargetRole::Unified.provided_caps();
        let game = GemTargetRole::Game.provided_caps();
        assert_eq!(unified, game);
        assert!(unified.has(HostCapability::HasClient));
        assert!(unified.has(HostCapability::HasAuthority));
    }

    #[test]
    fn asset_worker_has_no_app() {
        assert!(!GemTargetRole::AssetWorker.hosts_world());
    }
}
