//! Wwise configuration constants.

/// Default Wwise generated-bank path used by Lumberyard.
///
/// Lumberyard reference: `dev/Gems/AudioEngineWwise/Code/Source/Engine/Config_wwise.h:20`.
pub const WWISE_DEFAULT_BANKS_PATH: &str = "sounds/wwise/";

/// Default Wwise external-sources folder used by Lumberyard.
///
/// Lumberyard reference: `dev/Gems/AudioEngineWwise/Code/Source/Engine/Config_wwise.h:21`.
pub const WWISE_EXTERNAL_SOURCES_PATH: &str = "external";

/// Lumberyard Wwise configuration file name.
///
/// Lumberyard reference: `dev/Gems/AudioEngineWwise/Code/Source/Engine/Config_wwise.h:22`.
pub const WWISE_CONFIG_FILE: &str = "wwise_config.json";

/// Wwise soundbank extension used by Lumberyard.
///
/// Lumberyard reference: `dev/Gems/AudioEngineWwise/Code/Source/Engine/Config_wwise.h:23`.
pub const WWISE_BANK_EXTENSION: &str = ".bnk";

/// Wwise encoded-media extension used by Lumberyard.
///
/// Lumberyard reference: `dev/Gems/AudioEngineWwise/Code/Source/Engine/Config_wwise.h:24`.
pub const WWISE_MEDIA_EXTENSION: &str = ".wem";

/// File extensions handled by the Wwise soundbank asset loader.
pub const WWISE_SOUND_BANK_ASSET_EXTENSIONS: &[&str] = &["bnk"];

/// File extensions handled by the Wwise media asset loader.
pub const WWISE_MEDIA_ASSET_EXTENSIONS: &[&str] = &["wem"];

/// Wwise initialization soundbank loaded before regular banks.
///
/// Lumberyard reference: `dev/Gems/AudioEngineWwise/Code/Source/Engine/Config_wwise.h:25`.
pub const WWISE_INIT_BANK: &str = "init.bnk";
