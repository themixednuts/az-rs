//! Shared Cap'n Proto protocol types for Azoth service boundaries.
//!
//! This crate is intentionally small: every other protocol crate can depend on
//! it for service identity, protocol negotiation, capabilities, endpoints, and
//! side-channel handles without importing project-specific schemas.

/// Declares a module wrapping one capnpc-generated schema file from `OUT_DIR`.
///
/// This exists because `capnp::generated_code!` allows only `clippy::all` on
/// the module it creates, and `clippy::all` does not contain the `pedantic` or
/// `nursery` groups that this workspace denies. The wrapped file is written by
/// `build.rs` into `target/.../out` and rewritten on every build, so it has no
/// checked-in line to annotate and no site a fix could land on — the module
/// declaration is the only place a suppression can live. Widening upstream's
/// existing allow to the two groups we deny keeps that suppression scoped to
/// generated code; hand-written code in these crates is still fully linted.
///
/// `env!("OUT_DIR")` expands at the call site, so each crate resolves its own
/// build directory.
#[macro_export]
macro_rules! generated_schema {
    ($vis:vis mod $name:ident, $file:expr) => {
        $vis mod $name {
            #![allow(clippy::all, clippy::pedantic, clippy::nursery)]
            include!(concat!(env!("OUT_DIR"), "/", $file));
        }
    };
}

// Machine-generated Cap'n Proto output: written into OUT_DIR by build.rs and
// regenerated on every build, so it is not in git and has no per-site fix. The
// macro completes upstream's own `#![allow(clippy::all)]` for the pedantic and
// nursery groups this workspace denies.
generated_schema!(pub mod core_capnp, "azoth/core_capnp.rs");

use std::collections::BTreeSet;
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use capnp::{Error, message, serialize_packed};
use thiserror::Error;
use uuid::Uuid;

pub use core_capnp as azoth_core_capnp;

pub const PROTOCOL_MAJOR: u16 = 0;
// 0.3 adds package/service selectors to daemon planning and preparation RPCs.
// Cap'n Proto readers ignore unknown fields, so this must remain a lockstep
// boundary: a 0.2 daemon would otherwise accept a focused request and execute
// the legacy full-graph plan.
pub const PROTOCOL_MINOR: u16 = 3;
pub const PROTOCOL_PATCH: u16 = 0;
pub const SIDE_CHANNEL_BLAKE3_HASH_BYTES: usize = blake3::OUT_LEN;
pub const EDITOR_SERVICE_NAMESPACE: &str = "azoth";
pub const EDITOR_SERVICE_NAME: &str = "editor";
/// Audience and permission for the generic entrypoint-owned lifecycle RPC.
pub const SERVICE_LIFECYCLE_AUDIENCE: &str = "service-lifecycle";
pub const SERVICE_LIFECYCLE_SHUTDOWN_PERMISSION: &str = "service.lifecycle.shutdown";

