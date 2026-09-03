use super::{
    App, AssetProcessorClient, AssetProcessorEvent, AssetSourceSchemaAuthoringData, Builder,
    DEFAULT_ASSET_PRODUCT_PLATFORM, EditorAssetBrowserController, EditorAssetBrowserStatus,
    EditorAssetBuilderCatalog, EditorAssetProcessorActivity, EditorAssetProcessorEventInbox,
    EditorAssetProcessorEventPublisher, EditorAssetSourceDependentsPreview, EditorAttachSession,
    EditorCatalogProductsStatus, EditorError, EditorResult, LocalSet, ServiceHealthStateData,
    WorkspaceRootData, asset_processor_activity_to_ui, error, info, oneshot,
    publish_asset_browser_error, publish_asset_browser_status,
    publish_asset_processor_event_update, source_path_matches_extensions,
    spawn_asset_browser_load_more, spawn_asset_browser_snapshot_refresh,
    spawn_asset_force_reprocess, spawn_asset_source_delete, spawn_asset_source_dependents_preview,
    spawn_asset_source_file_create, spawn_asset_source_reconcile, spawn_asset_source_rename,
    spawn_catalog_products_refresh, spawn_job_inspection, thread, validate_asset_db_relative_path,
    workspace_roots_to_ui,
};

use std::sync::Arc;

use crate::watch_handle::WatchHandle;

pub fn install_asset_browser_action_handlers(cx: &mut App) {
    install_asset_browser_read_action_handlers(cx);
    install_asset_source_write_action_handlers(cx);
}

/// Scan, refresh, paging, and job inspection: the actions that only read
/// asset-processor state.
fn install_asset_browser_read_action_handlers(cx: &mut App) {
    cx.on_action(|_: &az_editor_ui::actions::ScanAssets, cx| {
        if let Err(err) = scan_asset_sources(cx) {
            error!(error = %err, "failed to handle asset source scan action");
        }
    });
    cx.on_action(|_: &az_editor_ui::actions::RefreshAssets, cx| {
        if let Err(err) = refresh_asset_browser_status(cx) {
            error!(error = %err, "failed to handle asset browser refresh action");
        }
    });
    cx.on_action(
        |action: &az_editor_ui::actions::RefreshCatalogProducts, cx| {
            if let Err(err) = refresh_catalog_products(cx, &action.platform) {
                error!(
                    error = %err,
                    platform = %action.platform,
                    "failed to handle catalog products refresh action"
                );
            }
        },
    );
    cx.on_action(|_: &az_editor_ui::actions::LoadMoreAssets, cx| {
        if let Err(err) = load_more_asset_browser_status(cx) {
            error!(error = %err, "failed to handle asset browser load-more action");
        }
    });
    cx.on_action(|action: &az_editor_ui::actions::InspectJob, cx| {
        if let Err(err) = inspect_job(cx, action.job_id, action.attempt_id) {
            error!(
                error = %err,
                job_id = action.job_id,
                attempt_id = ?action.attempt_id,
                "failed to handle asset job inspection action"
            );
        }
    });
}

/// Create, delete, rename, and force-reprocess: the actions that write source
/// files through the asset processor.
fn install_asset_source_write_action_handlers(cx: &mut App) {
    cx.on_action(
        |action: &az_editor_ui::actions::CreateAssetSourceFile, cx| {
            if let Err(err) = create_asset_source_file(
                cx,
                action.source_root.clone(),
                action.source_path.clone(),
                action.schema_type.clone(),
            ) {
                error!(
                    error = %err,
                    source_root = %action.source_root,
                    source_path = %action.source_path,
                    schema_type = %action.schema_type,
                    "failed to handle asset source create action"
                );
            }
        },
    );
    cx.on_action(
        |action: &az_editor_ui::actions::PreviewDeleteAssetSource, cx| {
            if let Err(err) = preview_delete_asset_source(
                cx,
                action.source_root.clone(),
                action.source_path.clone(),
            ) {
                error!(
                    error = %err,
                    source_root = %action.source_root,
                    source_path = %action.source_path,
                    "failed to handle asset source delete preview action"
                );
            }
        },
    );
    cx.on_action(|action: &az_editor_ui::actions::DeleteAssetSource, cx| {
        if let Err(err) =
            delete_asset_source(cx, action.source_root.clone(), action.source_path.clone())
        {
            error!(
                error = %err,
                source_root = %action.source_root,
                source_path = %action.source_path,
                "failed to handle asset source delete action"
            );
        }
    });
    cx.on_action(|action: &az_editor_ui::actions::RenameAssetSource, cx| {
        if let Err(err) = rename_asset_source(
            cx,
            action.source_root.clone(),
            action.from_source_path.clone(),
            action.to_source_path.clone(),
        ) {
            error!(
                error = %err,
                source_root = %action.source_root,
                from_source_path = %action.from_source_path,
                to_source_path = %action.to_source_path,
                "failed to handle asset source rename action"
            );
        }
    });
    cx.on_action(|action: &az_editor_ui::actions::ForceReprocessAsset, cx| {
        if let Err(err) =
            force_reprocess_asset(cx, action.source_root.clone(), action.source_path.clone())
        {
            error!(
                error = %err,
                source_root = %action.source_root,
                source_path = %action.source_path,
                "failed to handle asset source force-reprocess action"
            );
        }
    });
}

