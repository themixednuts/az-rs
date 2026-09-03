//! Background spawn/run workers for the asset browser and job inspection.

use super::{
    ASSET_BROWSER_SNAPSHOT_REFRESH_TIMEOUT, ASSET_PROCESSOR_SERVICE_NAME, App,
    AssetProcessorSnapshotAdmission, AssetRootScope, ConsoleState, DEFAULT_ASSET_PRODUCT_PLATFORM,
    EditorAssetBrowserController, EditorAssetBrowserStatus, EditorAssetBuilderCatalog,
    EditorAssetSourceDependentsPreview, EditorAttachSession, EditorCatalogProductsStatus,
    EditorError, EditorJobInspection, HashMap, HashSet, InspectJobSelector, LogLevel,
    UnboundedSender, append_asset_browser_status, debug, error, info, job_inspection_console_lines,
    normalize_catalog_product_platform, publish_asset_browser_status, source_dependents_to_ui,
    thread, unbounded_channel,
};

#[derive(Default)]
pub struct EditorAssetBrowserSnapshotRefreshState {
    in_flight: HashSet<String>,
    pending: HashMap<String, PendingAssetBrowserSnapshotRefresh>,
}

pub enum AssetBrowserSnapshotRefreshRequest {
    Start,
    Coalesced,
}

pub struct PendingAssetBrowserSnapshotRefresh {
    pub(crate) session: EditorAttachSession,
    pub(crate) fence: crate::controller_set::ControllerFence,
    pub(crate) reason: &'static str,
}

impl EditorAssetBrowserSnapshotRefreshState {
    pub(crate) fn request(
        &mut self,
        session: &EditorAttachSession,
        fence: crate::controller_set::ControllerFence,
        reason: &'static str,
    ) -> AssetBrowserSnapshotRefreshRequest {
        let session_id = session.session_id.to_string();
        if self.in_flight.insert(session_id.clone()) {
            return AssetBrowserSnapshotRefreshRequest::Start;
        }

        self.pending.insert(
            session_id,
            PendingAssetBrowserSnapshotRefresh {
                session: session.clone(),
                fence,
                reason,
            },
        );
        AssetBrowserSnapshotRefreshRequest::Coalesced
    }

    pub(crate) fn complete(
        &mut self,
        session_id: &str,
    ) -> Option<PendingAssetBrowserSnapshotRefresh> {
        self.in_flight.remove(session_id);
        self.pending.remove(session_id)
    }
}

pub struct AssetBrowserSnapshot {
    status: EditorAssetBrowserStatus,
    builder_catalog: EditorAssetBuilderCatalog,
    catalog_products: EditorCatalogProductsStatus,
}

pub fn spawn_asset_browser_snapshot_refresh(
    cx: &mut App,
    session: EditorAttachSession,
    fence: crate::controller_set::ControllerFence,
    reason: &'static str,
) {
    let session_id = session.session_id.to_string();
    let Some(request) =
        crate::controller_set::request_asset_browser_snapshot_refresh(cx, fence, &session, reason)
    else {
        return;
    };
    match request {
        AssetBrowserSnapshotRefreshRequest::Start => {}
        AssetBrowserSnapshotRefreshRequest::Coalesced => {
            debug!(
                session_id = %session_id,
                reason,
                "coalesced asset browser snapshot refresh"
            );
            return;
        }
    }
    let stream_cursor = crate::controller_set::asset_processor_event_stream_cursor(cx, &session_id);

    let (refresh_tx, mut refresh_rx) = unbounded_channel::<Result<AssetBrowserSnapshot, String>>();
    let thread_name = format!(
        "az-editor-asset-refresh-{}-{}",
        session.session_slug, reason
    );
    let context = SnapshotRefreshContext {
        session: session.clone(),
        session_id: session_id.clone(),
        fence,
        reason,
    };
    match thread::Builder::new()
        .name(thread_name)
        .spawn(move || run_asset_browser_snapshot_refresh(session, reason, &refresh_tx))
    {
        Ok(_thread) => {}
        Err(err) => {
            let _ = crate::controller_set::complete_asset_browser_snapshot_refresh(
                cx,
                fence,
                &session_id,
            );
            publish_asset_browser_error(
                cx,
                &session_id,
                format!("failed to start asset browser refresh thread: {err}"),
            );
            return;
        }
    }

    cx.spawn(async move |cx| {
        let result = refresh_rx.recv().await;
        let admission_session_id = context.session_id.clone();
        let admission = cx.update(move |cx| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return AssetProcessorSnapshotAdmission::Superseded;
            }
            crate::controller_set::asset_processor_snapshot_admission(
                cx,
                &admission_session_id,
                stream_cursor,
            )
        });
        if admission == AssetProcessorSnapshotAdmission::Accept {
            publish_snapshot_outcome(cx, &context, result);
        } else {
            finish_rejected_snapshot(cx, &context, admission);
        }
    })
    .detach();
}

