//! HTTP service resource

use bevy::prelude::*;
use reqwest::{Client, IntoUrl, Method, RequestBuilder as ReqwestRequestBuilder};
use std::sync::Arc;
use tracing::warn;

use crate::error::{HttpError, Result};

/// HTTP service resource
///
/// Provides a `reqwest::Client` API that automatically works with Tokio runtime.
/// All HTTP methods (get, post, etc.) forward to `reqwest::Client`, and `.send()` calls
/// are automatically spawned on the Tokio runtime.
///
/// Use it exactly like `reqwest::Client`:
/// ```no_run
/// # async fn example(http_service: http_plugin::HttpService) -> http_plugin::Result<()> {
/// let response = http_service.get("https://example.com").send().await?;
/// # Ok(())
/// # }
/// ```
#[derive(Resource, Clone)]
pub struct HttpService {
    /// The reqwest client - used internally
    client: Arc<Client>,
    /// Tokio runtime - kept alive for the lifetime of `HttpService`
    ///
    /// Storing the Runtime in Arc prevents it from shutting down (per Tokio docs).
    /// The multi-threaded runtime has its own worker threads that keep running,
    /// so no background thread is needed to drive it.
    rt: Arc<tokio::runtime::Runtime>,
}

/// Wrapper around `reqwest::RequestBuilder` that ensures `.send()` runs on Tokio runtime
pub struct RequestBuilder {
    inner: ReqwestRequestBuilder,
    handle: tokio::runtime::Handle,
}

impl RequestBuilder {
    const fn new(inner: ReqwestRequestBuilder, handle: tokio::runtime::Handle) -> Self {
        Self { inner, handle }
    }

    /// Send the request - automatically spawns on Tokio runtime
    ///
    /// # Errors
    ///
    /// Returns [`HttpError::Tokio`] if the spawned task panics or is cancelled
    /// (for example because the runtime is shutting down), or
    /// [`HttpError::Reqwest`] if the request itself fails to connect, times out,
    /// or is aborted mid-response.
    pub async fn send(self) -> Result<reqwest::Response> {
        // Spawn the reqwest call on Tokio runtime
        // If the task panics or is cancelled, convert JoinError to HttpError
        let join_result = self
            .handle
            .spawn(async move { self.inner.send().await })
            .await;

        match join_result {
            Ok(request_result) => request_result.map_err(HttpError::from),
            Err(e) => Err(HttpError::from(e)),
        }
    }
}

impl RequestBuilder {
    // Forward common RequestBuilder methods
    // Using the same signatures as reqwest::RequestBuilder
    #[must_use]
    pub fn header<K, V>(self, key: K, value: V) -> Self
    where
        http::HeaderName: TryFrom<K>,
        <http::HeaderName as TryFrom<K>>::Error: Into<http::Error>,
        http::HeaderValue: TryFrom<V>,
        <http::HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
    {
        Self {
            inner: self.inner.header(key, value),
            handle: self.handle,
        }
    }

    #[must_use]
    pub fn headers(self, headers: http::HeaderMap) -> Self {
        Self {
            inner: self.inner.headers(headers),
            handle: self.handle,
        }
    }

    #[must_use]
    pub fn body<T: Into<reqwest::Body>>(self, body: T) -> Self {
        Self {
            inner: self.inner.body(body),
            handle: self.handle,
        }
    }

    #[must_use]
    pub fn json<T: serde::Serialize + ?Sized>(self, json: &T) -> Self {
        Self {
            inner: self.inner.json(json),
            handle: self.handle,
        }
    }

    #[must_use]
    pub fn form<T: serde::Serialize + ?Sized>(self, form: &T) -> Self {
        Self {
            inner: self.inner.form(form),
            handle: self.handle,
        }
    }

    #[must_use]
    pub fn query<T: serde::Serialize + ?Sized>(self, query: &T) -> Self {
        Self {
            inner: self.inner.query(query),
            handle: self.handle,
        }
    }

    #[must_use]
    pub fn timeout(self, timeout: std::time::Duration) -> Self {
        Self {
            inner: self.inner.timeout(timeout),
            handle: self.handle,
        }
    }

    #[must_use]
    pub fn version(self, version: http::Version) -> Self {
        Self {
            inner: self.inner.version(version),
            handle: self.handle,
        }
    }
}

impl HttpService {
    /// Create a new HTTP service with default client configuration
    ///
    /// Sets up a Tokio runtime for reqwest and stores it.
    #[must_use]
    pub fn new() -> Self {
        let runtime = Self::create_runtime();

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|e| {
                warn!(
                    "Failed to create HTTP client with timeout, using default: {}",
                    e
                );
                Client::new()
            });

        Self {
            client: Arc::new(client),
            rt: Arc::new(runtime),
        }
    }

    /// Create a new HTTP service with a custom client
    #[must_use]
    pub fn with_client(client: Client) -> Self {
        let runtime = Self::create_runtime();

        Self {
            client: Arc::new(client),
            rt: Arc::new(runtime),
        }
    }

    /// Get the Tokio runtime handle for spawning tasks
    #[must_use]
    pub fn tokio_handle(&self) -> tokio::runtime::Handle {
        self.rt.handle().clone()
    }

    /// Create a new Tokio runtime for reqwest
    ///
    /// Uses multi-threaded runtime which has its own worker threads that keep running.
    /// The runtime is kept alive by storing it in Arc<Runtime> - this prevents shutdown.
    /// No background thread needed - the multi-threaded runtime manages its own threads.
    fn create_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2) // Minimal threads for HTTP requests
            .enable_io()
            .enable_time()
            .build()
            .expect("Failed to create Tokio runtime for reqwest")
    }

    // Forward reqwest::Client methods
    pub fn get<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        RequestBuilder::new(self.client.get(url), self.rt.handle().clone())
    }

    pub fn post<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        RequestBuilder::new(self.client.post(url), self.rt.handle().clone())
    }

    pub fn put<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        RequestBuilder::new(self.client.put(url), self.rt.handle().clone())
    }

    pub fn patch<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        RequestBuilder::new(self.client.patch(url), self.rt.handle().clone())
    }

    pub fn delete<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        RequestBuilder::new(self.client.delete(url), self.rt.handle().clone())
    }

    pub fn head<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        RequestBuilder::new(self.client.head(url), self.rt.handle().clone())
    }

    pub fn request<U: IntoUrl>(&self, method: Method, url: U) -> RequestBuilder {
        RequestBuilder::new(self.client.request(method, url), self.rt.handle().clone())
    }
}

impl Default for HttpService {
    fn default() -> Self {
        Self::new()
    }
}