/// Cancellation and reclamation handle for one Asset Processor event
/// subscription thread.
///
/// Cancellation is [`WatchHandle`]'s: dropping the handle fires the shutdown
/// edge that every await in [`run_asset_processor_event_subscription`] selects
/// against. Unlike the poll-only watchers behind that handle, this thread owns
/// a live IPC event stream, so the subscription also joins it — the stream is
/// torn down before its replacement is retained, not merely told to stop.
pub(crate) struct AssetProcessorEventSubscription {
    shutdown: Option<Arc<WatchHandle>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl AssetProcessorEventSubscription {
    pub(crate) const fn new(shutdown: Arc<WatchHandle>, thread: thread::JoinHandle<()>) -> Self {
        Self {
            shutdown: Some(shutdown),
            thread: Some(thread),
        }
    }
}

impl Drop for AssetProcessorEventSubscription {
    fn drop(&mut self) {
        // Signal before reclaiming. The subscription selects on this edge at
        // every await, so the join meets a thread already unwinding its RPC
        // connection rather than one still parked on the stream.
        drop(self.shutdown.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AssetProcessorEventStreamToken(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AssetProcessorEventStreamCursor {
    token: AssetProcessorEventStreamToken,
    event_watermark: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AssetProcessorSnapshotAdmission {
    Accept,
    Stale,
    Superseded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AssetProcessorEventAdmission {
    Apply { missed_events: bool },
    Ignore,
    Superseded,
}

struct ActiveAssetProcessorEventStream {
    token: AssetProcessorEventStreamToken,
    browser_owner_id: String,
    event_watermark: u64,
    accepting_events: bool,
    subscription: Option<AssetProcessorEventSubscription>,
}

/// Owns exactly one editor Asset Processor stream across attach/reinstall.
///
/// Replacing the active stream drops its subscription, which signals and joins
/// the connection thread before a new subscription is retained. The token and
/// event watermark also fence late install callbacks and DB snapshot results.
#[derive(Default)]
pub(crate) struct EditorAssetProcessorEventStreamOwner {
    next_token: u64,
    active: Option<ActiveAssetProcessorEventStream>,
}

impl EditorAssetProcessorEventStreamOwner {
    pub(crate) fn begin_install(
        &mut self,
        browser_owner_id: String,
    ) -> AssetProcessorEventStreamToken {
        self.next_token = self.next_token.saturating_add(1).max(1);
        let token = AssetProcessorEventStreamToken(self.next_token);
        self.active = Some(ActiveAssetProcessorEventStream {
            token,
            browser_owner_id,
            event_watermark: 0,
            accepting_events: true,
            subscription: None,
        });
        token
    }

    pub(crate) fn is_current(&self, token: AssetProcessorEventStreamToken) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.token == token && active.accepting_events)
    }

    pub(crate) fn retain_subscription(
        &mut self,
        token: AssetProcessorEventStreamToken,
        subscription: AssetProcessorEventSubscription,
    ) -> Result<(), AssetProcessorEventSubscription> {
        let Some(active) = self
            .active
            .as_mut()
            .filter(|active| active.token == token && active.accepting_events)
        else {
            return Err(subscription);
        };
        active.subscription = Some(subscription);
        Ok(())
    }

    pub(crate) fn admit_initial(
        &mut self,
        token: AssetProcessorEventStreamToken,
        event_watermark: u64,
    ) -> bool {
        let Some(active) = self
            .active
            .as_mut()
            .filter(|active| active.token == token && active.accepting_events)
        else {
            return false;
        };
        active.event_watermark = active.event_watermark.max(event_watermark);
        true
    }

    pub(crate) fn admit_event(
        &mut self,
        token: AssetProcessorEventStreamToken,
        event_seq: u64,
    ) -> AssetProcessorEventAdmission {
        let Some(active) = self
            .active
            .as_mut()
            .filter(|active| active.token == token && active.accepting_events)
        else {
            return AssetProcessorEventAdmission::Superseded;
        };
        if event_seq <= active.event_watermark {
            return AssetProcessorEventAdmission::Ignore;
        }
        let missed_events = event_seq > active.event_watermark.saturating_add(1);
        active.event_watermark = event_seq;
        AssetProcessorEventAdmission::Apply { missed_events }
    }

    pub(crate) fn cursor(&self, browser_owner_id: &str) -> Option<AssetProcessorEventStreamCursor> {
        self.active
            .as_ref()
            .filter(|active| active.browser_owner_id == browser_owner_id && active.accepting_events)
            .map(|active| AssetProcessorEventStreamCursor {
                token: active.token,
                event_watermark: active.event_watermark,
            })
    }

    pub(crate) fn snapshot_admission(
        &self,
        browser_owner_id: &str,
        cursor: Option<AssetProcessorEventStreamCursor>,
    ) -> AssetProcessorSnapshotAdmission {
        match (self.active.as_ref(), cursor) {
            (None, None) => AssetProcessorSnapshotAdmission::Accept,
            (Some(active), Some(cursor))
                if active.accepting_events
                    && active.browser_owner_id == browser_owner_id
                    && active.token == cursor.token =>
            {
                if active.event_watermark == cursor.event_watermark {
                    AssetProcessorSnapshotAdmission::Accept
                } else {
                    AssetProcessorSnapshotAdmission::Stale
                }
            }
            _ => AssetProcessorSnapshotAdmission::Superseded,
        }
    }

    pub(crate) fn finish(&mut self, token: AssetProcessorEventStreamToken) -> bool {
        let Some(active) = self.active.as_mut().filter(|active| active.token == token) else {
            return false;
        };
        active.accepting_events = false;
        drop(active.subscription.take());
        true
    }

    pub(crate) fn retire(&mut self) {
        self.active = None;
    }
}

pub(crate) fn spawn_asset_processor_event_subscription(
    session: EditorAttachSession,
) -> EditorResult<(
    AssetProcessorEventSubscription,
    EditorAssetProcessorEventInbox,
)> {
    let (publisher, inbox) = EditorAssetProcessorEventPublisher::new();
    let session_label = session.session_slug.clone();
    let thread_name = format!("az-editor-asset-events-{session_label}");
    let builder = thread::Builder::new().name(thread_name);
    let (shutdown, shutdown_rx) = WatchHandle::channel();
    let thread = builder.spawn(move || {
        if let Err(err) = run_asset_processor_event_subscription(&session, publisher, shutdown_rx) {
            error!(error = %err, "asset processor event subscription stopped");
        }
    })?;
    Ok((
        AssetProcessorEventSubscription::new(shutdown, thread),
        inbox,
    ))
}

pub(crate) fn run_asset_processor_event_subscription(
    session: &EditorAttachSession,
    mut publisher: EditorAssetProcessorEventPublisher,
    mut shutdown: oneshot::Receiver<()>,
) -> EditorResult<()> {
    let runtime = Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|source| EditorError::RpcRuntime { source })?;
    let local = LocalSet::new();
    let result: EditorResult<()> = local.block_on(&runtime, async {
        let connect = AssetProcessorClient::connect_event_stream_for_session(
            &session.services.asset_processor,
            session.session_id,
        );
        let client = tokio::select! {
            result = connect => result?,
            _ = &mut shutdown => return Ok(()),
        };
        let event_sender = publisher.event_sender();
        let subscription = tokio::select! {
            result = client.subscribe_events(event_sender) => result?,
            _ = &mut shutdown => return Ok(()),
        };
        if publisher
            .publish_initial(subscription.initial_health)
            .is_err()
        {
            return Ok(());
        }
        info!(
            session = %session.session_slug,
            "subscribed to asset processor event stream"
        );
        let mut connection_closed = client.connection_closed();
        tokio::select! {
            _ = connection_closed.changed() => Err(EditorError::ServiceDiscovery(
                "asset processor connection closed".to_string(),
            )),
            _ = &mut shutdown => Ok(()),
        }
    });
    if let Err(err) = &result {
        publisher.publish_terminal(err.to_string());
    }
    result
}

pub(crate) fn install_asset_browser_slot(
    cx: &mut App,
    session: EditorAttachSession,
    fence: crate::controller_set::ControllerFence,
) {
    let session_label = session.session_slug.clone();
    let session_id = session.session_id.to_string();
    let Some(stream_token) =
        crate::controller_set::begin_asset_processor_stream(cx, fence, session_id.clone())
    else {
        return;
    };
    let install = AssetBrowserInstall {
        subscription_session: session.clone(),
        session_label,
        session_id,
        initial_roots: workspace_roots_to_ui(&session.workspace_snapshot),
        fence,
        stream_token,
    };

    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "asset-browser-install",
        move || async move { EditorAssetBrowserController::connect_attached(&session).await },
        move |cx, result| match result {
            Ok(controller) => complete_asset_browser_install(cx, controller, install),
            Err(err) => fail_asset_browser_install(cx, &err, install),
        },
    );
}

/// What the deferred asset-browser install carries from the request into
/// whichever completion arm the connect resolves to.
struct AssetBrowserInstall {
    subscription_session: EditorAttachSession,
    session_label: String,
    session_id: String,
    initial_roots: Vec<WorkspaceRootData>,
    fence: crate::controller_set::ControllerFence,
    stream_token: AssetProcessorEventStreamToken,
}

/// Finishes an install whose controller connected: starts the event
/// subscription thread, retains it against the stream token, and publishes the
/// first projection set.
fn complete_asset_browser_install(
    cx: &mut App,
    controller: EditorAssetBrowserController,
    install: AssetBrowserInstall,
) {
    let AssetBrowserInstall {
        subscription_session,
        session_label,
        session_id,
        initial_roots,
        fence,
        stream_token,
    } = install;
    if !crate::controller_set::asset_processor_stream_is_current(cx, fence, stream_token) {
        return;
    }
    let (subscription, inbox) = match spawn_asset_processor_event_subscription(subscription_session)
    {
        Ok(subscription) => subscription,
        Err(err) => {
            let _ = crate::controller_set::finish_asset_processor_stream(cx, fence, stream_token);
            crate::controller_set::fail_controller(cx, fence, err.to_string());
            error!(
                error = %err,
                "failed to spawn asset processor event subscription thread"
            );
            return;
        }
    };
    if let Err(subscription) =
        crate::controller_set::retain_asset_processor_stream(cx, fence, stream_token, subscription)
    {
        drop(subscription);
        return;
    }
    if !crate::controller_set::complete_asset_browser(cx, fence, controller) {
        let _ = crate::controller_set::finish_asset_processor_stream(cx, fence, stream_token);
        return;
    }
    spawn_asset_processor_event_pump(cx, inbox, fence, stream_token, session_id.clone());
    let catalog_products = EditorCatalogProductsStatus::new(
        session_id.clone(),
        DEFAULT_ASSET_PRODUCT_PLATFORM,
        Vec::new(),
    );
    let root_count = initial_roots.len();
    let status = EditorAssetBrowserStatus::new(session_id.clone(), initial_roots, Vec::new(), None);
    let builder_catalog = EditorAssetBuilderCatalog::new(Vec::new(), Vec::new());
    let activity = EditorAssetProcessorActivity::new(
        session_id,
        ServiceHealthStateData::Starting,
        "health",
        "awaiting asset processor event subscription",
    );
    cx.set_global(builder_catalog);
    cx.set_global(catalog_products);
    publish_asset_browser_status(cx, status);
    cx.set_global(activity);
    cx.refresh_windows();

    info!(
        session = %session_label,
        root_count,
        product_platform = DEFAULT_ASSET_PRODUCT_PLATFORM,
        "installed asset browser controller; event subscription publishes initial asset-processor health"
    );
}

/// Reports an install whose controller never connected: releases the stream
/// token, fails the controller slot, and publishes the error to the browser.
fn fail_asset_browser_install(cx: &mut App, error: &EditorError, install: AssetBrowserInstall) {
    if !crate::controller_set::finish_asset_processor_stream(
        cx,
        install.fence,
        install.stream_token,
    ) {
        return;
    }
    let message = error.to_string();
    crate::controller_set::fail_controller(cx, install.fence, message.clone());
    publish_asset_browser_status(
        cx,
        EditorAssetBrowserStatus::error(install.session_id, message),
    );
    cx.refresh_windows();
    error!(
        error = %error,
        session = %install.session_label,
        "failed to connect asset browser controller"
    );
}

pub(crate) fn spawn_asset_processor_event_pump(
    cx: &App,
    inbox: EditorAssetProcessorEventInbox,
    fence: crate::controller_set::ControllerFence,
    stream_token: AssetProcessorEventStreamToken,
    browser_owner_id: String,
) {
    cx.spawn(async move |cx| {
        let EditorAssetProcessorEventInbox {
            mut initial,
            mut events,
            mut terminal,
        } = inbox;
        let health = tokio::select! {
            biased;
            terminal = &mut terminal => {
                if let Ok(detail) = terminal {
                    publish_asset_processor_stream_terminal(
                        cx,
                        fence,
                        stream_token,
                        &browser_owner_id,
                        detail,
                    );
                }
                return;
            },
            initial = &mut initial => match initial {
                Ok(health) => health,
                Err(_) => return,
            },
        };
        let session_id = browser_owner_id.clone();
        let initial_event_watermark = health.last_event_seq;
        let admitted = cx.update(move |cx| {
            if !crate::controller_set::admit_asset_processor_stream_initial(
                cx,
                fence,
                stream_token,
                initial_event_watermark,
            ) {
                return false;
            }
            cx.set_global(asset_processor_activity_to_ui(&session_id, health));
            if let Some(session) = cx.try_global::<EditorAttachSession>().cloned() {
                spawn_asset_browser_snapshot_refresh(cx, session, fence, "asset-stream-initial");
            }
            cx.refresh_windows();
            true
        });
        if !admitted {
            return;
        }

        loop {
            tokio::select! {
                biased;
                terminal = &mut terminal => {
                    if let Ok(detail) = terminal {
                        publish_asset_processor_stream_terminal(
                        cx,
                        fence,
                            stream_token,
                            &browser_owner_id,
                            detail,
                        );
                    }
                    return;
                }
                event = events.recv() => {
                    let Some(event) = event else {
                        return;
                    };
                    if !apply_asset_processor_stream_event(
                        cx,
                        event,
                        fence,
                        stream_token,
                        &browser_owner_id,
                    ) {
                        return;
                    }
                }
            }
        }
    })
    .detach();
}

/// Admits one stream event and applies it, closing an admission gap with a
/// full snapshot refresh. Returns `false` once the stream is superseded and the
/// pump must stop.
fn apply_asset_processor_stream_event(
    cx: &gpui::AsyncApp,
    event: AssetProcessorEvent,
    fence: crate::controller_set::ControllerFence,
    stream_token: AssetProcessorEventStreamToken,
    browser_owner_id: &str,
) -> bool {
    let event_seq = event.seq;
    let admission = cx.update(move |cx| {
        crate::controller_set::admit_asset_processor_stream_event(
            cx,
            fence,
            stream_token,
            event_seq,
        )
    });
    match admission {
        AssetProcessorEventAdmission::Apply { missed_events } => {
            publish_asset_processor_event_update(cx, fence, browser_owner_id, event);
            if missed_events {
                let browser_owner_id = browser_owner_id.to_owned();
                let () = cx.update(move |cx| {
                    let Some(session) = cx
                        .try_global::<EditorAttachSession>()
                        .filter(|session| session.session_id.to_string() == browser_owner_id)
                        .cloned()
                    else {
                        return;
                    };
                    spawn_asset_browser_snapshot_refresh(cx, session, fence, "asset-stream-gap");
                });
            }
            true
        }
        AssetProcessorEventAdmission::Ignore => true,
        AssetProcessorEventAdmission::Superseded => false,
    }
}

fn publish_asset_processor_stream_terminal(
    cx: &gpui::AsyncApp,
    fence: crate::controller_set::ControllerFence,
    stream_token: AssetProcessorEventStreamToken,
    browser_owner_id: &str,
    detail: String,
) {
    let session_id = browser_owner_id.to_owned();
    let () = cx.update(move |cx| {
        if !crate::controller_set::finish_asset_processor_stream(cx, fence, stream_token) {
            return;
        }
        cx.set_global(EditorAssetProcessorActivity::unavailable(
            session_id, detail,
        ));
        cx.refresh_windows();
    });
}

/// Queues a refresh of the asset browser against the current session.
///
/// # Errors
///
/// Returns [`EditorError::ControllerInstalling`],
/// [`EditorError::ControllerFailed`], or
/// [`EditorError::ControllerUnavailable`] when the asset browser controller
/// slot is not ready. The refresh itself runs on the RPC runtime and reports
/// its own failures through the published browser status, not through this
/// return value.
pub fn refresh_asset_browser_status(cx: &mut App) -> EditorResult<()> {
    let attached = crate::controller_set::asset_browser_controller(cx)?;
    if let Some(session) = cx.try_global::<EditorAttachSession>().cloned() {
        spawn_asset_browser_snapshot_refresh(cx, session, attached.fence, "manual");
        return Ok(());
    }
    spawn_asset_browser_refresh(cx, attached.fence, attached.controller);
    Ok(())
}

pub fn refresh_asset_browser_status_if_available(cx: &mut App) {
    let Ok(attached) = crate::controller_set::asset_browser_controller(cx) else {
        return;
    };
    if let Some(session) = cx.try_global::<EditorAttachSession>().cloned() {
        spawn_asset_browser_snapshot_refresh(cx, session, attached.fence, "manual");
        return;
    }
    spawn_asset_browser_refresh(cx, attached.fence, attached.controller);
}

/// Queues a full reconcile of the session's asset source roots.
///
/// # Errors
///
/// Returns [`EditorError::ControllerInstalling`],
/// [`EditorError::ControllerFailed`], or
/// [`EditorError::ControllerUnavailable`] when the asset browser controller
/// slot is not ready. The reconcile RPC itself runs asynchronously and reports
/// its own failures through the published browser status.
pub fn scan_asset_sources(cx: &mut App) -> EditorResult<()> {
    let attached = crate::controller_set::asset_browser_controller(cx)?;
    spawn_asset_source_reconcile(cx, attached.fence, attached.controller);
    Ok(())
}

/// Queues a catalog products refresh for `platform`.
///
/// # Errors
///
/// Returns [`EditorError::ControllerInstalling`],
/// [`EditorError::ControllerFailed`], or
/// [`EditorError::ControllerUnavailable`] when the asset browser controller
/// slot is not ready. `platform` is normalized by the spawned refresh, which
/// reports its own failures through the published catalog status.
pub fn refresh_catalog_products(cx: &mut App, platform: &str) -> EditorResult<()> {
    let attached = crate::controller_set::asset_browser_controller(cx)?;
    spawn_catalog_products_refresh(cx, attached.fence, attached.controller, platform);
    Ok(())
}

/// Queues the next page of asset browser entries, if one is published.
///
/// # Errors
///
/// Returns [`EditorError::ControllerInstalling`],
/// [`EditorError::ControllerFailed`], or
/// [`EditorError::ControllerUnavailable`] when the asset browser controller
/// slot is not ready. A published status carrying no next-page cursor is not
/// an error; it returns `Ok` having queued nothing.
pub fn load_more_asset_browser_status(cx: &mut App) -> EditorResult<()> {
    let attached = crate::controller_set::asset_browser_controller(cx)?;
    let Some(after_entry_id) = cx
        .try_global::<EditorAssetBrowserStatus>()
        .and_then(|status| status.next_after_entry_id)
    else {
        return Ok(());
    };

    spawn_asset_browser_load_more(cx, attached.fence, attached.controller, after_entry_id);
    Ok(())
}

/// Queues an inspection of one asset job, or of a specific attempt.
///
/// # Errors
///
/// Returns [`EditorError::ControllerInstalling`],
/// [`EditorError::ControllerFailed`], or
/// [`EditorError::ControllerUnavailable`] when the asset browser controller
/// slot is not ready. The inspection RPC itself runs asynchronously and reports
/// its own failures through the published inspection state.
pub fn inspect_job(cx: &mut App, job_id: i64, attempt_id: Option<i64>) -> EditorResult<()> {
    let attached = crate::controller_set::asset_browser_controller(cx)?;
    spawn_job_inspection(cx, attached.fence, attached.controller, job_id, attempt_id);
    Ok(())
}

/// Validates a create request against the published builder catalog, then
/// queues the create.
///
/// # Errors
///
/// Returns [`EditorError::ControllerInstalling`],
/// [`EditorError::ControllerFailed`], or
/// [`EditorError::ControllerUnavailable`] when the asset browser controller
/// slot is not ready; [`EditorError::MissingAssetBuilderCatalog`] when no
/// builder catalog has been published yet;
/// [`EditorError::AssetSourceWorkflowNotPublished`] when the catalog publishes
/// no creatable file workflow for `schema_type` under `source_root`; and
/// [`EditorError::InvalidAssetSourceCreateRequest`] when `source_path` is not a
/// canonical asset-db relative path or its extension does not match the
/// workflow's.
pub fn create_asset_source_file(
    cx: &mut App,
    source_root: String,
    source_path: String,
    schema_type: String,
) -> EditorResult<()> {
    let attached = crate::controller_set::asset_browser_controller(cx)?;
    let catalog = cx
        .try_global::<EditorAssetBuilderCatalog>()
        .ok_or(EditorError::MissingAssetBuilderCatalog)?;
    ensure_asset_source_file_workflow_is_published(
        catalog,
        &source_root,
        &source_path,
        &schema_type,
    )?;
    spawn_asset_source_file_create(
        cx,
        attached.fence,
        attached.controller,
        source_root,
        source_path,
        schema_type,
    );
    Ok(())
}

/// Publishes a loading dependents preview and queues the lookup behind it.
///
/// # Errors
///
/// Returns [`EditorError::InvalidAssetSourceCreateRequest`] if `source_path` is
/// not a canonical asset-db relative path, or
/// [`EditorError::ControllerInstalling`], [`EditorError::ControllerFailed`], or
/// [`EditorError::ControllerUnavailable`] when the asset browser controller
/// slot is not ready.
pub fn preview_delete_asset_source(
    cx: &mut App,
    source_root: String,
    source_path: String,
) -> EditorResult<()> {
    validate_asset_db_relative_path(&source_path).ok_or_else(|| {
        EditorError::InvalidAssetSourceCreateRequest {
            source_path: source_path.clone(),
            message: "source path must be a canonical asset-db relative path".to_string(),
        }
    })?;
    let attached = crate::controller_set::asset_browser_controller(cx)?;
    cx.set_global(EditorAssetSourceDependentsPreview::loading(
        attached.controller.session_id().to_string(),
        source_root.clone(),
        source_path.clone(),
    ));
    cx.refresh_windows();
    spawn_asset_source_dependents_preview(
        cx,
        attached.fence,
        attached.controller,
        source_root,
        source_path,
    );
    Ok(())
}

/// Queues the deletion of one asset source file.
///
/// # Errors
///
/// Returns [`EditorError::InvalidAssetSourceCreateRequest`] if `source_path` is
/// not a canonical asset-db relative path, or
/// [`EditorError::ControllerInstalling`], [`EditorError::ControllerFailed`], or
/// [`EditorError::ControllerUnavailable`] when the asset browser controller
/// slot is not ready.
pub fn delete_asset_source(
    cx: &mut App,
    source_root: String,
    source_path: String,
) -> EditorResult<()> {
    validate_asset_db_relative_path(&source_path).ok_or_else(|| {
        EditorError::InvalidAssetSourceCreateRequest {
            source_path: source_path.clone(),
            message: "source path must be a canonical asset-db relative path".to_string(),
        }
    })?;
    let attached = crate::controller_set::asset_browser_controller(cx)?;
    spawn_asset_source_delete(
        cx,
        attached.fence,
        attached.controller,
        source_root,
        source_path,
    );
    Ok(())
}

/// Queues the rename of one asset source file within its root.
///
/// # Errors
///
/// Returns [`EditorError::InvalidAssetSourceCreateRequest`] if either path is
/// not a canonical asset-db relative path, or if the two paths are equal, and
/// [`EditorError::ControllerInstalling`], [`EditorError::ControllerFailed`], or
/// [`EditorError::ControllerUnavailable`] when the asset browser controller
/// slot is not ready.
pub fn rename_asset_source(
    cx: &mut App,
    source_root: String,
    from_source_path: String,
    to_source_path: String,
) -> EditorResult<()> {
    validate_asset_db_relative_path(&from_source_path).ok_or_else(|| {
        EditorError::InvalidAssetSourceCreateRequest {
            source_path: from_source_path.clone(),
            message: "source path must be a canonical asset-db relative path".to_string(),
        }
    })?;
    validate_asset_db_relative_path(&to_source_path).ok_or_else(|| {
        EditorError::InvalidAssetSourceCreateRequest {
            source_path: to_source_path.clone(),
            message: "source path must be a canonical asset-db relative path".to_string(),
        }
    })?;
    if from_source_path == to_source_path {
        return Err(EditorError::InvalidAssetSourceCreateRequest {
            source_path: to_source_path,
            message: "new source path must differ from the current source path".to_string(),
        });
    }
    let attached = crate::controller_set::asset_browser_controller(cx)?;
    spawn_asset_source_rename(
        cx,
        attached.fence,
        attached.controller,
        source_root,
        from_source_path,
        to_source_path,
    );
    Ok(())
}

/// Queues a forced reprocess of every builder job for one asset source.
///
/// # Errors
///
/// Returns [`EditorError::InvalidAssetSourceCreateRequest`] if `source_path` is
/// not a canonical asset-db relative path, or
/// [`EditorError::ControllerInstalling`], [`EditorError::ControllerFailed`], or
/// [`EditorError::ControllerUnavailable`] when the asset browser controller
/// slot is not ready.
pub fn force_reprocess_asset(
    cx: &mut App,
    source_root: String,
    source_path: String,
) -> EditorResult<()> {
    validate_asset_db_relative_path(&source_path).ok_or_else(|| {
        EditorError::InvalidAssetSourceCreateRequest {
            source_path: source_path.clone(),
            message: "source path must be a canonical asset-db relative path".to_string(),
        }
    })?;
    let attached = crate::controller_set::asset_browser_controller(cx)?;
    spawn_asset_force_reprocess(
        cx,
        attached.fence,
        attached.controller,
        source_root,
        source_path,
    );
    Ok(())
}

pub(crate) fn ensure_asset_source_file_workflow_is_published(
    catalog: &EditorAssetBuilderCatalog,
    source_root: &str,
    source_path: &str,
    schema_type: &str,
) -> EditorResult<()> {
    let Some(workflow) = catalog.source_schemas.iter().find_map(|source_schema| {
        if source_schema.schema_type != schema_type {
            return None;
        }
        let AssetSourceSchemaAuthoringData::File { workflow } = &source_schema.authoring else {
            return None;
        };
        (workflow.can_create && workflow.source_root == source_root).then_some(workflow)
    }) else {
        return Err(EditorError::AssetSourceWorkflowNotPublished {
            schema_type: schema_type.to_string(),
            source_root: source_root.to_string(),
        });
    };

    validate_asset_db_relative_path(source_path).ok_or_else(|| {
        EditorError::InvalidAssetSourceCreateRequest {
            source_path: source_path.to_string(),
            message: "source path must be a canonical asset-db relative path".to_string(),
        }
    })?;
    if source_path_matches_extensions(source_path, &workflow.extensions) {
        return Ok(());
    }

    Err(EditorError::InvalidAssetSourceCreateRequest {
        source_path: source_path.to_string(),
        message: format!(
            "source path extension must match {}",
            az_editor_ui::panels::extension_hint(&workflow.extensions)
        ),
    })
}

pub(crate) fn spawn_asset_browser_refresh(
    cx: &mut App,
    fence: crate::controller_set::ControllerFence,
    controller: EditorAssetBrowserController,
) {
    let session_id = controller.session_id().to_string();
    let publish_session_id = session_id.clone();
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "asset-browser-refresh",
        move || {
            let worker_session_id = session_id.clone();
            async move {
                let controller = controller.refresh_client_from_supervisor().await?;
                let status = controller.load_first_page().await?;
                let builder_catalog = controller.load_builder_catalog().await?;
                let catalog_products = EditorCatalogProductsStatus::new(
                    worker_session_id,
                    DEFAULT_ASSET_PRODUCT_PLATFORM,
                    Vec::new(),
                );
                Ok((controller, status, builder_catalog, catalog_products))
            }
        },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok((controller, status, builder_catalog, catalog_products)) => {
                    let entry_count = status.entries.len();
                    let builder_count = builder_catalog.builders.len();
                    let product_controller = controller.clone();
                    let _ = crate::controller_set::replace_asset_browser(cx, fence, controller);
                    cx.set_global(builder_catalog);
                    cx.set_global(catalog_products);
                    publish_asset_browser_status(cx, status);
                    spawn_catalog_products_refresh(
                        cx,
                        fence,
                        product_controller,
                        DEFAULT_ASSET_PRODUCT_PLATFORM,
                    );
                    cx.refresh_windows();
                    info!(
                        session_id = %publish_session_id,
                        entry_count,
                        builder_count,
                        product_platform = DEFAULT_ASSET_PRODUCT_PLATFORM,
                        "refreshed asset browser status and queued catalog refresh"
                    );
                }
                Err(err) => {
                    let message = err.to_string();
                    error!(
                        error = %err,
                        session_id = %publish_session_id,
                        "failed to refresh asset browser status"
                    );
                    publish_asset_browser_error(cx, &publish_session_id, message);
                }
            }
        },
    );
}
