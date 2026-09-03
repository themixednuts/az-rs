//! HTTP plugin for Bevy

use crate::service::HttpService;
use bevy::prelude::*;
use tracing::info;

/// HTTP plugin for Bevy
///
/// Provides a shared `reqwest::Client` resource for making HTTP requests.
/// This plugin simply initializes the `HttpService` resource - you can then
/// use it to spawn HTTP tasks on `IoTaskPool` as needed.
///
/// # Usage
///
/// ```no_run
/// use bevy::prelude::*;
/// use bevy::tasks::IoTaskPool;
/// use http_plugin::{HttpPlugin, HttpService};
///
/// App::new()
///     .add_plugins(DefaultPlugins)
///     .add_plugins(HttpPlugin)
///     .add_systems(Update, make_request);
///
/// fn make_request(http_service: Res<HttpService>) {
///     let service = http_service.clone();
///     IoTaskPool::get().spawn(async move {
///         if let Ok(response) = service.get("https://api.example.com").send().await {
///             // Handle response...
///         }
///     }).detach();
/// }
/// ```
pub struct HttpPlugin;

impl Plugin for HttpPlugin {
    fn build(&self, app: &mut App) {
        // Register this module for logging (automatically detects module name)
        logging_registry::register!();

        // Initialize HTTP service resource
        app.init_resource::<HttpService>();

        info!("HTTP plugin initialized");
    }
}
