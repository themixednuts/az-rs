use crate::types::{
    ClientCapabilities, JwtClaims, SteamAppId, SteamTicketHex, TokenClientCapabilities,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub struct LoginQueueRequest {
    pub character_id: String,
    pub client_capabilities: ClientCapabilities,
    pub is_trial_owner: bool,
    pub steam_app_id: SteamAppId,
    pub steam_auth_ticket: SteamTicketHex,
    pub world_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub struct LoginQueueRequestEnvelope {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_queue_request: Option<LoginQueueRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_application: Option<Vec<LoginQueueClientApplication>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub struct LoginQueueClientApplication {}

#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub struct LoginQueueRefreshRequest {
    pub client_capabilities: ClientCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub struct LoginQueueData {
    pub ticket_id: String,
    pub allow_queue_transfer: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_time: Option<i32>,
    pub position: i32,
    pub refresh_interval: i32,
    pub queue_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<LoginQueueToken>,
}

#[derive(Clone, Default, Serialize, Deserialize, ToSchema, JsonSchema, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "PascalCase")]
pub struct LoginQueueToken {
    pub token_version: u32,
    pub host_hash: String,
    pub account_age: u32,
    pub character_id: String,
    #[serde(default, skip_serializing_if = "TokenClientCapabilities::is_empty")]
    pub client_capabilities: TokenClientCapabilities,
    pub generate_time: u32,
    pub issue_time: u32,
    pub is_permanent_app_owner: bool,
    pub is_trial_owner: bool,
    pub jwt_claims: JwtClaims,
    pub persona_id: String,
    pub rep_address: String,
    pub ticket_id: String,
    pub steam_app_id: u32,
    pub steam_user_id: String,
    pub channel_id: String,
    pub location_group_id: String,
    pub location_id: String,
    pub world_id: String,
    pub signature: String,
}

impl std::fmt::Debug for LoginQueueToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LoginQueueToken")
            .field("value", &"[REDACTED]")
            .field("token_version", &self.token_version)
            .field("signature_len", &self.signature.len())
            .field("jwt_claims_len", &self.jwt_claims.len())
            // Deliberately partial: the remaining fields carry the bearer token
            // and player identifiers, so this impl reports lengths instead.
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub struct LoginQueueResponse {
    pub login_queue_response: LoginQueueData,
}

#[cfg(test)]
mod tests {
    use super::{LoginQueueRefreshRequest, LoginQueueRequestEnvelope, LoginQueueResponse};

    #[test]
    fn refresh_request_can_be_client_application_only() {
        let request: LoginQueueRequestEnvelope =
            serde_json::from_str(r#"{"ClientApplication":[]}"#).expect("login queue request");

        assert!(request.login_queue_request.is_none());
        assert_eq!(
            request
                .client_application
                .as_ref()
                .expect("client application")
                .len(),
            0
        );

        let value = serde_json::to_value(&request).expect("login queue request value");
        assert_eq!(value["ClientApplication"], serde_json::json!([]));
        assert!(value.get("LoginQueueRequest").is_none());
    }

    #[test]
    fn response_can_omit_estimated_time() {
        let response: LoginQueueResponse = serde_json::from_str(
            r#"{"LoginQueueResponse":{"TicketId":"ticket-id","AllowQueueTransfer":false,"Position":0,"RefreshInterval":2,"QueueName":"DEFAULT000"}}"#,
        )
        .expect("login queue response");

        assert_eq!(response.login_queue_response.estimated_time, None);

        let value = serde_json::to_value(&response).expect("login queue response value");
        assert!(value["LoginQueueResponse"].get("EstimatedTime").is_none());
    }

    #[test]
    fn refresh_request_is_typed_client_capabilities_body() {
        let request: LoginQueueRefreshRequest =
            serde_json::from_str(r#"{"ClientCapabilities":[]}"#).expect("refresh request");

        assert!(request.client_capabilities.is_empty());
        assert_eq!(
            serde_json::to_value(&request).expect("refresh request value"),
            serde_json::json!({"ClientCapabilities":[]})
        );
    }
}
