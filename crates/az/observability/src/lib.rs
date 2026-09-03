//! Tracing integration helpers for Azoth observability.
//!
//! Service code should emit `tracing` events. This crate provides the shared
//! conversion layer from those events into `az-proto-observability` records so
//! editor console views, session log streams, diagnostic bundles, and later
//! OpenTelemetry exporters all start from the same event model.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{IsTerminal as _, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use az_proto_core::{ServiceId, ServiceRole, SideChannelHandle, write_named_staging_file_atomic};
pub use az_proto_observability::{
    DiagnosticBundleFile, DiagnosticBundleManifest, LogChannel, LogLevel, LogVerbosity,
    ObservedField, ObservedLogRecord, ObservedSpan, STRUCTURED_LOG_EXTENSION, TraceContext,
    decode_diagnostic_bundle_manifest, decode_observed_log_record,
    encode_diagnostic_bundle_manifest, encode_observed_log_record,
    load_diagnostic_bundle_manifest_side_channel,
};
#[cfg(feature = "otlp")]
use opentelemetry::trace::TracerProvider as _;
#[cfg(feature = "otlp")]
use opentelemetry_otlp::WithExportConfig;
#[cfg(feature = "otlp")]
use opentelemetry_sdk::Resource;
use thiserror::Error;
use tracing::{Event, Subscriber, span};
use tracing_subscriber::{
    EnvFilter, Layer, fmt::writer::BoxMakeWriter, layer::Context, prelude::*, registry::LookupSpan,
    util::SubscriberInitExt,
};
use uuid::Uuid;

pub const MAX_STRUCTURED_LOG_RECORD_BYTES: usize = 16 * 1024 * 1024;
pub const DIAGNOSTIC_BUNDLE_MANIFEST_FILE: &str = "manifest.capnp.packed";
pub const DIAGNOSTIC_BUNDLE_FILES_DIR: &str = "files";
pub const OTEL_EXPORTER_OTLP_ENDPOINT_ENV: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedLogContext {
    pub service: ServiceId,
    pub role: ServiceRole,
    pub run: Uuid,
    pub session_slug: String,
}

impl ObservedLogContext {
    #[must_use]
    pub const fn new(service: ServiceId, role: ServiceRole, run: Uuid) -> Self {
        Self {
            service,
            role,
            run,
            session_slug: String::new(),
        }
    }

    #[must_use]
    pub fn with_session_slug(mut self, session_slug: impl Into<String>) -> Self {
        self.session_slug = session_slug.into();
        self
    }
}

#[derive(Debug, Clone)]
pub struct ServiceObservabilityConfig {
    pub context: ObservedLogContext,
    pub structured_log: Option<PathBuf>,
    pub observed_log_sink: Option<ObservedLogSink>,
    pub otlp_endpoint: Option<String>,
    pub fmt: bool,
    pub default_directives: String,
    pub ansi: bool,
}

impl ServiceObservabilityConfig {
    #[must_use]
    pub fn new(context: ObservedLogContext) -> Self {
        Self {
            context,
            structured_log: None,
            observed_log_sink: None,
            otlp_endpoint: None,
            fmt: true,
            default_directives: "info".to_string(),
            ansi: std::env::var_os("NO_COLOR").is_none() && std::io::stderr().is_terminal(),
        }
    }

    #[must_use]
    pub fn with_structured_log(mut self, path: impl Into<PathBuf>) -> Self {
        self.structured_log = Some(path.into());
        self
    }

    #[must_use]
    pub fn with_observed_log_sink(mut self, sink: ObservedLogSink) -> Self {
        self.observed_log_sink = Some(sink);
        self
    }

    #[must_use]
    pub fn with_otlp_endpoint(mut self, endpoint: Option<impl Into<String>>) -> Self {
        self.otlp_endpoint = endpoint.map(Into::into);
        self
    }

    #[must_use]
    pub const fn without_fmt(mut self) -> Self {
        self.fmt = false;
        self
    }

    #[must_use]
    pub fn with_default_directives(mut self, directives: impl Into<String>) -> Self {
        self.default_directives = directives.into();
        self
    }

    #[must_use]
    pub const fn with_ansi(mut self, ansi: bool) -> Self {
        self.ansi = ansi;
        self
    }

    fn resolved_otlp_endpoint(&self) -> Option<String> {
        self.otlp_endpoint
            .clone()
            .or_else(|| std::env::var(OTEL_EXPORTER_OTLP_ENDPOINT_ENV).ok())
            .filter(|endpoint| !endpoint.trim().is_empty())
    }
}

#[derive(Clone)]
pub struct ObservedLogSink {
    sink: Arc<dyn Fn(ObservedLogRecord) + Send + Sync + 'static>,
}

impl ObservedLogSink {
    #[must_use]
    pub fn new(sink: impl Fn(ObservedLogRecord) + Send + Sync + 'static) -> Self {
        Self {
            sink: Arc::new(sink),
        }
    }

    pub fn emit(&self, record: ObservedLogRecord) {
        (self.sink)(record);
    }
}

impl fmt::Debug for ObservedLogSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ObservedLogSink").finish_non_exhaustive()
    }
}

/// Install the composed subscriber as this process's global tracing subscriber.
///
/// # Errors
///
/// Returns [`ObservedLogFileError::CreateDir`] or
/// [`ObservedLogFileError::Open`] if `config.structured_log` names a path whose
/// parent directory cannot be created or whose file cannot be opened for
/// append, and [`ObservedLogFileError::SubscriberInit`] if a global tracing
/// subscriber has already been installed for this process.
pub fn install_service_observability(
    config: ServiceObservabilityConfig,
) -> Result<(), ObservedLogFileError> {
    let subscriber = build_service_subscriber(config)?;
    subscriber.dispatch.try_init()?;

    // Only after the global subscriber is installed: publish the reload handle
    // (type-erased) and the directive state so the runtime-verbosity API works.
    let _ = LOG_FILTER_STATE.set(Mutex::new(LogFilterState {
        base_directives: subscriber.directives,
        overrides: BTreeMap::new(),
    }));
    let _ = LOG_FILTER_RELOAD.set(subscriber.reload);

    Ok(())
}

/// The composed subscriber plus the directive state that drives it. Production
/// installs `dispatch` globally; tests scope it with
/// [`tracing::dispatcher::with_default`] so the same stack is exercised without
/// claiming the process-wide subscriber.
struct ServiceSubscriber {
    dispatch: tracing::Dispatch,
    directives: String,
    reload: LogFilterReload,
}