/// Identity and fencing for one in-flight asset-browser snapshot refresh.
struct SnapshotRefreshContext {
    session: EditorAttachSession,
    session_id: String,
    fence: crate::controller_set::ControllerFence,
    reason: &'static str,
}

/// Starts a refresh that was coalesced behind the one just finished.
fn respawn_pending_snapshot_refresh(
    cx: &gpui::AsyncApp,
    pending: Option<PendingAssetBrowserSnapshotRefresh>,
) {
    let Some(pending) = pending else {
        return;
    };
    let () = cx.update(move |cx| {
        spawn_asset_browser_snapshot_refresh(cx, pending.session, pending.fence, pending.reason);
    });
}

/// Releases the in-flight slot and, while the fence still holds, publishes
/// through `publish`. Returns whatever refresh was coalesced behind this one.
fn complete_and_publish(
    cx: &gpui::AsyncApp,
    context: &SnapshotRefreshContext,
    publish: impl FnOnce(&mut App, &str),
) -> Option<PendingAssetBrowserSnapshotRefresh> {
    let session_id = context.session_id.clone();
    let fence = context.fence;
    cx.update(move |cx| {
        let pending =
            crate::controller_set::complete_asset_browser_snapshot_refresh(cx, fence, &session_id);
        if !crate::controller_set::is_current_fence(cx, fence) {
            return None;
        }
        publish(cx, &session_id);
        pending
    })
}

/// Retires a snapshot that lost its race: releases the in-flight slot, and
/// re-queues a catch-up refresh when the loss was only a stale event cursor.
fn finish_rejected_snapshot(
    cx: &gpui::AsyncApp,
    context: &SnapshotRefreshContext,
    admission: AssetProcessorSnapshotAdmission,
) {
    let session_id = context.session_id.clone();
    let fence = context.fence;
    let pending = cx.update(move |cx| {
        crate::controller_set::complete_asset_browser_snapshot_refresh(cx, fence, &session_id)
    });
    let retry = pending.or_else(|| {
        (admission == AssetProcessorSnapshotAdmission::Stale).then(|| {
            PendingAssetBrowserSnapshotRefresh {
                session: context.session.clone(),
                fence,
                reason: "asset-stream-catch-up",
            }
        })
    });
    respawn_pending_snapshot_refresh(cx, retry);
}

/// Publishes the worker's result — a snapshot, an error, or a worker that ended
/// without either — then starts any refresh coalesced behind it.
fn publish_snapshot_outcome(
    cx: &gpui::AsyncApp,
    context: &SnapshotRefreshContext,
    result: Option<Result<AssetBrowserSnapshot, String>>,
) {
    let session_id = context.session_id.as_str();
    let reason = context.reason;
    let Some(result) = result else {
        let pending = complete_and_publish(cx, context, |cx, session_id| {
            publish_asset_browser_error(
                cx,
                session_id,
                "asset browser refresh worker ended before publishing a result".to_string(),
            );
        });
        respawn_pending_snapshot_refresh(cx, pending);
        return;
    };
    match result {
        Ok(snapshot) => {
            let entry_count = snapshot.status.entries.len();
            let builder_count = snapshot.builder_catalog.builders.len();
            let product_count = snapshot.catalog_products.entries.len();
            let pending = complete_and_publish(cx, context, move |cx, _| {
                let status = snapshot.status;
                cx.set_global(snapshot.builder_catalog);
                cx.set_global(snapshot.catalog_products);
                publish_asset_browser_status(cx, status);
                cx.refresh_windows();
            });
            info!(
                session_id = %session_id,
                reason,
                entry_count,
                builder_count,
                product_count,
                "refreshed asset browser snapshot"
            );
            respawn_pending_snapshot_refresh(cx, pending);
        }
        Err(message) => {
            let log_message = message.clone();
            let pending = complete_and_publish(cx, context, move |cx, session_id| {
                publish_asset_browser_error(cx, session_id, message);
            });
            error!(
                session_id = %session_id,
                reason,
                error = %log_message,
                "failed to refresh asset browser snapshot"
            );
            respawn_pending_snapshot_refresh(cx, pending);
        }
    }
}

