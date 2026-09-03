use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;
use utoipa::ToSchema;
use zeroize::{Zeroize, ZeroizeOnDrop};

pub const GAME_AUTH_STEAM_TICKET_HEX_CHARS: usize = 276 * 2;

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum PlatformType {
    #[default]
    Steam,
}

impl PlatformType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Steam => "steam",
        }
    }
}

impl fmt::Display for PlatformType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum IdentityType {
    #[default]
    Steam,
    Ags,
}

impl IdentityType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Steam => "steam",
            Self::Ags => "ags",
        }
    }
}

impl fmt::Display for IdentityType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, JsonSchema,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgeGroup {
    Adult,
    #[default]
    Unknown,
}

impl AgeGroup {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Adult => "ADULT",
            Self::Unknown => "UNKNOWN",
        }
    }
}

impl fmt::Display for AgeGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum AgsAccountKind {
    #[default]
    Full,
}

impl AgsAccountKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
        }
    }
}

impl fmt::Display for AgsAccountKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, JsonSchema,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DisplayNameUpdateReason {
    #[default]
    PlayerInitiatedFirstTime,
}

impl DisplayNameUpdateReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PlayerInitiatedFirstTime => "PLAYER_INITIATED_FIRST_TIME",
        }
    }
}

impl fmt::Display for DisplayNameUpdateReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, JsonSchema,
)]
#[serde(transparent)]
pub struct SteamAppId(pub u32);

impl SteamAppId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for SteamAppId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, JsonSchema,
)]
#[serde(transparent)]
pub struct SteamAuthProductId(pub u32);

impl SteamAuthProductId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for SteamAuthProductId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, JsonSchema,
)]
#[serde(transparent)]
pub struct AgsTokenVersion(pub u32);

impl AgsTokenVersion {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for AgsTokenVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(
    Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, JsonSchema,
)]
#[serde(transparent)]
pub struct AgsGameSlug(String);

impl AgsGameSlug {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        debug_assert!(!value.trim().is_empty(), "AGS game slug cannot be empty");
        Self(value)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgsGameSlug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(
    Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, JsonSchema,
)]
#[serde(transparent)]
pub struct AgsGatewayServiceTag(String);

impl AgsGatewayServiceTag {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        debug_assert!(
            !value.trim().is_empty(),
            "AGS gateway service tag cannot be empty"
        );
        Self(value)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgsGatewayServiceTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Project-owned HTTP header carrying the AGS access token to game services.
///
/// AGS supplies the token, but each project owns the gateway header name. The
/// engine protocol crate therefore carries the name as configuration instead
/// of baking a project protocol into its HTTP client.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AgsAuthHeaderName(String);

impl AgsAuthHeaderName {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        debug_assert!(
            !value.trim().is_empty(),
            "AGS auth header name cannot be empty"
        );
        Self(value)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, JsonSchema,
)]
#[serde(transparent)]
pub struct SteamId(pub u64);

impl SteamId {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for SteamId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(
    Clone, Default, PartialEq, Eq, Hash, Serialize, ToSchema, JsonSchema, Zeroize, ZeroizeOnDrop,
)]
#[serde(transparent)]
pub struct SteamTicketHex(String);

impl fmt::Debug for SteamTicketHex {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SteamTicketHex")
            .field("value", &"[REDACTED]")
            .field("len", &self.0.len())
            .finish()
    }
}

impl SteamTicketHex {
    /// Wraps a hex-encoded Steam session ticket.
    ///
    /// # Errors
    ///
    /// Returns [`SteamTicketHexError`] if `value` contains any byte that is not
    /// an ASCII hex digit. The empty string is accepted.
    pub fn new(value: impl Into<String>) -> Result<Self, SteamTicketHexError> {
        let value = value.into();
        validate_hex_or_empty(&value)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn truncated_game_auth_hex(&self) -> &str {
        self.0
            .char_indices()
            .nth(GAME_AUTH_STEAM_TICKET_HEX_CHARS)
            .map_or(self.0.as_str(), |(end, _)| &self.0[..end])
    }

    #[must_use]
    pub fn prefixed_game_auth_ticket(&self) -> PrefixedSteamAuthTicket {
        PrefixedSteamAuthTicket(format!("steam|{}", self.truncated_game_auth_hex()))
    }
}

impl<'de> Deserialize<'de> for SteamTicketHex {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamTicketHexError;

impl fmt::Display for SteamTicketHexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Steam ticket is not hex encoded")
    }
}

impl std::error::Error for SteamTicketHexError {}

fn validate_hex_or_empty(value: &str) -> Result<(), SteamTicketHexError> {
    if value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(SteamTicketHexError)
    }
}

#[derive(
    Clone,
    Default,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    Zeroize,
    ZeroizeOnDrop,
)]
#[serde(transparent)]
pub struct PrefixedSteamAuthTicket(String);

impl PrefixedSteamAuthTicket {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(
    Clone,
    Default,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    Zeroize,
    ZeroizeOnDrop,
)]
#[serde(transparent)]
pub struct AgsAccessToken(String);

impl fmt::Debug for AgsAccessToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgsAccessToken")
            .field("value", &"[REDACTED]")
            .field("len", &self.0.len())
            .finish()
    }
}

impl AgsAccessToken {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(
    Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, JsonSchema,
)]
#[serde(transparent)]
pub struct ClientCapability(String);

impl ClientCapability {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(
    Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, JsonSchema,
)]
#[serde(transparent)]
pub struct ClientCapabilities(Vec<ClientCapability>);