type LogFilterReload = Box<dyn Fn(&str) -> Result<(), LogFilterError> + Send + Sync>;

fn build_service_subscriber(
    config: ServiceObservabilityConfig,
) -> Result<ServiceSubscriber, ObservedLogFileError> {
    let structured_sink = match config.structured_log.as_ref() {
        Some(path) => Some(ObservedLogFileSink::append(path)?),
        None => None,
    };
    // Console verbosity and durable telemetry have different floors. The
    // default CLI `warn` must not erase info-level product measurements, while
    // an unfiltered durable layer would enable every trace callsite in the
    // process. Both still honor explicit RUST_LOG target directives.
    let fmt_writer = if config.fmt {
        BoxMakeWriter::new(std::io::stderr)
    } else {
        BoxMakeWriter::new(std::io::sink)
    };
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(fmt_writer)
        .with_ansi(config.ansi);
    // Start with the CLI-selected default and append RUST_LOG directives so
    // targeted environment settings layer over the reserved -v/-q mapping.
    // Keep the combined string so live per-channel overrides can be re-applied.
    let base = config.default_directives.trim();
    let base = if base.is_empty() { "info" } else { base };
    let environment = std::env::var(EnvFilter::DEFAULT_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty());
    let requested = environment.as_deref().map_or_else(
        || base.to_string(),
        |environment| format!("{base},{environment}"),
    );
    let durable_requested = durable_log_directives(environment.as_deref());
    // Store the directives that are actually applied, not the requested string:
    // an invalid RUST_LOG falls back to the CLI default, and the saved base must reflect
    // that so `current_log_directives()` and per-channel merges stay valid.
    let (env_filter, base_directives) = EnvFilter::try_new(&requested).map_or_else(
        |_| (EnvFilter::new(base), base.to_string()),
        |filter| (filter, requested),
    );

    // Everything that borrows `config` is done above; the sink and context are
    // moved out below so this really does consume the config it was handed.
    #[cfg(feature = "otlp")]
    let otel_layer = build_optional_otel_layer(&config);
    #[cfg(not(feature = "otlp"))]
    if let Some(endpoint) = config.resolved_otlp_endpoint() {
        eprintln!(
            "warn: OTLP endpoint `{endpoint}` ignored because az-observability was built without the `otlp` feature"
        );
    }

    let observed_sink = config.observed_log_sink;
    let observed_layer = if structured_sink.is_some() || observed_sink.is_some() {
        Some(ObservedLogLayer::new(config.context, move |record| match (
            observed_sink.as_ref(),
            structured_sink.as_ref(),
        ) {
            (Some(sink), Some(file)) => {
                sink.emit(record.clone());
                let _ = file.write_record(&record);
            }
            (Some(sink), None) => sink.emit(record),
            (None, Some(file)) => {
                let _ = file.write_record(&record);
            }
            (None, None) => {}
        }))
    } else {
        None
    };

    #[cfg(feature = "otlp")]
    let (dispatch, reload_handle) = {
        let (filter, reload_handle) = tracing_subscriber::reload::Layer::new(env_filter);
        let observed_filter = filter_or_base(&durable_requested, "info");
        let otel_filter = filter_or_base(&durable_requested, "info");
        let dispatch = tracing::Dispatch::new(
            tracing_subscriber::registry()
                .with(otel_layer.with_filter(otel_filter))
                .with(fmt_layer.with_filter(filter))
                .with(observed_layer.with_filter(observed_filter)),
        );
        (dispatch, reload_handle)
    };

    #[cfg(not(feature = "otlp"))]
    let (dispatch, reload_handle) = {
        let (filter, reload_handle) = tracing_subscriber::reload::Layer::new(env_filter);
        let observed_filter = filter_or_base(&durable_requested, "info");
        let dispatch = tracing::Dispatch::new(
            tracing_subscriber::registry()
                .with(fmt_layer.with_filter(filter))
                .with(observed_layer.with_filter(observed_filter)),
        );
        (dispatch, reload_handle)
    };

    Ok(ServiceSubscriber {
        dispatch,
        directives: base_directives,
        reload: Box::new(move |directives| {
            reload_handle
                .reload(parse_log_directives(directives)?)
                .map_err(|error| LogFilterError::Reload(error.to_string()))
        }),
    })
}

fn filter_or_base(requested: &str, base: &str) -> EnvFilter {
    EnvFilter::try_new(requested).unwrap_or_else(|_| EnvFilter::new(base))
}

fn durable_log_directives(environment: Option<&str>) -> String {
    // A bare RUST_LOG level is commonly inherited from the service launcher and
    // describes console verbosity. Keep only scoped overrides here so it cannot
    // lower the durable info floor; `target=trace` and similar directives still
    // refine durable telemetry deliberately.
    let scoped = environment
        .into_iter()
        .flat_map(|directives| directives.split(','))
        .map(str::trim)
        .filter(|directive| !directive.is_empty() && directive.contains('='));
    std::iter::once("info")
        .chain(scoped)
        .collect::<Vec<_>>()
        .join(",")
}

/// Runtime log-verbosity control. Stores the base directives plus per-channel
/// overrides; rendering merges them (`info,az.assets=debug`) so dialing one
/// channel never silences the rest.
struct LogFilterState {
    base_directives: String,
    overrides: BTreeMap<LogChannel, LogVerbosity>,
}

static LOG_FILTER_STATE: OnceLock<Mutex<LogFilterState>> = OnceLock::new();
static LOG_FILTER_RELOAD: OnceLock<LogFilterReload> = OnceLock::new();

/// Error from the runtime log-verbosity controls.
#[derive(Debug, Error)]
pub enum LogFilterError {
    #[error("observability is not installed in this process; call install_*_observability first")]
    NotInstalled,
    #[error("invalid log filter directives `{directives}`: {reason}")]
    Parse { directives: String, reason: String },
    #[error("failed to apply log filter: {0}")]
    Reload(String),
}

fn render_log_directives(state: &LogFilterState) -> String {
    let mut directives = state.base_directives.clone();
    for (channel, verbosity) in &state.overrides {
        directives.push(',');
        directives.push_str(channel.as_target());
        directives.push('=');
        directives.push_str(verbosity.as_str());
    }
    directives
}

fn parse_log_directives(directives: &str) -> Result<EnvFilter, LogFilterError> {
    EnvFilter::try_new(directives).map_err(|error| LogFilterError::Parse {
        directives: directives.to_string(),
        reason: error.to_string(),
    })
}

