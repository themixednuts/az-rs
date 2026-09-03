//! HTTP Plugin for Bevy
//!
//! Provides HTTP task execution using Bevy's `IoTaskPool` and reqwest.
//! This plugin simply provides a shared `reqwest::Client` resource and
//! utilities for spawning HTTP tasks on `IoTaskPool`.
//!
//! # Usage
//!
//! ```no_run
//! use bevy::prelude::*;
//! use bevy::tasks::IoTaskPool;
//! use http_plugin::{HttpPlugin, HttpService};
//!
//! App::new()
//!     .add_plugins(DefaultPlugins)
//!     .add_plugins(HttpPlugin);
//!
//! fn make_request(http_service: Res<HttpService>) {
//!     let service = http_service.clone();
//!     IoTaskPool::get().spawn(async move {
//!         if let Ok(response) = service.get("https://api.example.com").send().await {
//!             // Handle response...
//!         }
//!     }).detach();
//! }
//! ```

pub mod error;
pub mod plugin;
pub mod service;

pub use error::{HttpError, Result};
pub use plugin::HttpPlugin;
pub use service::{HttpService, RequestBuilder};
