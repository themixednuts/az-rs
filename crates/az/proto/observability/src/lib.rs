//! Cap'n Proto protocol types for Azoth observability services.
//!
//! Runtime code should instrument with `tracing`. This crate defines the
//! process-safe DTOs that can carry those events through local log readers,
//! editor panels, diagnostic bundles, and later OpenTelemetry exporters.

// Machine-generated Cap'n Proto output: written into OUT_DIR by build.rs and
// regenerated on every build, so it is not in git and has no per-site fix. The
// macro completes upstream's own `#![allow(clippy::all)]` for the pedantic and
// nursery groups this workspace denies.
az_proto_core::generated_schema!(pub mod observability_capnp, "azoth/observability_capnp.rs");

use std::collections::BTreeSet;
use std::fmt;
use std::io::Cursor;

use az_proto_core::{
    self as core, Capability, ServiceId, ServiceRole, StagingFileSideChannelError,
};
use capnp::{Error, message, serialize_packed};
use uuid::Uuid;

pub use az_proto_core as core_proto;

pub const OBSERVABILITY_NAMESPACE: &str = "azoth";
pub const OBSERVABILITY_SERVICE_NAME: &str = "observability";
pub const OBSERVABILITY_AUDIENCE: &str = "observability";
pub const OBSERVABILITY_READ_PERMISSION: &str = "observability.read";
pub const OBSERVABILITY_WRITE_PERMISSION: &str = "observability.write";
pub const OBSERVABILITY_BUNDLE_PERMISSION: &str = "observability.bundle";
pub const DIAGNOSTIC_BUNDLE_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const STRUCTURED_LOG_EXTENSION: &str = "capnp.log";
pub const OTEL_TRACE_ID_BYTES: usize = 16;
pub const OTEL_SPAN_ID_BYTES: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    #[must_use]
    pub const fn to_capnp(self) -> observability_capnp::LogLevel {
        match self {
            Self::Trace => observability_capnp::LogLevel::Trace,
            Self::Debug => observability_capnp::LogLevel::Debug,
            Self::Info => observability_capnp::LogLevel::Info,
            Self::Warn => observability_capnp::LogLevel::Warn,
            Self::Error => observability_capnp::LogLevel::Error,
        }
    }

    #[must_use]
    pub const fn from_capnp(level: observability_capnp::LogLevel) -> Self {
        match level {
            observability_capnp::LogLevel::Trace => Self::Trace,
            observability_capnp::LogLevel::Debug => Self::Debug,
            observability_capnp::LogLevel::Info => Self::Info,
            observability_capnp::LogLevel::Warn => Self::Warn,
            observability_capnp::LogLevel::Error => Self::Error,
        }
    }

    #[must_use]
    pub const fn from_tracing(level: tracing::Level) -> Self {
        match level {
            tracing::Level::TRACE => Self::Trace,
            tracing::Level::DEBUG => Self::Debug,
            tracing::Level::INFO => Self::Info,
            tracing::Level::WARN => Self::Warn,
            tracing::Level::ERROR => Self::Error,
        }
    }

    #[must_use]
    pub const fn to_tracing(self) -> tracing::Level {
        match self {
            Self::Trace => tracing::Level::TRACE,
            Self::Debug => tracing::Level::DEBUG,
            Self::Info => tracing::Level::INFO,
            Self::Warn => tracing::Level::WARN,
            Self::Error => tracing::Level::ERROR,
        }
    }
}

/// Engine log channels — the deliberate, subsystem-level taxonomy for "what is
/// this log about".
///
/// Each channel maps to a stable tracing `target` (`az.<name>`) so it composes
/// with `EnvFilter` verbosity (`RUST_LOG=az.assets=debug`) and is carried
/// verbatim in [`ObservedLogRecord::target`] for UI filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LogChannel {
    Core,
    Editor,
    Project,
    Session,
    Assets,
    Runtime,
    Gpu,
    Render,
    Scripting,
    Physics,
    Audio,
    Net,
    Gameplay,
    Anim,
    Ui,
}

impl LogChannel {
    /// Every channel, for UI enumeration and reverse lookup.
    pub const ALL: &'static [Self] = &[
        Self::Core,
        Self::Editor,
        Self::Project,
        Self::Session,
        Self::Assets,
        Self::Runtime,
        Self::Gpu,
        Self::Render,
        Self::Scripting,
        Self::Physics,
        Self::Audio,
        Self::Net,
        Self::Gameplay,
        Self::Anim,
        Self::Ui,
    ];

    /// The stable tracing `target` for this channel (e.g. `"az.assets"`).
    #[must_use]
    pub const fn as_target(self) -> &'static str {
        match self {
            Self::Core => "az.core",
            Self::Editor => "az.editor",
            Self::Project => "az.project",
            Self::Session => "az.session",
            Self::Assets => "az.assets",
            Self::Runtime => "az.runtime",
            Self::Gpu => "az.gpu",
            Self::Render => "az.render",
            Self::Scripting => "az.scripting",
            Self::Physics => "az.physics",
            Self::Audio => "az.audio",
            Self::Net => "az.net",
            Self::Gameplay => "az.gameplay",
            Self::Anim => "az.anim",
            Self::Ui => "az.ui",
        }
    }

    /// A short human label for the channel (e.g. `"Assets"`, `"GPU"`).
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Core => "Core",
            Self::Editor => "Editor",
            Self::Project => "Project",
            Self::Session => "Session",
            Self::Assets => "Assets",
            Self::Runtime => "Runtime",
            Self::Gpu => "GPU",
            Self::Render => "Render",
            Self::Scripting => "Scripting",
            Self::Physics => "Physics",
            Self::Audio => "Audio",
            Self::Net => "Net",
            Self::Gameplay => "Gameplay",
            Self::Anim => "Anim",
            Self::Ui => "UI",
        }
    }

    /// Resolve a channel from a tracing target string, if it is a known channel.
    #[must_use]
    pub fn from_target(target: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|ch| ch.as_target() == target)
    }
}

/// A verbosity *threshold* (a level filter).
///
/// Unlike [`LogLevel`] (which labels an emitted record) this includes `Off`, so
/// it can express "silence this channel" for runtime dialing and the
/// cross-process control protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogVerbosity {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogVerbosity {
    pub const ALL: &'static [Self] = &[
        Self::Off,
        Self::Error,
        Self::Warn,
        Self::Info,
        Self::Debug,
        Self::Trace,
    ];

    /// The `EnvFilter`-compatible level string (`"off"`, `"trace"`, …).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }

    /// Parse a verbosity from a CLI/console token (case-insensitive).
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        match token.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "silent" => Some(Self::Off),
            "error" | "err" => Some(Self::Error),
            "warn" | "warning" => Some(Self::Warn),
            "info" => Some(Self::Info),
            "debug" => Some(Self::Debug),
            "trace" | "verbose" => Some(Self::Trace),
            _ => None,
        }
    }

    #[must_use]
    pub const fn to_capnp(self) -> observability_capnp::LogVerbosity {
        match self {
            Self::Off => observability_capnp::LogVerbosity::Off,
            Self::Error => observability_capnp::LogVerbosity::Error,
            Self::Warn => observability_capnp::LogVerbosity::Warn,
            Self::Info => observability_capnp::LogVerbosity::Info,
            Self::Debug => observability_capnp::LogVerbosity::Debug,
            Self::Trace => observability_capnp::LogVerbosity::Trace,
        }
    }

    #[must_use]
    pub const fn from_capnp(value: observability_capnp::LogVerbosity) -> Self {
        match value {
            observability_capnp::LogVerbosity::Off => Self::Off,
            observability_capnp::LogVerbosity::Error => Self::Error,
            observability_capnp::LogVerbosity::Warn => Self::Warn,
            observability_capnp::LogVerbosity::Info => Self::Info,
            observability_capnp::LogVerbosity::Debug => Self::Debug,
            observability_capnp::LogVerbosity::Trace => Self::Trace,
        }
    }
}

/// Capability permission required to dial a process's log verbosity over the
/// `ObservabilityControl` interface.
pub const OBSERVABILITY_CONTROL_PERMISSION: &str = "observability.control";

/// Hidden re-export so the `az_*!` macros can reference `tracing` without the
/// caller crate importing it (it still needs `tracing` as a dependency).
#[doc(hidden)]
pub use ::tracing;

/// Emit a structured log event on an engine [`LogChannel`] at an explicit level.
///
/// `az_log!(LogChannel::Assets, $crate::tracing::Level::INFO, count = n, "indexed")`.
/// Prefer the per-level shortcuts (`az_info!`, `az_warn!`, …).
///
/// The channel must be a **constant** `LogChannel` expression (a variant like
/// `LogChannel::Assets`), because `tracing` stores the target in `'static`
/// callsite metadata — `as_target()` is a `const fn`, so a literal variant is
/// fine but a runtime `LogChannel` variable will not compile. That is by design:
/// the subsystem is always known at the log site. To act on a *runtime* channel
/// (e.g. a console `log <channel> <level>` command), use the channel functions
/// (`LogChannel::from_target`, `set_channel_level`), not these macros.
#[macro_export]
macro_rules! az_log {
    ($channel:expr, $level:expr, $($args:tt)*) => {
        $crate::tracing::event!(
            target: $crate::LogChannel::as_target($channel),
            $level,
            $($args)*
        )
    };
}

/// `az_trace!(LogChannel::Assets, field = x, "message")`
#[macro_export]
macro_rules! az_trace {
    ($channel:expr, $($args:tt)*) => {
        $crate::az_log!($channel, $crate::tracing::Level::TRACE, $($args)*)
    };
}

