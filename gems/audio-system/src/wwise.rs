//! Wwise bank and media asset support.
//!
//! Lumberyard reference: `dev/Gems/AudioEngineWwise/Code/Source/Engine/Config_wwise.h`.

mod asset;
mod bank;
mod config;
mod control_registry;
mod error;
mod ids;
mod media_file;
mod metadata;
mod parse;
mod summary;
mod trigger_bank_map;

pub use asset::*;
pub use bank::*;
pub use config::*;
pub use control_registry::*;
pub use error::*;
pub use ids::*;
pub use media_file::*;
pub use metadata::*;
pub use summary::*;
pub use trigger_bank_map::*;

#[cfg(test)]
mod tests;