impl ClientCapabilities {
    #[must_use]
    pub const fn new(value: Vec<ClientCapability>) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn empty() -> Self {
        Self(Vec::new())
    }

    #[must_use]
    pub fn as_slice(&self) -> &[ClientCapability] {
        &self.0
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    Zeroize,
)]
#[serde(transparent)]
pub struct TokenClientCapabilities(String);

impl TokenClientCapabilities {
    #[must_use]
    pub const fn empty() -> Self {
        Self(String::new())
    }

    #[must_use]
    pub fn as_registration_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(
    Clone,
    Default,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    Zeroize,
    ZeroizeOnDrop,
)]
#[serde(transparent)]
pub struct JwtClaims(String);

impl JwtClaims {
    /// Wraps a raw JWT claims document, rejecting one that is not valid claims JSON.
    ///
    /// # Errors
    ///
    /// Returns the [`serde_json::Error`] from parsing `value` as
    /// [`AgsJwtClaims`] when it is neither empty nor well-formed claims JSON.
    pub fn new(value: impl Into<String>) -> Result<Self, serde_json::Error> {
        let value = value.into();
        if !value.is_empty() {
            let _: AgsJwtClaims = serde_json::from_str(&value)?;
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn empty() -> Self {
        Self(String::new())
    }

    #[must_use]
    pub fn raw(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Parses the stored claims, yielding `None` when this value is empty.
    ///
    /// # Errors
    ///
    /// Returns the [`serde_json::Error`] from parsing the stored document when
    /// it is not well-formed [`AgsJwtClaims`] JSON. This can only happen for a
    /// value deserialized straight into the field, since [`Self::new`]
    /// validates up front.
    pub fn decode(&self) -> Result<Option<AgsJwtClaims>, serde_json::Error> {
        if self.0.is_empty() {
            Ok(None)
        } else {
            serde_json::from_str(&self.0).map(Some)
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct AgsJwtClaims {
    #[serde(default)]
    pub aud: JwtAudience,
    #[serde(default)]
    pub az_ags_age_group: String,
    #[serde(default)]
    pub az_ags_identity: String,
    #[serde(default)]
    pub az_ags_persona_id: String,
    #[serde(default)]
    pub az_ags_type: String,
    #[serde(default)]
    pub az_game: String,
    #[serde(default)]
    pub az_identities: JwtIdentities,
    #[serde(default)]
    pub az_persona_id: String,
    #[serde(default)]
    pub az_platform: String,
    #[serde(default)]
    pub az_platform_age_group: String,
    #[serde(default)]
    pub az_platform_id: String,
    #[serde(default)]
    pub az_platform_name: String,
    #[serde(default)]
    pub az_region: String,
    #[serde(default)]
    pub az_version: String,
    #[serde(default)]
    pub exp: String,
    #[serde(default)]
    pub iat: String,
    #[serde(default)]
    pub iss: String,
    #[serde(default)]
    pub nbf: String,
    #[serde(default)]
    pub sid: String,
    #[serde(default)]
    pub sub: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(untagged)]
pub enum JwtAudience {
    #[default]
    Empty,
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(untagged)]
pub enum JwtIdentities {
    #[default]
    Empty,
    LegacyText(String),
    Structured(Vec<JwtIdentity>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JwtIdentity {
    #[serde(default)]
    pub age_group: String,
    #[serde(default)]
    pub identity_id: String,
    #[serde(default)]
    pub identity_type: String,
    #[serde(default)]
    pub is_new_account: Option<bool>,
    #[serde(default)]
    pub persona_id: String,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default, rename = "type")]
    pub identity_kind: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steam_ticket_rejects_non_hex_text() {
        assert!(SteamTicketHex::new("deadbeef").is_ok());
        assert!(SteamTicketHex::new("steam|deadbeef").is_err());
    }

    #[test]
    fn steam_ticket_truncates_to_game_auth_limit() {
        let ticket = SteamTicketHex::new("a".repeat(GAME_AUTH_STEAM_TICKET_HEX_CHARS + 8))
            .expect("hex ticket");

        assert_eq!(
            ticket.prefixed_game_auth_ticket().as_str(),
            format!("steam|{}", "a".repeat(GAME_AUTH_STEAM_TICKET_HEX_CHARS))
        );
    }

    #[test]
    fn jwt_claims_parse_legacy_audience_and_identity_text() {
        let claims = JwtClaims::new(
            r#"{"aud":"[amzn1.organizationId.prod.example-game EOS.prod.example-game]","az_identities":"[map[identityType:steam]]","sub":"persona"}"#,
        )
        .expect("typed claims");

        let decoded = claims.decode().expect("decoded claims").expect("claims");
        assert_eq!(decoded.sub, "persona");
        assert!(matches!(decoded.aud, JwtAudience::Single(_)));
        assert!(matches!(
            decoded.az_identities,
            JwtIdentities::LegacyText(_)
        ));
    }

    #[test]
    fn secret_wrapper_debug_and_errors_never_render_values() {
        let ticket_value = "deadbeefcafebabe";
        let ticket = SteamTicketHex::new(ticket_value).unwrap();
        let access_token = AgsAccessToken::new("header.payload.signature");

        for rendered in [format!("{ticket:?}"), format!("{access_token:?}")] {
            assert!(rendered.contains("[REDACTED]"));
            assert!(!rendered.contains(ticket_value));
            assert!(!rendered.contains("header.payload.signature"));
        }

        let error = SteamTicketHex::new("invalid-ticket-value").unwrap_err();
        assert!(!error.to_string().contains("invalid-ticket-value"));
    }
}