pub fn run_asset_browser_snapshot_refresh(
    session: EditorAttachSession,
    reason: &'static str,
    refresh_tx: &UnboundedSender<Result<AssetBrowserSnapshot, String>>,
) {
    let session_id = session.session_id.to_string();
    let result = crate::rpc_runtime::block_on_editor_rpc(async move {
        let refresh = async {
            let controller = EditorAssetBrowserController::connect_attached(&session).await?;
            let status = controller.load_first_page().await?;
            let builder_catalog = controller.load_builder_catalog().await?;
            let catalog_products = load_catalog_products_status_or_error(
                &controller,
                &session_id,
                DEFAULT_ASSET_PRODUCT_PLATFORM.to_string(),
            )
            .await;
            Ok(AssetBrowserSnapshot {
                status,
                builder_catalog,
                catalog_products,
            })
        };
        tokio::time::timeout(ASSET_BROWSER_SNAPSHOT_REFRESH_TIMEOUT, refresh)
            .await
            .unwrap_or_else(|_| {
                Err(EditorError::ServiceDiscovery(format!(
                    "asset browser {reason} refresh did not answer within {} ms",
                    ASSET_BROWSER_SNAPSHOT_REFRESH_TIMEOUT.as_millis()
                )))
            })
    });
    let _ = refresh_tx.send(result.map_err(|err| err.to_string()));
}

pub fn spawn_catalog_products_refresh(
    cx: &mut App,
    fence: crate::controller_set::ControllerFence,
    controller: EditorAssetBrowserController,
    platform: &str,
) {
    let session_id = controller.session_id().to_string();
    let platform = normalize_catalog_product_platform(platform);
    let publish_session_id = session_id.clone();
    let publish_platform = platform.clone();
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "asset-catalog-products-refresh",
        move || {
            let worker_session_id = session_id.clone();
            let worker_platform = platform.clone();
            async move {
                let controller = controller.refresh_client_from_supervisor().await?;
                let products = load_catalog_products_status_or_error(
                    &controller,
                    &worker_session_id,
                    worker_platform,
                )
                .await;
                Ok((controller, products))
            }
        },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok((controller, products)) => {
                    let product_count = products.entries.len();
                    let product_failed = products.status_error.is_some();
                    let _ = crate::controller_set::replace_asset_browser(cx, fence, controller);
                    cx.set_global(products);
                    cx.refresh_windows();
                    if product_failed {
                        error!(
                            session_id = %publish_session_id,
                            platform = %publish_platform,
                            "failed to refresh catalog products"
                        );
                    } else {
                        info!(
                            session_id = %publish_session_id,
                            platform = %publish_platform,
                            product_count,
                            "refreshed catalog products"
                        );
                    }
                }
                Err(err) => {
                    let message = err.to_string();
                    let products = EditorCatalogProductsStatus::error(
                        publish_session_id.clone(),
                        publish_platform.clone(),
                        message.clone(),
                    );
                    error!(
                        error = %err,
                        session_id = %publish_session_id,
                        platform = %publish_platform,
                        "failed to refresh asset-processor client before catalog products refresh"
                    );
                    cx.set_global(products);
                    publish_console_log(
                        cx,
                        LogLevel::Error,
                        ASSET_PROCESSOR_SERVICE_NAME,
                        format!("catalog product refresh failed before request: {message}"),
                    );
                    cx.refresh_windows();
                }
            }
        },
    );
}

