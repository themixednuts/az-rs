//! AWS Plugin for Bevy

use crate::service::{AwsService, LoginContext};
use ags_auth_protocol::client::AgsGameClientProfile;
use bevy::prelude::*;
use bevy::tasks::{IoTaskPool, Task};
use futures::channel::oneshot;
use http_plugin::{HttpPlugin, HttpService};
use std::sync::Arc;
use tracing::trace;

/// Login request event - triggers authentication flow
///
/// This event can be sent by:
/// - CLI: Automatically on startup (after Steam/AWS are ready)
/// - GUI/TUI: When user clicks login button
///
/// Systems listening to this event will:
/// 1. Get Steam ticket from Steamworks
/// 2. Get JWT token from Amazon token service
/// 3. Fetch login queue ticket and populate `LoginContext`
#[derive(Message, Debug, Clone)]
pub struct LoginRequest;

/// Project-supplied AGS client profile used by the AWS auth flow.
#[derive(Resource, Debug, Clone)]
pub struct AwsAuthProfile {
    client: AgsGameClientProfile,
}

impl AwsAuthProfile {
    #[must_use]
    pub const fn new(client: AgsGameClientProfile) -> Self {
        Self { client }
    }

    #[must_use]
    pub const fn client(&self) -> &AgsGameClientProfile {
        &self.client
    }
}

/// Resource to track AWS plugin task handles
#[derive(Resource, Default, Debug)]
struct AwsTaskHandles {
    region_config_task: Option<Task<()>>,
    login_task: Option<Task<()>>,
    login_result_rx: Option<futures::channel::oneshot::Receiver<Option<LoginContext>>>,
}

/// Resource to store pending Steam ticket handles (shared between login systems)
#[derive(Resource, Default, Debug)]
struct PendingTicketHandles {
    handles: Vec<bevy_steamworks::AuthTicket>,
}

/// Resource to track login state (shared between login systems)
#[derive(Resource, Default, Debug)]
struct LoginState {
    in_progress: bool,
}

/// Bevy plugin for AWS API operations
///
/// This plugin depends on `HttpPlugin` and will automatically add it if not already present.
/// The `HttpPlugin` must be initialized before `AwsPlugin` can use `HttpService`.
pub struct AwsPlugin;

impl Plugin for AwsPlugin {
    fn build(&self, app: &mut App) {
        // Register this module for logging (automatically detects module name)
        logging_registry::register!();

        trace!("[AWS] AWS plugin building...");

        // Ensure HttpPlugin is added (dependency)
        // This ensures HttpService resource is available
        if !app.is_plugin_added::<HttpPlugin>() {
            app.add_plugins(HttpPlugin);
            trace!("[AWS] HttpPlugin added as dependency");
        }

        // Register messages
        app.add_message::<LoginRequest>();

        // Initialize LoginContext resource with default values
        // This ensures the resource exists even if not yet populated with real data
        app.init_resource::<LoginContext>();

        // Initialize shared state resources for login flow
        app.init_resource::<PendingTicketHandles>();
        app.init_resource::<LoginState>();
        app.init_resource::<AwsTaskHandles>();

        // Initialize AwsService after HttpService is available
        // Commands are applied at the end of the schedule, so load_region_config
        // must run in a later schedule to see the resource
        app.add_systems(PostStartup, init_aws_service);

        // Load region config when AwsService becomes available
        // Use resource_exists to ensure it only runs when the resource exists
        // and Local<bool> to ensure it only runs once
        app.add_systems(
            Update,
            load_region_config.run_if(resource_exists::<AwsService>),
        );

        // System to handle login requests and fetch login queue ticket
        // Only runs when AwsService exists (it will check internally if region config is loaded)
        app.add_systems(
            Update,
            (
                monitor_aws_tasks,
                handle_login_request
                    .run_if(resource_exists::<AwsService>)
                    .after(load_region_config),
                handle_ticket_response
                    .run_if(resource_exists::<AwsService>)
                    .after(handle_login_request),
            )
                .chain(),
        );

        trace!("[AWS] AWS plugin initialized");
    }
}