/// `az_debug!(LogChannel::Assets, …)`
#[macro_export]
macro_rules! az_debug {
    ($channel:expr, $($args:tt)*) => {
        $crate::az_log!($channel, $crate::tracing::Level::DEBUG, $($args)*)
    };
}

/// `az_info!(LogChannel::Assets, …)`
#[macro_export]
macro_rules! az_info {
    ($channel:expr, $($args:tt)*) => {
        $crate::az_log!($channel, $crate::tracing::Level::INFO, $($args)*)
    };
}

/// `az_warn!(LogChannel::Assets, …)`
#[macro_export]
macro_rules! az_warn {
    ($channel:expr, $($args:tt)*) => {
        $crate::az_log!($channel, $crate::tracing::Level::WARN, $($args)*)
    };
}

/// `az_error!(LogChannel::Assets, …)`
#[macro_export]
macro_rules! az_error {
    ($channel:expr, $($args:tt)*) => {
        $crate::az_log!($channel, $crate::tracing::Level::ERROR, $($args)*)
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedField {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedSpan {
    pub name: String,
    pub target: String,
    pub fields: Vec<ObservedField>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TraceContext {
    pub trace_id: Vec<u8>,
    pub span_id: Vec<u8>,
    pub parent_span_id: Vec<u8>,
    pub trace_flags: u32,
    pub trace_state: String,
}

impl TraceContext {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.trace_id.is_empty() && self.span_id.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedLogRecord {
    pub sequence: u64,
    pub timestamp_unix_ns: u64,
    pub service: ServiceId,
    pub role: ServiceRole,
    /// `UUIDv7` launch label for attribution only.
    pub run: Uuid,
    pub session_slug: String,
    pub target: String,
    pub level: LogLevel,
    pub message: String,
    pub fields: Vec<ObservedField>,
    pub trace: TraceContext,
    pub spans: Vec<ObservedSpan>,
    pub file: String,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendLogBatchRequest {
    pub capability: Capability,
    pub records: Vec<ObservedLogRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendLogBatchResult {
    pub accepted: bool,
    pub next_sequence: u64,
    pub rejected_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogQueryRequest {
    pub capability: Capability,
    pub session_slug: String,
    pub service: Option<ServiceId>,
    pub run: Option<Uuid>,
    pub min_level: LogLevel,
    pub after_sequence: u64,
    pub page_size: u32,
    pub include_fields: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogQueryResult {
    pub records: Vec<ObservedLogRecord>,
    pub next_sequence: u64,
    pub truncated: bool,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticBundleFile {
    pub path: String,
    pub category: String,
    pub byte_length: u64,
    pub content_hash: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticBundleManifest {
    pub schema_version: u32,
    pub session_slug: String,
    pub created_unix_ms: u64,
    pub reason: String,
    pub files: Vec<DiagnosticBundleFile>,
}

impl DiagnosticBundleManifest {
    #[must_use]
    pub fn new(
        session_slug: impl Into<String>,
        created_unix_ms: u64,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: DIAGNOSTIC_BUNDLE_MANIFEST_SCHEMA_VERSION,
            session_slug: session_slug.into(),
            created_unix_ms,
            reason: reason.into(),
            files: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticBundleRequest {
    pub capability: Capability,
    pub session_slug: String,
    pub reason: String,
    pub include_logs: bool,
    pub include_crash_dumps: bool,
    pub max_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticBundleResult {
    pub accepted: bool,
    pub manifest: core::SideChannelHandle,
    pub diagnostic: String,
}

/// Request to set (or clear) the verbosity override for one channel `target`
/// (`ObservabilityControl.setLogChannelLevel`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetLogChannelLevelRequest {
    pub capability: Capability,
    pub target: String,
    pub verbosity: LogVerbosity,
    pub clear: bool,
}

/// One channel-level override pair (`target` → threshold), as carried in
/// [`ListChannelLevelsResult::overrides`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelLevel {
    pub target: String,
    pub verbosity: LogVerbosity,
}

/// Result of [`SetLogChannelLevelRequest`]: the directive string now in effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetLogChannelLevelResult {
    pub accepted: bool,
    pub effective_directives: String,
    pub diagnostic: String,
}

/// Result of `ObservabilityControl.listChannelLevels`: the effective directive
/// string plus the explicit per-channel overrides on top of the base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListChannelLevelsResult {
    pub effective_directives: String,
    pub overrides: Vec<ChannelLevel>,
}

#[derive(Debug)]
pub enum DiagnosticBundleManifestSideChannelError {
    InvalidHandle(StagingFileSideChannelError),
    Decode(Error),
}

impl fmt::Display for DiagnosticBundleManifestSideChannelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidHandle(error) => {
                write!(
                    f,
                    "invalid diagnostic bundle manifest side-channel handle: {error}"
                )
            }
            Self::Decode(error) => {
                write!(
                    f,
                    "failed to decode diagnostic bundle manifest side-channel: {error}"
                )
            }
        }
    }
}

impl std::error::Error for DiagnosticBundleManifestSideChannelError {}

impl From<StagingFileSideChannelError> for DiagnosticBundleManifestSideChannelError {
    fn from(error: StagingFileSideChannelError) -> Self {
        Self::InvalidHandle(error)
    }
}

impl From<Error> for DiagnosticBundleManifestSideChannelError {
    fn from(error: Error) -> Self {
        Self::Decode(error)
    }
}

fn write_observed_field(
    field: &ObservedField,
    mut builder: observability_capnp::observed_field::Builder<'_>,
) -> Result<(), Error> {
    validate_observed_field_for_transport(field)?;
    builder.set_name(&field.name);
    builder.set_value(&field.value);
    Ok(())
}

fn read_observed_field(
    reader: observability_capnp::observed_field::Reader<'_>,
) -> Result<ObservedField, Error> {
    let field = ObservedField {
        name: reader.get_name()?.to_string()?,
        value: reader.get_value()?.to_string()?,
    };
    validate_observed_field_for_transport(&field)?;
    Ok(field)
}

fn write_observed_span(
    span: &ObservedSpan,
    mut builder: observability_capnp::observed_span::Builder<'_>,
) -> Result<(), Error> {
    validate_observed_span_for_transport(span)?;
    builder.set_name(&span.name);
    builder.set_target(&span.target);
    let mut fields = builder
        .reborrow()
        .init_fields(capnp_list_index(span.fields.len())?);
    for (index, field) in span.fields.iter().enumerate() {
        (field).to_capnp(fields.reborrow().get(capnp_list_index(index)?))?;
    }
    Ok(())
}

fn read_observed_span(
    reader: observability_capnp::observed_span::Reader<'_>,
) -> Result<ObservedSpan, Error> {
    let fields = reader
        .get_fields()?
        .iter()
        .map(read_observed_field)
        .collect::<Result<Vec<_>, Error>>()?;
    let span = ObservedSpan {
        name: reader.get_name()?.to_string()?,
        target: reader.get_target()?.to_string()?,
        fields,
    };
    validate_observed_span_for_transport(&span)?;
    Ok(span)
}

fn validate_observed_span_for_transport(span: &ObservedSpan) -> Result<(), Error> {
    if span.name.trim().is_empty() {
        return Err(invalid_observability_protocol_value(
            "observed span",
            "name must be non-empty",
        ));
    }
    if span.target.trim().is_empty() {
        return Err(invalid_observability_protocol_value(
            "observed span",
            "target must be non-empty",
        ));
    }
    for field in &span.fields {
        validate_observed_field_for_transport(field)?;
    }
    Ok(())
}

fn validate_observed_field_for_transport(field: &ObservedField) -> Result<(), Error> {
    if field.name.trim().is_empty() {
        return Err(invalid_observability_protocol_value(
            "observed field",
            "name must be non-empty",
        ));
    }
    Ok(())
}

fn write_trace_context(
    trace: &TraceContext,
    mut builder: observability_capnp::trace_context::Builder<'_>,
) -> Result<(), Error> {
    validate_trace_context(trace)?;
    builder.set_trace_id(&trace.trace_id);
    builder.set_span_id(&trace.span_id);
    builder.set_parent_span_id(&trace.parent_span_id);
    builder.set_trace_flags(trace.trace_flags);
    builder.set_trace_state(&trace.trace_state);
    Ok(())
}

fn read_trace_context(
    reader: observability_capnp::trace_context::Reader<'_>,
) -> Result<TraceContext, Error> {
    let trace = TraceContext {
        trace_id: reader.get_trace_id()?.to_vec(),
        span_id: reader.get_span_id()?.to_vec(),
        parent_span_id: reader.get_parent_span_id()?.to_vec(),
        trace_flags: reader.get_trace_flags(),
        trace_state: reader.get_trace_state()?.to_string()?,
    };
    validate_trace_context(&trace)?;
    Ok(trace)
}

fn validate_trace_context(trace: &TraceContext) -> Result<(), Error> {
    if trace.is_empty() {
        if !trace.parent_span_id.is_empty() {
            return Err(Error::failed(
                "trace context parent span id requires a non-empty trace id and span id"
                    .to_string(),
            ));
        }
        return Ok(());
    }

    if trace.trace_id.len() != OTEL_TRACE_ID_BYTES {
        return Err(Error::failed(format!(
            "trace context trace id must be {OTEL_TRACE_ID_BYTES} bytes, got {}",
            trace.trace_id.len()
        )));
    }
    if trace.span_id.len() != OTEL_SPAN_ID_BYTES {
        return Err(Error::failed(format!(
            "trace context span id must be {OTEL_SPAN_ID_BYTES} bytes, got {}",
            trace.span_id.len()
        )));
    }
    if !trace.parent_span_id.is_empty() && trace.parent_span_id.len() != OTEL_SPAN_ID_BYTES {
        return Err(Error::failed(format!(
            "trace context parent span id must be empty or {OTEL_SPAN_ID_BYTES} bytes, got {}",
            trace.parent_span_id.len()
        )));
    }
    Ok(())
}

fn write_observed_log_record(
    record: &ObservedLogRecord,
    mut builder: observability_capnp::observed_log_record::Builder<'_>,
) -> Result<(), Error> {
    validate_observed_log_record_for_transport(record)?;
    builder.set_sequence(record.sequence);
    builder.set_timestamp_unix_ns(record.timestamp_unix_ns);
    record.service.to_capnp(builder.reborrow().init_service());
    builder.set_role(record.role.to_capnp());
    builder.set_run(record.run.as_bytes());
    builder.set_session_slug(&record.session_slug);
    builder.set_target(&record.target);
    builder.set_level(record.level.to_capnp());
    builder.set_message(&record.message);
    let mut fields = builder
        .reborrow()
        .init_fields(capnp_list_index(record.fields.len())?);
    for (index, field) in record.fields.iter().enumerate() {
        (field).to_capnp(fields.reborrow().get(capnp_list_index(index)?))?;
    }
    (record.trace).to_capnp(builder.reborrow().init_trace())?;
    let mut spans = builder
        .reborrow()
        .init_spans(capnp_list_index(record.spans.len())?);
    for (index, span) in record.spans.iter().enumerate() {
        (span).to_capnp(spans.reborrow().get(capnp_list_index(index)?))?;
    }
    builder.set_file(&record.file);
    builder.set_line(record.line);
    Ok(())
}

fn read_observed_log_record(
    reader: observability_capnp::observed_log_record::Reader<'_>,
) -> Result<ObservedLogRecord, Error> {
    let role = reader
        .get_role()
        .map(ServiceRole::from_capnp)
        .map_err(|error| Error::failed(format!("unknown service role: {error:?}")))?;
    let level = reader
        .get_level()
        .map(LogLevel::from_capnp)
        .map_err(|error| Error::failed(format!("unknown log level: {error:?}")))?;
    let fields = reader
        .get_fields()?
        .iter()
        .map(read_observed_field)
        .collect::<Result<Vec<_>, Error>>()?;
    let spans = reader
        .get_spans()?
        .iter()
        .map(read_observed_span)
        .collect::<Result<Vec<_>, Error>>()?;

    let record = ObservedLogRecord {
        sequence: reader.get_sequence(),
        timestamp_unix_ns: reader.get_timestamp_unix_ns(),
        service: ServiceId::from_capnp(reader.get_service()?)?,
        role,
        run: Uuid::from_slice(reader.get_run()?)
            .map_err(|error| Error::failed(format!("invalid observed run UUID: {error}")))?,
        session_slug: reader.get_session_slug()?.to_string()?,
        target: reader.get_target()?.to_string()?,
        level,
        message: reader.get_message()?.to_string()?,
        fields,
        trace: TraceContext::from_capnp(reader.get_trace()?)?,
        spans,
        file: reader.get_file()?.to_string()?,
        line: reader.get_line(),
    };
    validate_observed_log_record_for_transport(&record)?;
    Ok(record)
}

fn validate_observed_log_record_for_transport(record: &ObservedLogRecord) -> Result<(), Error> {
    if record.sequence == 0 {
        return Err(invalid_observability_protocol_value(
            "observed log record",
            "sequence must be positive",
        ));
    }
    if record.timestamp_unix_ns == 0 {
        return Err(invalid_observability_protocol_value(
            "observed log record",
            "timestamp must be positive",
        ));
    }
    if record.service.namespace.trim().is_empty() || record.service.name.trim().is_empty() {
        return Err(invalid_observability_protocol_value(
            "observed log record",
            "service id must be non-empty",
        ));
    }
    if record.role == ServiceRole::Unknown {
        return Err(invalid_observability_protocol_value(
            "observed log record",
            "service role cannot be Unknown",
        ));
    }
    if record.run == Uuid::nil() {
        return Err(invalid_observability_protocol_value(
            "observed log record",
            "run must be a non-nil UUID",
        ));
    }
    if record.session_slug.trim().is_empty() {
        return Err(invalid_observability_protocol_value(
            "observed log record",
            "session slug must be non-empty",
        ));
    }
    if record.target.trim().is_empty() {
        return Err(invalid_observability_protocol_value(
            "observed log record",
            "target must be non-empty",
        ));
    }
    for field in &record.fields {
        validate_observed_field_for_transport(field)?;
    }
    for span in &record.spans {
        validate_observed_span_for_transport(span)?;
    }
    validate_trace_context(&record.trace)?;
    Ok(())
}

/// Serializes an observed log record for a side-channel blob payload.
///
/// # Errors
///
/// Returns an error if the record fails its transport validation, or if the
/// message runs out of space while writing the record.
pub fn encode_observed_log_record(record: &ObservedLogRecord) -> Result<Vec<u8>, Error> {
    let mut message = message::Builder::new_default();
    (record)
        .to_capnp(message.init_root::<observability_capnp::observed_log_record::Builder<'_>>())?;

    let mut bytes = Vec::new();
    serialize_packed::write_message(&mut bytes, &message)?;
    Ok(bytes)
}

/// Deserializes an observed log record from a side-channel blob payload.
///
/// # Errors
///
/// Returns an error if `bytes` is not a readable packed Cap'n Proto message, or
/// if a field of the record is absent from the message or is not valid UTF-8.
pub fn decode_observed_log_record(bytes: &[u8]) -> Result<ObservedLogRecord, Error> {
    let reader =
        serialize_packed::read_message(&mut Cursor::new(bytes), message::ReaderOptions::new())?;
    ObservedLogRecord::from_capnp(
        reader.get_root::<observability_capnp::observed_log_record::Reader<'_>>()?,
    )
}

fn write_append_log_batch_request(
    request: &AppendLogBatchRequest,
    mut builder: observability_capnp::append_log_batch_request::Builder<'_>,
) -> Result<(), Error> {
    validate_append_log_batch_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    let mut records = builder
        .reborrow()
        .init_records(capnp_list_index(request.records.len())?);
    for (index, record) in request.records.iter().enumerate() {
        (record).to_capnp(records.reborrow().get(capnp_list_index(index)?))?;
    }
    Ok(())
}

fn read_append_log_batch_request(
    reader: observability_capnp::append_log_batch_request::Reader<'_>,
) -> Result<AppendLogBatchRequest, Error> {
    let records = reader
        .get_records()?
        .iter()
        .map(read_observed_log_record)
        .collect::<Result<Vec<_>, Error>>()?;

    let request = AppendLogBatchRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        records,
    };
    validate_append_log_batch_request_for_transport(&request)?;
    Ok(request)
}

fn write_append_log_batch_result(
    result: &AppendLogBatchResult,
    mut builder: observability_capnp::append_log_batch_result::Builder<'_>,
) -> Result<(), Error> {
    validate_append_log_batch_result_for_transport(result)?;
    builder.set_accepted(result.accepted);
    builder.set_next_sequence(result.next_sequence);
    builder.set_rejected_reason(&result.rejected_reason);
    Ok(())
}

fn read_append_log_batch_result(
    reader: observability_capnp::append_log_batch_result::Reader<'_>,
) -> Result<AppendLogBatchResult, Error> {
    let result = AppendLogBatchResult {
        accepted: reader.get_accepted(),
        next_sequence: reader.get_next_sequence(),
        rejected_reason: reader.get_rejected_reason()?.to_string()?,
    };
    validate_append_log_batch_result_for_transport(&result)?;
    Ok(result)
}

fn validate_append_log_batch_result_for_transport(
    result: &AppendLogBatchResult,
) -> Result<(), Error> {
    if result.accepted && result.next_sequence == 0 {
        return Err(invalid_observability_protocol_value(
            "append log batch result",
            "accepted result must carry a positive next sequence",
        ));
    }
    if result.accepted && !result.rejected_reason.trim().is_empty() {
        return Err(invalid_observability_protocol_value(
            "append log batch result",
            "accepted result cannot carry a rejected reason",
        ));
    }
    if !result.accepted && result.rejected_reason.trim().is_empty() {
        return Err(invalid_observability_protocol_value(
            "append log batch result",
            "rejected result must carry a reason",
        ));
    }
    Ok(())
}

fn validate_append_log_batch_request_for_transport(
    request: &AppendLogBatchRequest,
) -> Result<(), Error> {
    validate_observability_capability_request_for_transport(
        "append log batch request",
        &request.capability,
        OBSERVABILITY_WRITE_PERMISSION,
    )?;
    if request.records.is_empty() {
        return Err(invalid_observability_protocol_value(
            "append log batch request",
            "records cannot be empty",
        ));
    }

    let mut last_sequence = None;
    let mut session_slug = None;
    for record in &request.records {
        validate_observed_log_record_for_transport(record)?;
        if let Some(last) = last_sequence
            && record.sequence <= last
        {
            return Err(invalid_observability_protocol_value(
                "append log batch request",
                "records must be ordered by strictly increasing sequence",
            ));
        }
        if let Some(expected) = session_slug
            && record.session_slug != expected
        {
            return Err(invalid_observability_protocol_value(
                "append log batch request",
                "records cannot mix session slugs",
            ));
        }
        last_sequence = Some(record.sequence);
        session_slug = Some(record.session_slug.as_str());
    }
    Ok(())
}

fn write_log_query_request(
    request: &LogQueryRequest,
    mut builder: observability_capnp::log_query_request::Builder<'_>,
) -> Result<(), Error> {
    validate_log_query_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_session_slug(&request.session_slug);
    if let Some(service) = &request.service {
        service.to_capnp(builder.reborrow().init_service());
    }
    match request.run {
        Some(run) => builder.reborrow().init_run().set_value(run.as_bytes()),
        None => builder.reborrow().init_run().set_none(()),
    }
    builder.set_min_level(request.min_level.to_capnp());
    builder.set_after_sequence(request.after_sequence);
    builder.set_page_size(request.page_size);
    builder.set_include_fields(request.include_fields);
    Ok(())
}

fn read_log_query_request(
    reader: observability_capnp::log_query_request::Reader<'_>,
) -> Result<LogQueryRequest, Error> {
    let min_level = reader
        .get_min_level()
        .map(LogLevel::from_capnp)
        .map_err(|error| Error::failed(format!("unknown log level: {error:?}")))?;

    let request = LogQueryRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        session_slug: reader.get_session_slug()?.to_string()?,
        service: if reader.has_service() {
            Some(ServiceId::from_capnp(reader.get_service()?)?)
        } else {
            None
        },
        run: match reader.get_run()?.which()? {
            core::azoth_core_capnp::optional_uuid::Which::None(()) => None,
            core::azoth_core_capnp::optional_uuid::Which::Value(value) => {
                Some(Uuid::from_slice(value?).map_err(|error| {
                    Error::failed(format!("invalid log query run UUID: {error}"))
                })?)
            }
        },
        min_level,
        after_sequence: reader.get_after_sequence(),
        page_size: reader.get_page_size(),
        include_fields: reader.get_include_fields(),
    };
    validate_log_query_request_for_transport(&request)?;
    Ok(request)
}

fn validate_log_query_request_for_transport(request: &LogQueryRequest) -> Result<(), Error> {
    validate_observability_capability_request_for_transport(
        "log query request",
        &request.capability,
        OBSERVABILITY_READ_PERMISSION,
    )?;
    if request.session_slug.trim().is_empty() {
        return Err(invalid_observability_protocol_value(
            "log query request",
            "session slug must be non-empty",
        ));
    }
    if let Some(service) = &request.service
        && (service.namespace.trim().is_empty() || service.name.trim().is_empty())
    {
        return Err(invalid_observability_protocol_value(
            "log query request",
            "service filter must be non-empty",
        ));
    }
    if let Some(run) = request.run
        && run == Uuid::nil()
    {
        return Err(invalid_observability_protocol_value(
            "log query request",
            "run filter must be non-nil",
        ));
    }
    if request.run.is_some() && request.service.is_none() {
        return Err(invalid_observability_protocol_value(
            "log query request",
            "run filter requires a service filter",
        ));
    }
    if request.page_size == 0 {
        return Err(invalid_observability_protocol_value(
            "log query request",
            "page size must be positive",
        ));
    }
    Ok(())
}

fn write_log_query_result(
    result: &LogQueryResult,
    mut builder: observability_capnp::log_query_result::Builder<'_>,
) -> Result<(), Error> {
    validate_log_query_result_for_transport(result)?;
    let mut records = builder
        .reborrow()
        .init_records(capnp_list_index(result.records.len())?);
    for (index, record) in result.records.iter().enumerate() {
        (record).to_capnp(records.reborrow().get(capnp_list_index(index)?))?;
    }
    builder.set_next_sequence(result.next_sequence);
    builder.set_truncated(result.truncated);
    builder.set_source(&result.source);
    Ok(())
}

fn read_log_query_result(
    reader: observability_capnp::log_query_result::Reader<'_>,
) -> Result<LogQueryResult, Error> {
    let records = reader
        .get_records()?
        .iter()
        .map(read_observed_log_record)
        .collect::<Result<Vec<_>, Error>>()?;
    let result = LogQueryResult {
        records,
        next_sequence: reader.get_next_sequence(),
        truncated: reader.get_truncated(),
        source: reader.get_source()?.to_string()?,
    };
    validate_log_query_result_for_transport(&result)?;
    Ok(result)
}

fn validate_log_query_result_for_transport(result: &LogQueryResult) -> Result<(), Error> {
    if result.source.trim().is_empty() {
        return Err(invalid_observability_protocol_value(
            "log query result",
            "source must be non-empty",
        ));
    }

    let mut last_sequence = None;
    for record in &result.records {
        validate_observed_log_record_for_transport(record)?;
        if let Some(last) = last_sequence
            && record.sequence <= last
        {
            return Err(invalid_observability_protocol_value(
                "log query result",
                "records must be ordered by strictly increasing sequence",
            ));
        }
        last_sequence = Some(record.sequence);
    }
    if let Some(last) = last_sequence
        && result.next_sequence <= last
    {
        return Err(invalid_observability_protocol_value(
            "log query result",
            "next sequence must advance past the last record",
        ));
    }
    Ok(())
}

fn write_diagnostic_bundle_file(
    file: &DiagnosticBundleFile,
    mut builder: observability_capnp::diagnostic_bundle_file::Builder<'_>,
) -> Result<(), Error> {
    validate_diagnostic_bundle_file_for_transport(file)?;
    builder.set_path(&file.path);
    builder.set_category(&file.category);
    builder.set_byte_length(file.byte_length);
    builder.set_content_hash(&file.content_hash);
    Ok(())
}

fn read_diagnostic_bundle_file(
    reader: observability_capnp::diagnostic_bundle_file::Reader<'_>,
) -> Result<DiagnosticBundleFile, Error> {
    let file = DiagnosticBundleFile {
        path: reader.get_path()?.to_string()?,
        category: reader.get_category()?.to_string()?,
        byte_length: reader.get_byte_length(),
        content_hash: reader.get_content_hash()?.to_vec(),
    };
    validate_diagnostic_bundle_file_for_transport(&file)?;
    Ok(file)
}

fn validate_diagnostic_bundle_file_for_transport(file: &DiagnosticBundleFile) -> Result<(), Error> {
    if file.path.trim().is_empty() || file.category.trim().is_empty() {
        return Err(invalid_observability_protocol_value(
            "diagnostic bundle file",
            "path and category must be non-empty",
        ));
    }
    if !is_portable_relative_bundle_path(&file.path) {
        return Err(invalid_observability_protocol_value(
            "diagnostic bundle file",
            format!(
                "path `{}` must be a portable relative bundle path",
                file.path
            ),
        ));
    }
    if file.content_hash.len() != core::SIDE_CHANNEL_BLAKE3_HASH_BYTES {
        return Err(invalid_observability_protocol_value(
            "diagnostic bundle file",
            format!(
                "content hash must be {} bytes, got {}",
                core::SIDE_CHANNEL_BLAKE3_HASH_BYTES,
                file.content_hash.len()
            ),
        ));
    }
    Ok(())
}

fn is_portable_relative_bundle_path(path: &str) -> bool {
    !path.starts_with('/')
        && !path.contains(':')
        && !path.contains('\\')
        && path
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..")
}

fn write_diagnostic_bundle_manifest(
    manifest: &DiagnosticBundleManifest,
    mut builder: observability_capnp::diagnostic_bundle_manifest::Builder<'_>,
) -> Result<(), Error> {
    validate_diagnostic_bundle_manifest_for_transport(manifest)?;
    builder.set_schema_version(manifest.schema_version);
    builder.set_session_slug(&manifest.session_slug);
    builder.set_created_unix_ms(manifest.created_unix_ms);
    builder.set_reason(&manifest.reason);
    let mut files = builder
        .reborrow()
        .init_files(capnp_list_index(manifest.files.len())?);
    for (index, file) in manifest.files.iter().enumerate() {
        (file).to_capnp(files.reborrow().get(capnp_list_index(index)?))?;
    }
    Ok(())
}

fn read_diagnostic_bundle_manifest(
    reader: observability_capnp::diagnostic_bundle_manifest::Reader<'_>,
) -> Result<DiagnosticBundleManifest, Error> {
    let schema_version = reader.get_schema_version();
    if schema_version != DIAGNOSTIC_BUNDLE_MANIFEST_SCHEMA_VERSION {
        return Err(Error::failed(format!(
            "unsupported diagnostic bundle manifest schema version {schema_version}; expected {DIAGNOSTIC_BUNDLE_MANIFEST_SCHEMA_VERSION}"
        )));
    }

    let files = reader
        .get_files()?
        .iter()
        .map(read_diagnostic_bundle_file)
        .collect::<Result<Vec<_>, Error>>()?;

    let manifest = DiagnosticBundleManifest {
        schema_version,
        session_slug: reader.get_session_slug()?.to_string()?,
        created_unix_ms: reader.get_created_unix_ms(),
        reason: reader.get_reason()?.to_string()?,
        files,
    };
    validate_diagnostic_bundle_manifest_for_transport(&manifest)?;
    Ok(manifest)
}

fn validate_diagnostic_bundle_manifest_for_transport(
    manifest: &DiagnosticBundleManifest,
) -> Result<(), Error> {
    if manifest.schema_version != DIAGNOSTIC_BUNDLE_MANIFEST_SCHEMA_VERSION {
        return Err(Error::failed(format!(
            "unsupported diagnostic bundle manifest schema version {}; expected {DIAGNOSTIC_BUNDLE_MANIFEST_SCHEMA_VERSION}",
            manifest.schema_version
        )));
    }
    if manifest.session_slug.trim().is_empty()
        || manifest.created_unix_ms == 0
        || manifest.reason.trim().is_empty()
    {
        return Err(invalid_observability_protocol_value(
            "diagnostic bundle manifest",
            "session slug, created timestamp, and reason must be populated",
        ));
    }

    let mut seen_paths = BTreeSet::new();
    for file in &manifest.files {
        validate_diagnostic_bundle_file_for_transport(file)?;
        if !seen_paths.insert(file.path.as_str()) {
            return Err(invalid_observability_protocol_value(
                "diagnostic bundle manifest",
                format!("duplicate bundle file path `{}`", file.path),
            ));
        }
    }
    Ok(())
}

/// Serializes a diagnostic bundle manifest for a side-channel blob payload.
///
/// # Errors
///
/// Returns an error if the manifest fails its transport validation, or if the
/// message runs out of space while writing the manifest.
pub fn encode_diagnostic_bundle_manifest(
    manifest: &DiagnosticBundleManifest,
) -> Result<Vec<u8>, Error> {
    let mut message = message::Builder::new_default();
    (manifest).to_capnp(
        message.init_root::<observability_capnp::diagnostic_bundle_manifest::Builder<'_>>(),
    )?;

    let mut bytes = Vec::new();
    serialize_packed::write_message(&mut bytes, &message)?;
    Ok(bytes)
}

/// Deserializes a diagnostic bundle manifest from a side-channel blob payload.
///
/// # Errors
///
/// Returns an error if `bytes` is not a readable packed Cap'n Proto message, or
/// if a field of the manifest is absent from the message or is not valid UTF-8.
pub fn decode_diagnostic_bundle_manifest(bytes: &[u8]) -> Result<DiagnosticBundleManifest, Error> {
    let reader = serialize_packed::read_message(
        &mut std::io::Cursor::new(bytes),
        message::ReaderOptions::new(),
    )?;
    DiagnosticBundleManifest::from_capnp(
        reader.get_root::<observability_capnp::diagnostic_bundle_manifest::Reader<'_>>()?,
    )
}

/// # Errors
///
/// Returns an error if the staging file behind `handle` cannot be read or fails
/// its hash verification, plus any error
/// [`decode_diagnostic_bundle_manifest`] returns for those bytes.
pub fn load_diagnostic_bundle_manifest_side_channel(
    handle: &core::SideChannelHandle,
) -> Result<DiagnosticBundleManifest, DiagnosticBundleManifestSideChannelError> {
    let file = core::read_verified_staging_file(handle)?;
    Ok(decode_diagnostic_bundle_manifest(&file.bytes)?)
}

fn write_diagnostic_bundle_request(
    request: &DiagnosticBundleRequest,
    mut builder: observability_capnp::diagnostic_bundle_request::Builder<'_>,
) -> Result<(), Error> {
    validate_diagnostic_bundle_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_session_slug(&request.session_slug);
    builder.set_reason(&request.reason);
    builder.set_include_logs(request.include_logs);
    builder.set_include_crash_dumps(request.include_crash_dumps);
    builder.set_max_bytes(request.max_bytes);
    Ok(())
}

fn read_diagnostic_bundle_request(
    reader: observability_capnp::diagnostic_bundle_request::Reader<'_>,
) -> Result<DiagnosticBundleRequest, Error> {
    let request = DiagnosticBundleRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        session_slug: reader.get_session_slug()?.to_string()?,
        reason: reader.get_reason()?.to_string()?,
        include_logs: reader.get_include_logs(),
        include_crash_dumps: reader.get_include_crash_dumps(),
        max_bytes: reader.get_max_bytes(),
    };
    validate_diagnostic_bundle_request_for_transport(&request)?;
    Ok(request)
}

fn validate_diagnostic_bundle_request_for_transport(
    request: &DiagnosticBundleRequest,
) -> Result<(), Error> {
    validate_observability_capability_request_for_transport(
        "diagnostic bundle request",
        &request.capability,
        OBSERVABILITY_BUNDLE_PERMISSION,
    )?;
    if request.session_slug.trim().is_empty() || request.reason.trim().is_empty() {
        return Err(invalid_observability_protocol_value(
            "diagnostic bundle request",
            "session slug and reason must be non-empty",
        ));
    }
    if request.max_bytes == 0 {
        return Err(invalid_observability_protocol_value(
            "diagnostic bundle request",
            "max bytes must be positive",
        ));
    }
    Ok(())
}

fn validate_observability_capability_request_for_transport(
    kind: &'static str,
    capability: &Capability,
    required_permission: &'static str,
) -> Result<(), Error> {
    if capability.is_empty() {
        return Err(invalid_observability_protocol_value(
            kind,
            "capability cannot be empty",
        ));
    }
    if capability.audience != OBSERVABILITY_AUDIENCE {
        return Err(invalid_observability_protocol_value(
            kind,
            format!(
                "capability audience must be `{OBSERVABILITY_AUDIENCE}`, got `{}`",
                capability.audience
            ),
        ));
    }
    if capability.role == ServiceRole::Unknown {
        return Err(invalid_observability_protocol_value(
            kind,
            "capability role cannot be Unknown",
        ));
    }
    if !capability.has_permissions(&[required_permission]) {
        return Err(invalid_observability_protocol_value(
            kind,
            format!("capability missing `{required_permission}`"),
        ));
    }
    if capability.session.is_none() {
        return Err(invalid_observability_protocol_value(
            kind,
            format!("`{required_permission}` capability must be session-scoped"),
        ));
    }
    capability
        .validate_lifetime()
        .map_err(|error| invalid_observability_protocol_value(kind, error.to_string()))?;
    Ok(())
}

fn write_diagnostic_bundle_result(
    result: &DiagnosticBundleResult,
    mut builder: observability_capnp::diagnostic_bundle_result::Builder<'_>,
) -> Result<(), Error> {
    validate_diagnostic_bundle_result_for_transport(result)?;
    builder.set_accepted(result.accepted);
    if result.accepted {
        core::SideChannelHandle::to_capnp(&result.manifest, builder.reborrow().init_manifest())?;
    }
    builder.set_diagnostic(&result.diagnostic);
    Ok(())
}

fn read_diagnostic_bundle_result(
    reader: observability_capnp::diagnostic_bundle_result::Reader<'_>,
) -> Result<DiagnosticBundleResult, Error> {
    let result = DiagnosticBundleResult {
        accepted: reader.get_accepted(),
        manifest: core::SideChannelHandle::from_capnp(reader.get_manifest()?)?,
        diagnostic: reader.get_diagnostic()?.to_string()?,
    };
    validate_diagnostic_bundle_result_for_transport(&result)?;
    Ok(result)
}

fn validate_diagnostic_bundle_result_for_transport(
    result: &DiagnosticBundleResult,
) -> Result<(), Error> {
    if result.accepted {
        core::validated_staging_file_path(&result.manifest).map_err(|error| {
            invalid_observability_protocol_value(
                "diagnostic bundle result",
                format!("manifest handle is invalid: {error}"),
            )
        })?;
    } else if result.diagnostic.trim().is_empty() {
        return Err(invalid_observability_protocol_value(
            "diagnostic bundle result",
            "rejected result must carry a diagnostic",
        ));
    }
    Ok(())
}

fn write_set_log_channel_level_request(
    request: &SetLogChannelLevelRequest,
    mut builder: observability_capnp::set_log_channel_level_request::Builder<'_>,
) -> Result<(), Error> {
    validate_set_log_channel_level_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_target(&request.target);
    builder.set_verbosity(request.verbosity.to_capnp());
    builder.set_clear(request.clear);
    Ok(())
}

fn read_set_log_channel_level_request(
    reader: observability_capnp::set_log_channel_level_request::Reader<'_>,
) -> Result<SetLogChannelLevelRequest, Error> {
    let verbosity = reader
        .get_verbosity()
        .map(LogVerbosity::from_capnp)
        .map_err(|error| Error::failed(format!("unknown log verbosity: {error:?}")))?;
    let request = SetLogChannelLevelRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        target: reader.get_target()?.to_string()?,
        verbosity,
        clear: reader.get_clear(),
    };
    validate_set_log_channel_level_request_for_transport(&request)?;
    Ok(request)
}

fn validate_set_log_channel_level_request_for_transport(
    request: &SetLogChannelLevelRequest,
) -> Result<(), Error> {
    validate_observability_capability_request_for_transport(
        "set log channel level request",
        &request.capability,
        OBSERVABILITY_CONTROL_PERMISSION,
    )?;
    if request.target.trim().is_empty() {
        return Err(invalid_observability_protocol_value(
            "set log channel level request",
            "target must be non-empty",
        ));
    }
    Ok(())
}

fn write_channel_level(
    level: &ChannelLevel,
    mut builder: observability_capnp::channel_level::Builder<'_>,
) -> Result<(), Error> {
    if level.target.trim().is_empty() {
        return Err(invalid_observability_protocol_value(
            "channel level",
            "target must be non-empty",
        ));
    }
    builder.set_target(&level.target);
    builder.set_verbosity(level.verbosity.to_capnp());
    Ok(())
}

fn read_channel_level(
    reader: observability_capnp::channel_level::Reader<'_>,
) -> Result<ChannelLevel, Error> {
    let verbosity = reader
        .get_verbosity()
        .map(LogVerbosity::from_capnp)
        .map_err(|error| Error::failed(format!("unknown log verbosity: {error:?}")))?;
    let level = ChannelLevel {
        target: reader.get_target()?.to_string()?,
        verbosity,
    };
    if level.target.trim().is_empty() {
        return Err(invalid_observability_protocol_value(
            "channel level",
            "target must be non-empty",
        ));
    }
    Ok(level)
}

fn write_set_log_channel_level_result(
    result: &SetLogChannelLevelResult,
    mut builder: observability_capnp::set_log_channel_level_result::Builder<'_>,
) -> Result<(), Error> {
    validate_set_log_channel_level_result_for_transport(result)?;
    builder.set_accepted(result.accepted);
    builder.set_effective_directives(&result.effective_directives);
    builder.set_diagnostic(&result.diagnostic);
    Ok(())
}

fn read_set_log_channel_level_result(
    reader: observability_capnp::set_log_channel_level_result::Reader<'_>,
) -> Result<SetLogChannelLevelResult, Error> {
    let result = SetLogChannelLevelResult {
        accepted: reader.get_accepted(),
        effective_directives: reader.get_effective_directives()?.to_string()?,
        diagnostic: reader.get_diagnostic()?.to_string()?,
    };
    validate_set_log_channel_level_result_for_transport(&result)?;
    Ok(result)
}

fn validate_set_log_channel_level_result_for_transport(
    result: &SetLogChannelLevelResult,
) -> Result<(), Error> {
    if result.accepted && result.effective_directives.trim().is_empty() {
        return Err(invalid_observability_protocol_value(
            "set log channel level result",
            "accepted result must carry the effective directives",
        ));
    }
    if !result.accepted && result.diagnostic.trim().is_empty() {
        return Err(invalid_observability_protocol_value(
            "set log channel level result",
            "rejected result must carry a diagnostic",
        ));
    }
    Ok(())
}

fn write_list_channel_levels_result(
    result: &ListChannelLevelsResult,
    mut builder: observability_capnp::list_channel_levels_result::Builder<'_>,
) -> Result<(), Error> {
    builder.set_effective_directives(&result.effective_directives);
    let mut overrides = builder
        .reborrow()
        .init_overrides(capnp_list_index(result.overrides.len())?);
    for (index, level) in result.overrides.iter().enumerate() {
        (level).to_capnp(overrides.reborrow().get(capnp_list_index(index)?))?;
    }
    Ok(())
}

fn read_list_channel_levels_result(
    reader: observability_capnp::list_channel_levels_result::Reader<'_>,
) -> Result<ListChannelLevelsResult, Error> {
    let overrides = reader
        .get_overrides()?
        .iter()
        .map(read_channel_level)
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(ListChannelLevelsResult {
        effective_directives: reader.get_effective_directives()?.to_string()?,
        overrides,
    })
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

fn invalid_observability_protocol_value(kind: &'static str, reason: impl Into<String>) -> Error {
    Error::failed(format!("invalid {kind}: {}", reason.into()))
}

impl ObservedField {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the observed field.
    pub fn to_capnp(
        &self,
        builder: observability_capnp::observed_field::Builder<'_>,
    ) -> Result<(), Error> {
        write_observed_field(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the observed field is absent from the message or is not valid
    /// UTF-8.
    pub fn from_capnp(
        reader: observability_capnp::observed_field::Reader<'_>,
    ) -> Result<Self, Error> {
        read_observed_field(reader)
    }
}

impl ObservedSpan {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the observed span.
    pub fn to_capnp(
        &self,
        builder: observability_capnp::observed_span::Builder<'_>,
    ) -> Result<(), Error> {
        write_observed_span(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the observed span is absent from the message or is not valid
    /// UTF-8.
    pub fn from_capnp(
        reader: observability_capnp::observed_span::Reader<'_>,
    ) -> Result<Self, Error> {
        read_observed_span(reader)
    }
}

impl TraceContext {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the trace context.
    pub fn to_capnp(
        &self,
        builder: observability_capnp::trace_context::Builder<'_>,
    ) -> Result<(), Error> {
        write_trace_context(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the trace context is absent from the message or is not valid
    /// UTF-8.
    pub fn from_capnp(
        reader: observability_capnp::trace_context::Reader<'_>,
    ) -> Result<Self, Error> {
        read_trace_context(reader)
    }
}

impl ObservedLogRecord {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the observed log record.
    pub fn to_capnp(
        &self,
        builder: observability_capnp::observed_log_record::Builder<'_>,
    ) -> Result<(), Error> {
        write_observed_log_record(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the observed log record is absent from the message or is not
    /// valid UTF-8.
    pub fn from_capnp(
        reader: observability_capnp::observed_log_record::Reader<'_>,
    ) -> Result<Self, Error> {
        read_observed_log_record(reader)
    }
}

impl AppendLogBatchRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the append log batch
    /// request.
    pub fn to_capnp(
        &self,
        builder: observability_capnp::append_log_batch_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_append_log_batch_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the append log batch request is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: observability_capnp::append_log_batch_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_append_log_batch_request(reader)
    }
}

impl AppendLogBatchResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the append log batch result.
    pub fn to_capnp(
        &self,
        builder: observability_capnp::append_log_batch_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_append_log_batch_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the append log batch result is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: observability_capnp::append_log_batch_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_append_log_batch_result(reader)
    }
}

impl LogQueryRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the log query request.
    pub fn to_capnp(
        &self,
        builder: observability_capnp::log_query_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_log_query_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the log query request is absent from the message or is not
    /// valid UTF-8.
    pub fn from_capnp(
        reader: observability_capnp::log_query_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_log_query_request(reader)
    }
}

impl LogQueryResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the log query result.
    pub fn to_capnp(
        &self,
        builder: observability_capnp::log_query_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_log_query_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the log query result is absent from the message or is not
    /// valid UTF-8.
    pub fn from_capnp(
        reader: observability_capnp::log_query_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_log_query_result(reader)
    }
}

impl DiagnosticBundleFile {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the diagnostic bundle file.
    pub fn to_capnp(
        &self,
        builder: observability_capnp::diagnostic_bundle_file::Builder<'_>,
    ) -> Result<(), Error> {
        write_diagnostic_bundle_file(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the diagnostic bundle file is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: observability_capnp::diagnostic_bundle_file::Reader<'_>,
    ) -> Result<Self, Error> {
        read_diagnostic_bundle_file(reader)
    }
}

impl DiagnosticBundleManifest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the diagnostic bundle
    /// manifest.
    pub fn to_capnp(
        &self,
        builder: observability_capnp::diagnostic_bundle_manifest::Builder<'_>,
    ) -> Result<(), Error> {
        write_diagnostic_bundle_manifest(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the diagnostic bundle manifest is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: observability_capnp::diagnostic_bundle_manifest::Reader<'_>,
    ) -> Result<Self, Error> {
        read_diagnostic_bundle_manifest(reader)
    }
}

impl DiagnosticBundleRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the diagnostic bundle
    /// request.
    pub fn to_capnp(
        &self,
        builder: observability_capnp::diagnostic_bundle_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_diagnostic_bundle_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the diagnostic bundle request is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: observability_capnp::diagnostic_bundle_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_diagnostic_bundle_request(reader)
    }
}

impl DiagnosticBundleResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the diagnostic bundle
    /// result.
    pub fn to_capnp(
        &self,
        builder: observability_capnp::diagnostic_bundle_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_diagnostic_bundle_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the diagnostic bundle result is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: observability_capnp::diagnostic_bundle_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_diagnostic_bundle_result(reader)
    }
}

impl SetLogChannelLevelRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the set log channel level
    /// request.
    pub fn to_capnp(
        &self,
        builder: observability_capnp::set_log_channel_level_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_set_log_channel_level_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the set log channel level request is absent from the message
    /// or is not valid UTF-8.
    pub fn from_capnp(
        reader: observability_capnp::set_log_channel_level_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_set_log_channel_level_request(reader)
    }
}

impl ChannelLevel {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the channel level.
    pub fn to_capnp(
        &self,
        builder: observability_capnp::channel_level::Builder<'_>,
    ) -> Result<(), Error> {
        write_channel_level(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the channel level is absent from the message or is not valid
    /// UTF-8.
    pub fn from_capnp(
        reader: observability_capnp::channel_level::Reader<'_>,
    ) -> Result<Self, Error> {
        read_channel_level(reader)
    }
}

impl SetLogChannelLevelResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the set log channel level
    /// result.
    pub fn to_capnp(
        &self,
        builder: observability_capnp::set_log_channel_level_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_set_log_channel_level_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the set log channel level result is absent from the message
    /// or is not valid UTF-8.
    pub fn from_capnp(
        reader: observability_capnp::set_log_channel_level_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_set_log_channel_level_result(reader)
    }
}

impl ListChannelLevelsResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the list channel levels
    /// result.
    pub fn to_capnp(
        &self,
        builder: observability_capnp::list_channel_levels_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_list_channel_levels_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the list channel levels result is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: observability_capnp::list_channel_levels_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_list_channel_levels_result(reader)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use az_proto_core::{SideChannelHandle, SideChannelKind, StagingFileSideChannelError};

    use super::*;

    fn capability(permission: &str) -> Capability {
        Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_audience(OBSERVABILITY_AUDIENCE)
            .with_session(uuid::Uuid::from_bytes([0x0b; 16]))
            .with_permissions([permission])
    }

    fn log_record() -> ObservedLogRecord {
        ObservedLogRecord {
            sequence: 42,
            timestamp_unix_ns: 1_700_000_000_123_456_789,
            service: ServiceId::new("azoth", "project-host"),
            role: ServiceRole::ProjectHost,
            run: uuid::Uuid::now_v7(),
            session_slug: "lighting-pass".to_string(),
            target: "az_project_host::save".to_string(),
            level: LogLevel::Info,
            message: "saved authored document".to_string(),
            fields: vec![
                ObservedField {
                    name: "document".to_string(),
                    value: "prefabs/door.prefab.ron".to_string(),
                },
                ObservedField {
                    name: "revision".to_string(),
                    value: "7".to_string(),
                },
            ],
            trace: TraceContext {
                trace_id: vec![0xab; 16],
                span_id: vec![0xcd; 8],
                parent_span_id: vec![0xef; 8],
                trace_flags: 1,
                trace_state: "azoth=local".to_string(),
            },
            spans: vec![ObservedSpan {
                name: "save_document".to_string(),
                target: "az_project_host::save".to_string(),
                fields: vec![ObservedField {
                    name: "document".to_string(),
                    value: "prefabs/door.prefab.ron".to_string(),
                }],
            }],
            file: "crates/az/project-host/src/lib.rs".to_string(),
            line: 128,
        }
    }

    fn temp_test_dir(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("az-proto-observability-{name}-{nanos}"));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn log_record_round_trips_structured_fields_trace_context_and_spans() {
        let expected = log_record();
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<observability_capnp::observed_log_record::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<observability_capnp::observed_log_record::Reader<'_>>()
            .unwrap();
        assert_eq!(ObservedLogRecord::from_capnp(reader).unwrap(), expected);
    }

    #[test]
    fn log_record_encodes_as_packed_capnp_payload() {
        let expected = log_record();

        let bytes = encode_observed_log_record(&expected).unwrap();
        let actual = decode_observed_log_record(&bytes).unwrap();

        assert_eq!(actual, expected);
    }

    #[test]
    fn empty_trace_context_round_trips_as_no_active_span() {
        let mut message = message::Builder::new_default();
        (TraceContext::default())
            .to_capnp(message.init_root::<observability_capnp::trace_context::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<observability_capnp::trace_context::Reader<'_>>()
            .unwrap();

        assert_eq!(
            TraceContext::from_capnp(reader).unwrap(),
            TraceContext::default()
        );
    }

    #[test]
    fn trace_context_rejects_non_otel_id_lengths() {
        let mut message = message::Builder::new_default();
        let mut trace = message.init_root::<observability_capnp::trace_context::Builder<'_>>();
        trace.set_trace_id(&[0xab; OTEL_TRACE_ID_BYTES - 1]);
        trace.set_span_id(&[0xcd; OTEL_SPAN_ID_BYTES]);

        let reader = message
            .get_root_as_reader::<observability_capnp::trace_context::Reader<'_>>()
            .unwrap();
        let error = TraceContext::from_capnp(reader).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("trace context trace id must be 16 bytes")
        );
    }

    #[test]
    fn trace_context_rejects_parent_span_without_active_span() {
        let mut message = message::Builder::new_default();
        let mut trace = message.init_root::<observability_capnp::trace_context::Builder<'_>>();
        trace.set_parent_span_id(&[0xef; OTEL_SPAN_ID_BYTES]);

        let reader = message
            .get_root_as_reader::<observability_capnp::trace_context::Reader<'_>>()
            .unwrap();
        let error = TraceContext::from_capnp(reader).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("parent span id requires a non-empty trace id and span id")
        );
    }

    #[test]
    fn append_log_batch_request_round_trips_capability_and_records() {
        let expected = AppendLogBatchRequest {
            capability: capability(OBSERVABILITY_WRITE_PERMISSION),
            records: vec![log_record()],
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(
                message.init_root::<observability_capnp::append_log_batch_request::Builder<'_>>(),
            )
            .unwrap();

        let reader = message
            .get_root_as_reader::<observability_capnp::append_log_batch_request::Reader<'_>>()
            .unwrap();
        assert_eq!(AppendLogBatchRequest::from_capnp(reader).unwrap(), expected);
    }

    #[test]
    fn append_log_batch_request_rejects_read_capability() {
        let request = AppendLogBatchRequest {
            capability: capability(OBSERVABILITY_READ_PERMISSION),
            records: vec![log_record()],
        };
        let mut message = message::Builder::new_default();

        let error = (request)
            .to_capnp(
                message.init_root::<observability_capnp::append_log_batch_request::Builder<'_>>(),
            )
            .unwrap_err();

        assert!(
            error.to_string().contains(OBSERVABILITY_WRITE_PERMISSION),
            "{error}"
        );
    }

    #[test]
    fn append_log_batch_request_rejects_mixed_session_slugs() {
        let first = log_record();
        let mut second = first.clone();
        second.sequence += 1;
        second.session_slug = "other-session".to_string();
        let request = AppendLogBatchRequest {
            capability: capability(OBSERVABILITY_WRITE_PERMISSION),
            records: vec![first, second],
        };
        let mut message = message::Builder::new_default();

        let error = (request)
            .to_capnp(
                message.init_root::<observability_capnp::append_log_batch_request::Builder<'_>>(),
            )
            .unwrap_err();

        assert!(error.to_string().contains("mix session slugs"), "{error}");
    }

    #[test]
    fn append_log_batch_request_rejects_unscoped_capability() {
        let request = AppendLogBatchRequest {
            capability: capability(OBSERVABILITY_WRITE_PERMISSION).with_session(uuid::Uuid::nil()),
            records: vec![log_record()],
        };
        let mut request = request;
        request.capability.session = None;
        let mut message = message::Builder::new_default();

        let error = (request)
            .to_capnp(
                message.init_root::<observability_capnp::append_log_batch_request::Builder<'_>>(),
            )
            .unwrap_err();

        assert!(error.to_string().contains("session-scoped"), "{error}");
    }

    #[test]
    fn log_query_request_round_trips_optional_filters() {
        let expected = LogQueryRequest {
            capability: capability(OBSERVABILITY_READ_PERMISSION),
            session_slug: "lighting-pass".to_string(),
            service: Some(ServiceId::new("azoth", "asset-processor")),
            run: Some(uuid::Uuid::now_v7()),
            min_level: LogLevel::Warn,
            after_sequence: 128,
            page_size: 256,
            include_fields: true,
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<observability_capnp::log_query_request::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<observability_capnp::log_query_request::Reader<'_>>()
            .unwrap();
        assert_eq!(LogQueryRequest::from_capnp(reader).unwrap(), expected);
    }

    #[test]
    fn log_query_request_rejects_run_without_service() {
        let request = LogQueryRequest {
            capability: capability(OBSERVABILITY_READ_PERMISSION),
            session_slug: "lighting-pass".to_string(),
            service: None,
            run: Some(uuid::Uuid::now_v7()),
            min_level: LogLevel::Info,
            after_sequence: 0,
            page_size: 256,
            include_fields: false,
        };
        let mut message = message::Builder::new_default();

        let error = (request)
            .to_capnp(message.init_root::<observability_capnp::log_query_request::Builder<'_>>())
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("run filter requires a service filter"),
            "{error}"
        );
    }

    #[test]
    fn log_query_result_round_trips_paged_records() {
        let expected = LogQueryResult {
            records: vec![log_record()],
            next_sequence: 43,
            truncated: false,
            source: "session-run/log-index".to_string(),
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<observability_capnp::log_query_result::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<observability_capnp::log_query_result::Reader<'_>>()
            .unwrap();
        assert_eq!(LogQueryResult::from_capnp(reader).unwrap(), expected);
    }

    #[test]
    fn diagnostic_bundle_manifest_encodes_as_packed_capnp() {
        let mut expected =
            DiagnosticBundleManifest::new("lighting-pass", 1_700_000_001, "project-host crashed");
        expected.files.push(DiagnosticBundleFile {
            path: "logs/project-host.jsonl".to_string(),
            category: "log".to_string(),
            byte_length: 4096,
            content_hash: vec![0x5a; 32],
        });

        let bytes = encode_diagnostic_bundle_manifest(&expected).unwrap();
        let decoded = decode_diagnostic_bundle_manifest(&bytes).unwrap();

        assert_eq!(decoded, expected);
    }

    #[test]
    fn diagnostic_bundle_manifest_rejects_unsupported_schema_version() {
        let mut message = message::Builder::new_default();
        let mut manifest =
            message.init_root::<observability_capnp::diagnostic_bundle_manifest::Builder<'_>>();
        manifest.set_schema_version(DIAGNOSTIC_BUNDLE_MANIFEST_SCHEMA_VERSION + 1);
        manifest.set_session_slug("lighting-pass");
        manifest.set_created_unix_ms(1_700_000_001);
        manifest.set_reason("project-host crashed");
        manifest.init_files(0);

        let mut bytes = Vec::new();
        serialize_packed::write_message(&mut bytes, &message).unwrap();
        let error = decode_diagnostic_bundle_manifest(&bytes).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("unsupported diagnostic bundle manifest schema version")
        );
    }

    #[test]
    fn diagnostic_bundle_manifest_side_channel_validates_hash_and_decodes() {
        let expected =
            DiagnosticBundleManifest::new("lighting-pass", 1_700_000_001, "project-host crashed");
        let bytes = encode_diagnostic_bundle_manifest(&expected).unwrap();
        let hash = blake3::hash(&bytes).as_bytes().to_vec();
        let dir = temp_test_dir("bundle-manifest");
        let path = dir.join("bundle-manifest.capnp.packed");
        std::fs::write(&path, &bytes).unwrap();
        let handle = SideChannelHandle::staging_file(
            path.to_string_lossy(),
            bytes.len() as u64,
            hash,
            std::env::consts::OS,
        );

        let decoded = load_diagnostic_bundle_manifest_side_channel(&handle).unwrap();
        assert_eq!(decoded, expected);

        let mut corrupted = handle;
        corrupted.content_hash = vec![0; 32];
        assert!(matches!(
            load_diagnostic_bundle_manifest_side_channel(&corrupted),
            Err(DiagnosticBundleManifestSideChannelError::InvalidHandle(
                StagingFileSideChannelError::HashMismatch { .. }
            ))
        ));

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn diagnostic_bundle_request_and_result_round_trip_manifest_handle() {
        let request = DiagnosticBundleRequest {
            capability: capability(OBSERVABILITY_BUNDLE_PERMISSION),
            session_slug: "lighting-pass".to_string(),
            reason: "manual export".to_string(),
            include_logs: true,
            include_crash_dumps: false,
            max_bytes: 16 * 1024 * 1024,
        };
        let mut message = message::Builder::new_default();
        (request)
            .to_capnp(
                message.init_root::<observability_capnp::diagnostic_bundle_request::Builder<'_>>(),
            )
            .unwrap();
        let reader = message
            .get_root_as_reader::<observability_capnp::diagnostic_bundle_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            DiagnosticBundleRequest::from_capnp(reader).unwrap(),
            request
        );

        let dir = temp_test_dir("bundle-result");
        let path = dir.join("bundle-manifest.capnp.packed");
        let result = DiagnosticBundleResult {
            accepted: true,
            manifest: SideChannelHandle {
                kind: SideChannelKind::StagingFile,
                capability: None,
                locator: path.to_string_lossy().to_string(),
                byte_length: 512,
                content_hash: vec![0x71; 32],
                platform: std::env::consts::OS.to_string(),
            },
            diagnostic: String::new(),
        };
        let mut message = message::Builder::new_default();
        (result)
            .to_capnp(
                message.init_root::<observability_capnp::diagnostic_bundle_result::Builder<'_>>(),
            )
            .unwrap();
        let reader = message
            .get_root_as_reader::<observability_capnp::diagnostic_bundle_result::Reader<'_>>()
            .unwrap();
        assert_eq!(DiagnosticBundleResult::from_capnp(reader).unwrap(), result);

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn diagnostic_bundle_request_rejects_read_capability() {
        let request = DiagnosticBundleRequest {
            capability: capability(OBSERVABILITY_READ_PERMISSION),
            session_slug: "lighting-pass".to_string(),
            reason: "manual export".to_string(),
            include_logs: true,
            include_crash_dumps: false,
            max_bytes: 16 * 1024 * 1024,
        };
        let mut message = message::Builder::new_default();

        let error = (request)
            .to_capnp(
                message.init_root::<observability_capnp::diagnostic_bundle_request::Builder<'_>>(),
            )
            .unwrap_err();

        assert!(
            error.to_string().contains(OBSERVABILITY_BUNDLE_PERMISSION),
            "{error}"
        );
    }

    #[test]
    fn log_record_encode_rejects_empty_field_name() {
        let mut record = log_record();
        record.fields[0].name.clear();

        let error = encode_observed_log_record(&record).unwrap_err();

        assert!(error.to_string().contains("name must be non-empty"));
    }

    #[test]
    fn append_log_batch_result_encode_rejects_accepted_reason() {
        let result = AppendLogBatchResult {
            accepted: true,
            next_sequence: 2,
            rejected_reason: "should be empty".to_string(),
        };
        let mut message = message::Builder::new_default();

        let error = (result)
            .to_capnp(
                message.init_root::<observability_capnp::append_log_batch_result::Builder<'_>>(),
            )
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("accepted result cannot carry a rejected reason")
        );
    }

    #[test]
    fn log_query_result_encode_rejects_unordered_sequences() {
        let first = log_record();
        let mut second = first.clone();
        second.message = "older duplicate".to_string();
        let result = LogQueryResult {
            records: vec![first, second],
            next_sequence: 43,
            truncated: false,
            source: "session-run/log-index".to_string(),
        };
        let mut message = message::Builder::new_default();

        let error = (result)
            .to_capnp(message.init_root::<observability_capnp::log_query_result::Builder<'_>>())
            .unwrap_err();

        assert!(error.to_string().contains("strictly increasing sequence"));
    }

    #[test]
    fn diagnostic_bundle_manifest_encode_rejects_duplicate_paths() {
        let mut manifest =
            DiagnosticBundleManifest::new("lighting-pass", 1_700_000_001, "project-host crashed");
        manifest.files.push(DiagnosticBundleFile {
            path: "logs/project-host.jsonl".to_string(),
            category: "log".to_string(),
            byte_length: 4096,
            content_hash: vec![0x5a; 32],
        });
        manifest.files.push(DiagnosticBundleFile {
            path: "logs/project-host.jsonl".to_string(),
            category: "log".to_string(),
            byte_length: 4096,
            content_hash: vec![0x5a; 32],
        });

        let error = encode_diagnostic_bundle_manifest(&manifest).unwrap_err();

        assert!(error.to_string().contains("duplicate bundle file path"));
    }

    #[test]
    fn diagnostic_bundle_result_encode_rejects_relative_manifest_handle() {
        let result = DiagnosticBundleResult {
            accepted: true,
            manifest: SideChannelHandle {
                kind: SideChannelKind::StagingFile,
                capability: None,
                locator: "relative/bundle-manifest.capnp.packed".to_string(),
                byte_length: 512,
                content_hash: vec![0x71; 32],
                platform: std::env::consts::OS.to_string(),
            },
            diagnostic: String::new(),
        };
        let mut message = message::Builder::new_default();

        let error = (result)
            .to_capnp(
                message.init_root::<observability_capnp::diagnostic_bundle_result::Builder<'_>>(),
            )
            .unwrap_err();

        assert!(error.to_string().contains("manifest handle is invalid"));
    }

    #[test]
    fn log_level_maps_to_tracing_without_otel_dependency() {
        assert_eq!(LogLevel::from_tracing(tracing::Level::WARN), LogLevel::Warn);
        assert_eq!(LogLevel::Error.to_tracing(), tracing::Level::ERROR);
    }
}

#[cfg(test)]
mod log_channel_tests {
    use super::{LogChannel, LogVerbosity};

    #[test]
    fn channel_targets_round_trip() {
        for &ch in LogChannel::ALL {
            assert_eq!(LogChannel::from_target(ch.as_target()), Some(ch));
            assert!(ch.as_target().starts_with("az."));
        }
        assert_eq!(LogChannel::from_target("not.a.channel"), None);
    }

    #[test]
    fn verbosity_parses_and_renders() {
        assert_eq!(LogVerbosity::parse("DEBUG"), Some(LogVerbosity::Debug));
        assert_eq!(LogVerbosity::parse("off"), Some(LogVerbosity::Off));
        assert_eq!(LogVerbosity::Trace.as_str(), "trace");
    }

    #[test]
    fn verbosity_capnp_round_trips() {
        for &v in LogVerbosity::ALL {
            assert_eq!(LogVerbosity::from_capnp(v.to_capnp()), v);
        }
    }

    #[test]
    fn set_log_channel_level_request_round_trips() {
        use super::{
            Capability, OBSERVABILITY_AUDIENCE, OBSERVABILITY_CONTROL_PERMISSION, ServiceId,
            ServiceRole, SetLogChannelLevelRequest, message, observability_capnp,
        };
        let expected = SetLogChannelLevelRequest {
            capability: Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
                .with_audience(OBSERVABILITY_AUDIENCE)
                .with_session(uuid::Uuid::from_bytes([0x0c; 16]))
                .with_permissions([OBSERVABILITY_CONTROL_PERMISSION]),
            target: LogChannel::Assets.as_target().to_string(),
            verbosity: LogVerbosity::Debug,
            clear: false,
        };
        let mut msg = message::Builder::new_default();
        (expected)
            .to_capnp(
                msg.init_root::<observability_capnp::set_log_channel_level_request::Builder<'_>>(),
            )
            .unwrap();
        let reader = msg
            .get_root_as_reader::<observability_capnp::set_log_channel_level_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            SetLogChannelLevelRequest::from_capnp(reader).unwrap(),
            expected
        );
    }

    #[test]
    fn list_channel_levels_result_round_trips() {
        use super::{ChannelLevel, ListChannelLevelsResult, message, observability_capnp};
        let expected = ListChannelLevelsResult {
            effective_directives: "info,az.assets=debug".to_string(),
            overrides: vec![ChannelLevel {
                target: LogChannel::Assets.as_target().to_string(),
                verbosity: LogVerbosity::Debug,
            }],
        };
        let mut msg = message::Builder::new_default();
        (expected)
            .to_capnp(
                msg.init_root::<observability_capnp::list_channel_levels_result::Builder<'_>>(),
            )
            .unwrap();
        let reader = msg
            .get_root_as_reader::<observability_capnp::list_channel_levels_result::Reader<'_>>()
            .unwrap();
        assert_eq!(
            ListChannelLevelsResult::from_capnp(reader).unwrap(),
            expected
        );
    }

    #[test]
    fn channel_macros_expand_and_emit() {
        // Compiles only if `target: <expr>` + structured fields work.
        az_info!(LogChannel::Assets, count = 3_u32, "indexed assets");
        az_warn!(LogChannel::Gpu, "device lost");
        az_error!(LogChannel::Session, code = 7, "session failed");
        az_log!(
            LogChannel::Render,
            crate::tracing::Level::DEBUG,
            frame = 12_u64,
            "drew frame"
        );
    }
}