#[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
async fn load_catalog_products_status_or_error(
    controller: &EditorAssetBrowserController,
    session_id: &str,
    platform: String,
) -> EditorCatalogProductsStatus {
    match controller.load_catalog_products(platform.clone()).await {
        Ok(products) => products,
        Err(err) => {
            error!(
                error = %err,
                session_id = %session_id,
                platform = %platform,
                "failed to load catalog products"
            );
            EditorCatalogProductsStatus::error(session_id.to_string(), platform, err.to_string())
        }
    }
}

pub fn spawn_asset_source_reconcile(
    cx: &mut App,
    fence: crate::controller_set::ControllerFence,
    controller: EditorAssetBrowserController,
) {
    let session_id = controller.session_id().to_string();
    let publish_session_id = session_id.clone();
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "asset-source-reconcile",
        move || {
            let worker_session_id = session_id.clone();
            async move {
                let controller = controller.refresh_client_from_supervisor().await?;
                let result = controller
                    .reconcile_asset_sources(AssetRootScope::All)
                    .await?;
                let refreshed_status = match controller.load_first_page().await {
                    Ok(status) => Some(status),
                    Err(err) => {
                        error!(
                            error = %err,
                            session_id = %worker_session_id,
                            "failed to refresh asset browser status after asset source scan"
                        );
                        None
                    }
                };
                let refreshed_catalog = match controller.load_builder_catalog().await {
                    Ok(catalog) => Some(catalog),
                    Err(err) => {
                        error!(
                            error = %err,
                            session_id = %worker_session_id,
                            "failed to refresh asset builder catalog after asset source scan"
                        );
                        None
                    }
                };
                Ok((controller, result, refreshed_status, refreshed_catalog))
            }
        },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok((controller, result, refreshed_status, refreshed_catalog)) => {
                    let product_controller = controller.clone();
                    let _ = crate::controller_set::replace_asset_browser(cx, fence, controller);
                    if let Some(catalog) = refreshed_catalog {
                        cx.set_global(catalog);
                    }
                    if let Some(status) = refreshed_status {
                        publish_asset_browser_status(cx, status);
                    }
                    spawn_catalog_products_refresh(
                        cx,
                        fence,
                        product_controller,
                        DEFAULT_ASSET_PRODUCT_PLATFORM,
                    );
                    publish_console_log(
                        cx,
                        LogLevel::Info,
                        ASSET_PROCESSOR_SERVICE_NAME,
                        format!(
                            "scanned {} source roots; recorded {}, deleted {}",
                            result.source_root_count,
                            result.recorded_source_asset_count,
                            result.deleted_source_asset_count
                        ),
                    );
                    cx.refresh_windows();
                    info!(
                        session_id = %publish_session_id,
                        source_root_count = result.source_root_count,
                        recorded_source_asset_count = result.recorded_source_asset_count,
                        deleted_source_asset_count = result.deleted_source_asset_count,
                        "reconciled asset source roots"
                    );
                }
                Err(err) => {
                    let message = err.to_string();
                    error!(
                        error = %err,
                        session_id = %publish_session_id,
                        "failed to reconcile asset source roots"
                    );
                    publish_console_log(
                        cx,
                        LogLevel::Error,
                        ASSET_PROCESSOR_SERVICE_NAME,
                        format!("asset source scan failed: {message}"),
                    );
                    publish_asset_browser_error(cx, &publish_session_id, message);
                }
            }
        },
    );
}

