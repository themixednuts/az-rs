//! Logging registry for automatic module filtering
//!
//! Allows plugins to register their module names so they're automatically included
//! in log filtering without manual maintenance.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

/// Global registry for application module names
/// Uses `OnceLock` for lazy initialization and `Mutex` for interior mutability
static LOGGING_REGISTRY: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

/// Initialize the logging registry
///
/// This should be called once before plugins are added.
/// Safe to call multiple times - only the first call initializes the registry.
#[doc(hidden)]
pub fn init_registry() {
    LOGGING_REGISTRY.get_or_init(|| Mutex::new(HashSet::new()));
}

/// Register the current module for logging
///
/// Automatically detects the module name using `module_path!()` macro.
/// This should be called by plugins during their `build` method to ensure
/// their logs are visible at the configured verbosity level.
///
/// The module name is extracted from the full module path (e.g., `aws_plugin` from `aws_plugin::plugin`).
///
/// # Example
///
/// ```rust,ignore
/// use logging_registry::register;
///
/// impl Plugin for MyPlugin {
///     fn build(&self, _app: &mut App) {
///         register!(); // Automatically uses current module name
///         // ... rest of plugin setup
///     }
/// }
/// ```
#[macro_export]
macro_rules! register {
    () => {{
        $crate::init_registry();
        let module_path = module_path!();
        // Extract the crate name (first segment) from the module path
        // e.g., "aws_plugin::plugin" -> "aws_plugin"
        let module_name = module_path.split("::").next().unwrap_or(module_path);
        $crate::register_module(module_name);
    }};
}

/// Register a specific module name for logging
///
/// This is a fallback if automatic detection doesn't work or you need to register
/// a different module name. Prefer using `register()` instead.
///
/// # Panics
///
/// Panics if the registry mutex was poisoned by a thread that panicked while
/// holding it.
pub fn register_module(module_name: &str) {
    init_registry();

    if let Some(registry) = LOGGING_REGISTRY.get() {
        registry.lock().unwrap().insert(module_name.to_string());
    }
}

/// Build a filter string from registered modules
///
/// Returns a filter that:
/// - Shows registered modules at the specified verbosity level
/// - Hides everything else (including dependency warnings)
/// - Can be overridden by the `RUST_LOG` environment variable
///
/// # Panics
///
/// Panics if the registry mutex was poisoned by a thread that panicked while
/// holding it.
#[must_use]
pub fn build_filter(app_verbosity: &str) -> String {
    let Some(registry) = LOGGING_REGISTRY.get() else {
        // Registry not initialized, use default
        return app_verbosity.to_string();
    };

    let filter_parts = {
        let modules = registry.lock().unwrap();

        if modules.is_empty() {
            // No modules registered, use default verbosity for everything
            return app_verbosity.to_string();
        }

        // Build filter: registered modules at app verbosity, everything else
        // off. "off" is the default for unregistered modules; the two
        // steamworks entries silence known noisy native library output.
        let mut parts = vec![
            "off".to_string(),
            "steamworks_sys=off".to_string(),
            "bevy_steamworks=off".to_string(),
        ];
        parts.extend(
            modules
                .iter()
                .map(|module| format!("{module}={app_verbosity}")),
        );
        parts
    };

    filter_parts.join(",")
}
