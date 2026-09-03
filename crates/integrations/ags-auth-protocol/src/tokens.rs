use crate::types::{
    AgeGroup, AgsAccessToken, AgsAccountKind, DisplayNameUpdateReason, IdentityType, PlatformType,
    SteamTicketHex,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema, JsonSchema)]
pub struct PlatformAuth {
    pub ticket: SteamTicketHex,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttributionData {
    pub os: String,
    pub resolution: String,
    pub language: String,
    pub platform: String,
    pub created: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TokensRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_type: Option<PlatformType>,
    pub platform_auth: PlatformAuth,
    pub fallback_token: AgsAccessToken,
    pub attribution_data: Option<AttributionData>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub identity_type: IdentityType,
    pub identity_id: String,
    pub persona_id: String,
    pub age_group: AgeGroup,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub identity_kind: Option<AgsAccountKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_new_account: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name_update_reason: Option<DisplayNameUpdateReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<PlatformType>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TokensResponse {
    pub access_token: AgsAccessToken,
    pub access_token_expiration_date: i64,
    pub expires_in: i64,
    pub fallback_token: AgsAccessToken,
    pub platform_account: Identity,
    pub account: Identity,
    pub auxiliary_platform_accounts: Vec<Identity>,
    pub penalties: Vec<TokenPenalty>,
    pub region: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TokenPenalty {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub penalty_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::TokensRequest;
    use crate::types::PlatformType;

    #[test]
    fn request_can_omit_platform_type() {
        let request: TokensRequest = serde_json::from_str(
            r#"{"platformAuth":{"ticket":"deadbeef"},"fallbackToken":"fallback","attributionData":null}"#,
        )
        .expect("tokens request");

        assert_eq!(request.platform_type, None);
        assert_eq!(request.platform_auth.ticket.as_str(), "deadbeef");
        assert_eq!(request.fallback_token.as_str(), "fallback");

        let value = serde_json::to_value(&request).expect("tokens request value");
        assert!(value.get("platformType").is_none());
    }

    #[test]
    fn request_typed_platform_serializes_to_live_wire_string() {
        let request: TokensRequest = serde_json::from_str(
            r#"{"platformType":"steam","platformAuth":{"ticket":"deadbeef"},"fallbackToken":"fallback","attributionData":null}"#,
        )
        .expect("tokens request");

        assert_eq!(request.platform_type, Some(PlatformType::Steam));
        assert_eq!(
            serde_json::to_value(&request).expect("value")["platformType"],
            "steam"
        );
    }
}