/// Initialize AWS service using `HttpService`
// Bevy system: `Res`, `Option<Res>` and friends are owned `SystemParam`
// wrappers. Taking them by reference does not implement `SystemParam`, so
// the suggested signature would not register as a system.
#[allow(clippy::needless_pass_by_value)]
fn init_aws_service(http_service: Res<HttpService>, mut commands: Commands) {
    trace!("[AWS] init_aws_service system running...");
    // Clone HttpService reference for AwsService
    // HttpService is a Resource, so we can clone the Arc internally
    let http_service = Arc::new((*http_service).clone());
    let aws_service = AwsService::new(http_service);
    commands.insert_resource(aws_service);
    trace!("[AWS] AWS service resource initialized - will be available next frame");
}

/// Load region configuration
///
/// This spawns an async task on `IoTaskPool` that calls `AwsService::load_region_config()`.
/// Bevy systems must be synchronous, so we spawn an async task to call the async method.
/// The `HttpService` client automatically uses Tokio runtime, so no special handling needed.
// Bevy system: `Res`, `Option<Res>` and friends are owned `SystemParam`
// wrappers. Taking them by reference does not implement `SystemParam`, so
// the suggested signature would not register as a system.
#[allow(clippy::needless_pass_by_value)]
fn load_region_config(
    aws_service: Res<AwsService>,
    auth_profile: Option<Res<AwsAuthProfile>>,
    mut handles: ResMut<AwsTaskHandles>,
    mut has_loaded: Local<bool>,
) {
    // Only run once
    if *has_loaded {
        return;
    }

    let Some(auth_profile) = auth_profile.as_deref() else {
        trace!("[AWS] Waiting for AGS auth profile before loading region config");
        return;
    };

    *has_loaded = true;
    trace!("[AWS] ✅ load_region_config system triggered - AwsService resource is available");

    // Clone the necessary pieces for async access
    let client_profile = auth_profile.client().clone();
    let http_service = aws_service.http_service().clone();
    let region_config = aws_service.region_config.clone();

    trace!("[AWS] Spawning region config load task on IoTaskPool...");

    // Spawn async task on IoTaskPool and store handle
    let task = IoTaskPool::get().spawn(async move {
        trace!("[AWS] Async task started - calling load_region_config...");
        // Construct temporary AwsService to call the async method
        let temp_service = AwsService {
            region_config,
            http_service,
        };

        // HttpService handles Tokio spawning internally
        let result = temp_service.load_region_config(&client_profile).await;
        match result {
            Ok(()) => {
                trace!("[AWS] ✅ Region config fetch completed successfully");
            }
            Err(e) => {
                trace!("[AWS] ❌ Failed to load region config: {}", e);
            }
        }
    });

    // Store task handle for monitoring
    handles.region_config_task = Some(task);
}

