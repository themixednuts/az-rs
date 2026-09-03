//! Materialize the proprietary Oodle SDKs out of a licensed Unreal Engine
//! checkout.
//!
//! Unreal ships Oodle as pack blobs listed in `Engine/Build/Commit.gitdeps.xml`
//! rather than as files in the repository, so the libraries have to be fetched
//! from Epic's CDN and sliced out of their packs. The stages are separate
//! modules: [`manifest`] parses the dependency manifest, [`mod@select`] joins its
//! tables into a plan, [`platform`] resolves what `--platform` asked for, and
//! [`mod@materialize`] extracts and writes. Everything is pure over its inputs
//! except [`materialize()`] and [`copy_headers()`], which take the filesystem
//! and the pack transport as parameters.

pub mod manifest;
pub mod materialize;
pub mod platform;
pub mod select;

pub use manifest::{Blob, Manifest, ManifestError, ManifestFile, Pack};
pub use materialize::{
    HeaderError, MaterializeError, PackFetchError, PackSource, copy_headers, materialize,
};
pub use platform::{Platform, PlatformRequest, UnknownHostError, host_platform, resolve_platforms};
pub use select::{Plan, PlannedFile, Product, SelectionError, select};