fn apply_log_directives(directives: &str) -> Result<(), LogFilterError> {
    let reload = LOG_FILTER_RELOAD
        .get()
        .ok_or(LogFilterError::NotInstalled)?;
    reload(directives)
}

/// Set a single engine channel's verbosity threshold at runtime, merged over the
/// base directives. Returns the effective directive string.
///
/// Render + reload + commit happen under one lock with rollback on failure, so
/// the stored state never diverges from the live subscriber and concurrent
/// callers cannot interleave a stale directive after a newer one.
///
/// Note: this overlays the channel's coarse level. A more *specific* base
/// directive (a span/field filter like `az.assets[load]=off`) still wins by
/// `EnvFilter` specificity; use [`set_log_directives`] to fully override.
///
/// # Errors
///
/// Returns [`LogFilterError::NotInstalled`] if
/// [`install_service_observability`] has not run, or
/// [`LogFilterError::Reload`] / the parse error from
/// [`parse_log_directives`] if the merged directive string is rejected by the
/// live subscriber — in which case `channel`'s previous override is restored.
///
/// # Panics
///
/// Panics if the log filter state mutex was poisoned by a previous panic while
/// directives were being applied.
pub fn set_channel_level(
    channel: LogChannel,
    verbosity: LogVerbosity,
) -> Result<String, LogFilterError> {
    let state = LOG_FILTER_STATE.get().ok_or(LogFilterError::NotInstalled)?;
    let mut guard = state.lock().expect("log filter state poisoned");
    let previous = guard.overrides.insert(channel, verbosity);
    let directives = render_log_directives(&guard);
    if let Err(error) = apply_log_directives(&directives) {
        match previous {
            Some(value) => guard.overrides.insert(channel, value),
            None => guard.overrides.remove(&channel),
        };
        return Err(error);
    }
    drop(guard);
    Ok(directives)
}

/// Remove a channel's override so it inherits the base verbosity again. Returns
/// the effective directive string. (Distinct from setting it to `Off`.)
///
/// # Errors
///
/// Returns [`LogFilterError::NotInstalled`] if
/// [`install_service_observability`] has not run, or the reload/parse error if
/// the directive string left after removing the override is rejected — in which
/// case the removed override is put back.
///
/// # Panics
///
/// Panics if the log filter state mutex was poisoned by a previous panic while
/// directives were being applied.
pub fn clear_channel_level(channel: LogChannel) -> Result<String, LogFilterError> {
    let state = LOG_FILTER_STATE.get().ok_or(LogFilterError::NotInstalled)?;
    let mut guard = state.lock().expect("log filter state poisoned");
    let previous = guard.overrides.remove(&channel);
    let directives = render_log_directives(&guard);
    if let Err(error) = apply_log_directives(&directives) {
        if let Some(value) = previous {
            guard.overrides.insert(channel, value);
        }
        return Err(error);
    }
    drop(guard);
    Ok(directives)
}

/// Replace the entire filter with raw directives, clearing per-channel
/// overrides.
///
/// Takes a `RUST_LOG`-style string and returns the applied directive string.
/// The stored base is only updated after a successful reload.
///
/// # Errors
///
/// Returns [`LogFilterError::NotInstalled`] if
/// [`install_service_observability`] has not run, or the reload/parse error if
/// `directives` is not a valid filter string — in which case the stored base and
/// overrides are left untouched.
///
/// # Panics
///
/// Panics if the log filter state mutex was poisoned by a previous panic while
/// directives were being applied.
pub fn set_log_directives(directives: &str) -> Result<String, LogFilterError> {
    let state = LOG_FILTER_STATE.get().ok_or(LogFilterError::NotInstalled)?;
    let mut guard = state.lock().expect("log filter state poisoned");
    apply_log_directives(directives)?;
    guard.base_directives = directives.to_string();
    guard.overrides.clear();
    drop(guard);
    Ok(directives.to_string())
}

/// The currently effective log directive string, if observability is installed.
///
/// # Panics
///
/// Panics if the log filter state mutex was poisoned by a previous panic while
/// directives were being applied.
#[must_use]
pub fn current_log_directives() -> Option<String> {
    let state = LOG_FILTER_STATE.get()?;
    let guard = state.lock().expect("log filter state poisoned");
    Some(render_log_directives(&guard))
}

/// The active per-channel overrides (for UI/RPC sync without parsing the filter
/// string). Channels not present inherit the base verbosity.
///
/// # Panics
///
/// Panics if the log filter state mutex was poisoned by a previous panic while
/// directives were being applied.
#[must_use]
pub fn current_channel_levels() -> Vec<(LogChannel, LogVerbosity)> {
    LOG_FILTER_STATE.get().map_or_else(Vec::new, |state| {
        state
            .lock()
            .expect("log filter state poisoned")
            .overrides
            .iter()
            .map(|(channel, verbosity)| (*channel, *verbosity))
            .collect()
    })
}

#[cfg(feature = "otlp")]
fn build_optional_otel_layer(
    config: &ServiceObservabilityConfig,
) -> Option<
    tracing_opentelemetry::OpenTelemetryLayer<
        tracing_subscriber::Registry,
        opentelemetry_sdk::trace::Tracer,
    >,
> {
    let endpoint = config.resolved_otlp_endpoint()?;
    match build_otel_layer(&endpoint, &otel_service_name(&config.context)) {
        Ok(layer) => Some(layer),
        Err(error) => {
            eprintln!(
                "warn: OTLP exporter setup failed ({error}); continuing with local tracing sinks"
            );
            None
        }
    }
}

#[cfg(feature = "otlp")]
fn build_otel_layer(
    endpoint: &str,
    service_name: &str,
) -> Result<
    tracing_opentelemetry::OpenTelemetryLayer<
        tracing_subscriber::Registry,
        opentelemetry_sdk::trace::Tracer,
    >,
    Box<dyn std::error::Error>,
> {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()?;
    let provider = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
        .with_resource(Resource::new(vec![opentelemetry::KeyValue::new(
            "service.name",
            service_name.to_string(),
        )]))
        .build();
    let tracer = provider.tracer(service_name.to_string());
    opentelemetry::global::set_tracer_provider(provider);
    Ok(tracing_opentelemetry::layer().with_tracer(tracer))
}

#[cfg(feature = "otlp")]
fn otel_service_name(context: &ObservedLogContext) -> String {
    format!("{}.{}", context.service.namespace, context.service.name)
}