/// System to handle login requests and trigger Steam ticket request
///
/// Flow:
/// 1. On startup: Steam and AWS initialize (`SteamworksPlugin`, `AwsPlugin`)
/// 2. When `LoginRequest` event is received (CLI sends automatically, GUI/TUI waits for user):
///    - Request Steam ticket from Steamworks (starts async ticket generation)
///    - Store ticket handle to match with response
/// 3. `handle_steam_ticket_response` system processes `TicketForWebApiResponse` event:
///    - Get JWT token from Amazon token service (using Steam ticket)
///    - Call `fetch_login_queue_ticket` with both
///    - Populate `LoginContext` resource
// Bevy system: `Res`, `Option<Res>` and friends are owned `SystemParam`
// wrappers. Taking them by reference does not implement `SystemParam`, so
// the suggested signature would not register as a system.
#[allow(clippy::needless_pass_by_value)]
fn handle_login_request(
    mut login_requests: MessageReader<LoginRequest>,
    registration_artifacts: Res<LoginContext>,
    auth_profile: Option<Res<AwsAuthProfile>>,
    steam_client: Option<Res<bevy_steamworks::Client>>,
    mut pending_ticket_handles: ResMut<PendingTicketHandles>,
    mut has_logged_missing: Local<bool>,
    mut login_in_progress: ResMut<LoginState>,
) {
    // Check if we already have registration artifacts populated
    if !registration_artifacts.ticket_id.is_empty()
        && !registration_artifacts.signature.is_empty()
        && !registration_artifacts.host_hash.is_empty()
        && !registration_artifacts.jwt_claims.is_empty()
    {
        // Already logged in, ignore requests
        let _ = login_requests.read().count(); // Drain any pending requests
        return;
    }

    // Don't start a new login if one is already in progress
    if login_in_progress.in_progress {
        let _ = login_requests.read().count(); // Drain any pending requests
        return;
    }

    // Process login requests
    let request_count = login_requests.read().count();
    if request_count == 0 {
        return;
    }

    trace!(
        "[AWS] 🔐 Login request received ({} pending)",
        request_count
    );

    let Some(auth_profile) = auth_profile.as_deref() else {
        trace!("[AWS] ⚠️  AGS auth profile not available - cannot start login");
        return;
    };
    trace!(
        "[AWS] Using AGS profile: game={} gateway={} steam_app_id={} token_version={}",
        auth_profile.client().game,
        auth_profile.client().gateway_service_tag,
        auth_profile.client().steam_app_id,
        auth_profile.client().token_version
    );

    // Check if Steam client is available
    let Some(steam_client) = steam_client.as_deref() else {
        if !*has_logged_missing {
            trace!("[AWS] ⚠️  Steam client not available - cannot get Steam ticket");
            trace!("[AWS]    Make sure SteamworksPlugin is initialized");
            *has_logged_missing = true;
        }
        return;
    };

    // Mark login as in progress
    login_in_progress.in_progress = true;

    trace!("[AWS] 🚀 Starting authentication flow...");

    // Step 1: Request Steam ticket from Steamworks
    trace!("[AWS] 📝 Step 1: Requesting Steam ticket from Steamworks...");

    // Get Steam User interface
    let user = steam_client.user();

    // Request auth ticket for web API
    // The AGS identity endpoint accepts an empty identifier for this flow.
    let ticket_handle = user.authentication_session_ticket_for_webapi("");

    // Store ticket handle to match with response
    pending_ticket_handles.handles.push(ticket_handle);

    trace!("[AWS] ⏳ Steam ticket requested, waiting for TicketForWebApiResponse event...");
}

/// System to monitor AWS task handles and handle completion
fn monitor_aws_tasks(
    mut handles: ResMut<AwsTaskHandles>,
    mut registration_artifacts: ResMut<LoginContext>,
    mut login_in_progress: ResMut<LoginState>,
) {
    // Check region config task
    if let Some(task) = handles.region_config_task.as_mut()
        && task.is_finished()
    {
        handles.region_config_task = None;
    }

    // Check login task result channel
    if let Some(rx) = handles.login_result_rx.as_mut()
        && let Ok(Some(inner)) = rx.try_recv()
    {
        if let Some(artifacts) = inner {
            trace!("[AWS] ✅ Login queue ticket obtained");
            trace!("[AWS]    ticket_id_len: {}", artifacts.ticket_id.len());

            // Set LoginContext directly (only once per login)
            if registration_artifacts.ticket_id.is_empty() {
                trace!("[AWS]    rep_address: {}", artifacts.rep_address);
                trace!("[AWS]    persona_id: {}", artifacts.persona_id);
                trace!("[AWS]    steam_id: {}", artifacts.steam_id);
                trace!("[AWS]    character_id: {}", artifacts.character_id);
                trace!("[AWS]    generate_time: {}", artifacts.generate_time);
                trace!("[AWS]    issue_time: {}", artifacts.issue_time);
                trace!("[AWS]    account_age: {}", artifacts.account_age);
                trace!("[AWS]    token_version: {}", artifacts.token_version);
                trace!(
                    "[AWS]    location_group_id: {}",
                    artifacts.location_group_id
                );
                trace!("[AWS]    location_id: {}", artifacts.location_id);
                *registration_artifacts = artifacts;
                trace!("[AWS] ✅ LoginContext set successfully");
            } else {
                trace!("[AWS] ⚠️ LoginContext already set, ignoring update");
            }
        } else {
            trace!("[AWS] ❌ Failed to fetch login queue ticket");
        }

        login_in_progress.in_progress = false;
        handles.login_result_rx = None;
    }

    // Check login task completion
    if let Some(task) = handles.login_task.as_mut()
        && task.is_finished()
    {
        handles.login_task = None;
    }
}