pub fn spawn_job_inspection(
    cx: &mut App,
    fence: crate::controller_set::ControllerFence,
    controller: EditorAssetBrowserController,
    job_id: i64,
    attempt_id: Option<i64>,
) {
    let session_id = controller.session_id().to_string();
    let publish_session_id = session_id;
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "asset-job-inspection",
        move || async move {
            let controller = controller.refresh_client_from_supervisor().await?;
            let selector =
                attempt_id.map_or(InspectJobSelector::Job(job_id), InspectJobSelector::Attempt);
            let inspection = controller.inspect_job(selector).await?;
            let console_lines = job_inspection_console_lines(&inspection);
            Ok((controller, inspection, console_lines))
        },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok((controller, inspection, console_lines)) => {
                    let _ = crate::controller_set::replace_asset_browser(cx, fence, controller);
                    for (level, line) in console_lines {
                        publish_console_log(cx, level, ASSET_PROCESSOR_SERVICE_NAME, line);
                    }
                    cx.set_global(inspection);
                    cx.refresh_windows();
                }
                Err(err) => {
                    let message = err.to_string();
                    let console_message = format!("job {job_id} inspection failed: {message}");
                    error!(
                        error = %err,
                        session_id = %publish_session_id,
                        job_id,
                        attempt_id = ?attempt_id,
                        "failed to inspect job"
                    );
                    publish_console_log(
                        cx,
                        LogLevel::Error,
                        ASSET_PROCESSOR_SERVICE_NAME,
                        console_message,
                    );
                    cx.set_global(EditorJobInspection::error(job_id, message));
                    cx.refresh_windows();
                }
            }
        },
    );
}

pub fn spawn_asset_source_file_create(
    cx: &mut App,
    fence: crate::controller_set::ControllerFence,
    controller: EditorAssetBrowserController,
    source_root: String,
    source_path: String,
    schema_type: String,
) {
    let session_id = controller.session_id().to_string();
    let publish_session_id = session_id.clone();
    let publish_source_path = source_path.clone();
    let publish_schema_type = schema_type.clone();
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "asset-source-create",
        move || {
            let worker_session_id = session_id.clone();
            async move {
                let controller = controller.refresh_client_from_supervisor().await?;
                let result = controller
                    .create_source_file_from_default_template(
                        source_root,
                        source_path,
                        schema_type.clone(),
                    )
                    .await?;
                let created_source_path = result.record.entry.source_path.clone();
                let created_schema_type = result
                    .record
                    .entry
                    .schema_type
                    .clone()
                    .unwrap_or(schema_type);
                let created_asset_guid = result.record.asset_guid;
                let refreshed_status = match controller.load_first_page().await {
                    Ok(status) => Some(status),
                    Err(err) => {
                        error!(
                            error = %err,
                            session_id = %worker_session_id,
                            source_path = %created_source_path,
                            "failed to refresh asset browser status after asset source create"
                        );
                        None
                    }
                };
                Ok((
                    controller,
                    refreshed_status,
                    created_source_path,
                    created_schema_type,
                    created_asset_guid,
                ))
            }
        },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok((
                    controller,
                    refreshed_status,
                    created_source_path,
                    created_schema_type,
                    created_asset_guid,
                )) => {
                    publish_asset_source_mutation(
                        cx,
                        fence,
                        controller,
                        refreshed_status,
                        format!(
                            "created asset source `{created_source_path}` as `{created_schema_type}` ({created_asset_guid})"
                        ),
                    );
                    info!(
                        session_id = %publish_session_id,
                        source_path = %created_source_path,
                        schema_type = %created_schema_type,
                        asset_guid = %created_asset_guid,
                        "created asset source file; waiting for asset processor event"
                    );
                }
                Err(err) => {
                    let message = err.to_string();
                    let console_message =
                        format!("asset source `{publish_source_path}` create failed: {message}");
                    error!(
                        error = %err,
                        session_id = %publish_session_id,
                        source_path = %publish_source_path,
                        schema_type = %publish_schema_type,
                        "failed to create asset source file"
                    );
                    publish_asset_source_mutation_failure(
                        cx,
                        &publish_session_id,
                        message,
                        console_message,
                    );
                }
            }
        },
    );
}

/// Retains the reconnected controller, republishes the refreshed browser page,
/// re-reads catalog products, and writes one console line. Shared by the source
/// mutation workers so their publish order stays identical.
fn publish_asset_source_mutation(
    cx: &mut App,
    fence: crate::controller_set::ControllerFence,
    controller: EditorAssetBrowserController,
    refreshed_status: Option<EditorAssetBrowserStatus>,
    console_message: String,
) {
    let product_controller = controller.clone();
    let _ = crate::controller_set::replace_asset_browser(cx, fence, controller);
    if let Some(status) = refreshed_status {
        publish_asset_browser_status(cx, status);
    }
    spawn_catalog_products_refresh(
        cx,
        fence,
        product_controller,
        DEFAULT_ASSET_PRODUCT_PLATFORM,
    );
    publish_console_log(
        cx,
        LogLevel::Info,
        ASSET_PROCESSOR_SERVICE_NAME,
        console_message,
    );
    cx.refresh_windows();
}

