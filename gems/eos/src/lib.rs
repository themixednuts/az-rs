//! Epic Online Services platform gem.
//!
//! The gem is the project-visible capability. The native SDK bindings and Bevy
//! runtime plugin remain implementation crates under `crates/integrations/eos`.

#![forbid(unsafe_code)]

use az_gem_contract::{Contribution, GemContext, contribution};
use serde::{Deserialize, Serialize};

#[cfg(feature = "client-runtime")]
pub use eos_plugin::EosClientPlugin;
#[cfg(any(
    feature = "client-runtime",
    feature = "anti-cheat-client",
    feature = "server-runtime"
))]
pub use eos_plugin::EosService;
#[cfg(feature = "anti-cheat-client")]
pub use eos_plugin::{AntiCheatClientState, AntiCheatService, EosAntiCheatReady, EosPlugin};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EosPlatformSettings {
    pub product_id: String,
    pub sandbox_id: String,
    pub deployment_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
}

impl EosPlatformSettings {
    #[must_use]
    pub fn new(
        product_id: impl Into<String>,
        sandbox_id: impl Into<String>,
        deployment_id: impl Into<String>,
    ) -> Self {
        Self {
            product_id: product_id.into(),
            sandbox_id: sandbox_id.into(),
            deployment_id: deployment_id.into(),
            client_id: None,
            client_secret: None,
        }
    }

    #[must_use]
    pub fn with_client_credentials(
        mut self,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> Self {
        self.client_id = Some(client_id.into());
        self.client_secret = Some(client_secret.into());
        self
    }
}

// This package implements three of its gem's contributions, so each block
// names which one it is with a bare token. All three are declared
// unconditionally: the cargo features decide what the SDK crate *links*, not
// whether a declared contribution exists, so a feature belongs inside a body
// and never in whether the entry item is there to call. Today every body is
// empty — the platform runtime is a Bevy plugin, and a plugin is not a registry
// entry, so this gem owns no AZ type, asset type, lowering, or prefab type.

/// The gem's `client-runtime` contribution.
struct Client;

#[contribution(client_runtime)]
impl Contribution for Client {
    fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {}
}

/// The gem's `anti-cheat-client` contribution.
struct AntiCheat;

#[contribution(anti_cheat_client)]
impl Contribution for AntiCheat {
    fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {}
}

/// The gem's `server-runtime` contribution.
struct Server;

#[contribution(server_runtime)]
impl Contribution for Server {
    fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {}
}
