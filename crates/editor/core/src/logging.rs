use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use az_observability::{
    LogLevel, ObservedLogContext, ObservedLogFileError, ObservedLogRecord, ObservedLogSink,
    ServiceObservabilityConfig, format_log_record_for_console, format_log_record_scope_for_console,
    install_service_observability,
};
use az_proto_core::{ServiceId, ServiceRole};
use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use uuid::Uuid;

type LogRecord = ObservedLogRecord;
type LogSender = UnboundedSender<LogRecord>;
type LogReceiver = UnboundedReceiver<LogRecord>;

static LOG_SENDER: OnceLock<LogSender> = OnceLock::new();
static LOG_RX: OnceLock<Mutex<Option<LogReceiver>>> = OnceLock::new();
static EDITOR_RUN: OnceLock<Uuid> = OnceLock::new();
static OBSERVABILITY_INSTALLED: AtomicBool = AtomicBool::new(false);

struct LogForwarderLifecycle {
    _quitting: Arc<AtomicBool>,
    _quit_subscription: gpui::Subscription,
}

impl gpui::Global for LogForwarderLifecycle {}

/// Install the full process-wide tracing subscriber for the editor binary.
///
/// Installing twice is a no-op, and so is installing before the output channel
/// exists.
///
/// # Errors
///
/// Returns [`ObservedLogFileError`] when the observability installer cannot open
/// or write the process log file.
pub fn install_process_observability(
    default_directives: &str,
    ansi: bool,
) -> Result<(), ObservedLogFileError> {
    if OBSERVABILITY_INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }

    let Some(sink) = output_observed_log_sink() else {
        return Ok(());
    };
    install_service_observability(
        ServiceObservabilityConfig::new(editor_observed_log_context())
            .with_observed_log_sink(sink)
            .with_default_directives(default_directives)
            .with_ansi(ansi),
    )?;
    OBSERVABILITY_INSTALLED.store(true, Ordering::Release);
    Ok(())
}

/// Install a tracing layer that forwards events into an async channel when the
/// standalone binary did not already install the full process subscriber.
pub fn hook() {
    if OBSERVABILITY_INSTALLED.load(Ordering::Acquire) {
        return;
    }

    let Some(sink) = output_observed_log_sink() else {
        return;
    };
    if install_service_observability(
        ServiceObservabilityConfig::new(editor_observed_log_context())
            .with_observed_log_sink(sink)
            .without_fmt(),
    )
    .is_ok()
    {
        OBSERVABILITY_INSTALLED.store(true, Ordering::Release);
    }
}

fn output_observed_log_sink() -> Option<ObservedLogSink> {
    ensure_output_channel().map(|tx| {
        ObservedLogSink::new(move |record| {
            let _ = tx.unbounded_send(record);
        })
    })
}

fn ensure_output_channel() -> Option<LogSender> {
    if LOG_SENDER.get().is_none() {
        let (tx, rx) = unbounded::<LogRecord>();
        let _ = LOG_SENDER.set(tx);
        let _ = LOG_RX.set(Mutex::new(Some(rx)));
    }

    LOG_SENDER.get().cloned()
}

fn editor_observed_log_context() -> ObservedLogContext {
    ObservedLogContext::new(
        ServiceId::new("azoth", "editor"),
        ServiceRole::Editor,
        *EDITOR_RUN.get_or_init(Uuid::now_v7),
    )
}

/// Run a foreground async task that receives logs and updates the UI via `AsyncApp`.
pub fn run(app: &mut gpui::App) {
    // Take ownership of the receiver so we can await without holding a lock.
    let rx: Option<LogReceiver> = LOG_RX
        .get()
        .and_then(|cell| cell.lock().ok().and_then(|mut opt| opt.take()));
    let Some(mut rx) = rx else {
        return;
    };
    let quitting = Arc::new(AtomicBool::new(false));
    let quit_subscription = app.on_app_quit({
        let quitting = quitting.clone();
        move |_| {
            quitting.store(true, Ordering::Release);
            std::future::ready(())
        }
    });
    app.set_global(LogForwarderLifecycle {
        _quitting: quitting.clone(),
        _quit_subscription: quit_subscription,
    });
    let a_async = app.to_async();
    let fe = {
        let fe_ref = a_async.foreground_executor();
        fe_ref.clone()
    };
    fe.spawn(async move {
        while let Some(record) = rx.next().await {
            if quitting.load(Ordering::Acquire) {
                break;
            }
            let lvl = ui_log_level(record.level);
            let source = format_log_record_scope_for_console(&record);
            let message = format_log_record_for_console(&record);
            a_async.update(|app| {
                app.default_global::<az_editor_ui::ConsoleState>()
                    .log_from_source(lvl, source, message);
            });
            if quitting.load(Ordering::Acquire) {
                break;
            }
            a_async.refresh();
        }
    })
    .detach();
}

const fn ui_log_level(level: LogLevel) -> az_editor_ui::LogLevel {
    match level {
        LogLevel::Error => az_editor_ui::LogLevel::Error,
        LogLevel::Warn => az_editor_ui::LogLevel::Warn,
        LogLevel::Info => az_editor_ui::LogLevel::Info,
        LogLevel::Debug => az_editor_ui::LogLevel::Debug,
        LogLevel::Trace => az_editor_ui::LogLevel::Trace,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_level_mapping_matches_observability_level() {
        assert_eq!(ui_log_level(LogLevel::Warn), az_editor_ui::LogLevel::Warn);
        assert_eq!(ui_log_level(LogLevel::Trace), az_editor_ui::LogLevel::Trace);
    }
}