/// Reports a failed source mutation to the console and to the browser
/// projection, which is what the panel renders as its error state.
fn publish_asset_source_mutation_failure(
    cx: &mut App,
    session_id: &str,
    message: String,
    console_message: String,
) {
    publish_console_log(
        cx,
        LogLevel::Error,
        ASSET_PROCESSOR_SERVICE_NAME,
        console_message,
    );
    publish_asset_browser_error(cx, session_id, message);
}

pub fn spawn_asset_source_dependents_preview(
    cx: &mut App,
    fence: crate::controller_set::ControllerFence,
    controller: EditorAssetBrowserController,
    source_root: String,
    source_path: String,
) {
    let session_id = controller.session_id().to_string();
    let publish_session_id = session_id.clone();
    let publish_source_root = source_root.clone();
    let publish_source_path = source_path.clone();
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "asset-source-dependents-preview",
        move || {
            let worker_session_id = session_id.clone();
            async move {
                let controller = controller.refresh_client_from_supervisor().await?;
                let result = controller
                    .source_dependents(source_root.clone(), source_path)
                    .await?;
                let preview = source_dependents_to_ui(worker_session_id, source_root, result);
                Ok((controller, preview))
            }
        },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok((controller, preview)) => {
                    let _ = crate::controller_set::replace_asset_browser(cx, fence, controller);
                    cx.set_global(preview);
                    cx.refresh_windows();
                }
                Err(err) => {
                    let message = err.to_string();
                    let preview = EditorAssetSourceDependentsPreview::failed(
                        publish_session_id.clone(),
                        publish_source_root,
                        publish_source_path,
                        message,
                    );
                    error!(
                        error = %err,
                        session_id = %publish_session_id,
                        "failed to query source dependents"
                    );
                    cx.set_global(preview);
                    cx.refresh_windows();
                }
            }
        },
    );
}

pub fn spawn_asset_source_delete(
    cx: &mut App,
    fence: crate::controller_set::ControllerFence,
    controller: EditorAssetBrowserController,
    source_root: String,
    source_path: String,
) {
    let session_id = controller.session_id().to_string();
    let publish_session_id = session_id.clone();
    let publish_source_path = source_path.clone();
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "asset-source-delete",
        move || {
            let worker_session_id = session_id.clone();
            async move {
                let controller = controller.refresh_client_from_supervisor().await?;
                let result = controller
                    .delete_source_file(source_root, source_path)
                    .await?;
                let deleted_source_path = result.record.entry.source_path.clone();
                let refreshed_status = match controller.load_first_page().await {
                    Ok(status) => Some(status),
                    Err(err) => {
                        error!(
                            error = %err,
                            session_id = %worker_session_id,
                            source_path = %deleted_source_path,
                            "failed to refresh asset browser status after asset source delete"
                        );
                        None
                    }
                };
                Ok((controller, refreshed_status, deleted_source_path))
            }
        },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok((controller, refreshed_status, deleted_source_path)) => {
                    let product_controller = controller.clone();
                    let _ = crate::controller_set::replace_asset_browser(cx, fence, controller);
                    if let Some(status) = refreshed_status {
                        publish_asset_browser_status(cx, status);
                    }
                    spawn_catalog_products_refresh(
                        cx,
                        fence,
                        product_controller,
                        DEFAULT_ASSET_PRODUCT_PLATFORM,
                    );
                    publish_console_log(
                        cx,
                        LogLevel::Info,
                        ASSET_PROCESSOR_SERVICE_NAME,
                        format!("deleted asset source `{deleted_source_path}`"),
                    );
                    cx.refresh_windows();
                    info!(
                        session_id = %publish_session_id,
                        source_path = %deleted_source_path,
                        "deleted asset source file"
                    );
                }
                Err(err) => {
                    let message = err.to_string();
                    let console_message =
                        format!("asset source `{publish_source_path}` delete failed: {message}");
                    error!(
                        error = %err,
                        session_id = %publish_session_id,
                        source_path = %publish_source_path,
                        "failed to delete asset source file"
                    );
                    publish_console_log(
                        cx,
                        LogLevel::Error,
                        ASSET_PROCESSOR_SERVICE_NAME,
                        console_message,
                    );
                    publish_asset_browser_error(cx, &publish_session_id, message);
                }
            }
        },
    );
}