/// System to handle Steam ticket responses and continue authentication flow
// Bevy system: `Res`, `Option<Res>` and friends are owned `SystemParam`
// wrappers. Taking them by reference does not implement `SystemParam`, so
// the suggested signature would not register as a system.
#[allow(clippy::needless_pass_by_value)]
fn handle_ticket_response(
    aws_service: Res<AwsService>,
    auth_profile: Option<Res<AwsAuthProfile>>,
    steam_client: Option<Res<bevy_steamworks::Client>>,
    mut steam_events: MessageReader<bevy_steamworks::SteamworksEvent>,
    mut pending_ticket_handles: ResMut<PendingTicketHandles>,
    mut handles: ResMut<AwsTaskHandles>,
    mut login_in_progress: ResMut<LoginState>,
) {
    let Some(auth_profile) = auth_profile.as_deref() else {
        return;
    };

    // Check for TicketForWebApiResponse events
    for event in steam_events.read() {
        if let bevy_steamworks::SteamworksEvent::CallbackResult(
            bevy_steamworks::CallbackResult::TicketForWebApiResponse(response),
        ) = event
        {
            // Check if this response matches any pending ticket handle
            if pending_ticket_handles
                .handles
                .iter()
                .any(|h| h == &response.ticket_handle)
            {
                // Remove the matching handle
                pending_ticket_handles
                    .handles
                    .retain(|h| h != &response.ticket_handle);

                // Check if the ticket was successfully generated
                match &response.result {
                    Ok(()) => {
                        trace!(
                            "[AWS] ✅ Steam ticket obtained, length: {} bytes",
                            response.ticket.len()
                        );

                        let Some(steam_client) = steam_client.as_ref() else {
                            trace!("[AWS] ❌ Steam client disappeared before ticket processing");
                            login_in_progress.in_progress = false;
                            return;
                        };
                        let steam_id = steam_client.user().steam_id().raw();
                        trace!("[AWS] Steam ID: {}", steam_id);

                        // Convert ticket bytes to hex string
                        let ticket_hex = hex::encode(&response.ticket);

                        // Clone what we need for the async task
                        let client_profile = auth_profile.client().clone();
                        let http_service = aws_service.http_service().clone();
                        let region_config = aws_service.region_config.clone();
                        let character_id = std::env::var("AZOTH_CHARACTER_ID")
                            .ok()
                            .filter(|value| !value.trim().is_empty());
                        if let Some(character_id) = &character_id {
                            trace!("[AWS] Using character override: {}", character_id);
                        }

                        // Create channel for result
                        let (tx, rx) = oneshot::channel();

                        // Spawn async task to complete the authentication flow
                        let task = IoTaskPool::get().spawn(async move {
                            trace!(
                                "[AWS] 🔄 Step 2: Fetching JWT token from Amazon token service..."
                            );

                            // Create temporary AwsService for async operations
                            let temp_service = AwsService {
                                region_config,
                                http_service: http_service.clone(),
                            };

                            // Step 2: Fetch JWT token using Steam ticket
                            let jwt_result = temp_service
                                .fetch_jwt_token(&client_profile, &ticket_hex)
                                .await;

                            let jwt = match jwt_result {
                                Ok(token) => {
                                    trace!("[AWS] ✅ JWT token obtained");
                                    token
                                }
                                Err(e) => {
                                    trace!("[AWS] ❌ Failed to fetch JWT token: {}", e);
                                    let _ = tx.send(None);
                                    return;
                                }
                            };

                            trace!("[AWS] 🔄 Step 3: Fetching login queue ticket...");

                            // Step 3: Fetch login queue ticket (now with steam_id)
                            let result = match temp_service
                                .fetch_login_queue_ticket(
                                    &client_profile,
                                    &jwt,
                                    &ticket_hex,
                                    steam_id,
                                    character_id.as_deref(),
                                    None,
                                )
                                .await
                            {
                                Ok(login_queue_result) => Some(login_queue_result),
                                Err(e) => {
                                    trace!("[AWS] ❌ Failed to fetch login queue ticket: {}", e);
                                    None
                                }
                            };
                            let _ = tx.send(result);
                        });

                        // Store task handle and receiver for monitoring
                        handles.login_task = Some(task);
                        handles.login_result_rx = Some(rx);
                    }
                    Err(e) => {
                        trace!("[AWS] ❌ Failed to get Steam ticket: {:?}", e);
                        login_in_progress.in_progress = false;
                    }
                }
                break; // Process one event per frame
            }
        }
    }
}
