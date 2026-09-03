use crate::types::{
    AgsAuthHeaderName, AgsGameSlug, AgsGatewayServiceTag, AgsTokenVersion, ClientCapabilities,
    SteamAppId, SteamAuthProductId,
};
use az_core::BuildIdentity;

/// Project-owned routing identity for the AGS gateway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgsGatewayProfile {
    pub game: AgsGameSlug,
    pub service_tag: AgsGatewayServiceTag,
    pub auth_header_name: AgsAuthHeaderName,
}

impl AgsGatewayProfile {
    #[must_use]
    pub const fn new(
        game: AgsGameSlug,
        service_tag: AgsGatewayServiceTag,
        auth_header_name: AgsAuthHeaderName,
    ) -> Self {
        Self {
            game,
            service_tag,
            auth_header_name,
        }
    }
}

/// Resolved AGS client identity for one runnable game target.
///
/// This is supplied by the project/runtime profile. Protocol clients consume it
/// when shaping token-service, login-queue, and native registration payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgsGameClientProfile {
    pub game: AgsGameSlug,
    pub gateway_service_tag: AgsGatewayServiceTag,
    pub auth_header_name: AgsAuthHeaderName,
    pub build: BuildIdentity,
    pub steam_app_id: SteamAppId,
    pub steam_auth_product_id: SteamAuthProductId,
    pub token_version: AgsTokenVersion,
    pub client_capabilities: ClientCapabilities,
}

impl AgsGameClientProfile {
    #[must_use]
    pub fn new(
        gateway: AgsGatewayProfile,
        build: BuildIdentity,
        steam_app_id: SteamAppId,
        steam_auth_product_id: SteamAuthProductId,
        token_version: AgsTokenVersion,
        client_capabilities: ClientCapabilities,
    ) -> Self {
        Self {
            game: gateway.game,
            gateway_service_tag: gateway.service_tag,
            auth_header_name: gateway.auth_header_name,
            build,
            steam_app_id,
            steam_auth_product_id,
            token_version,
            client_capabilities,
        }
    }
}