pub struct ObservedLogLayer {
    context: ObservedLogContext,
    sequence: Arc<AtomicU64>,
    sink: Arc<dyn Fn(ObservedLogRecord) + Send + Sync + 'static>,
}

impl ObservedLogLayer {
    #[must_use]
    pub fn new(
        context: ObservedLogContext,
        sink: impl Fn(ObservedLogRecord) + Send + Sync + 'static,
    ) -> Self {
        Self {
            context,
            sequence: Arc::new(AtomicU64::new(1)),
            sink: Arc::new(sink),
        }
    }
}

impl<S> Layer<S> for ObservedLogLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attrs: &span::Attributes<'_>, id: &span::Id, ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        attrs.record(&mut visitor);
        let observed_span = ObservedSpan {
            name: attrs.metadata().name().to_string(),
            target: attrs.metadata().target().to_string(),
            fields: visitor.fields,
        };
        let parent_id = attrs.parent().cloned().or_else(|| {
            attrs
                .is_contextual()
                .then(|| ctx.current_span().id().cloned())
                .flatten()
        });
        let parent_trace = parent_id
            .as_ref()
            .and_then(|parent_id| ctx.span(parent_id))
            .and_then(|parent| parent.extensions().get::<ObservedSpanTrace>().cloned());
        let trace = ObservedSpanTrace::new(&self.context, id, parent_trace.as_ref());

        if let Some(span) = ctx.span(id) {
            let mut extensions = span.extensions_mut();
            extensions.insert(trace);
            extensions.insert(observed_span);
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let meta = event.metadata();
        let mut visitor = EventVisitor {
            message: String::new(),
            fields: Vec::new(),
        };
        event.record(&mut visitor);
        let trace = ctx
            .event_span(event)
            .and_then(|span| {
                span.extensions()
                    .get::<ObservedSpanTrace>()
                    .map(TraceContext::from)
            })
            .unwrap_or_default();
        let spans = ctx
            .event_scope(event)
            .map(|scope| {
                scope
                    .from_root()
                    .filter_map(|span| span.extensions().get::<ObservedSpan>().cloned())
                    .collect()
            })
            .unwrap_or_default();

        (self.sink)(ObservedLogRecord {
            sequence: self.sequence.fetch_add(1, Ordering::Relaxed),
            timestamp_unix_ns: current_unix_ns(),
            service: self.context.service.clone(),
            role: self.context.role,
            run: self.context.run,
            session_slug: self.context.session_slug.clone(),
            target: meta.target().to_string(),
            level: LogLevel::from_tracing(*meta.level()),
            message: visitor.message,
            fields: visitor.fields,
            trace,
            spans,
            file: meta.file().unwrap_or_default().to_string(),
            line: meta.line().unwrap_or_default(),
        });
    }
}

/// The OTLP trace/span/parent identifiers carried alongside a record. Field
/// names drop the `_id` suffix the wire type uses; every field here is an ID.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedSpanTrace {
    trace: Vec<u8>,
    span: Vec<u8>,
    parent_span: Vec<u8>,
}

impl ObservedSpanTrace {
    fn new(context: &ObservedLogContext, id: &span::Id, parent: Option<&Self>) -> Self {
        let span = span_id_bytes(id);
        let trace =
            parent.map_or_else(|| root_trace_id(context, id), |parent| parent.trace.clone());
        let parent_span = parent.map(|parent| parent.span.clone()).unwrap_or_default();

        Self {
            trace,
            span,
            parent_span,
        }
    }
}

