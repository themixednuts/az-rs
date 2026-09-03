use az_proto_asset::asset_capnp;
use az_proto_core::{Endpoint, EndpointKind};
use az_rpc::{AzRpcTransportError, ScopedTwopartyClient};
use thiserror::Error;

/// Errors raised while connecting a worker to its asset-processor RPC peer.
///
/// This deliberately contains only transport concerns. Database-open and
/// database-query failures belong to the asset-processor host and must not
/// become worker dependencies through an error conversion.
#[derive(Debug, Error)]
pub enum AssetWorkerRpcTransportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Rpc(#[from] AzRpcTransportError),

    #[error("endpoint kind {0:?} is not supported on this platform")]
    UnsupportedEndpoint(EndpointKind),
}

/// Connect to an asset-processor Cap'n Proto endpoint without opening a DB.
///
/// The returned owner keeps the RPC system alive for incoming source-codec
/// callbacks and gives the worker an explicit graceful-close boundary.
///
/// # Errors
///
/// Returns [`AssetWorkerRpcTransportError::UnsupportedEndpoint`] if the
/// endpoint kind is not supported on this platform, and a transport error if
/// the connection or the Cap'n Proto bootstrap handshake fails.
pub async fn connect_asset_processor_rpc_client(
    endpoint: &Endpoint,
) -> Result<ScopedTwopartyClient<asset_capnp::asset_processor::Client>, AssetWorkerRpcTransportError>
{
    Ok(az_rpc::connect_twoparty_bootstrap_scoped(endpoint).await?)
}