pub fn spawn_asset_source_rename(
    cx: &mut App,
    fence: crate::controller_set::ControllerFence,
    controller: EditorAssetBrowserController,
    source_root: String,
    from_source_path: String,
    to_source_path: String,
) {
    let session_id = controller.session_id().to_string();
    let publish_session_id = session_id.clone();
    let publish_from_source_path = from_source_path.clone();
    let publish_to_source_path = to_source_path.clone();
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "asset-source-rename",
        move || {
            let worker_session_id = session_id.clone();
            async move {
                let controller = controller.refresh_client_from_supervisor().await?;
                let result = controller
                    .move_source_file(source_root, from_source_path, to_source_path)
                    .await?;
                let old_source_path = result.old_source_path.clone();
                let moved_source_path = result.record.entry.source_path.clone();
                let moved_asset_guid = result.record.asset_guid;
                let refreshed_status = match controller.load_first_page().await {
                    Ok(status) => Some(status),
                    Err(err) => {
                        error!(
                            error = %err,
                            session_id = %worker_session_id,
                            source_path = %moved_source_path,
                            "failed to refresh asset browser status after asset source rename"
                        );
                        None
                    }
                };
                Ok((
                    controller,
                    refreshed_status,
                    old_source_path,
                    moved_source_path,
                    moved_asset_guid,
                ))
            }
        },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok((
                    controller,
                    refreshed_status,
                    old_source_path,
                    moved_source_path,
                    moved_asset_guid,
                )) => {
                    publish_asset_source_mutation(
                        cx,
                        fence,
                        controller,
                        refreshed_status,
                        format!(
                            "renamed asset source `{old_source_path}` to `{moved_source_path}` ({moved_asset_guid})"
                        ),
                    );
                    info!(
                        session_id = %publish_session_id,
                        from_source_path = %old_source_path,
                        to_source_path = %moved_source_path,
                        asset_guid = %moved_asset_guid,
                        "renamed asset source file"
                    );
                }
                Err(err) => {
                    let message = err.to_string();
                    let console_message = format!(
                        "asset source `{publish_from_source_path}` rename failed: {message}"
                    );
                    error!(
                        error = %err,
                        session_id = %publish_session_id,
                        from_source_path = %publish_from_source_path,
                        to_source_path = %publish_to_source_path,
                        "failed to rename asset source file"
                    );
                    publish_asset_source_mutation_failure(
                        cx,
                        &publish_session_id,
                        message,
                        console_message,
                    );
                }
            }
        },
    );
}

