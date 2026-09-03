//! Engine-owned asset-processor host.
//!
//! This binary is the Azoth equivalent of O3DE's single `AssetProcessor`
//! application: it owns the durable asset DB, source-root registration,
//! watch/reconcile orchestration, job queue, and RPC surface. It has
//! **zero** project gem linkage. Project builders are loaded only by the
//! per-project `asset-worker` processes supervised with the project instance.

use az_service_entrypoint::{AssetProcessorCli, ServiceRole};
use clap::Parser;

// `redundant_closure_for_method_calls`: the method's owning type lives in
// `az-observability-control`, which is not a direct dependency here, so the suggested
// method path does not resolve.
#[allow(clippy::redundant_closure_for_method_calls)]
// `significant_drop_tightening`: the registered database handle is handed to the RPC
// server and must stay alive for the whole host process.
#[allow(clippy::significant_drop_tightening)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = AssetProcessorCli::parse().into_service_args()?;
    az_service_entrypoint::install_observability(&args, ServiceRole::AssetProcessor)?;
    let observability_control = az_service_entrypoint::start_observability_control_server(
        &args,
        ServiceRole::AssetProcessor,
    )?;
    let (lifecycle_control, lifetime) =
        az_service_entrypoint::start_service_lifecycle_control(&args)?;
    tracing::info!(
        structured_log = %args.structured_log.display(),
        "observability initialized"
    );
    let db_path = args
        .asset_db
        .as_ref()
        .ok_or("--asset-db is required for asset-processor")?;
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let registered_db = az_asset_processor::open_registered_workspace_asset_db(
        db_path,
        &args.project_id,
        &args.workspace_root,
        &args.branch,
        az_service_entrypoint::now_unix_ms_i64()?,
    )?;
    let registration = registered_db.registration();
    if registration.source_roots.is_empty() {
        return Err("asset-processor registered no DB-owned asset source roots".into());
    }
    let workspace_id = registration.workspace_id;
    let source_root_count = registration.source_roots.len();
    let server = az_asset_processor::start_asset_processor_rpc_server_with_registered_workspace_db(
        registered_db,
        args.endpoint.clone(),
        args.run,
        &args.capability_grants,
    )?;
    az_service_entrypoint::write_ready_file_with_observability(
        &{
            let mut ready_args = args.clone();
            ready_args.lifecycle_endpoint = lifecycle_control.endpoint().clone();
            ready_args
        },
        ServiceRole::AssetProcessor,
        server.endpoint(),
        observability_control
            .as_ref()
            // The method's owning type lives in `az-observability-control`, which is not a
            // direct dependency here, so clippy's method-path suggestion does not resolve.
            .map(|server| server.endpoint()),
    )?;
    tracing::info!(
        project_id = %args.project_id,
        owner_id = %args.owner_id,
        owner_root = %args.owner_root.display(),
        service = %args.service,
        workspace_id,
        source_root_count,
        endpoint_kind = ?server.endpoint().kind,
        endpoint = %server.endpoint().address,
        "service ready"
    );
    let mut termination = az_service_entrypoint::ServiceTermination::new(lifetime.wait());
    termination.record("asset-processor RPC server", server.stop());
    drop(observability_control);
    termination.record("lifecycle control server", lifecycle_control.stop());
    termination.finish()?;
    Ok(())
}