static STAGING_FILE_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl ProtocolVersion {
    pub const CURRENT: Self = Self {
        major: PROTOCOL_MAJOR,
        minor: PROTOCOL_MINOR,
        patch: PROTOCOL_PATCH,
    };

    #[must_use]
    pub const fn is_compatible_with(self, min_accepted: Self) -> bool {
        self.major == min_accepted.major
            && (self.minor > min_accepted.minor
                || (self.minor == min_accepted.minor && self.patch >= min_accepted.patch))
    }

    pub fn to_capnp(self, mut builder: azoth_core_capnp::protocol_version::Builder<'_>) {
        builder.set_major(self.major);
        builder.set_minor(self.minor);
        builder.set_patch(self.patch);
    }

    #[must_use]
    pub fn from_capnp(reader: azoth_core_capnp::protocol_version::Reader<'_>) -> Self {
        Self {
            major: reader.get_major(),
            minor: reader.get_minor(),
            patch: reader.get_patch(),
        }
    }

    /// Verifies that this value matches the expected lockstep protocol exactly.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolVersionMismatch`] when the process or persisted record
    /// was produced by a different protocol build and must be restarted.
    pub fn require(self, expected: Self) -> Result<(), ProtocolVersionMismatch> {
        if self == expected {
            return Ok(());
        }
        Err(ProtocolVersionMismatch {
            expected_major: expected.major,
            expected_minor: expected.minor,
            expected_patch: expected.patch,
            actual_major: self.major,
            actual_minor: self.minor,
            actual_patch: self.patch,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServiceId {
    pub namespace: String,
    pub name: String,
}

impl ServiceId {
    #[must_use]
    pub fn new(namespace: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
        }
    }

    pub fn to_capnp(&self, mut builder: azoth_core_capnp::service_id::Builder<'_>) {
        builder.set_namespace(&self.namespace);
        builder.set_name(&self.name);
    }

    /// # Errors
    ///
    /// Returns an error if the `namespace` or `name` field is absent from the message or is not valid UTF-8.
    pub fn from_capnp(reader: azoth_core_capnp::service_id::Reader<'_>) -> Result<Self, Error> {
        Ok(Self {
            namespace: reader.get_namespace()?.to_string()?,
            name: reader.get_name()?.to_string()?,
        })
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.namespace.is_empty() && self.name.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(Uuid);

impl SessionId {
    #[must_use]
    pub const fn new(uuid: Uuid) -> Self {
        Self(uuid)
    }

    #[must_use]
    pub const fn into_uuid(self) -> Uuid {
        self.0
    }

    pub fn to_capnp(self, builder: azoth_core_capnp::session_id::Builder<'_>) {
        write_uuid(self.0, builder);
    }

    /// # Errors
    ///
    /// Returns an error if the UUID field is absent from the message or is not the expected 16 bytes.
    pub fn from_capnp(reader: azoth_core_capnp::session_id::Reader<'_>) -> Result<Self, Error> {
        read_uuid(reader).map(Self)
    }
}

impl From<Uuid> for SessionId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl From<SessionId> for Uuid {
    fn from(session_id: SessionId) -> Self {
        session_id.0
    }
}

fn write_uuid(uuid: Uuid, mut builder: azoth_core_capnp::session_id::Builder<'_>) {
    builder.set_uuid(uuid.as_bytes());
}

fn read_uuid(reader: azoth_core_capnp::session_id::Reader<'_>) -> Result<Uuid, Error> {
    let bytes = reader.get_uuid()?;
    Uuid::from_slice(bytes).map_err(|error| Error::failed(format!("invalid UUID bytes: {error}")))
}

fn write_optional_uuid(
    value: Option<Uuid>,
    mut builder: azoth_core_capnp::optional_uuid::Builder<'_>,
) {
    match value {
        Some(value) => builder.set_value(value.as_bytes()),
        None => builder.set_none(()),
    }
}

fn read_optional_uuid(
    reader: azoth_core_capnp::optional_uuid::Reader<'_>,
) -> Result<Option<Uuid>, Error> {
    match reader.which()? {
        azoth_core_capnp::optional_uuid::Which::None(()) => Ok(None),
        azoth_core_capnp::optional_uuid::Which::Value(value) => Uuid::from_slice(value?)
            .map(Some)
            .map_err(|error| Error::failed(format!("invalid optional UUID bytes: {error}"))),
    }
}

fn read_required_uuid(bytes: &[u8], context: &str) -> Result<Uuid, Error> {
    let value = Uuid::from_slice(bytes)
        .map_err(|error| Error::failed(format!("invalid {context} UUID bytes: {error}")))?;
    if value == Uuid::nil() {
        return Err(Error::failed(format!("{context} must not be nil")));
    }
    Ok(value)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ServiceRole {
    Unknown,
    Editor,
    Daemon,
    SessionSupervisor,
    ProjectHost,
    AssetProcessor,
    RuntimeHost,
    Worker,
}

impl ServiceRole {
    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Editor => "Editor",
            Self::Daemon => "Daemon",
            Self::SessionSupervisor => "SessionSupervisor",
            Self::ProjectHost => "ProjectHost",
            Self::AssetProcessor => "AssetProcessor",
            Self::RuntimeHost => "RuntimeHost",
            Self::Worker => "Worker",
        }
    }

    #[must_use]
    pub const fn to_capnp(self) -> azoth_core_capnp::ServiceRole {
        match self {
            Self::Unknown => azoth_core_capnp::ServiceRole::Unknown,
            Self::Editor => azoth_core_capnp::ServiceRole::Editor,
            Self::Daemon => azoth_core_capnp::ServiceRole::Daemon,
            Self::SessionSupervisor => azoth_core_capnp::ServiceRole::SessionSupervisor,
            Self::ProjectHost => azoth_core_capnp::ServiceRole::ProjectHost,
            Self::AssetProcessor => azoth_core_capnp::ServiceRole::AssetProcessor,
            Self::RuntimeHost => azoth_core_capnp::ServiceRole::RuntimeHost,
            Self::Worker => azoth_core_capnp::ServiceRole::Worker,
        }
    }

    #[must_use]
    pub const fn from_capnp(role: azoth_core_capnp::ServiceRole) -> Self {
        match role {
            azoth_core_capnp::ServiceRole::Unknown => Self::Unknown,
            azoth_core_capnp::ServiceRole::Editor => Self::Editor,
            azoth_core_capnp::ServiceRole::Daemon => Self::Daemon,
            azoth_core_capnp::ServiceRole::SessionSupervisor => Self::SessionSupervisor,
            azoth_core_capnp::ServiceRole::ProjectHost => Self::ProjectHost,
            azoth_core_capnp::ServiceRole::AssetProcessor => Self::AssetProcessor,
            azoth_core_capnp::ServiceRole::RuntimeHost => Self::RuntimeHost,
            azoth_core_capnp::ServiceRole::Worker => Self::Worker,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EndpointKind {
    WindowsNamedPipe,
    UnixDomainSocket,
    Tcp,
    InProcess,
}

impl EndpointKind {
    #[must_use]
    pub const fn to_capnp(self) -> azoth_core_capnp::EndpointKind {
        match self {
            Self::WindowsNamedPipe => azoth_core_capnp::EndpointKind::WindowsNamedPipe,
            Self::UnixDomainSocket => azoth_core_capnp::EndpointKind::UnixDomainSocket,
            Self::Tcp => azoth_core_capnp::EndpointKind::Tcp,
            Self::InProcess => azoth_core_capnp::EndpointKind::InProcess,
        }
    }

    #[must_use]
    pub const fn from_capnp(kind: azoth_core_capnp::EndpointKind) -> Self {
        match kind {
            azoth_core_capnp::EndpointKind::WindowsNamedPipe => Self::WindowsNamedPipe,
            azoth_core_capnp::EndpointKind::UnixDomainSocket => Self::UnixDomainSocket,
            azoth_core_capnp::EndpointKind::Tcp => Self::Tcp,
            azoth_core_capnp::EndpointKind::InProcess => Self::InProcess,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub kind: EndpointKind,
    pub address: String,
}

impl Endpoint {
    #[must_use]
    pub fn new(kind: EndpointKind, address: impl Into<String>) -> Self {
        Self {
            kind,
            address: address.into(),
        }
    }

    #[must_use]
    pub fn in_process(address: impl Into<String>) -> Self {
        Self::new(EndpointKind::InProcess, address)
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.address.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capability {
    pub session: Option<Uuid>,
    pub service: ServiceId,
    pub role: ServiceRole,
    pub permissions: Vec<String>,
    pub audience: String,
    pub expires_unix_ms: u64,
    pub token_hash: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CapabilityLifetimeError {
    #[error("capability expired at {expires_unix_ms} (now {now_unix_ms})")]
    Expired {
        expires_unix_ms: u64,
        now_unix_ms: u64,
    },

    #[error("system clock is before the Unix epoch")]
    ClockBeforeUnixEpoch,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CapabilityTemplateError {
    #[error("capability template must carry a brokered token hash")]
    MissingBrokeredToken,

    #[error("capability template must not use the nil session id")]
    NilSession,

    #[error("capability template must carry a non-empty audience")]
    MissingAudience,

    #[error("capability template must carry at least one permission")]
    MissingPermissions,

    #[error("capability template lifetime is invalid: {reason}")]
    InvalidLifetime { reason: CapabilityLifetimeError },
}

impl Capability {
    #[must_use]
    pub const fn new(service: ServiceId, role: ServiceRole) -> Self {
        Self {
            session: None,
            service,
            role,
            permissions: Vec::new(),
            audience: String::new(),
            expires_unix_ms: 0,
            token_hash: Vec::new(),
        }
    }

    #[must_use]
    pub const fn with_session(mut self, session: Uuid) -> Self {
        self.session = Some(session);
        self
    }

    #[must_use]
    pub fn with_audience(mut self, audience: impl Into<String>) -> Self {
        self.audience = audience.into();
        self
    }

    #[must_use]
    pub fn with_permissions(
        mut self,
        permissions: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.permissions = permissions.into_iter().map(Into::into).collect();
        self
    }

    #[must_use]
    pub const fn with_expires_unix_ms(mut self, expires_unix_ms: u64) -> Self {
        self.expires_unix_ms = expires_unix_ms;
        self
    }

    #[must_use]
    pub fn with_token_hash(mut self, token_hash: impl Into<Vec<u8>>) -> Self {
        self.token_hash = token_hash.into();
        self
    }

    #[must_use]
    pub const fn is_non_expiring(&self) -> bool {
        self.expires_unix_ms == 0
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.session.is_none()
            && self.service.namespace.is_empty()
            && self.service.name.is_empty()
            && self.role == ServiceRole::Unknown
            && self.permissions.is_empty()
            && self.audience.is_empty()
            && self.expires_unix_ms == 0
            && self.token_hash.is_empty()
    }

    #[must_use]
    pub const fn is_expired_at_unix_ms(&self, now_unix_ms: u64) -> bool {
        !self.is_non_expiring() && now_unix_ms >= self.expires_unix_ms
    }

    /// # Errors
    ///
    /// Returns [`CapabilityLifetimeError::Expired`] if the capability carries an expiry and `now_unix_ms` is at or past it.
    pub const fn validate_lifetime_at_unix_ms(
        &self,
        now_unix_ms: u64,
    ) -> Result<(), CapabilityLifetimeError> {
        if self.is_expired_at_unix_ms(now_unix_ms) {
            Err(CapabilityLifetimeError::Expired {
                expires_unix_ms: self.expires_unix_ms,
                now_unix_ms,
            })
        } else {
            Ok(())
        }
    }

    /// # Errors
    ///
    /// Returns [`CapabilityLifetimeError::ClockBeforeUnixEpoch`] if the system clock reports a time before the Unix epoch, or
    /// [`CapabilityLifetimeError::Expired`] if the capability has already expired.
    pub fn validate_lifetime(&self) -> Result<(), CapabilityLifetimeError> {
        self.validate_lifetime_at_unix_ms(current_unix_ms()?)
    }

    #[must_use]
    pub fn has_permissions(&self, required_permissions: &[&str]) -> bool {
        required_permissions.iter().all(|required| {
            self.permissions
                .iter()
                .any(|permission| permission == required)
        })
    }

    #[must_use]
    fn matches_template_request(
        &self,
        role: ServiceRole,
        audience: &str,
        required_permissions: &[&str],
        session: Option<Uuid>,
    ) -> bool {
        if self.role != role
            || self.audience != audience
            || !self.has_permissions(required_permissions)
        {
            return false;
        }

        match (session, self.session) {
            (Some(expected), Some(actual)) => expected == actual,
            _ => true,
        }
    }

    #[must_use]
    pub fn is_brokered_template(&self) -> bool {
        self.validate_brokered_template().is_ok()
    }

    /// # Errors
    ///
    /// Returns [`CapabilityTemplateError`] if the template carries no brokered token hash, names the nil session, has a blank
    /// audience, grants no permissions, or has an invalid lifetime.
    pub fn validate_brokered_template(&self) -> Result<(), CapabilityTemplateError> {
        if self.token_hash.is_empty() {
            return Err(CapabilityTemplateError::MissingBrokeredToken);
        }

        if matches!(self.session, Some(session) if session == Uuid::nil()) {
            return Err(CapabilityTemplateError::NilSession);
        }

        if self.audience.trim().is_empty() {
            return Err(CapabilityTemplateError::MissingAudience);
        }

        if self.permissions.is_empty() {
            return Err(CapabilityTemplateError::MissingPermissions);
        }

        self.validate_lifetime()
            .map_err(|reason| CapabilityTemplateError::InvalidLifetime { reason })?;

        Ok(())
    }

    #[must_use]
    pub fn matches_brokered_template_request(
        &self,
        role: ServiceRole,
        audience: &str,
        required_permissions: &[&str],
        session: Option<Uuid>,
    ) -> bool {
        self.is_brokered_template()
            && self.matches_template_request(role, audience, required_permissions, session)
    }

    #[must_use]
    pub const fn scoped_to(mut self, session: Option<Uuid>) -> Self {
        if self.session.is_none() {
            self.session = session;
        }
        self
    }
}

/// # Errors
///
/// Returns [`CapabilityLifetimeError::ClockBeforeUnixEpoch`] if the system clock reports a time before the Unix epoch.
pub fn current_unix_ms() -> Result<u64, CapabilityLifetimeError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CapabilityLifetimeError::ClockBeforeUnixEpoch)?;
    Ok(u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityGrantSet {
    grants: Vec<Capability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CapabilityGrantError {
    #[error("capability grant set is empty")]
    Empty,

    #[error("capability has no brokered token hash for `{required_permission}`")]
    MissingBrokeredToken { required_permission: String },

    #[error("capability lifetime is invalid for `{required_permission}`: {reason}")]
    InvalidLifetime {
        required_permission: String,
        reason: String,
    },

    #[error("capability session must not be nil for `{required_permission}`")]
    NilSession { required_permission: String },

    #[error("capability is not brokered for `{required_permission}`")]
    NotGranted { required_permission: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityGrantRequirement<'a> {
    pub service_namespace: &'a str,
    pub service_name: &'a str,
    pub role: ServiceRole,
    pub audience: &'a str,
    pub permissions: &'a [&'a str],
}

impl<'a> CapabilityGrantRequirement<'a> {
    #[must_use]
    pub const fn new(
        service_namespace: &'a str,
        service_name: &'a str,
        role: ServiceRole,
        audience: &'a str,
        permissions: &'a [&'a str],
    ) -> Self {
        Self {
            service_namespace,
            service_name,
            role,
            audience,
            permissions,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CapabilityGrantSetValidationError {
    #[error("capability grant set is empty")]
    Empty,

    #[error("capability grant `{service}` role {role:?} must be scoped to a session")]
    MissingSession { service: String, role: ServiceRole },

    #[error(
        "capability grant `{service}` role {role:?} is scoped to session `{actual}`, expected `{expected}`"
    )]
    WrongSession {
        service: String,
        role: ServiceRole,
        expected: Uuid,
        actual: Uuid,
    },

    #[error(
        "capability grant `{service}` role {role:?} is session-scoped to `{actual}`, expected project scope"
    )]
    UnexpectedSession {
        service: String,
        role: ServiceRole,
        actual: Uuid,
    },

    #[error("capability grant set must not use the nil session id")]
    NilSession,

    #[error("capability grant `{service}` role {role:?} must carry a brokered token hash")]
    MissingBrokeredToken { service: String, role: ServiceRole },

    #[error("capability grant `{service}` role {role:?} must carry audience and permissions")]
    MissingAudienceOrPermissions { service: String, role: ServiceRole },

    #[error("capability grant `{service}` role {role:?} has invalid lifetime: {reason}")]
    InvalidLifetime {
        service: String,
        role: ServiceRole,
        reason: String,
    },

    #[error(
        "missing required capability grant `{}/{}` role {:?} audience `{}` permissions `{:?}` for {}",
        .0.service_namespace, .0.service_name, .0.role, .0.audience, .0.permissions, .0.scope
    )]
    MissingRequiredGrant(Box<RequiredCapabilityGrant>),

    #[error(
        "unexpected capability grant `{}` role {:?} audience `{}` permissions `{:?}` for {}",
        .0.service, .0.role, .0.audience, .0.permissions, .0.scope
    )]
    UnexpectedGrant(Box<ObservedCapabilityGrant>),

    #[error(
        "capability grant `{}/{}` role {:?} audience `{}` permissions `{:?}` for {} must appear exactly once, found {}",
        .0.service_namespace, .0.service_name, .0.role, .0.audience, .0.permissions, .0.scope, .1
    )]
    DuplicateRequiredGrant(Box<RequiredCapabilityGrant>, usize),
}

/// The identity of a capability grant the validator required.
///
/// Boxed at the variants above. Inline, these six fields pushed
/// `CapabilityGrantSetValidationError` past 128 bytes, so every
/// `Result<_, CapabilityGrantSetValidationError>` in the workspace paid for the
/// rarest failure it can carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiredCapabilityGrant {
    pub service_namespace: String,
    pub service_name: String,
    pub role: ServiceRole,
    pub audience: String,
    pub permissions: Vec<String>,
    pub scope: CapabilityGrantScope,
}

/// The identity of a capability grant the set actually carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedCapabilityGrant {
    pub service: String,
    pub role: ServiceRole,
    pub audience: String,
    pub permissions: Vec<String>,
    pub scope: CapabilityGrantScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityGrantScope {
    Project,
    Session(Uuid),
}

impl CapabilityGrantScope {
    #[must_use]
    pub const fn session(self) -> Option<Uuid> {
        match self {
            Self::Project => None,
            Self::Session(session) => Some(session),
        }
    }
}

impl std::fmt::Display for CapabilityGrantScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Project => f.write_str("project scope"),
            Self::Session(session) => write!(f, "session `{session}`"),
        }
    }
}

impl CapabilityGrantSet {
    #[must_use]
    pub const fn new() -> Self {
        Self { grants: Vec::new() }
    }

    #[must_use]
    pub fn from_grants(grants: impl Into<Vec<Capability>>) -> Self {
        Self {
            grants: grants.into(),
        }
    }

    #[must_use]
    pub fn grants(&self) -> &[Capability] {
        &self.grants
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.grants.is_empty()
    }

    /// # Errors
    ///
    /// Returns [`CapabilityGrantError`] if the set holds no grants, if `capability` carries no brokered token hash, has an
    /// invalid lifetime, or names the nil session, or if no grant in the set covers `required_permission`.
    pub fn validate(
        &self,
        capability: &Capability,
        required_permission: &str,
    ) -> Result<(), CapabilityGrantError> {
        if self.grants.is_empty() {
            return Err(CapabilityGrantError::Empty);
        }

        if capability.token_hash.is_empty() {
            return Err(CapabilityGrantError::MissingBrokeredToken {
                required_permission: required_permission.to_string(),
            });
        }

        if let Err(error) = capability.validate_lifetime() {
            return Err(CapabilityGrantError::InvalidLifetime {
                required_permission: required_permission.to_string(),
                reason: error.to_string(),
            });
        }

        if capability.session == Some(Uuid::nil()) {
            return Err(CapabilityGrantError::NilSession {
                required_permission: required_permission.to_string(),
            });
        }

        if self
            .grants
            .iter()
            .any(|grant| grant_allows_capability(grant, capability, required_permission))
        {
            Ok(())
        } else {
            Err(CapabilityGrantError::NotGranted {
                required_permission: required_permission.to_string(),
            })
        }
    }

    /// # Errors
    ///
    /// Returns [`CapabilityGrantSetValidationError`] if the set holds no grants, if its first grant carries no session, or if
    /// the set does not validate as brokered for that session.
    pub fn single_brokered_session(&self) -> Result<Uuid, CapabilityGrantSetValidationError> {
        let Some(first) = self.grants.first() else {
            return Err(CapabilityGrantSetValidationError::Empty);
        };
        let Some(session) = first.session else {
            return Err(CapabilityGrantSetValidationError::MissingSession {
                service: service_label(&first.service),
                role: first.role,
            });
        };
        self.validate_brokered_for_session(session, &[])?;
        Ok(session)
    }

    /// # Errors
    ///
    /// Returns [`CapabilityGrantSetValidationError`] if the set holds no grants, if `session` is nil, if any grant is scoped to
    /// a different session or to none, carries no brokered token hash, has a blank audience, grants no permissions, or has an
    /// invalid lifetime, or if any entry in `requirements` has no matching grant.
    pub fn validate_brokered_for_session(
        &self,
        session: Uuid,
        requirements: &[CapabilityGrantRequirement<'_>],
    ) -> Result<(), CapabilityGrantSetValidationError> {
        self.validate_brokered(CapabilityGrantScope::Session(session), requirements)
    }

    /// # Errors
    ///
    /// Returns [`CapabilityGrantSetValidationError`] if the set holds no grants, if any grant is scoped to a session, carries
    /// no brokered token hash, has a blank audience, grants no permissions, or has an invalid lifetime, or if any entry in
    /// `requirements` has no matching grant.
    pub fn validate_brokered_for_project(
        &self,
        requirements: &[CapabilityGrantRequirement<'_>],
    ) -> Result<(), CapabilityGrantSetValidationError> {
        self.validate_brokered(CapabilityGrantScope::Project, requirements)
    }

    fn validate_brokered(
        &self,
        scope: CapabilityGrantScope,
        requirements: &[CapabilityGrantRequirement<'_>],
    ) -> Result<(), CapabilityGrantSetValidationError> {
        if self.grants.is_empty() {
            return Err(CapabilityGrantSetValidationError::Empty);
        }
        if matches!(scope, CapabilityGrantScope::Session(session) if session == Uuid::nil()) {
            return Err(CapabilityGrantSetValidationError::NilSession);
        }

        for grant in &self.grants {
            let service = service_label(&grant.service);
            match (scope, grant.session) {
                (_, Some(actual)) if actual == Uuid::nil() => {
                    return Err(CapabilityGrantSetValidationError::NilSession);
                }
                (CapabilityGrantScope::Session(expected), Some(actual)) if actual == expected => {}
                (CapabilityGrantScope::Session(expected), Some(actual)) => {
                    return Err(CapabilityGrantSetValidationError::WrongSession {
                        service,
                        role: grant.role,
                        expected,
                        actual,
                    });
                }
                (CapabilityGrantScope::Session(_), None) => {
                    return Err(CapabilityGrantSetValidationError::MissingSession {
                        service,
                        role: grant.role,
                    });
                }
                (CapabilityGrantScope::Project, Some(actual)) => {
                    return Err(CapabilityGrantSetValidationError::UnexpectedSession {
                        service,
                        role: grant.role,
                        actual,
                    });
                }
                (CapabilityGrantScope::Project, None) => {}
            }
            if grant.token_hash.is_empty() {
                return Err(CapabilityGrantSetValidationError::MissingBrokeredToken {
                    service,
                    role: grant.role,
                });
            }
            if grant.audience.trim().is_empty() || grant.permissions.is_empty() {
                return Err(
                    CapabilityGrantSetValidationError::MissingAudienceOrPermissions {
                        service,
                        role: grant.role,
                    },
                );
            }
            if let Err(error) = grant.validate_lifetime() {
                return Err(CapabilityGrantSetValidationError::InvalidLifetime {
                    service,
                    role: grant.role,
                    reason: error.to_string(),
                });
            }
        }

        for requirement in requirements {
            let has_required_grant = self.grants.iter().any(|grant| {
                grant.session == scope.session()
                    && grant.service.namespace == requirement.service_namespace
                    && grant.service.name == requirement.service_name
                    && grant.role == requirement.role
                    && grant.audience == requirement.audience
                    && !grant.token_hash.is_empty()
                    && grant.has_permissions(requirement.permissions)
            });
            if !has_required_grant {
                return Err(CapabilityGrantSetValidationError::MissingRequiredGrant(
                    Box::new(RequiredCapabilityGrant {
                        service_namespace: requirement.service_namespace.to_string(),
                        service_name: requirement.service_name.to_string(),
                        role: requirement.role,
                        audience: requirement.audience.to_string(),
                        permissions: requirement
                            .permissions
                            .iter()
                            .map(|permission| (*permission).to_string())
                            .collect(),
                        scope,
                    }),
                ));
            }
        }

        Ok(())
    }

    /// # Errors
    ///
    /// Returns [`CapabilityGrantSetValidationError`] for every reason [`Self::validate_brokered_for_session`] does, and
    /// additionally if the set holds any grant that `requirements` does not name.
    pub fn validate_exact_brokered_for_session(
        &self,
        session: Uuid,
        requirements: &[CapabilityGrantRequirement<'_>],
    ) -> Result<(), CapabilityGrantSetValidationError> {
        self.validate_exact_brokered(CapabilityGrantScope::Session(session), requirements)
    }

    /// # Errors
    ///
    /// Returns [`CapabilityGrantSetValidationError`] for every reason [`Self::validate_brokered_for_project`] does, and
    /// additionally if the set holds any grant that `requirements` does not name.
    pub fn validate_exact_brokered_for_project(
        &self,
        requirements: &[CapabilityGrantRequirement<'_>],
    ) -> Result<(), CapabilityGrantSetValidationError> {
        self.validate_exact_brokered(CapabilityGrantScope::Project, requirements)
    }

    fn validate_exact_brokered(
        &self,
        scope: CapabilityGrantScope,
        requirements: &[CapabilityGrantRequirement<'_>],
    ) -> Result<(), CapabilityGrantSetValidationError> {
        self.validate_brokered(scope, requirements)?;

        for grant in &self.grants {
            if requirements
                .iter()
                .any(|requirement| capability_exactly_matches_requirement(grant, requirement))
            {
                continue;
            }

            return Err(CapabilityGrantSetValidationError::UnexpectedGrant(
                Box::new(ObservedCapabilityGrant {
                    service: service_label(&grant.service),
                    role: grant.role,
                    audience: grant.audience.clone(),
                    permissions: grant.permissions.clone(),
                    scope,
                }),
            ));
        }

        for requirement in requirements {
            let count = self
                .grants
                .iter()
                .filter(|grant| capability_exactly_matches_requirement(grant, requirement))
                .count();

            if count == 1 {
                continue;
            }

            return Err(CapabilityGrantSetValidationError::DuplicateRequiredGrant(
                Box::new(RequiredCapabilityGrant {
                    service_namespace: requirement.service_namespace.to_string(),
                    service_name: requirement.service_name.to_string(),
                    role: requirement.role,
                    audience: requirement.audience.to_string(),
                    permissions: requirement
                        .permissions
                        .iter()
                        .map(|permission| (*permission).to_string())
                        .collect(),
                    scope,
                }),
                count,
            ));
        }

        Ok(())
    }
}

impl Default for CapabilityGrantSet {
    fn default() -> Self {
        Self::new()
    }
}

fn grant_allows_capability(
    grant: &Capability,
    capability: &Capability,
    required_permission: &str,
) -> bool {
    !grant.token_hash.is_empty()
        && grant.token_hash == capability.token_hash
        && grant.session == capability.session
        && grant.service == capability.service
        && grant.role == capability.role
        && grant.audience == capability.audience
        && grant.expires_unix_ms == capability.expires_unix_ms
        && grant.has_permissions(&[required_permission])
        && capability
            .permissions
            .iter()
            .all(|permission| grant.permissions.iter().any(|grant| grant == permission))
}

fn capability_exactly_matches_requirement(
    capability: &Capability,
    requirement: &CapabilityGrantRequirement<'_>,
) -> bool {
    capability.service.namespace == requirement.service_namespace
        && capability.service.name == requirement.service_name
        && capability.role == requirement.role
        && capability.audience == requirement.audience
        && capability.permissions.len() == requirement.permissions.len()
        && requirement.permissions.iter().all(|required| {
            capability
                .permissions
                .iter()
                .any(|permission| permission == required)
        })
}

fn service_label(service_id: &ServiceId) -> String {
    format!("{}/{}", service_id.namespace, service_id.name)
}

#[derive(Debug, Clone)]
pub struct ServiceDescriptor {
    pub id: ServiceId,
    pub role: ServiceRole,
    /// `UUIDv7` launch label for observability and retained-log selection only.
    pub run: Uuid,
    pub protocol: ProtocolVersion,
    pub endpoint: Endpoint,
    pub observability_endpoint: Option<Endpoint>,
    pub lifecycle_endpoint: Option<Endpoint>,
    pub capabilities: Vec<Capability>,
}

// A launch run is an observation attached to a descriptor, not part of its
// control identity. Keeping that rule in equality prevents innocent
// whole-descriptor comparisons from turning the run label into routing or
// freshness state.
impl PartialEq for ServiceDescriptor {
    fn eq(&self, other: &Self) -> bool {
        self.has_same_connection_contract(other)
    }
}

impl Eq for ServiceDescriptor {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ServiceHealthState {
    Unknown,
    Starting,
    Ready,
    Busy,
    Degraded,
    Stopping,
    Failed,
}

impl ServiceHealthState {
    #[must_use]
    pub const fn to_capnp(self) -> azoth_core_capnp::ServiceHealthState {
        match self {
            Self::Unknown => azoth_core_capnp::ServiceHealthState::Unknown,
            Self::Starting => azoth_core_capnp::ServiceHealthState::Starting,
            Self::Ready => azoth_core_capnp::ServiceHealthState::Ready,
            Self::Busy => azoth_core_capnp::ServiceHealthState::Busy,
            Self::Degraded => azoth_core_capnp::ServiceHealthState::Degraded,
            Self::Stopping => azoth_core_capnp::ServiceHealthState::Stopping,
            Self::Failed => azoth_core_capnp::ServiceHealthState::Failed,
        }
    }

    #[must_use]
    pub const fn from_capnp(state: azoth_core_capnp::ServiceHealthState) -> Self {
        match state {
            azoth_core_capnp::ServiceHealthState::Unknown => Self::Unknown,
            azoth_core_capnp::ServiceHealthState::Starting => Self::Starting,
            azoth_core_capnp::ServiceHealthState::Ready => Self::Ready,
            azoth_core_capnp::ServiceHealthState::Busy => Self::Busy,
            azoth_core_capnp::ServiceHealthState::Degraded => Self::Degraded,
            azoth_core_capnp::ServiceHealthState::Stopping => Self::Stopping,
            azoth_core_capnp::ServiceHealthState::Failed => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceHealth {
    pub service: ServiceId,
    pub role: ServiceRole,
    /// `UUIDv7` launch label for observability only.
    pub run: Uuid,
    pub protocol_version: ProtocolVersion,
    pub state: ServiceHealthState,
    pub ready: bool,
    pub degraded: bool,
    pub pid: u32,
    pub checked_unix_ms: u64,
    pub uptime_ms: u64,
    pub active_operation: String,
    pub message: String,
    pub last_event_seq: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error(
    "service protocol version {actual_major}.{actual_minor}.{actual_patch} does not match expected {expected_major}.{expected_minor}.{expected_patch}"
)]
pub struct ProtocolVersionMismatch {
    expected_major: u16,
    expected_minor: u16,
    expected_patch: u16,
    actual_major: u16,
    actual_minor: u16,
    actual_patch: u16,
}

impl ServiceHealth {
    #[must_use]
    pub fn ready(
        service: ServiceId,
        role: ServiceRole,
        run: Uuid,
        protocol_version: ProtocolVersion,
    ) -> Self {
        Self {
            service,
            role,
            run,
            protocol_version,
            state: ServiceHealthState::Ready,
            ready: true,
            degraded: false,
            pid: std::process::id(),
            checked_unix_ms: current_unix_ms().unwrap_or(0),
            uptime_ms: 0,
            active_operation: String::new(),
            message: String::new(),
            last_event_seq: 0,
        }
    }

    /// Verifies that this health snapshot uses the expected lockstep protocol.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolVersionMismatch`] when the service was built against a
    /// different protocol version and must be restarted.
    pub fn require_protocol_version(
        &self,
        expected: ProtocolVersion,
    ) -> Result<(), ProtocolVersionMismatch> {
        self.protocol_version.require(expected)
    }

    #[must_use]
    pub const fn with_uptime_ms(mut self, uptime_ms: u64) -> Self {
        self.uptime_ms = uptime_ms;
        self
    }

    #[must_use]
    pub fn with_state(mut self, state: ServiceHealthState) -> Self {
        self.state = state;
        self.ready = matches!(
            state,
            ServiceHealthState::Ready | ServiceHealthState::Busy | ServiceHealthState::Degraded
        );
        self.degraded = state == ServiceHealthState::Degraded;
        self
    }

    #[must_use]
    pub fn with_active_operation(mut self, active_operation: impl Into<String>) -> Self {
        self.active_operation = active_operation.into();
        self
    }

    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    #[must_use]
    pub const fn with_last_event_seq(mut self, last_event_seq: u64) -> Self {
        self.last_event_seq = last_event_seq;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ServiceDescriptorCapabilityTemplateError {
    #[error("service `{id:?}` with role `{role:?}` has no brokered capability templates")]
    MissingCapabilities { id: ServiceId, role: ServiceRole },

    #[error(
        "service `{id:?}` with role `{role:?}` capability template {index} is not brokered: {reason}"
    )]
    InvalidCapability {
        id: ServiceId,
        role: ServiceRole,
        index: usize,
        reason: CapabilityTemplateError,
    },
}

impl ServiceDescriptor {
    #[must_use]
    pub const fn new(id: ServiceId, role: ServiceRole, endpoint: Endpoint) -> Self {
        Self {
            id,
            role,
            run: Uuid::nil(),
            protocol: ProtocolVersion::CURRENT,
            endpoint,
            observability_endpoint: None,
            lifecycle_endpoint: None,
            capabilities: Vec::new(),
        }
    }

    #[must_use]
    pub const fn with_run(mut self, run: Uuid) -> Self {
        self.run = run;
        self
    }

    #[must_use]
    pub fn with_lifecycle_endpoint(mut self, endpoint: Endpoint) -> Self {
        self.lifecycle_endpoint = Some(endpoint);
        self
    }

    /// Returns whether two descriptors advertise the same callable service
    /// contract. The launch `run` label is deliberately excluded: it is
    /// observability metadata and must never select or authorize a service.
    #[must_use]
    pub fn has_same_connection_contract(&self, other: &Self) -> bool {
        self.id == other.id
            && self.role == other.role
            && self.protocol == other.protocol
            && self.endpoint == other.endpoint
            && self.observability_endpoint == other.observability_endpoint
            && self.lifecycle_endpoint == other.lifecycle_endpoint
            && self.capabilities == other.capabilities
    }

    /// Verifies that this discovery descriptor matches the expected lockstep
    /// protocol before a client opens the advertised service endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolVersionMismatch`] when the descriptor was published by
    /// a service from a different build and that service must be restarted.
    pub fn require_protocol_version(
        &self,
        expected: ProtocolVersion,
    ) -> Result<(), ProtocolVersionMismatch> {
        self.protocol.require(expected)
    }

    #[must_use]
    pub fn with_observability_endpoint(mut self, endpoint: Endpoint) -> Self {
        self.observability_endpoint = Some(endpoint);
        self
    }

    #[must_use]
    pub fn with_capability(mut self, capability: Capability) -> Self {
        self.capabilities.push(capability);
        self
    }

    #[must_use]
    pub fn brokered_capability_template(
        &self,
        role: ServiceRole,
        audience: &str,
        required_permissions: &[&str],
        session: Option<Uuid>,
    ) -> Option<Capability> {
        self.capabilities
            .iter()
            .find(|capability| {
                capability.matches_brokered_template_request(
                    role,
                    audience,
                    required_permissions,
                    session,
                )
            })
            .cloned()
            .map(|capability| capability.scoped_to(session))
    }

    /// # Errors
    ///
    /// Returns [`ServiceDescriptorCapabilityTemplateError`] if the descriptor declares no capabilities, or if any declared
    /// capability is not a valid brokered template.
    pub fn validate_brokered_capability_templates(
        &self,
    ) -> Result<(), ServiceDescriptorCapabilityTemplateError> {
        if self.capabilities.is_empty() {
            return Err(
                ServiceDescriptorCapabilityTemplateError::MissingCapabilities {
                    id: self.id.clone(),
                    role: self.role,
                },
            );
        }

        for (index, capability) in self.capabilities.iter().enumerate() {
            capability.validate_brokered_template().map_err(|reason| {
                ServiceDescriptorCapabilityTemplateError::InvalidCapability {
                    id: self.id.clone(),
                    role: self.role,
                    index,
                    reason,
                }
            })?;
        }

        Ok(())
    }
}

impl Endpoint {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the endpoint.
    pub fn to_capnp(&self, builder: azoth_core_capnp::endpoint::Builder<'_>) -> Result<(), Error> {
        write_endpoint(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the endpoint is absent from the message or is not valid UTF-8.
    pub fn from_capnp(reader: azoth_core_capnp::endpoint::Reader<'_>) -> Result<Self, Error> {
        read_endpoint(reader)
    }
}

fn write_endpoint(
    endpoint: &Endpoint,
    mut builder: azoth_core_capnp::endpoint::Builder<'_>,
) -> Result<(), Error> {
    validate_endpoint_for_transport(endpoint)?;
    builder.set_kind(endpoint.kind.to_capnp());
    builder.set_address(&endpoint.address);
    Ok(())
}

fn read_endpoint(reader: azoth_core_capnp::endpoint::Reader<'_>) -> Result<Endpoint, Error> {
    let kind = reader
        .get_kind()
        .map(EndpointKind::from_capnp)
        .map_err(|error| Error::failed(format!("unknown endpoint kind: {error:?}")))?;
    let endpoint = Endpoint::new(kind, reader.get_address()?.to_string()?);
    validate_endpoint_for_transport(&endpoint)?;
    Ok(endpoint)
}

fn validate_endpoint_for_transport(endpoint: &Endpoint) -> Result<(), Error> {
    if endpoint.address.trim().is_empty() {
        return Err(invalid_core_protocol_value(
            "endpoint",
            "address must be non-empty",
        ));
    }
    if endpoint.kind == EndpointKind::Tcp
        && endpoint.address.parse::<std::net::SocketAddr>().is_err()
    {
        return Err(invalid_core_protocol_value(
            "endpoint",
            format!("TCP address `{}` must be host:port", endpoint.address),
        ));
    }
    Ok(())
}

impl Capability {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the capability.
    pub fn to_capnp(
        &self,
        builder: azoth_core_capnp::capability::Builder<'_>,
    ) -> Result<(), Error> {
        write_capability(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the capability is absent from the message or is not valid UTF-8.
    pub fn from_capnp(reader: azoth_core_capnp::capability::Reader<'_>) -> Result<Self, Error> {
        read_capability(reader)
    }
}

/// Narrows a `usize` list length or element index to the `u32` Cap'n Proto
/// uses for both.
///
/// The casts this replaces were lossy on 64-bit targets: an oversized
/// collection would have silently written a wrapped length and produced a
/// message that decodes to the wrong number of elements. Failing the write is
/// the honest outcome, and every caller is already in a `Result`.
///
/// # Errors
///
/// Returns an error if `value` does not fit in a `u32`.
fn capnp_list_index(value: usize) -> Result<u32, Error> {
    u32::try_from(value)
        .map_err(|_| Error::failed("Cap'n Proto list length exceeds u32 range".to_string()))
}

fn write_capability(
    capability: &Capability,
    mut builder: azoth_core_capnp::capability::Builder<'_>,
) -> Result<(), Error> {
    validate_capability_for_transport(capability)?;
    write_optional_uuid(capability.session, builder.reborrow().init_session());
    capability
        .service
        .to_capnp(builder.reborrow().init_service());
    builder.set_role(capability.role.to_capnp());
    {
        let mut permissions = builder
            .reborrow()
            .init_permissions(capnp_list_index(capability.permissions.len())?);
        for (index, permission) in capability.permissions.iter().enumerate() {
            permissions.set(capnp_list_index(index)?, permission);
        }
    }
    builder.set_audience(&capability.audience);
    builder.set_expires_unix_ms(capability.expires_unix_ms);
    builder.set_token_hash(&capability.token_hash);
    Ok(())
}

fn read_capability(reader: azoth_core_capnp::capability::Reader<'_>) -> Result<Capability, Error> {
    let role = reader
        .get_role()
        .map(ServiceRole::from_capnp)
        .map_err(|error| Error::failed(format!("unknown service role: {error:?}")))?;
    let permissions = reader
        .get_permissions()?
        .iter()
        .map(|permission| Ok(permission?.to_string()?))
        .collect::<Result<Vec<_>, Error>>()?;

    let capability = Capability {
        session: read_optional_uuid(reader.get_session()?)?,
        service: ServiceId::from_capnp(reader.get_service()?)?,
        role,
        permissions,
        audience: reader.get_audience()?.to_string()?,
        expires_unix_ms: reader.get_expires_unix_ms(),
        token_hash: reader.get_token_hash()?.to_vec(),
    };
    if !capability.is_empty() {
        validate_capability_for_transport(&capability)?;
    }
    Ok(capability)
}

fn validate_capability_for_transport(capability: &Capability) -> Result<(), Error> {
    if capability.is_empty() {
        return Err(invalid_core_protocol_value(
            "capability",
            "empty capability cannot be published",
        ));
    }
    validate_service_id_for_transport(&capability.service, "capability service id")?;
    if capability.role == ServiceRole::Unknown {
        return Err(invalid_core_protocol_value(
            "capability",
            "role must not be Unknown",
        ));
    }
    if capability.audience.trim().is_empty() {
        return Err(invalid_core_protocol_value(
            "capability",
            "audience must be non-empty",
        ));
    }
    if capability.permissions.is_empty() {
        return Err(invalid_core_protocol_value(
            "capability",
            "permissions must be non-empty",
        ));
    }
    let mut seen_permissions = BTreeSet::new();
    for permission in &capability.permissions {
        if permission.trim().is_empty() {
            return Err(invalid_core_protocol_value(
                "capability",
                "permissions cannot contain empty values",
            ));
        }
        if !seen_permissions.insert(permission.as_str()) {
            return Err(invalid_core_protocol_value(
                "capability",
                format!("duplicate permission `{permission}`"),
            ));
        }
    }
    if matches!(capability.session, Some(session) if session == Uuid::nil()) {
        return Err(invalid_core_protocol_value(
            "capability",
            "session id cannot be nil",
        ));
    }
    capability
        .validate_lifetime()
        .map_err(|reason| invalid_core_protocol_value("capability", reason.to_string()))?;
    Ok(())
}

impl CapabilityGrantSet {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the grant set.
    pub fn to_capnp(
        &self,
        builder: azoth_core_capnp::capability_grant_set::Builder<'_>,
    ) -> Result<(), Error> {
        write_capability_grant_set(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if the grant list, or any grant within it, cannot be read from the message.
    pub fn from_capnp(
        reader: azoth_core_capnp::capability_grant_set::Reader<'_>,
    ) -> Result<Self, Error> {
        read_capability_grant_set(reader)
    }
}

fn write_capability_grant_set(
    grant_set: &CapabilityGrantSet,
    mut builder: azoth_core_capnp::capability_grant_set::Builder<'_>,
) -> Result<(), Error> {
    validate_capability_grant_set_for_transport(grant_set)?;
    let mut grants = builder
        .reborrow()
        .init_grants(capnp_list_index(grant_set.grants.len())?);
    for (index, grant) in grant_set.grants.iter().enumerate() {
        grant.to_capnp(grants.reborrow().get(capnp_list_index(index)?))?;
    }
    Ok(())
}

fn read_capability_grant_set(
    reader: azoth_core_capnp::capability_grant_set::Reader<'_>,
) -> Result<CapabilityGrantSet, Error> {
    let grants = reader
        .get_grants()?
        .iter()
        .map(Capability::from_capnp)
        .collect::<Result<Vec<_>, Error>>()?;
    let grant_set = CapabilityGrantSet::from_grants(grants);
    validate_capability_grant_set_for_transport(&grant_set)?;
    Ok(grant_set)
}

/// Serializes the capability grant set as a side-channel blob payload.
///
/// # Errors
///
/// Returns an error if the grant set cannot be written into a Cap'n Proto message.
pub fn encode_capability_grant_set(grant_set: &CapabilityGrantSet) -> Result<Vec<u8>, Error> {
    let mut message = message::Builder::new_default();
    let root = message.init_root::<azoth_core_capnp::capability_grant_set::Builder<'_>>();
    grant_set.to_capnp(root)?;

    let mut bytes = Vec::new();
    serialize_packed::write_message(&mut bytes, &message)?;
    Ok(bytes)
}

/// Deserializes a capability grant set from a side-channel blob payload.
///
/// # Errors
///
/// Returns an error if `bytes` is not a well-formed packed Cap'n Proto message, or does not contain a readable grant set.
pub fn decode_capability_grant_set(bytes: &[u8]) -> Result<CapabilityGrantSet, Error> {
    let reader =
        serialize_packed::read_message(&mut Cursor::new(bytes), message::ReaderOptions::new())?;
    let root = reader.get_root::<azoth_core_capnp::capability_grant_set::Reader<'_>>()?;
    CapabilityGrantSet::from_capnp(root)
}

fn validate_capability_grant_set_for_transport(
    grant_set: &CapabilityGrantSet,
) -> Result<(), Error> {
    if grant_set.grants.is_empty() {
        return Err(invalid_core_protocol_value(
            "capability grant set",
            "grant set cannot be empty",
        ));
    }
    for grant in &grant_set.grants {
        validate_capability_for_transport(grant)?;
        if grant.token_hash.is_empty() {
            return Err(invalid_core_protocol_value(
                "capability grant set",
                format!(
                    "grant `{}` role {:?} must carry a brokered token hash",
                    service_label(&grant.service),
                    grant.role
                ),
            ));
        }
    }
    Ok(())
}

impl ServiceDescriptor {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the descriptor.
    pub fn to_capnp(
        &self,
        builder: azoth_core_capnp::service_descriptor::Builder<'_>,
    ) -> Result<(), Error> {
        write_service_descriptor(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the descriptor is absent from the message or is not valid UTF-8.
    pub fn from_capnp(
        reader: azoth_core_capnp::service_descriptor::Reader<'_>,
    ) -> Result<Self, Error> {
        read_service_descriptor(reader)
    }
}

fn write_service_descriptor(
    descriptor: &ServiceDescriptor,
    mut builder: azoth_core_capnp::service_descriptor::Builder<'_>,
) -> Result<(), Error> {
    validate_service_descriptor_for_transport(descriptor)?;
    descriptor.id.to_capnp(builder.reborrow().init_id());
    builder.set_role(descriptor.role.to_capnp());
    builder.set_run(descriptor.run.as_bytes());
    descriptor
        .protocol
        .to_capnp(builder.reborrow().init_protocol());
    descriptor
        .endpoint
        .to_capnp(builder.reborrow().init_endpoint())?;
    if let Some(endpoint) = &descriptor.observability_endpoint {
        endpoint.to_capnp(builder.reborrow().init_observability_endpoint())?;
    }
    if let Some(endpoint) = &descriptor.lifecycle_endpoint {
        endpoint.to_capnp(builder.reborrow().init_lifecycle_endpoint())?;
    }
    {
        let mut capabilities = builder
            .reborrow()
            .init_capabilities(capnp_list_index(descriptor.capabilities.len())?);
        for (index, capability) in descriptor.capabilities.iter().enumerate() {
            capability.to_capnp(capabilities.reborrow().get(capnp_list_index(index)?))?;
        }
    }
    Ok(())
}

fn read_service_descriptor(
    reader: azoth_core_capnp::service_descriptor::Reader<'_>,
) -> Result<ServiceDescriptor, Error> {
    let role = reader
        .get_role()
        .map(ServiceRole::from_capnp)
        .map_err(|error| Error::failed(format!("unknown service role: {error:?}")))?;
    let capabilities = reader
        .get_capabilities()?
        .iter()
        .map(Capability::from_capnp)
        .collect::<Result<Vec<_>, _>>()?;

    let descriptor = ServiceDescriptor {
        id: ServiceId::from_capnp(reader.get_id()?)?,
        role,
        run: read_required_uuid(reader.get_run()?, "service descriptor run")?,
        protocol: ProtocolVersion::from_capnp(reader.get_protocol()?),
        endpoint: Endpoint::from_capnp(reader.get_endpoint()?)?,
        observability_endpoint: if reader.has_observability_endpoint() {
            Some(Endpoint::from_capnp(reader.get_observability_endpoint()?)?)
        } else {
            None
        },
        lifecycle_endpoint: if reader.has_lifecycle_endpoint() {
            Some(Endpoint::from_capnp(reader.get_lifecycle_endpoint()?)?)
        } else {
            None
        },
        capabilities,
    };
    validate_service_descriptor_for_transport(&descriptor)?;
    Ok(descriptor)
}

impl ServiceHealth {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the health record.
    pub fn to_capnp(
        &self,
        builder: azoth_core_capnp::service_health::Builder<'_>,
    ) -> Result<(), Error> {
        write_service_health(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the health record is absent from the message or is not valid UTF-8.
    pub fn from_capnp(reader: azoth_core_capnp::service_health::Reader<'_>) -> Result<Self, Error> {
        read_service_health(reader)
    }
}

fn write_service_health(
    health: &ServiceHealth,
    mut builder: azoth_core_capnp::service_health::Builder<'_>,
) -> Result<(), Error> {
    validate_service_id_for_transport(&health.service, "service health id")?;
    if health.role == ServiceRole::Unknown {
        return Err(invalid_core_protocol_value(
            "service health",
            "role must not be Unknown",
        ));
    }
    if health.run == Uuid::nil() {
        return Err(invalid_core_protocol_value(
            "service health",
            "run must be a non-nil UUID",
        ));
    }

    health.service.to_capnp(builder.reborrow().init_service());
    builder.set_role(health.role.to_capnp());
    builder.set_run(health.run.as_bytes());
    health
        .protocol_version
        .to_capnp(builder.reborrow().init_protocol_version());
    builder.set_state(health.state.to_capnp());
    builder.set_ready(health.ready);
    builder.set_degraded(health.degraded);
    builder.set_pid(health.pid);
    builder.set_checked_unix_ms(health.checked_unix_ms);
    builder.set_uptime_ms(health.uptime_ms);
    builder.set_active_operation(&health.active_operation);
    builder.set_message(&health.message);
    builder.set_last_event_seq(health.last_event_seq);
    Ok(())
}

fn read_service_health(
    reader: azoth_core_capnp::service_health::Reader<'_>,
) -> Result<ServiceHealth, Error> {
    let state = reader
        .get_state()
        .map(ServiceHealthState::from_capnp)
        .map_err(|error| Error::failed(format!("unknown service health state: {error:?}")))?;
    let role = reader
        .get_role()
        .map(ServiceRole::from_capnp)
        .map_err(|error| Error::failed(format!("unknown service role: {error:?}")))?;
    let health = ServiceHealth {
        service: ServiceId::from_capnp(reader.get_service()?)?,
        role,
        run: read_required_uuid(reader.get_run()?, "service health run")?,
        protocol_version: ProtocolVersion::from_capnp(reader.get_protocol_version()?),
        state,
        ready: reader.get_ready(),
        degraded: reader.get_degraded(),
        pid: reader.get_pid(),
        checked_unix_ms: reader.get_checked_unix_ms(),
        uptime_ms: reader.get_uptime_ms(),
        active_operation: reader.get_active_operation()?.to_string()?,
        message: reader.get_message()?.to_string()?,
        last_event_seq: reader.get_last_event_seq(),
    };
    validate_service_id_for_transport(&health.service, "service health id")?;
    if health.role == ServiceRole::Unknown {
        return Err(invalid_core_protocol_value(
            "service health",
            "role must not be Unknown",
        ));
    }
    Ok(health)
}

fn validate_service_descriptor_for_transport(descriptor: &ServiceDescriptor) -> Result<(), Error> {
    validate_service_id_for_transport(&descriptor.id, "service descriptor id")?;
    if descriptor.role == ServiceRole::Unknown {
        return Err(invalid_core_protocol_value(
            "service descriptor",
            "role must not be Unknown",
        ));
    }
    if descriptor.run == Uuid::nil() {
        return Err(invalid_core_protocol_value(
            "service descriptor",
            "run must be a non-nil UUID",
        ));
    }
    validate_endpoint_for_transport(&descriptor.endpoint)?;
    if let Some(endpoint) = &descriptor.observability_endpoint {
        validate_endpoint_for_transport(endpoint)?;
    }
    if let Some(endpoint) = &descriptor.lifecycle_endpoint {
        validate_endpoint_for_transport(endpoint)?;
    }
    if descriptor.capabilities.is_empty() {
        return Err(invalid_core_protocol_value(
            "service descriptor",
            "capability templates must be non-empty",
        ));
    }
    for capability in &descriptor.capabilities {
        validate_capability_for_transport(capability)?;
    }
    descriptor
        .validate_brokered_capability_templates()
        .map_err(|error| invalid_core_protocol_value("service descriptor", error.to_string()))?;
    Ok(())
}

fn validate_service_id_for_transport(id: &ServiceId, kind: &'static str) -> Result<(), Error> {
    if id.namespace.trim().is_empty() || id.name.trim().is_empty() {
        return Err(invalid_core_protocol_value(
            kind,
            "namespace and name must be non-empty",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SideChannelKind {
    SharedMemory,
    CasBlob,
    StagingFile,
    GpuSurface,
    MmapFile,
}

impl SideChannelKind {
    #[must_use]
    pub const fn to_capnp(self) -> azoth_core_capnp::SideChannelKind {
        match self {
            Self::SharedMemory => azoth_core_capnp::SideChannelKind::SharedMemory,
            Self::CasBlob => azoth_core_capnp::SideChannelKind::CasBlob,
            Self::StagingFile => azoth_core_capnp::SideChannelKind::StagingFile,
            Self::GpuSurface => azoth_core_capnp::SideChannelKind::GpuSurface,
            Self::MmapFile => azoth_core_capnp::SideChannelKind::MmapFile,
        }
    }

    #[must_use]
    pub const fn from_capnp(kind: azoth_core_capnp::SideChannelKind) -> Self {
        match kind {
            azoth_core_capnp::SideChannelKind::SharedMemory => Self::SharedMemory,
            azoth_core_capnp::SideChannelKind::CasBlob => Self::CasBlob,
            azoth_core_capnp::SideChannelKind::StagingFile => Self::StagingFile,
            azoth_core_capnp::SideChannelKind::GpuSurface => Self::GpuSurface,
            azoth_core_capnp::SideChannelKind::MmapFile => Self::MmapFile,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideChannelHandle {
    pub kind: SideChannelKind,
    pub capability: Option<Capability>,
    pub locator: String,
    pub byte_length: u64,
    pub content_hash: Vec<u8>,
    pub platform: String,
}

impl SideChannelHandle {
    #[must_use]
    pub fn staging_file(
        locator: impl Into<String>,
        byte_length: u64,
        content_hash: Vec<u8>,
        platform: impl Into<String>,
    ) -> Self {
        Self {
            kind: SideChannelKind::StagingFile,
            capability: None,
            locator: locator.into(),
            byte_length,
            content_hash,
            platform: platform.into(),
        }
    }

    #[must_use]
    pub fn mmap_file(
        locator: impl Into<String>,
        byte_length: u64,
        platform: impl Into<String>,
    ) -> Self {
        Self {
            kind: SideChannelKind::MmapFile,
            capability: None,
            locator: locator.into(),
            byte_length,
            content_hash: Vec::new(),
            platform: platform.into(),
        }
    }

    #[must_use]
    pub fn with_capability(mut self, capability: Capability) -> Self {
        self.capability = Some(capability);
        self
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.capability.is_none()
            && self.locator.is_empty()
            && self.byte_length == 0
            && self.content_hash.is_empty()
            && self.platform.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SideChannelCapabilityError {
    #[error("side-channel handle for {purpose} is missing its brokered capability")]
    Missing { purpose: String },

    #[error("side-channel handle for {purpose} is not bound to the expected command capability")]
    Mismatch { purpose: String },
}

/// # Errors
///
/// Returns [`SideChannelCapabilityError::Missing`] if `handle` carries no capability.
pub fn require_side_channel_capability(
    handle: &SideChannelHandle,
    purpose: impl Into<String>,
) -> Result<&Capability, SideChannelCapabilityError> {
    handle
        .capability
        .as_ref()
        .ok_or_else(|| SideChannelCapabilityError::Missing {
            purpose: purpose.into(),
        })
}

/// # Errors
///
/// Returns [`SideChannelCapabilityError::Missing`] if `handle` carries no capability, or
/// [`SideChannelCapabilityError::Mismatch`] if the capability it carries is not `expected`.
pub fn validate_side_channel_capability_matches(
    handle: &SideChannelHandle,
    expected: &Capability,
    purpose: impl Into<String>,
) -> Result<(), SideChannelCapabilityError> {
    let purpose = purpose.into();
    let actual = require_side_channel_capability(handle, purpose.clone())?;
    if actual == expected {
        Ok(())
    } else {
        Err(SideChannelCapabilityError::Mismatch { purpose })
    }
}

#[derive(Debug)]
pub struct VerifiedStagingFile {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagingFileWrite {
    pub path: PathBuf,
    pub byte_length: u64,
    pub content_hash: Vec<u8>,
}

#[derive(Debug, Error)]
#[error("failed to write staging file side-channel {path}: {source}")]
pub struct StagingFileWriteError {
    pub path: PathBuf,
    #[source]
    pub source: std::io::Error,
}

#[derive(Debug)]
pub struct MmapFilePrefix {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum StagingFileSideChannelError {
    #[error("side-channel must be a staging file, got {kind:?}")]
    UnsupportedKind { kind: SideChannelKind },

    #[error("staging file side-channel locator cannot be empty")]
    EmptyLocator,

    #[error("staging file side-channel locator `{locator}` must be an absolute path")]
    RelativeLocator { locator: String },

    #[error("staging file side-channel locator `{locator}` must not contain `.` or `..`")]
    NonNormalLocator { locator: String },

    #[error("staging file side-channel hash must be 32 bytes, got {actual}")]
    InvalidContentHashLength { actual: usize },

    #[error("failed to read staging file side-channel {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("staging file side-channel {path} has {actual} bytes, expected {expected}")]
    LengthMismatch {
        path: PathBuf,
        expected: u64,
        actual: u64,
    },

    #[error("staging file side-channel {path} content hash did not match handle metadata")]
    HashMismatch {
        path: PathBuf,
        expected: Vec<u8>,
        actual: Vec<u8>,
    },
}

/// # Errors
///
/// Returns [`StagingFileSideChannelError`] if `handle` is not a staging-file handle, its locator is blank, relative, or not
/// a normalized absolute path, or its content hash is not [`SIDE_CHANNEL_BLAKE3_HASH_BYTES`] long.
pub fn validated_staging_file_path(
    handle: &SideChannelHandle,
) -> Result<PathBuf, StagingFileSideChannelError> {
    if handle.kind != SideChannelKind::StagingFile {
        return Err(StagingFileSideChannelError::UnsupportedKind { kind: handle.kind });
    }
    if handle.locator.trim().is_empty() {
        return Err(StagingFileSideChannelError::EmptyLocator);
    }
    if handle.content_hash.len() != SIDE_CHANNEL_BLAKE3_HASH_BYTES {
        return Err(StagingFileSideChannelError::InvalidContentHashLength {
            actual: handle.content_hash.len(),
        });
    }

    let path = PathBuf::from(&handle.locator);
    if !path.is_absolute() {
        return Err(StagingFileSideChannelError::RelativeLocator {
            locator: handle.locator.clone(),
        });
    }
    if !is_normal_absolute_path(&path) {
        return Err(StagingFileSideChannelError::NonNormalLocator {
            locator: handle.locator.clone(),
        });
    }
    Ok(path)
}

/// # Errors
///
/// Returns [`StagingFileSideChannelError`] if `handle` does not name a valid staging-file path, if the file cannot be read,
/// or if its contents do not match the length and BLAKE3 hash the handle declares.
pub fn read_verified_staging_file(
    handle: &SideChannelHandle,
) -> Result<VerifiedStagingFile, StagingFileSideChannelError> {
    let path = validated_staging_file_path(handle)?;
    let bytes = std::fs::read(&path).map_err(|source| StagingFileSideChannelError::Read {
        path: path.clone(),
        source,
    })?;

    let actual_len = bytes.len() as u64;
    if actual_len != handle.byte_length {
        return Err(StagingFileSideChannelError::LengthMismatch {
            path,
            expected: handle.byte_length,
            actual: actual_len,
        });
    }

    let actual_hash = blake3::hash(&bytes).as_bytes().to_vec();
    if actual_hash != handle.content_hash {
        return Err(StagingFileSideChannelError::HashMismatch {
            path,
            expected: handle.content_hash.clone(),
            actual: actual_hash,
        });
    }

    Ok(VerifiedStagingFile { path, bytes })
}

/// # Errors
///
/// Returns [`StagingFileWriteError`] if the staging directory cannot be created or the file cannot be written atomically.
pub fn write_content_addressed_staging_file(
    root: impl AsRef<Path>,
    prefix: &str,
    bytes: &[u8],
) -> Result<StagingFileWrite, StagingFileWriteError> {
    let content_hash = blake3::hash(bytes).as_bytes().to_vec();
    let file_name = format!("{prefix}-{}.capnp.packed", hex_lower(&content_hash));
    write_named_staging_file_atomic(root.as_ref().join(file_name), bytes)
}

/// # Errors
///
/// Returns [`StagingFileWriteError`] if `path` is not a valid staging write path, has no parent directory, or if the
/// directory cannot be created or the file cannot be written atomically.
pub fn write_named_staging_file_atomic(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<StagingFileWrite, StagingFileWriteError> {
    let path = path.as_ref();
    validate_staging_write_path(path)?;
    let parent = path.parent().ok_or_else(|| {
        staging_write_error(
            path,
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "staging file path has no parent",
            ),
        )
    })?;
    std::fs::create_dir_all(parent).map_err(|source| staging_write_error(parent, source))?;

    let content_hash = blake3::hash(bytes).as_bytes().to_vec();
    if staging_file_matches(path, bytes) {
        return Ok(StagingFileWrite {
            path: path.to_path_buf(),
            byte_length: bytes.len() as u64,
            content_hash,
        });
    }

    let temp_path = unique_staging_temp_path(parent, path);
    let write_result = (|| -> Result<(), std::io::Error> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        file.write_all(bytes)?;
        file.flush()?;
        Ok(())
    })();
    if let Err(source) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(staging_write_error(&temp_path, source));
    }

    publish_staging_temp_file(&temp_path, path, bytes)?;

    Ok(StagingFileWrite {
        path: path.to_path_buf(),
        byte_length: bytes.len() as u64,
        content_hash,
    })
}

fn validate_staging_write_path(path: &Path) -> Result<(), StagingFileWriteError> {
    if !path.is_absolute() {
        return Err(staging_write_error(
            path,
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "staging file path must be absolute",
            ),
        ));
    }
    if !is_normal_absolute_path(path) {
        return Err(staging_write_error(
            path,
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "staging file path must not contain `.` or `..`",
            ),
        ));
    }
    Ok(())
}

fn publish_staging_temp_file(
    temp_path: &Path,
    path: &Path,
    bytes: &[u8],
) -> Result<(), StagingFileWriteError> {
    match std::fs::rename(temp_path, path) {
        Ok(()) => Ok(()),
        Err(_) if path.exists() && staging_file_matches(path, bytes) => {
            let _ = std::fs::remove_file(temp_path);
            Ok(())
        }
        Err(_) if path.exists() => {
            std::fs::remove_file(path).map_err(|source| staging_write_error(path, source))?;
            std::fs::rename(temp_path, path).map_err(|source| staging_write_error(path, source))
        }
        Err(source) => {
            let _ = std::fs::remove_file(temp_path);
            Err(staging_write_error(path, source))
        }
    }
}

fn staging_file_matches(path: &Path, bytes: &[u8]) -> bool {
    std::fs::read(path).is_ok_and(|existing| existing == bytes)
}

fn unique_staging_temp_path(parent: &Path, path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map_or_else(|| "staging-file".into(), |name| name.to_string_lossy());
    let sequence = STAGING_FILE_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        sequence
    ))
}

fn staging_write_error(path: impl AsRef<Path>, source: std::io::Error) -> StagingFileWriteError {
    StagingFileWriteError {
        path: path.as_ref().to_path_buf(),
        source,
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[derive(Debug, Error)]
pub enum MmapFileSideChannelError {
    #[error("side-channel must be an mmap file, got {kind:?}")]
    UnsupportedKind { kind: SideChannelKind },

    #[error("mmap file side-channel byte length {byte_length} does not fit in memory")]
    ByteLengthTooLarge { byte_length: u64 },

    #[error("mmap file side-channel locator cannot be empty")]
    EmptyLocator,

    #[error("mmap file side-channel locator `{locator}` must be an absolute path")]
    RelativeLocator { locator: String },

    #[error("mmap file side-channel locator `{locator}` must not contain `.` or `..`")]
    NonNormalLocator { locator: String },

    #[error("failed to read mmap file side-channel {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("mmap file side-channel {path} has {actual} bytes, expected at least {expected}")]
    LengthMismatch {
        path: PathBuf,
        expected: u64,
        actual: u64,
    },
}

/// # Errors
///
/// Returns [`MmapFileSideChannelError`] if `handle` is not an mmap-file handle, or its locator is blank, relative, or not a
/// normalized absolute path.
pub fn validated_mmap_file_path(
    handle: &SideChannelHandle,
) -> Result<PathBuf, MmapFileSideChannelError> {
    if handle.kind != SideChannelKind::MmapFile {
        return Err(MmapFileSideChannelError::UnsupportedKind { kind: handle.kind });
    }
    if handle.locator.trim().is_empty() {
        return Err(MmapFileSideChannelError::EmptyLocator);
    }

    let path = PathBuf::from(&handle.locator);
    if !path.is_absolute() {
        return Err(MmapFileSideChannelError::RelativeLocator {
            locator: handle.locator.clone(),
        });
    }
    if !is_normal_absolute_path(&path) {
        return Err(MmapFileSideChannelError::NonNormalLocator {
            locator: handle.locator.clone(),
        });
    }
    Ok(path)
}

/// # Errors
///
/// Returns [`MmapFileSideChannelError`] if `handle` does not name a valid mmap-file path, if its declared byte length
/// exceeds `usize`, or if the file cannot be opened or read.
pub fn read_mmap_file_prefix(
    handle: &SideChannelHandle,
) -> Result<MmapFilePrefix, MmapFileSideChannelError> {
    let path = validated_mmap_file_path(handle)?;
    let expected = usize::try_from(handle.byte_length).map_err(|_| {
        MmapFileSideChannelError::ByteLengthTooLarge {
            byte_length: handle.byte_length,
        }
    })?;
    let mut file = std::fs::File::open(&path).map_err(|source| MmapFileSideChannelError::Read {
        path: path.clone(),
        source,
    })?;
    let actual = file
        .metadata()
        .map_err(|source| MmapFileSideChannelError::Read {
            path: path.clone(),
            source,
        })?
        .len();
    if actual < handle.byte_length {
        return Err(MmapFileSideChannelError::LengthMismatch {
            path,
            expected: handle.byte_length,
            actual,
        });
    }

    let mut bytes = vec![0; expected];
    file.read_exact(&mut bytes)
        .map_err(|source| MmapFileSideChannelError::Read {
            path: path.clone(),
            source,
        })?;
    Ok(MmapFilePrefix { path, bytes })
}

fn is_normal_absolute_path(path: &Path) -> bool {
    path.is_absolute()
        && path.components().all(|component| {
            matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::Normal(_)
            )
        })
}

impl SideChannelHandle {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the handle.
    pub fn to_capnp(
        &self,
        builder: azoth_core_capnp::side_channel_handle::Builder<'_>,
    ) -> Result<(), Error> {
        write_side_channel_handle(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the handle is absent from the message or is not valid UTF-8.
    pub fn from_capnp(
        reader: azoth_core_capnp::side_channel_handle::Reader<'_>,
    ) -> Result<Self, Error> {
        read_side_channel_handle(reader)
    }
}

fn write_side_channel_handle(
    handle: &SideChannelHandle,
    mut builder: azoth_core_capnp::side_channel_handle::Builder<'_>,
) -> Result<(), Error> {
    validate_side_channel_handle_for_transport(handle)?;
    builder.set_kind(handle.kind.to_capnp());
    if let Some(capability) = &handle.capability {
        capability.to_capnp(builder.reborrow().init_capability())?;
    }
    builder.set_locator(&handle.locator);
    builder.set_byte_length(handle.byte_length);
    builder.set_content_hash(&handle.content_hash);
    builder.set_platform(&handle.platform);
    Ok(())
}

fn read_side_channel_handle(
    reader: azoth_core_capnp::side_channel_handle::Reader<'_>,
) -> Result<SideChannelHandle, Error> {
    let kind = reader
        .get_kind()
        .map(SideChannelKind::from_capnp)
        .map_err(|error| Error::failed(format!("unknown side-channel kind: {error:?}")))?;

    let capability = Capability::from_capnp(reader.get_capability()?)?;
    let handle = SideChannelHandle {
        kind,
        capability: (!capability.is_empty()).then_some(capability),
        locator: reader.get_locator()?.to_string()?,
        byte_length: reader.get_byte_length(),
        content_hash: reader.get_content_hash()?.to_vec(),
        platform: reader.get_platform()?.to_string()?,
    };
    if !handle.is_empty() {
        validate_side_channel_handle_for_transport(&handle)?;
    }
    Ok(handle)
}

fn validate_side_channel_handle_for_transport(handle: &SideChannelHandle) -> Result<(), Error> {
    if handle.is_empty() {
        return Err(invalid_core_protocol_value(
            "side-channel handle",
            "empty handle cannot be published",
        ));
    }
    if handle.locator.trim().is_empty() || handle.platform.trim().is_empty() {
        return Err(invalid_core_protocol_value(
            "side-channel handle",
            "locator and platform must be non-empty",
        ));
    }
    if handle.byte_length == 0 {
        return Err(invalid_core_protocol_value(
            "side-channel handle",
            "byte length must be positive",
        ));
    }
    if let Some(capability) = &handle.capability {
        validate_capability_for_transport(capability)?;
    }

    match handle.kind {
        SideChannelKind::StagingFile => {
            validated_staging_file_path(handle).map_err(|reason| {
                invalid_core_protocol_value("side-channel handle", reason.to_string())
            })?;
        }
        SideChannelKind::MmapFile => {
            validated_mmap_file_path(handle).map_err(|reason| {
                invalid_core_protocol_value("side-channel handle", reason.to_string())
            })?;
            if !handle.content_hash.is_empty() {
                return Err(invalid_core_protocol_value(
                    "side-channel handle",
                    "mmap file handles must not carry a content hash",
                ));
            }
        }
        SideChannelKind::CasBlob => {
            if handle.content_hash.len() != SIDE_CHANNEL_BLAKE3_HASH_BYTES {
                return Err(invalid_core_protocol_value(
                    "side-channel handle",
                    format!(
                        "CAS blob hash must be {SIDE_CHANNEL_BLAKE3_HASH_BYTES} bytes, got {}",
                        handle.content_hash.len()
                    ),
                ));
            }
        }
        SideChannelKind::SharedMemory | SideChannelKind::GpuSurface => {}
    }
    Ok(())
}

fn invalid_core_protocol_value(kind: &'static str, reason: impl Into<String>) -> Error {
    Error::failed(format!("invalid {kind}: {}", reason.into()))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use capnp::{message, serialize_packed};

    use super::*;

    #[test]
    fn service_descriptor_control_equality_excludes_observational_run() {
        let first = ServiceDescriptor {
            id: ServiceId::new("azoth", "project-host"),
            role: ServiceRole::ProjectHost,
            run: Uuid::now_v7(),
            protocol: ProtocolVersion::CURRENT,
            endpoint: Endpoint::new(EndpointKind::Tcp, "127.0.0.1:41000"),
            observability_endpoint: None,
            lifecycle_endpoint: None,
            capabilities: Vec::new(),
        };
        let mut second = first.clone();
        second.run = Uuid::now_v7();

        assert_ne!(first.run, second.run);
        assert_eq!(first, second);

        second.endpoint = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:41001");
        assert_ne!(first, second);
    }

    #[test]
    fn protocol_version_round_trips_through_generated_capnp_type() {
        let mut message = message::Builder::new_default();
        let root = message.init_root::<azoth_core_capnp::protocol_version::Builder<'_>>();
        ProtocolVersion::CURRENT.to_capnp(root);

        let mut bytes = Vec::new();
        serialize_packed::write_message(&mut bytes, &message).unwrap();

        let reader =
            serialize_packed::read_message(&mut Cursor::new(bytes), message::ReaderOptions::new())
                .unwrap();
        let root = reader
            .get_root::<azoth_core_capnp::protocol_version::Reader<'_>>()
            .unwrap();

        assert_eq!(ProtocolVersion::from_capnp(root), ProtocolVersion::CURRENT);
    }

    #[test]
    fn service_id_round_trips_text_fields() {
        let mut message = message::Builder::new_default();
        let root = message.init_root::<azoth_core_capnp::service_id::Builder<'_>>();
        ServiceId::new("azoth", "project-host").to_capnp(root);

        let mut bytes = Vec::new();
        serialize_packed::write_message(&mut bytes, &message).unwrap();

        let reader =
            serialize_packed::read_message(&mut Cursor::new(bytes), message::ReaderOptions::new())
                .unwrap();
        let root = reader
            .get_root::<azoth_core_capnp::service_id::Reader<'_>>()
            .unwrap();

        assert_eq!(
            ServiceId::from_capnp(root).unwrap(),
            ServiceId::new("azoth", "project-host")
        );
    }

    #[test]
    fn session_id_rejects_non_uuid_byte_length() {
        let mut message = message::Builder::new_default();
        let mut root = message.init_root::<azoth_core_capnp::session_id::Builder<'_>>();
        root.set_uuid(&[1, 2, 3]);

        let reader = message
            .get_root_as_reader::<azoth_core_capnp::session_id::Reader<'_>>()
            .unwrap();

        assert!(SessionId::from_capnp(reader).is_err());
    }

    #[test]
    fn side_channel_handle_round_trips_transport_fields() {
        let capability =
            Capability::new(ServiceId::new("azoth", "asset-worker"), ServiceRole::Worker)
                .with_session(Uuid::from_bytes([0x5a; 16]))
                .with_audience("asset-processor")
                .with_permissions(["asset.jobs"])
                .with_token_hash([0x44, 0x55]);
        let handle = SideChannelHandle::staging_file(
            std::env::temp_dir()
                .join("schema.capnp.packed")
                .to_string_lossy(),
            128,
            vec![1; SIDE_CHANNEL_BLAKE3_HASH_BYTES],
            "windows",
        )
        .with_capability(capability);

        let mut message = message::Builder::new_default();
        let root = message.init_root::<azoth_core_capnp::side_channel_handle::Builder<'_>>();
        handle.to_capnp(root).unwrap();

        let reader = message
            .get_root_as_reader::<azoth_core_capnp::side_channel_handle::Reader<'_>>()
            .unwrap();

        assert_eq!(SideChannelHandle::from_capnp(reader).unwrap(), handle);
    }

    #[test]
    fn side_channel_capability_helpers_require_brokered_handle_capability() {
        let expected =
            Capability::new(ServiceId::new("azoth", "asset-worker"), ServiceRole::Worker)
                .with_session(Uuid::from_bytes([0x5a; 16]))
                .with_audience("asset-processor")
                .with_permissions(["asset.jobs"])
                .with_token_hash([0x44, 0x55]);
        let unbound = SideChannelHandle::staging_file(
            std::env::temp_dir()
                .join("product-manifest.capnp.packed")
                .to_string_lossy(),
            128,
            vec![1; SIDE_CHANNEL_BLAKE3_HASH_BYTES],
            "windows",
        );

        assert!(matches!(
            require_side_channel_capability(&unbound, "asset product manifest"),
            Err(SideChannelCapabilityError::Missing { purpose })
                if purpose == "asset product manifest"
        ));
        assert!(matches!(
            validate_side_channel_capability_matches(
                &unbound,
                &expected,
                "asset product manifest"
            ),
            Err(SideChannelCapabilityError::Missing { purpose })
                if purpose == "asset product manifest"
        ));

        let matched = unbound.with_capability(expected.clone());
        assert_eq!(
            require_side_channel_capability(&matched, "asset product manifest").unwrap(),
            &expected
        );
        validate_side_channel_capability_matches(&matched, &expected, "asset product manifest")
            .unwrap();

        let wrong = matched.with_capability(expected.clone().with_token_hash([0x99]));
        assert!(matches!(
            validate_side_channel_capability_matches(
                &wrong,
                &expected,
                "asset product manifest"
            ),
            Err(SideChannelCapabilityError::Mismatch { purpose })
                if purpose == "asset product manifest"
        ));
    }

    #[test]
    fn verified_staging_file_reads_matching_absolute_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("payload.capnp.packed");
        let bytes = b"side-channel payload";
        std::fs::write(&path, bytes).unwrap();
        let handle = SideChannelHandle::staging_file(
            path.to_string_lossy(),
            bytes.len() as u64,
            blake3::hash(bytes).as_bytes().to_vec(),
            std::env::consts::OS,
        );

        let verified = read_verified_staging_file(&handle).unwrap();
        assert_eq!(verified.path, path);
        assert_eq!(verified.bytes, bytes);
    }

    #[test]
    fn verified_staging_file_rejects_relative_locator() {
        let handle = SideChannelHandle::staging_file(
            "relative/payload.capnp.packed",
            1,
            vec![0; SIDE_CHANNEL_BLAKE3_HASH_BYTES],
            std::env::consts::OS,
        );

        assert!(matches!(
            read_verified_staging_file(&handle),
            Err(StagingFileSideChannelError::RelativeLocator { .. })
        ));
    }

    #[test]
    fn verified_staging_file_rejects_invalid_hash_length() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("payload.capnp.packed");
        std::fs::write(&path, b"payload").unwrap();
        let handle =
            SideChannelHandle::staging_file(path.to_string_lossy(), 7, vec![0; 31], "windows");

        assert!(matches!(
            read_verified_staging_file(&handle),
            Err(StagingFileSideChannelError::InvalidContentHashLength { actual: 31 })
        ));
    }

    #[test]
    fn verified_staging_file_rejects_hash_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("payload.capnp.packed");
        std::fs::write(&path, b"payload").unwrap();
        let handle = SideChannelHandle::staging_file(
            path.to_string_lossy(),
            7,
            vec![0; SIDE_CHANNEL_BLAKE3_HASH_BYTES],
            std::env::consts::OS,
        );

        assert!(matches!(
            read_verified_staging_file(&handle),
            Err(StagingFileSideChannelError::HashMismatch { .. })
        ));
    }

    #[test]
    fn named_staging_file_write_publishes_final_verified_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("nested").join("payload.capnp.packed");
        let written = write_named_staging_file_atomic(&path, b"payload").unwrap();

        assert_eq!(written.path, path);
        assert_eq!(written.byte_length, 7);
        assert_eq!(
            written.content_hash,
            blake3::hash(b"payload").as_bytes().to_vec()
        );
        let handle = SideChannelHandle::staging_file(
            path.to_string_lossy(),
            written.byte_length,
            written.content_hash,
            std::env::consts::OS,
        );
        assert_eq!(
            read_verified_staging_file(&handle).unwrap().bytes,
            b"payload"
        );
        assert_eq!(
            temp.path()
                .join("nested")
                .read_dir()
                .unwrap()
                .filter(|entry| entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with('.'))
                .count(),
            0
        );
    }

    #[test]
    fn named_staging_file_write_rejects_relative_path() {
        let path = PathBuf::from("relative").join("payload.capnp.packed");
        let error = write_named_staging_file_atomic(&path, b"payload").unwrap_err();

        assert_eq!(error.path, path);
        assert_eq!(error.source.kind(), std::io::ErrorKind::InvalidInput);
        assert!(
            error.source.to_string().contains("must be absolute"),
            "{}",
            error.source
        );
    }

    #[test]
    fn named_staging_file_write_rejects_non_normal_absolute_path() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp
            .path()
            .join("nested")
            .join("..")
            .join("payload.capnp.packed");
        let error = write_named_staging_file_atomic(&path, b"payload").unwrap_err();

        assert_eq!(error.path, path);
        assert_eq!(error.source.kind(), std::io::ErrorKind::InvalidInput);
        assert!(
            error.source.to_string().contains("must not contain"),
            "{}",
            error.source
        );
    }

    #[test]
    fn content_addressed_staging_file_write_rejects_relative_root() {
        let error =
            write_content_addressed_staging_file("relative-root", "runtime-launch", b"payload")
                .unwrap_err();

        assert_eq!(error.source.kind(), std::io::ErrorKind::InvalidInput);
        assert!(
            error.path.starts_with("relative-root"),
            "{}",
            error.path.display()
        );
        assert!(
            error.source.to_string().contains("must be absolute"),
            "{}",
            error.source
        );
    }

    #[test]
    fn content_addressed_staging_file_write_reuses_matching_file() {
        let temp = tempfile::tempdir().unwrap();
        let first = write_content_addressed_staging_file(temp.path(), "runtime-launch", b"payload")
            .unwrap();
        let second =
            write_content_addressed_staging_file(temp.path(), "runtime-launch", b"payload")
                .unwrap();

        assert_eq!(first, second);
        assert!(first.path.starts_with(temp.path()));
        assert_eq!(std::fs::read(first.path).unwrap(), b"payload");
    }

    #[test]
    fn mmap_file_prefix_reads_declared_absolute_file_prefix() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("viewport-frame.mmap");
        std::fs::write(&path, [1, 2, 3, 4, 9, 9, 9, 9]).unwrap();
        let handle = SideChannelHandle::mmap_file(path.to_string_lossy(), 4, std::env::consts::OS);

        let prefix = read_mmap_file_prefix(&handle).unwrap();

        assert_eq!(prefix.path, path);
        assert_eq!(prefix.bytes, vec![1, 2, 3, 4]);
    }

    #[test]
    fn mmap_file_prefix_rejects_unsupported_side_channel_kind() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("viewport-frame.bgra");
        let handle = SideChannelHandle::staging_file(
            path.to_string_lossy(),
            4,
            vec![0; SIDE_CHANNEL_BLAKE3_HASH_BYTES],
            std::env::consts::OS,
        );

        assert!(matches!(
            read_mmap_file_prefix(&handle),
            Err(MmapFileSideChannelError::UnsupportedKind {
                kind: SideChannelKind::StagingFile
            })
        ));
    }

    #[test]
    fn mmap_file_prefix_rejects_relative_locator() {
        let handle =
            SideChannelHandle::mmap_file("relative/viewport-frame.mmap", 4, std::env::consts::OS);

        assert!(matches!(
            read_mmap_file_prefix(&handle),
            Err(MmapFileSideChannelError::RelativeLocator { .. })
        ));
    }

    #[test]
    fn mmap_file_prefix_rejects_undersized_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("viewport-frame.mmap");
        std::fs::write(&path, [1, 2, 3, 4]).unwrap();
        let handle = SideChannelHandle::mmap_file(path.to_string_lossy(), 8, std::env::consts::OS);

        assert!(matches!(
            read_mmap_file_prefix(&handle),
            Err(MmapFileSideChannelError::LengthMismatch {
                expected: 8,
                actual: 4,
                ..
            })
        ));
    }

    #[test]
    fn service_descriptor_round_trips_transport_and_capabilities() {
        let service = ServiceId::new("azoth", "project-host");
        let session = Uuid::from_bytes([9; 16]);
        let capability = Capability::new(service.clone(), ServiceRole::ProjectHost)
            .with_session(session)
            .with_audience("editor")
            .with_permissions(["schemaCatalog", "applyEdit"])
            .with_token_hash([0xaa, 0xbb, 0xcc]);
        let descriptor = ServiceDescriptor::new(
            service,
            ServiceRole::ProjectHost,
            Endpoint::new(
                EndpointKind::WindowsNamedPipe,
                r"\\.\pipe\azoth\session\project-host",
            ),
        )
        .with_run(Uuid::now_v7())
        .with_capability(capability);

        let mut message = message::Builder::new_default();
        let root = message.init_root::<azoth_core_capnp::service_descriptor::Builder<'_>>();
        descriptor.to_capnp(root).unwrap();

        let reader = message
            .get_root_as_reader::<azoth_core_capnp::service_descriptor::Reader<'_>>()
            .unwrap();

        let round_trip = ServiceDescriptor::from_capnp(reader).unwrap();
        assert_eq!(round_trip.run, descriptor.run);
        assert_eq!(round_trip, descriptor);
    }

    #[test]
    fn service_descriptor_connection_contract_ignores_only_run() {
        let endpoint = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:42000");
        let descriptor = ServiceDescriptor::new(
            ServiceId::new("azoth", "project-host"),
            ServiceRole::ProjectHost,
            endpoint,
        )
        .with_run(Uuid::now_v7())
        .with_capability(
            Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
                .with_audience("project-host")
                .with_permissions(["schemaCatalog"])
                .with_token_hash([0x10, 0x20]),
        );

        let mut next_run = descriptor.clone();
        next_run.run = Uuid::now_v7();
        assert!(descriptor.has_same_connection_contract(&next_run));

        let mut changed_endpoint = next_run.clone();
        changed_endpoint.endpoint = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:42001");
        assert!(!descriptor.has_same_connection_contract(&changed_endpoint));

        let mut changed_capability = next_run;
        changed_capability.capabilities[0]
            .permissions
            .push("applyEdit".to_string());
        assert!(!descriptor.has_same_connection_contract(&changed_capability));
    }

    #[test]
    fn service_health_round_trips_transport_state() {
        let health = ServiceHealth::ready(
            ServiceId::new("azoth", "asset-processor"),
            ServiceRole::AssetProcessor,
            Uuid::now_v7(),
            ProtocolVersion::CURRENT,
        )
        .with_state(ServiceHealthState::Busy)
        .with_uptime_ms(34_000)
        .with_active_operation("build")
        .with_message("processing assets")
        .with_last_event_seq(99);

        let mut message = message::Builder::new_default();
        let root = message.init_root::<azoth_core_capnp::service_health::Builder<'_>>();
        health.to_capnp(root).unwrap();

        let reader = message
            .get_root_as_reader::<azoth_core_capnp::service_health::Reader<'_>>()
            .unwrap();

        assert_eq!(ServiceHealth::from_capnp(reader).unwrap(), health);
    }

    #[test]
    fn capability_lifetime_treats_zero_as_non_expiring() {
        let service = ServiceId::new("azoth", "editor");
        let non_expiring = Capability::new(service.clone(), ServiceRole::Editor);
        assert!(non_expiring.is_non_expiring());
        assert!(!non_expiring.is_expired_at_unix_ms(u64::MAX));
        non_expiring.validate_lifetime_at_unix_ms(u64::MAX).unwrap();

        let expiring = Capability::new(service, ServiceRole::Editor).with_expires_unix_ms(1_000);
        assert!(!expiring.is_non_expiring());
        assert!(!expiring.is_expired_at_unix_ms(999));
        assert!(expiring.is_expired_at_unix_ms(1_000));
        assert!(matches!(
            expiring.validate_lifetime_at_unix_ms(1_000),
            Err(CapabilityLifetimeError::Expired { .. })
        ));
    }

    #[test]
    fn capability_grant_set_requires_brokered_token_match() {
        let session = Uuid::from_bytes([0x42; 16]);
        let grant = Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_session(session)
            .with_audience("project-host")
            .with_permissions(["schemaCatalog", "applyEdit"])
            .with_token_hash([0x10, 0x20]);
        let grant_set = CapabilityGrantSet::from_grants(vec![grant.clone()]);

        grant_set.validate(&grant, "schemaCatalog").unwrap();

        let mut wrong_token = grant.clone();
        wrong_token.token_hash = vec![0x99];
        assert!(matches!(
            grant_set.validate(&wrong_token, "schemaCatalog"),
            Err(CapabilityGrantError::NotGranted { .. })
        ));

        let mut missing_token = grant.clone();
        missing_token.token_hash.clear();
        assert!(matches!(
            grant_set.validate(&missing_token, "schemaCatalog"),
            Err(CapabilityGrantError::MissingBrokeredToken { .. })
        ));

        let expired = grant.clone().with_expires_unix_ms(1);
        let expired_grant_set = CapabilityGrantSet::from_grants(vec![expired.clone()]);
        assert!(matches!(
            expired_grant_set.validate(&expired, "schemaCatalog"),
            Err(CapabilityGrantError::InvalidLifetime { .. })
        ));

        let mut extra_permission = grant;
        extra_permission
            .permissions
            .push("document.write".to_string());
        assert!(matches!(
            grant_set.validate(&extra_permission, "schemaCatalog"),
            Err(CapabilityGrantError::NotGranted { .. })
        ));
    }

    #[test]
    fn capability_grant_set_command_match_includes_scope_in_grant_identity() {
        let session = Uuid::from_bytes([0x49; 16]);
        let grant = Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_session(session)
            .with_audience("project-host")
            .with_permissions(["schemaCatalog"])
            .with_token_hash([0x10, 0x20]);
        let grant_set = CapabilityGrantSet::from_grants(vec![grant.clone()]);

        let unscoped = Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_audience("project-host")
            .with_permissions(["schemaCatalog"])
            .with_token_hash([0x10, 0x20]);
        assert!(matches!(
            grant_set.validate(&unscoped, "schemaCatalog"),
            Err(CapabilityGrantError::NotGranted { .. })
        ));

        let nil_session = grant.with_session(Uuid::nil());
        assert!(matches!(
            grant_set.validate(&nil_session, "schemaCatalog"),
            Err(CapabilityGrantError::NilSession { .. })
        ));
    }

    #[test]
    fn capability_grant_set_validates_brokered_session_requirements() {
        let session = Uuid::from_bytes([0x43; 16]);
        let editor = Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_session(session)
            .with_audience("project-host")
            .with_permissions(["project.schema", "project.edit"])
            .with_token_hash([0x10, 0x20]);
        let supervisor = Capability::new(
            ServiceId::new("azoth", "session-supervisor"),
            ServiceRole::SessionSupervisor,
        )
        .with_session(session)
        .with_audience("project-host")
        .with_permissions(["project.document.write"])
        .with_token_hash([0x30, 0x40]);
        let grant_set = CapabilityGrantSet::from_grants(vec![editor, supervisor]);

        assert_eq!(grant_set.single_brokered_session().unwrap(), session);
        grant_set
            .validate_brokered_for_session(
                session,
                &[
                    CapabilityGrantRequirement::new(
                        "azoth",
                        "editor",
                        ServiceRole::Editor,
                        "project-host",
                        &["project.schema", "project.edit"],
                    ),
                    CapabilityGrantRequirement::new(
                        "azoth",
                        "session-supervisor",
                        ServiceRole::SessionSupervisor,
                        "project-host",
                        &["project.document.write"],
                    ),
                ],
            )
            .unwrap();
    }

    #[test]
    fn capability_grant_set_exact_validation_rejects_unexpected_or_duplicate_grants() {
        let session = Uuid::from_bytes([0x45; 16]);
        let requirement = CapabilityGrantRequirement::new(
            "azoth",
            "editor",
            ServiceRole::Editor,
            "project-host",
            &["project.schema"],
        );
        let required = Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_session(session)
            .with_audience("project-host")
            .with_permissions(["project.schema"])
            .with_token_hash([0x10, 0x20]);
        let unexpected = Capability::new(ServiceId::new("azoth", "worker"), ServiceRole::Worker)
            .with_session(session)
            .with_audience("project-host")
            .with_permissions(["project.schema"])
            .with_token_hash([0x30, 0x40]);

        let grant_set = CapabilityGrantSet::from_grants(vec![required.clone(), unexpected]);
        assert!(matches!(
            grant_set.validate_exact_brokered_for_session(session, &[requirement]),
            Err(CapabilityGrantSetValidationError::UnexpectedGrant(..))
        ));

        let duplicate = required.clone().with_token_hash([0x55, 0x66]);
        let grant_set = CapabilityGrantSet::from_grants(vec![required, duplicate]);
        assert!(matches!(
            grant_set.validate_exact_brokered_for_session(session, &[requirement]),
            Err(CapabilityGrantSetValidationError::DuplicateRequiredGrant(
                _,
                2
            ))
        ));
    }

    #[test]
    fn brokered_session_grant_validation_rejects_unscoped_or_missing_required_grants() {
        let session = Uuid::from_bytes([0x44; 16]);
        let grant = Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_audience("project-host")
            .with_permissions(["project.schema"])
            .with_token_hash([0x10, 0x20]);
        let grant_set = CapabilityGrantSet::from_grants(vec![grant]);

        assert!(matches!(
            grant_set.validate_brokered_for_session(session, &[]),
            Err(CapabilityGrantSetValidationError::MissingSession { .. })
        ));

        let wrong_permission =
            Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
                .with_session(session)
                .with_audience("project-host")
                .with_permissions(["project.schema"])
                .with_token_hash([0x10, 0x20]);
        let grant_set = CapabilityGrantSet::from_grants(vec![wrong_permission]);

        assert!(matches!(
            grant_set.validate_brokered_for_session(
                session,
                &[CapabilityGrantRequirement::new(
                    "azoth",
                    "editor",
                    ServiceRole::Editor,
                    "project-host",
                    &["project.edit"],
                )],
            ),
            Err(CapabilityGrantSetValidationError::MissingRequiredGrant(..))
        ));
    }

    #[test]
    fn brokered_session_grant_validation_rejects_nil_sessions() {
        let session = Uuid::from_bytes([0x46; 16]);
        let grant = Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_session(Uuid::nil())
            .with_audience("project-host")
            .with_permissions(["project.schema"])
            .with_token_hash([0x10, 0x20]);
        let grant_set = CapabilityGrantSet::from_grants(vec![grant]);

        assert!(matches!(
            grant_set.single_brokered_session(),
            Err(CapabilityGrantSetValidationError::NilSession)
        ));
        assert!(matches!(
            grant_set.validate_brokered_for_session(session, &[]),
            Err(CapabilityGrantSetValidationError::NilSession)
        ));

        let valid_grant = Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_session(session)
            .with_audience("project-host")
            .with_permissions(["project.schema"])
            .with_token_hash([0x10, 0x20]);
        let grant_set = CapabilityGrantSet::from_grants(vec![valid_grant]);

        assert!(matches!(
            grant_set.validate_brokered_for_session(Uuid::nil(), &[]),
            Err(CapabilityGrantSetValidationError::NilSession)
        ));
    }

    #[test]
    fn capability_grant_set_round_trips_as_capnp() {
        let grant = Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_session(Uuid::from_bytes([0x24; 16]))
            .with_audience("runtime-host")
            .with_permissions(["runtime.read"])
            .with_token_hash([0xaa, 0xbb]);
        let grant_set = CapabilityGrantSet::from_grants(vec![grant]);

        let bytes = encode_capability_grant_set(&grant_set).unwrap();
        let decoded = decode_capability_grant_set(&bytes).unwrap();

        assert_eq!(decoded, grant_set);
    }

    #[test]
    fn project_scoped_capability_grant_set_round_trips_as_capnp() {
        let grant = Capability::new(ServiceId::new("azoth", "asset-worker"), ServiceRole::Worker)
            .with_audience("asset-processor")
            .with_permissions(["asset.jobs"])
            .with_token_hash([0xaa, 0xbb]);
        let grant_set = CapabilityGrantSet::from_grants(vec![grant]);

        let bytes = encode_capability_grant_set(&grant_set).unwrap();
        let decoded = decode_capability_grant_set(&bytes).unwrap();

        assert_eq!(decoded, grant_set);
        decoded.validate_brokered_for_project(&[]).unwrap();
    }

    #[test]
    fn service_descriptor_selects_matching_brokered_capability_template() {
        let session = Uuid::from_bytes([0x5a; 16]);
        let descriptor = ServiceDescriptor::new(
            ServiceId::new("azoth", "project-host"),
            ServiceRole::ProjectHost,
            Endpoint::in_process("project-host:test"),
        )
        .with_capability(
            Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
                .with_session(session)
                .with_audience("project-host")
                .with_permissions(["schemaCatalog", "applyEdit"])
                .with_token_hash([1, 2, 3]),
        );

        let selected = descriptor
            .brokered_capability_template(
                ServiceRole::Editor,
                "project-host",
                &["schemaCatalog"],
                Some(session),
            )
            .unwrap();
        assert_eq!(selected.token_hash, vec![1, 2, 3]);
        assert_eq!(selected.session, Some(session));

        assert!(
            descriptor
                .brokered_capability_template(
                    ServiceRole::Editor,
                    "project-host",
                    &["document.write"],
                    Some(session),
                )
                .is_none()
        );
    }

    #[test]
    fn service_descriptor_api_does_not_expose_unbrokered_template_selector() {
        let source = include_str!("lib.rs");
        let forbidden_descriptor_selector = ["pub fn ", "capability_template("].concat();
        let forbidden_capability_matcher = ["pub fn ", "matches_template_request("].concat();

        assert!(
            !source.contains(&forbidden_descriptor_selector),
            "ServiceDescriptor must expose brokered_capability_template as the public selection path"
        );
        assert!(
            !source.contains(&forbidden_capability_matcher),
            "Capability must expose matches_brokered_template_request as the public matching path"
        );
    }

    #[test]
    fn service_descriptor_brokered_template_skips_unbrokered_matches() {
        let session = Uuid::from_bytes([0x5b; 16]);
        let tokenless = Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_session(session)
            .with_audience("project-host")
            .with_permissions(["schemaCatalog"]);
        let expired = tokenless
            .clone()
            .with_token_hash([0xee, 0xee])
            .with_expires_unix_ms(1);
        let brokered = tokenless.clone().with_token_hash([0xbb, 0xbb]);
        let descriptor = ServiceDescriptor::new(
            ServiceId::new("azoth", "project-host"),
            ServiceRole::ProjectHost,
            Endpoint::in_process("project-host:test"),
        )
        .with_capability(tokenless)
        .with_capability(expired)
        .with_capability(brokered);

        let selected = descriptor
            .brokered_capability_template(
                ServiceRole::Editor,
                "project-host",
                &["schemaCatalog"],
                Some(session),
            )
            .unwrap();

        assert_eq!(selected.token_hash, vec![0xbb, 0xbb]);
        assert!(
            descriptor
                .brokered_capability_template(
                    ServiceRole::Editor,
                    "project-host",
                    &["document.write"],
                    Some(session),
                )
                .is_none()
        );
    }

    #[test]
    fn service_descriptor_validation_rejects_mixed_unbrokered_templates() {
        let tokenless = Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_audience("project-host")
            .with_permissions(["schemaCatalog"]);
        let brokered = tokenless.clone().with_token_hash([0xbb, 0xbb]);
        let descriptor = ServiceDescriptor::new(
            ServiceId::new("azoth", "project-host"),
            ServiceRole::ProjectHost,
            Endpoint::in_process("project-host:test"),
        )
        .with_capability(brokered)
        .with_capability(tokenless);

        let error = descriptor
            .validate_brokered_capability_templates()
            .expect_err("mixed brokered and tokenless templates must be rejected");

        assert_eq!(
            error,
            ServiceDescriptorCapabilityTemplateError::InvalidCapability {
                id: ServiceId::new("azoth", "project-host"),
                role: ServiceRole::ProjectHost,
                index: 1,
                reason: CapabilityTemplateError::MissingBrokeredToken,
            }
        );
    }

    #[test]
    fn service_descriptor_validation_rejects_nil_session_templates() {
        let nil_scoped = Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_session(Uuid::nil())
            .with_audience("project-host")
            .with_permissions(["schemaCatalog"])
            .with_token_hash([0xbb, 0xbb]);
        let descriptor = ServiceDescriptor::new(
            ServiceId::new("azoth", "project-host"),
            ServiceRole::ProjectHost,
            Endpoint::in_process("project-host:test"),
        )
        .with_capability(nil_scoped);

        let error = descriptor
            .validate_brokered_capability_templates()
            .expect_err("nil-session brokered templates must be rejected");

        assert_eq!(
            error,
            ServiceDescriptorCapabilityTemplateError::InvalidCapability {
                id: ServiceId::new("azoth", "project-host"),
                role: ServiceRole::ProjectHost,
                index: 0,
                reason: CapabilityTemplateError::NilSession,
            }
        );
        assert!(
            descriptor
                .brokered_capability_template(
                    ServiceRole::Editor,
                    "project-host",
                    &["schemaCatalog"],
                    None,
                )
                .is_none()
        );
    }

    #[test]
    fn service_descriptor_encode_rejects_missing_capability_templates() {
        let descriptor = ServiceDescriptor::new(
            ServiceId::new("azoth", "project-host"),
            ServiceRole::ProjectHost,
            Endpoint::in_process("project-host:test"),
        )
        .with_run(Uuid::now_v7());

        let mut message = message::Builder::new_default();
        let error = descriptor
            .to_capnp(message.init_root::<azoth_core_capnp::service_descriptor::Builder<'_>>())
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("capability templates must be non-empty")
        );
    }

    #[test]
    fn service_descriptor_encode_rejects_tokenless_capability() {
        let descriptor = ServiceDescriptor::new(
            ServiceId::new("azoth", "project-host"),
            ServiceRole::ProjectHost,
            Endpoint::in_process("project-host:test"),
        )
        .with_run(Uuid::now_v7())
        .with_capability(
            Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
                .with_audience("project-host")
                .with_permissions(["schema.read"]),
        );

        let mut message = message::Builder::new_default();
        let error = descriptor
            .to_capnp(message.init_root::<azoth_core_capnp::service_descriptor::Builder<'_>>())
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("capability template 0 is not brokered")
        );
        assert!(error.to_string().contains("brokered token hash"));
    }

    #[test]
    fn service_descriptor_encode_rejects_audienceless_descriptor_capability() {
        let descriptor = ServiceDescriptor::new(
            ServiceId::new("azoth", "project-host"),
            ServiceRole::ProjectHost,
            Endpoint::in_process("project-host:test"),
        )
        .with_run(Uuid::now_v7())
        .with_capability(
            Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
                .with_permissions(["schema.read"])
                .with_token_hash([1, 2, 3]),
        );

        let mut message = message::Builder::new_default();
        let error = descriptor
            .to_capnp(message.init_root::<azoth_core_capnp::service_descriptor::Builder<'_>>())
            .unwrap_err();

        assert!(error.to_string().contains("audience must be non-empty"));
    }

    #[test]
    fn service_descriptor_encode_rejects_permissionless_descriptor_capability() {
        let descriptor = ServiceDescriptor::new(
            ServiceId::new("azoth", "project-host"),
            ServiceRole::ProjectHost,
            Endpoint::in_process("project-host:test"),
        )
        .with_run(Uuid::now_v7())
        .with_capability(
            Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
                .with_audience("project-host")
                .with_token_hash([1, 2, 3]),
        );

        let mut message = message::Builder::new_default();
        let error = descriptor
            .to_capnp(message.init_root::<azoth_core_capnp::service_descriptor::Builder<'_>>())
            .unwrap_err();

        assert!(error.to_string().contains("permissions must be non-empty"));
    }

    #[test]
    fn service_descriptor_encode_rejects_expired_descriptor_capability() {
        let descriptor = ServiceDescriptor::new(
            ServiceId::new("azoth", "project-host"),
            ServiceRole::ProjectHost,
            Endpoint::in_process("project-host:test"),
        )
        .with_run(Uuid::now_v7())
        .with_capability(
            Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
                .with_audience("project-host")
                .with_permissions(["schema.read"])
                .with_token_hash([1, 2, 3])
                .with_expires_unix_ms(1),
        );

        let mut message = message::Builder::new_default();
        let error = descriptor
            .to_capnp(message.init_root::<azoth_core_capnp::service_descriptor::Builder<'_>>())
            .unwrap_err();

        assert!(error.to_string().contains("capability expired"));
    }

    #[test]
    fn service_descriptor_encode_rejects_mixed_brokered_and_unbrokered_capabilities() {
        let brokered = Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_audience("project-host")
            .with_permissions(["schema.read"])
            .with_token_hash([1, 2, 3]);
        let descriptor = ServiceDescriptor::new(
            ServiceId::new("azoth", "project-host"),
            ServiceRole::ProjectHost,
            Endpoint::in_process("project-host:test"),
        )
        .with_run(Uuid::now_v7())
        .with_capability(brokered)
        .with_capability(
            Capability::new(ServiceId::new("azoth", "daemon"), ServiceRole::Daemon)
                .with_audience("project-host")
                .with_permissions(["schema.read"]),
        );

        let mut message = message::Builder::new_default();
        let error = descriptor
            .to_capnp(message.init_root::<azoth_core_capnp::service_descriptor::Builder<'_>>())
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("capability template 1 is not brokered")
        );
    }
}