pub fn spawn_asset_force_reprocess(
    cx: &mut App,
    fence: crate::controller_set::ControllerFence,
    controller: EditorAssetBrowserController,
    source_root: String,
    source_path: String,
) {
    let session_id = controller.session_id().to_string();
    let publish_session_id = session_id.clone();
    let publish_source_path = source_path.clone();
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "asset-source-force-reprocess",
        move || {
            let worker_session_id = session_id.clone();
            async move {
                let controller = controller.refresh_client_from_supervisor().await?;
                let result = controller
                    .force_reprocess_asset(source_root, source_path)
                    .await?;
                let reprocessed_source_path = result.record.entry.source_path.clone();
                let reprocessed_asset_guid = result.record.asset_guid;
                let enqueued_jobs = result.enqueued_jobs;
                let refreshed_status = match controller.load_first_page().await {
                    Ok(status) => Some(status),
                    Err(err) => {
                        error!(
                            error = %err,
                            session_id = %worker_session_id,
                            source_path = %reprocessed_source_path,
                            "failed to refresh asset browser status after asset source reprocess"
                        );
                        None
                    }
                };
                Ok((
                    controller,
                    refreshed_status,
                    reprocessed_source_path,
                    reprocessed_asset_guid,
                    enqueued_jobs,
                ))
            }
        },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok((
                    controller,
                    refreshed_status,
                    reprocessed_source_path,
                    reprocessed_asset_guid,
                    enqueued_jobs,
                )) => {
                    let product_controller = controller.clone();
                    let _ = crate::controller_set::replace_asset_browser(cx, fence, controller);
                    if let Some(status) = refreshed_status {
                        publish_asset_browser_status(cx, status);
                    }
                    spawn_catalog_products_refresh(
                        cx,
                        fence,
                        product_controller,
                        DEFAULT_ASSET_PRODUCT_PLATFORM,
                    );
                    publish_console_log(
                        cx,
                        LogLevel::Info,
                        ASSET_PROCESSOR_SERVICE_NAME,
                        format!(
                            "reprocessed asset source `{reprocessed_source_path}` ({reprocessed_asset_guid}); queued {enqueued_jobs} jobs"
                        ),
                    );
                    cx.refresh_windows();
                    info!(
                        session_id = %publish_session_id,
                        source_path = %reprocessed_source_path,
                        asset_guid = %reprocessed_asset_guid,
                        enqueued_jobs,
                        "force-reprocessed asset source"
                    );
                }
                Err(err) => {
                    let message = err.to_string();
                    let console_message =
                        format!("asset source `{publish_source_path}` reprocess failed: {message}");
                    error!(
                        error = %err,
                        session_id = %publish_session_id,
                        source_path = %publish_source_path,
                        "failed to force reprocess asset source"
                    );
                    publish_console_log(
                        cx,
                        LogLevel::Error,
                        ASSET_PROCESSOR_SERVICE_NAME,
                        console_message,
                    );
                    publish_asset_browser_error(cx, &publish_session_id, message);
                }
            }
        },
    );
}

pub fn spawn_asset_browser_load_more(
    cx: &mut App,
    fence: crate::controller_set::ControllerFence,
    controller: EditorAssetBrowserController,
    after_entry_id: i64,
) {
    let session_id = controller.session_id().to_string();
    let publish_session_id = session_id;
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "asset-browser-load-more",
        move || async move {
            let controller = controller.refresh_client_from_supervisor().await?;
            let page = controller.load_page(Some(after_entry_id)).await?;
            Ok((controller, page))
        },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok((controller, page)) => {
                    let page_entry_count = page.entries.len();
                    if let Some(current) = cx.try_global::<EditorAssetBrowserStatus>().cloned() {
                        if current.session_id != page.session_id
                            || current.next_after_entry_id != Some(after_entry_id)
                        {
                            info!(
                                session_id = %page.session_id,
                                after_entry_id,
                                "discarded stale asset browser page"
                            );
                            return;
                        }

                        let status = append_asset_browser_status(current, page);
                        let _ = crate::controller_set::replace_asset_browser(cx, fence, controller);
                        publish_asset_browser_status(cx, status);
                    } else {
                        let _ = crate::controller_set::replace_asset_browser(cx, fence, controller);
                        publish_asset_browser_status(cx, page);
                    }
                    cx.refresh_windows();
                    info!(
                        session_id = %publish_session_id,
                        after_entry_id,
                        page_entry_count,
                        "loaded more asset browser status"
                    );
                }
                Err(err) => {
                    let message = err.to_string();
                    error!(
                        error = %err,
                        session_id = %publish_session_id,
                        after_entry_id,
                        "failed to load more asset browser status"
                    );
                    publish_asset_browser_error(cx, &publish_session_id, message);
                }
            }
        },
    );
}

pub fn publish_asset_browser_error(cx: &mut App, session_id: &str, message: String) {
    if let Some(current) = cx.try_global::<EditorAssetBrowserStatus>().cloned()
        && current.session_id == session_id
    {
        publish_asset_browser_status(cx, current.with_error(message));
    } else {
        publish_asset_browser_status(
            cx,
            EditorAssetBrowserStatus::error(session_id.to_string(), message),
        );
    }
    cx.refresh_windows();
}

pub fn publish_console_log(
    cx: &mut App,
    level: LogLevel,
    source: impl Into<String>,
    message: impl Into<String>,
) {
    let source = source.into();
    let message = message.into();
    cx.default_global::<ConsoleState>()
        .log_from_source(level, source, message);
}