impl From<&ObservedSpanTrace> for TraceContext {
    fn from(value: &ObservedSpanTrace) -> Self {
        Self {
            trace_id: value.trace.clone(),
            span_id: value.span.clone(),
            parent_span_id: value.parent_span.clone(),
            trace_flags: 1,
            trace_state: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ObservedLogFileSink {
    path: PathBuf,
    file: Arc<Mutex<File>>,
}

impl ObservedLogFileSink {
    /// Open `path` for append, creating its parent directory if needed.
    ///
    /// # Errors
    ///
    /// Returns [`ObservedLogFileError::CreateDir`] if `path`'s parent directory
    /// cannot be created, or [`ObservedLogFileError::Open`] if the file itself
    /// cannot be opened for append.
    pub fn append(path: impl AsRef<Path>) -> Result<Self, ObservedLogFileError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ObservedLogFileError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|source| ObservedLogFileError::Open {
                path: path.clone(),
                source,
            })?;

        Ok(Self {
            path,
            file: Arc::new(Mutex::new(file)),
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one length-prefixed record to the log file.
    ///
    /// # Errors
    ///
    /// Returns [`ObservedLogFileError::Protocol`] if `record` cannot be encoded,
    /// [`ObservedLogFileError::RecordTooLarge`] if the encoded payload exceeds
    /// [`u32::MAX`] bytes, [`ObservedLogFileError::LockPoisoned`] if the file
    /// mutex was poisoned by a previous panic, or
    /// [`ObservedLogFileError::Write`] if the length prefix or payload cannot be
    /// written.
    pub fn write_record(&self, record: &ObservedLogRecord) -> Result<(), ObservedLogFileError> {
        let payload = encode_observed_log_record(record)?;
        let len =
            u32::try_from(payload.len()).map_err(|_| ObservedLogFileError::RecordTooLarge {
                path: self.path.clone(),
                len: payload.len(),
            })?;
        let mut file = self
            .file
            .lock()
            .map_err(|_| ObservedLogFileError::LockPoisoned {
                path: self.path.clone(),
            })?;
        file.write_all(&len.to_le_bytes())
            .and_then(|()| file.write_all(&payload))
            .map_err(|source| ObservedLogFileError::Write {
                path: self.path.clone(),
                source,
            })?;
        drop(file);
        Ok(())
    }
}

/// Read every record in a structured log file.
///
/// # Errors
///
/// Returns any error [`read_observed_log_file_from_offset`] returns when called
/// with offset `0`.
pub fn read_observed_log_file(
    path: impl AsRef<Path>,
) -> Result<Vec<ObservedLogRecord>, ObservedLogFileError> {
    Ok(read_observed_log_file_from_offset(path, 0)?.records)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedLogFileRead {
    pub records: Vec<ObservedLogRecord>,
    pub next_offset: u64,
}

/// Read the records appended after `offset`, plus the offset to resume from.
///
/// # Errors
///
/// Returns [`ObservedLogFileError::Open`] if `path` cannot be opened,
/// [`ObservedLogFileError::Read`] if its metadata, a length prefix or a record
/// payload cannot be read (including a prefix that runs past the end of the
/// file), or [`ObservedLogFileError::Protocol`] if a record fails to decode.
pub fn read_observed_log_file_from_offset(
    path: impl AsRef<Path>,
    offset: u64,
) -> Result<ObservedLogFileRead, ObservedLogFileError> {
    let path = path.as_ref();
    let mut file = File::open(path).map_err(|source| ObservedLogFileError::Open {
        path: path.to_path_buf(),
        source,
    })?;
    let file_len = file
        .metadata()
        .map_err(|source| ObservedLogFileError::Read {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    let offset = offset.min(file_len);
    file.seek(SeekFrom::Start(offset))
        .map_err(|source| ObservedLogFileError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    let mut records = Vec::new();
    let mut next_offset = offset;

    loop {
        let record_offset = next_offset;
        let mut len_bytes = [0_u8; 4];
        let len_read = file
            .read(&mut len_bytes)
            .map_err(|source| ObservedLogFileError::Read {
                path: path.to_path_buf(),
                source,
            })?;
        if len_read == 0 {
            break;
        }
        if len_read < len_bytes.len() {
            next_offset = record_offset;
            break;
        }

        let len = u32::from_le_bytes(len_bytes) as usize;
        if len > MAX_STRUCTURED_LOG_RECORD_BYTES {
            return Err(ObservedLogFileError::RecordTooLarge {
                path: path.to_path_buf(),
                len,
            });
        }
        let mut payload = vec![0_u8; len];
        let mut payload_read = 0_usize;
        while payload_read < len {
            let read = file.read(&mut payload[payload_read..]).map_err(|source| {
                ObservedLogFileError::Read {
                    path: path.to_path_buf(),
                    source,
                }
            })?;
            if read == 0 {
                return Ok(ObservedLogFileRead {
                    records,
                    next_offset: record_offset,
                });
            }
            payload_read += read;
        }
        next_offset = file
            .stream_position()
            .map_err(|source| ObservedLogFileError::Read {
                path: path.to_path_buf(),
                source,
            })?;
        records.push(decode_observed_log_record(&payload)?);
    }

    Ok(ObservedLogFileRead {
        records,
        next_offset,
    })
}

#[derive(Debug, Error)]
pub enum ObservedLogFileError {
    #[error("failed to create structured log directory `{path}`: {source}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to open structured log `{path}`: {source}")]
    Open {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write structured log `{path}`: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to read structured log `{path}`: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("structured log `{path}` mutex was poisoned")]
    LockPoisoned { path: PathBuf },

    #[error("structured log record for `{path}` is too large: {len} bytes")]
    RecordTooLarge { path: PathBuf, len: usize },

    #[error("failed to encode/decode structured log record: {0}")]
    Protocol(#[from] capnp::Error),

    #[error("failed to install tracing subscriber: {0}")]
    SubscriberInit(#[from] tracing_subscriber::util::TryInitError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticBundleSourceFile {
    pub source_path: PathBuf,
    pub bundle_path: PathBuf,
    pub category: String,
}

impl DiagnosticBundleSourceFile {
    #[must_use]
    pub fn new(source_path: impl Into<PathBuf>, category: impl Into<String>) -> Self {
        let source_path = source_path.into();
        let category = category.into();
        let file_name = source_path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("file");
        let bundle_path = PathBuf::from(DIAGNOSTIC_BUNDLE_FILES_DIR)
            .join(safe_bundle_component(&category))
            .join(file_name);
        Self {
            source_path,
            bundle_path,
            category,
        }
    }

    #[must_use]
    pub fn with_bundle_path(mut self, bundle_path: impl Into<PathBuf>) -> Self {
        self.bundle_path = bundle_path.into();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticBundle {
    pub root: PathBuf,
    pub manifest_path: PathBuf,
    pub manifest_handle: SideChannelHandle,
    pub manifest: DiagnosticBundleManifest,
}

/// Copy `sources` into a bundle directory under `bundle_root` and write the
/// manifest describing them.
///
/// # Errors
///
/// Returns [`DiagnosticBundleError::InvalidBundlePath`] if a source's bundle
/// path is absolute or escapes the bundle root,
/// [`DiagnosticBundleError::Read`] if a source file cannot be read,
/// [`DiagnosticBundleError::CreateDir`] if the bundle root or a source's parent
/// directory cannot be created, [`DiagnosticBundleError::Protocol`] if the
/// manifest cannot be encoded, or [`DiagnosticBundleError::Write`] if a copied
/// file or the manifest cannot be written.
pub fn write_diagnostic_bundle(
    bundle_root: impl AsRef<Path>,
    session_slug: impl Into<String>,
    reason: impl Into<String>,
    sources: impl IntoIterator<Item = DiagnosticBundleSourceFile>,
) -> Result<DiagnosticBundle, DiagnosticBundleError> {
    let bundle_root = bundle_root.as_ref().to_path_buf();
    std::fs::create_dir_all(&bundle_root).map_err(|source| DiagnosticBundleError::CreateDir {
        path: bundle_root.clone(),
        source,
    })?;

    let mut manifest =
        DiagnosticBundleManifest::new(session_slug, current_unix_ms(), reason.into());

    for source in sources {
        if !valid_relative_bundle_path(&source.bundle_path) {
            return Err(DiagnosticBundleError::InvalidBundlePath {
                path: source.bundle_path,
            });
        }

        let bytes = match std::fs::read(&source.source_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(source_error) => {
                return Err(DiagnosticBundleError::Read {
                    path: source.source_path,
                    source: source_error,
                });
            }
        };
        let target_path = bundle_root.join(&source.bundle_path);
        if let Some(parent) = target_path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| DiagnosticBundleError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        std::fs::write(&target_path, &bytes).map_err(|source| DiagnosticBundleError::Write {
            path: target_path.clone(),
            source,
        })?;

        manifest.files.push(DiagnosticBundleFile {
            path: protocol_path(&source.bundle_path),
            category: source.category,
            byte_length: bytes.len() as u64,
            content_hash: blake3::hash(&bytes).as_bytes().to_vec(),
        });
    }

    let manifest_bytes = encode_diagnostic_bundle_manifest(&manifest)?;
    let manifest_path = bundle_root.join(DIAGNOSTIC_BUNDLE_MANIFEST_FILE);
    let written =
        write_named_staging_file_atomic(&manifest_path, &manifest_bytes).map_err(|error| {
            DiagnosticBundleError::Write {
                path: error.path,
                source: error.source,
            }
        })?;
    let manifest_handle = SideChannelHandle::staging_file(
        written.path.to_string_lossy(),
        written.byte_length,
        written.content_hash,
        std::env::consts::OS,
    );

    Ok(DiagnosticBundle {
        root: bundle_root,
        manifest_path,
        manifest_handle,
        manifest,
    })
}

#[derive(Debug, Error)]
pub enum DiagnosticBundleError {
    #[error("failed to create diagnostic bundle directory `{path}`: {source}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to read diagnostic bundle source file `{path}`: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write diagnostic bundle file `{path}`: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("diagnostic bundle path `{path}` must be relative and stay inside the bundle root")]
    InvalidBundlePath { path: PathBuf },

    #[error("failed to encode diagnostic bundle manifest: {0}")]
    Protocol(#[from] capnp::Error),
}

#[must_use]
pub fn format_log_record_for_console(record: &ObservedLogRecord) -> String {
    let mut line = String::new();
    if record.message.is_empty() {
        line.push_str("event");
    } else {
        line.push_str(&record.message);
    }

    for field in &record.fields {
        line.push(' ');
        line.push_str(&field.name);
        line.push('=');
        line.push_str(&field.value);
    }

    line
}

#[must_use]
pub fn format_log_record_scope_for_console(record: &ObservedLogRecord) -> String {
    let span_scope = record
        .spans
        .iter()
        .map(|span| span.name.trim())
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    if !span_scope.is_empty() {
        return span_scope.join("::");
    }

    record.service.name.clone()
}

fn current_unix_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
        })
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

fn span_id_bytes(id: &span::Id) -> Vec<u8> {
    id.into_u64().to_be_bytes().to_vec()
}

fn root_trace_id(context: &ObservedLogContext, root_span_id: &span::Id) -> Vec<u8> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(context.service.namespace.as_bytes());
    hasher.update(b"\0");
    hasher.update(context.service.name.as_bytes());
    hasher.update(b"\0");
    hasher.update(context.run.as_bytes());
    hasher.update(b"\0");
    hasher.update(context.session_slug.as_bytes());
    hasher.update(b"\0");
    hasher.update(&std::process::id().to_be_bytes());
    hasher.update(b"\0");
    hasher.update(&root_span_id.into_u64().to_be_bytes());
    hasher.finalize().as_bytes()[..16].to_vec()
}

fn valid_relative_bundle_path(path: &Path) -> bool {
    let mut has_component = false;
    for component in path.components() {
        match component {
            Component::Normal(_) => has_component = true,
            Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir | Component::ParentDir => return false,
        }
    }
    has_component
}

fn protocol_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => part.to_str(),
            Component::CurDir
            | Component::Prefix(_)
            | Component::RootDir
            | Component::ParentDir => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn safe_bundle_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_was_separator = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator && !out.is_empty() {
            out.push('-');
            last_was_separator = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "file".to_string()
    } else {
        out
    }
}

#[derive(Debug, Default)]
struct FieldVisitor {
    fields: Vec<ObservedField>,
}

impl tracing::field::Visit for FieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.record_value(field.name(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.record_value(field.name(), value.to_string());
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.record_value(field.name(), value.to_string());
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.record_value(field.name(), value.to_string());
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.record_value(field.name(), value.to_string());
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.record_value(field.name(), value.to_string());
    }
}

impl FieldVisitor {
    fn record_value(&mut self, name: &str, value: String) {
        self.fields.push(ObservedField {
            name: name.to_string(),
            value,
        });
    }
}

struct EventVisitor {
    message: String,
    fields: Vec<ObservedField>,
}

impl tracing::field::Visit for EventVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.record_value(field.name(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.record_value(field.name(), value.to_string());
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.record_value(field.name(), value.to_string());
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.record_value(field.name(), value.to_string());
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.record_value(field.name(), value.to_string());
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.record_value(field.name(), value.to_string());
    }
}

impl EventVisitor {
    fn record_value(&mut self, name: &str, value: String) {
        if name == "message" {
            self.message = value.trim_matches('"').to_string();
            return;
        }

        self.fields.push(ObservedField {
            name: name.to_string(),
            value,
        });
    }
}

#[cfg(test)]
mod log_filter_tests {
    use super::{LogChannel, LogFilterState, LogVerbosity, render_log_directives};
    use std::collections::BTreeMap;

    #[test]
    fn render_merges_base_with_channel_overrides() {
        let mut overrides = BTreeMap::new();
        overrides.insert(LogChannel::Assets, LogVerbosity::Debug);
        overrides.insert(LogChannel::Gpu, LogVerbosity::Off);
        let state = LogFilterState {
            base_directives: "info".to_string(),
            overrides,
        };
        let rendered = render_log_directives(&state);
        // Base is preserved; each override appends `<target>=<level>`.
        assert!(rendered.starts_with("info,"));
        assert!(rendered.contains("az.assets=debug"));
        assert!(rendered.contains("az.gpu=off"));
        // The merged string must itself be a valid EnvFilter.
        assert!(tracing_subscriber::EnvFilter::try_new(&rendered).is_ok());
    }

    #[test]
    fn render_with_no_overrides_is_just_base() {
        let state = LogFilterState {
            base_directives: "warn,az_session=debug".to_string(),
            overrides: BTreeMap::new(),
        };
        assert_eq!(render_log_directives(&state), "warn,az_session=debug");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use tracing_subscriber::{Registry, prelude::*};

    use super::*;

    #[test]
    fn console_format_preserves_message_and_structured_fields() {
        let record = ObservedLogRecord {
            sequence: 1,
            timestamp_unix_ns: 10,
            service: ServiceId::new("azoth", "editor"),
            role: ServiceRole::Editor,
            run: Uuid::now_v7(),
            session_slug: String::new(),
            target: "az_editor::attach".to_string(),
            level: LogLevel::Info,
            message: "attached session".to_string(),
            fields: vec![ObservedField {
                name: "slug".to_string(),
                value: "lighting".to_string(),
            }],
            trace: TraceContext::default(),
            spans: Vec::new(),
            file: String::new(),
            line: 0,
        };

        assert_eq!(
            format_log_record_for_console(&record),
            "attached session slug=lighting"
        );
    }

    #[test]
    fn console_scope_prefers_span_path_over_event_target() {
        let record = ObservedLogRecord {
            sequence: 1,
            timestamp_unix_ns: 10,
            service: ServiceId::new("azoth", "editor"),
            role: ServiceRole::Editor,
            run: Uuid::now_v7(),
            session_slug: String::new(),
            target: "az_editor::panels::asset_browser".to_string(),
            level: LogLevel::Info,
            message: "refreshing assets".to_string(),
            fields: Vec::new(),
            trace: TraceContext::default(),
            spans: vec![
                ObservedSpan {
                    name: "refresh_asset_browser".to_string(),
                    target: "az_editor::asset_browser".to_string(),
                    fields: Vec::new(),
                },
                ObservedSpan {
                    name: "query_asset_database".to_string(),
                    target: "az_editor::asset_browser".to_string(),
                    fields: vec![ObservedField {
                        name: "kind".to_string(),
                        value: "prefab".to_string(),
                    }],
                },
            ],
            file: String::new(),
            line: 0,
        };

        assert_eq!(
            format_log_record_scope_for_console(&record),
            "refresh_asset_browser::query_asset_database"
        );
        assert_eq!(format_log_record_for_console(&record), "refreshing assets");
    }

    #[test]
    fn console_scope_falls_back_to_service_name_without_span_scope() {
        let record = ObservedLogRecord {
            sequence: 1,
            timestamp_unix_ns: 10,
            service: ServiceId::new("azoth", "asset-processor"),
            role: ServiceRole::AssetProcessor,
            run: Uuid::now_v7(),
            session_slug: "lighting".to_string(),
            target: "az_asset_processor::db::scan".to_string(),
            level: LogLevel::Info,
            message: "scanned".to_string(),
            fields: Vec::new(),
            trace: TraceContext::default(),
            spans: Vec::new(),
            file: String::new(),
            line: 0,
        };

        assert_eq!(
            format_log_record_scope_for_console(&record),
            "asset-processor"
        );
    }

    #[test]
    fn observed_log_layer_captures_tracing_event_fields() {
        let records = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&records);
        let layer = ObservedLogLayer::new(
            ObservedLogContext::new(
                ServiceId::new("azoth", "project-host"),
                ServiceRole::ProjectHost,
                Uuid::now_v7(),
            )
            .with_session_slug("lighting"),
            move |record| captured.lock().unwrap().push(record),
        );
        let subscriber = Registry::default().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(
                target: "az_project_host::save",
                document = "prefabs/door.prefab.ron",
                revision = 7_u64,
                "saved authored document"
            );
        });

        let records = records.lock().unwrap();
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.sequence, 1);
        assert_eq!(record.service, ServiceId::new("azoth", "project-host"));
        assert_eq!(record.role, ServiceRole::ProjectHost);
        assert_eq!(record.run.get_version_num(), 7);
        assert_eq!(record.session_slug, "lighting");
        assert_eq!(record.target, "az_project_host::save");
        assert_eq!(record.level, LogLevel::Info);
        assert_eq!(record.message, "saved authored document");
        assert!(record.spans.is_empty());
        assert_eq!(
            record.fields,
            vec![
                ObservedField {
                    name: "document".to_string(),
                    value: "prefabs/door.prefab.ron".to_string(),
                },
                ObservedField {
                    name: "revision".to_string(),
                    value: "7".to_string(),
                },
            ]
        );
        assert!(record.trace.is_empty());
        drop(records);
    }

    #[test]
    fn observed_log_layer_captures_span_trace_context() {
        let records = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&records);
        let layer = ObservedLogLayer::new(
            ObservedLogContext::new(
                ServiceId::new("azoth", "asset-processor"),
                ServiceRole::AssetProcessor,
                Uuid::now_v7(),
            )
            .with_session_slug("lighting"),
            move |record| captured.lock().unwrap().push(record),
        );
        let subscriber = Registry::default().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            let root = tracing::info_span!("record_source_asset");
            let _root_guard = root.enter();
            tracing::info!(target: "az_asset_processor", "recording source");

            let child = tracing::info_span!("enqueue_jobs");
            let _child_guard = child.enter();
            tracing::info!(target: "az_asset_processor", "queued jobs");
        });

        let records = records.lock().unwrap();
        assert_eq!(records.len(), 2);
        let root_trace = &records[0].trace;
        let child_trace = &records[1].trace;

        assert_eq!(root_trace.trace_id.len(), 16);
        assert_eq!(root_trace.span_id.len(), 8);
        assert!(root_trace.parent_span_id.is_empty());
        assert_eq!(root_trace.trace_flags, 1);
        assert!(root_trace.trace_state.is_empty());

        assert_eq!(child_trace.trace_id, root_trace.trace_id);
        assert_eq!(child_trace.parent_span_id, root_trace.span_id);
        assert_ne!(child_trace.span_id, root_trace.span_id);
        assert_eq!(child_trace.trace_flags, 1);
        assert_eq!(
            records[0].spans,
            vec![ObservedSpan {
                name: "record_source_asset".to_string(),
                target: "az_observability::tests".to_string(),
                fields: Vec::new(),
            }]
        );
        assert_eq!(
            records[1].spans,
            vec![
                ObservedSpan {
                    name: "record_source_asset".to_string(),
                    target: "az_observability::tests".to_string(),
                    fields: Vec::new(),
                },
                ObservedSpan {
                    name: "enqueue_jobs".to_string(),
                    target: "az_observability::tests".to_string(),
                    fields: Vec::new(),
                },
            ]
        );
        assert_eq!(
            format_log_record_scope_for_console(&records[1]),
            "record_source_asset::enqueue_jobs"
        );
        drop(records);
    }

    #[test]
    fn observed_log_layer_captures_span_fields() {
        let records = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&records);
        let layer = ObservedLogLayer::new(
            ObservedLogContext::new(
                ServiceId::new("azoth", "asset-processor"),
                ServiceRole::AssetProcessor,
                Uuid::now_v7(),
            )
            .with_session_slug("lighting"),
            move |record| captured.lock().unwrap().push(record),
        );
        let subscriber = Registry::default().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!(
                "reconcile_asset_sources",
                session_slug = "lighting",
                root_count = 2_u64
            );
            let _guard = span.enter();
            tracing::info!(target: "az_asset_processor", "scanning source roots");
        });

        let records = records.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].spans,
            vec![ObservedSpan {
                name: "reconcile_asset_sources".to_string(),
                target: "az_observability::tests".to_string(),
                fields: vec![
                    ObservedField {
                        name: "session_slug".to_string(),
                        value: "lighting".to_string(),
                    },
                    ObservedField {
                        name: "root_count".to_string(),
                        value: "2".to_string(),
                    },
                ],
            }]
        );
        drop(records);
    }

    #[test]
    fn structured_log_file_sink_round_trips_length_delimited_records() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("az-sessiond.capnp.log");
        let sink = ObservedLogFileSink::append(&path).unwrap();
        let record = ObservedLogRecord {
            sequence: 1,
            timestamp_unix_ns: 10,
            service: ServiceId::new("azoth", "session-supervisor"),
            role: ServiceRole::SessionSupervisor,
            run: Uuid::now_v7(),
            session_slug: "editor".to_string(),
            target: "az_sessiond".to_string(),
            level: LogLevel::Info,
            message: "started".to_string(),
            fields: vec![ObservedField {
                name: "endpoint".to_string(),
                value: "pipe".to_string(),
            }],
            trace: TraceContext::default(),
            spans: Vec::new(),
            file: "main.rs".to_string(),
            line: 12,
        };

        sink.write_record(&record).unwrap();

        assert_eq!(read_observed_log_file(&path).unwrap(), vec![record]);
    }

    /// A service launched with no `-v` maps to the `"warn"` console directives
    /// (`az-service-entrypoint`'s `default_log_directives`). The console's
    /// verbosity must not decide what the durable record holds: `az session
    /// logs` reads the `.capnp.log`, and every permanent measurement field the
    /// pipeline emits is an `info!`.
    #[test]
    fn default_launch_verbosity_still_fills_the_structured_sink() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("asset-processor.capnp.log");
        let config = ServiceObservabilityConfig::new(
            ObservedLogContext::new(
                ServiceId::new("azoth", "asset-processor"),
                ServiceRole::AssetProcessor,
                Uuid::now_v7(),
            )
            .with_session_slug("lighting"),
        )
        .with_structured_log(&path)
        .with_default_directives("warn")
        .with_ansi(false);

        let subscriber = build_service_subscriber(config).unwrap();
        tracing::dispatcher::with_default(&subscriber.dispatch, || {
            tracing::trace!(
                target: "az_asset_processor::reconcile",
                "trace-only implementation detail"
            );
            tracing::info!(
                target: "az_asset_processor::reconcile",
                elapsed_ms = 12_u64,
                "reconciled asset sources"
            );
        });

        let written = std::fs::metadata(&path).unwrap().len();
        assert!(
            written > 0,
            "structured sink at `{}` stayed empty under the default console verbosity",
            path.display()
        );

        let records = read_observed_log_file(&path).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].level, LogLevel::Info);
        assert_eq!(records[0].message, "reconciled asset sources");
        assert_eq!(records[0].target, "az_asset_processor::reconcile");
        assert_eq!(records[0].session_slug, "lighting");
        assert_eq!(
            records[0].fields,
            vec![ObservedField {
                name: "elapsed_ms".to_string(),
                value: "12".to_string(),
            }]
        );
    }

    #[test]
    fn durable_filter_keeps_its_info_floor_and_scoped_environment_overrides() {
        assert_eq!(durable_log_directives(None), "info");
        assert_eq!(
            durable_log_directives(Some(
                "warn,az_asset_processor::reconcile=trace,az_editor=debug"
            )),
            "info,az_asset_processor::reconcile=trace,az_editor=debug"
        );
    }

    #[test]
    fn structured_log_file_reads_from_byte_offset() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("project-host.capnp.log");
        let sink = ObservedLogFileSink::append(&path).unwrap();
        let first = ObservedLogRecord {
            sequence: 1,
            timestamp_unix_ns: 10,
            service: ServiceId::new("azoth", "project-host"),
            role: ServiceRole::ProjectHost,
            run: Uuid::now_v7(),
            session_slug: "editor".to_string(),
            target: "az_project_host".to_string(),
            level: LogLevel::Info,
            message: "started".to_string(),
            fields: Vec::new(),
            trace: TraceContext::default(),
            spans: Vec::new(),
            file: String::new(),
            line: 0,
        };
        let mut second = first.clone();
        second.sequence = 2;
        second.message = "ready".to_string();

        sink.write_record(&first).unwrap();
        let offset = read_observed_log_file_from_offset(&path, 0)
            .unwrap()
            .next_offset;
        sink.write_record(&second).unwrap();

        let read = read_observed_log_file_from_offset(&path, offset).unwrap();

        assert_eq!(read.records, vec![second]);
        assert_eq!(read.next_offset, std::fs::metadata(&path).unwrap().len());
    }

    #[test]
    fn diagnostic_bundle_copies_sources_and_writes_verified_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let source_dir = temp.path().join("session-run");
        std::fs::create_dir_all(source_dir.join("logs")).unwrap();
        let failure = source_dir.join("failure.txt");
        let log = source_dir.join("logs").join("project-host.stderr.log");
        std::fs::write(&failure, "project-host crashed").unwrap();
        std::fs::write(&log, "panic during bootstrap").unwrap();

        let bundle = write_diagnostic_bundle(
            temp.path().join("bundle"),
            "editor",
            "project-host crashed",
            [
                DiagnosticBundleSourceFile::new(failure, "failure").with_bundle_path("failure.txt"),
                DiagnosticBundleSourceFile::new(log, "log")
                    .with_bundle_path("logs/project-host.stderr.log"),
                DiagnosticBundleSourceFile::new(source_dir.join("missing.log"), "log"),
            ],
        )
        .unwrap();

        assert!(bundle.root.join("failure.txt").exists());
        assert!(
            bundle
                .root
                .join("logs")
                .join("project-host.stderr.log")
                .exists()
        );
        assert_eq!(bundle.manifest.files.len(), 2);
        assert_eq!(bundle.manifest.files[0].path, "failure.txt");
        assert_eq!(bundle.manifest.files[0].category, "failure");
        assert_eq!(bundle.manifest.files[0].byte_length, 20);
        assert_eq!(
            load_diagnostic_bundle_manifest_side_channel(&bundle.manifest_handle).unwrap(),
            bundle.manifest
        );
        assert_eq!(
            bundle.manifest_handle.locator,
            bundle.manifest_path.to_string_lossy()
        );
        assert_eq!(
            bundle
                .root
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
    fn diagnostic_bundle_rejects_paths_that_escape_bundle_root() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("failure.txt");
        std::fs::write(&source, "failed").unwrap();

        let error =
            write_diagnostic_bundle(
                temp.path().join("bundle"),
                "editor",
                "failed",
                [DiagnosticBundleSourceFile::new(source, "failure")
                    .with_bundle_path("../failure.txt")],
            )
            .unwrap_err();

        assert!(matches!(
            error,
            DiagnosticBundleError::InvalidBundlePath { .. }
        ));
    }
}
