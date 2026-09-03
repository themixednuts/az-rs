use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

use crate::manifest::portable_package_id_error;

/// The single logical namespace shared by every native asset source root.
pub const ASSET_NAMESPACE_MOUNT: &str = "@assets@";

/// The namespace name inside [`ASSET_NAMESPACE_MOUNT`]'s `@` delimiters.
const ASSET_NAMESPACE_NAME: &str = "assets";

/// Where one source root sits in the native asset namespace (ADR 0034).
///
/// Tiers are declared strongest first and compare in that order, so ordering
/// a root list by tier is precedence order: project content shadows gem
/// content and engine content always sits below both, with no manifest
/// managing numbers. Roots inside one tier keep the resolved closure order
/// they were produced in.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum AssetRootTier {
    /// Roots the project declares in its own manifest.
    Project,
    /// Roots contributed by gems homed in the project.
    ProjectGem,
    /// Roots contributed by gems acquired from a registry.
    RegistryGem,
    /// Roots contributed by gems homed in the engine.
    EngineGem,
}

impl fmt::Display for AssetRootTier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Project => "project",
            Self::ProjectGem => "project-gem",
            Self::RegistryGem => "registry-gem",
            Self::EngineGem => "engine-gem",
        })
    }
}

/// A validated virtual mount point in the native asset namespace.
///
/// The grammar is `@`, a namespace name of ASCII lowercase letters, digits,
/// `-`, or `_`, then `@`. Exactly one namespace exists
/// ([`ASSET_NAMESPACE_MOUNT`]); every other well-formed name is rejected as
/// unknown so a typo fails at manifest load instead of mounting nothing.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AssetMount(String);

impl AssetMount {
    /// The one namespace every native source root mounts into.
    #[must_use]
    pub fn assets() -> Self {
        Self(ASSET_NAMESPACE_MOUNT.to_string())
    }

    /// Validate a declared mount point.
    ///
    /// # Errors
    ///
    /// Returns [`AssetMountError`] when the value is not `@`-delimited, names
    /// an empty or non-portable namespace, or names a namespace the engine
    /// does not publish.
    pub fn parse(value: &str) -> Result<Self, AssetMountError> {
        Self::validate(value)?;
        Ok(Self(value.to_string()))
    }

    fn validate(value: &str) -> Result<(), AssetMountError> {
        let Some(name) = value
            .strip_prefix('@')
            .and_then(|rest| rest.strip_suffix('@'))
        else {
            return Err(AssetMountError::Delimiters {
                mount: value.to_string(),
            });
        };
        if name.is_empty() {
            return Err(AssetMountError::EmptyNamespace);
        }
        if let Some(character) = name
            .chars()
            .find(|ch| !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '_')))
        {
            return Err(AssetMountError::UnsupportedCharacter {
                mount: value.to_string(),
                character,
            });
        }
        if name != ASSET_NAMESPACE_NAME {
            return Err(AssetMountError::UnknownNamespace {
                mount: value.to_string(),
            });
        }
        Ok(())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for AssetMount {
    fn default() -> Self {
        Self::assets()
    }
}

impl fmt::Display for AssetMount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl AsRef<str> for AssetMount {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl TryFrom<String> for AssetMount {
    type Error = AssetMountError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::validate(&value)?;
        Ok(Self(value))
    }
}

impl From<AssetMount> for String {
    fn from(mount: AssetMount) -> Self {
        mount.0
    }
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum AssetMountError {
    #[error("mount `{mount}` must be delimited by `@`, as in `{ASSET_NAMESPACE_MOUNT}`")]
    Delimiters { mount: String },
    #[error("mount namespace cannot be empty")]
    EmptyNamespace,
    #[error(
        "mount `{mount}` contains unsupported character `{character}`; use ASCII lowercase letters, digits, '-', or '_'"
    )]
    UnsupportedCharacter { mount: String, character: char },
    #[error(
        "mount `{mount}` names an unknown namespace; the only namespace is `{ASSET_NAMESPACE_MOUNT}`"
    )]
    UnknownNamespace { mount: String },
}

/// A stable, machine-portable identity for one native source root.
///
/// The grammar is `<owner>:<owner-id>:assets[:<root>]`, where `<owner>` is
/// `project` or `gem`, and `<owner-id>` and `<root>` are portable package
/// ids. Keys travel in `azoth.lock` and in the asset database, so a
/// malformed one is rejected where it is read rather than silently failing
/// to join later.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PortableKey(String);

/// The owner channel a [`PortableKey`] names.
pub const PORTABLE_KEY_PROJECT_OWNER: &str = "project";
/// The owner channel a gem-contributed [`PortableKey`] names.
pub const PORTABLE_KEY_GEM_OWNER: &str = "gem";
/// The fixed third segment naming the asset channel.
const PORTABLE_KEY_ASSET_CHANNEL: &str = "assets";

impl PortableKey {
    /// The key for a project's default asset root.
    #[must_use]
    pub fn project_assets(project_id: &str) -> Self {
        Self(format!(
            "{PORTABLE_KEY_PROJECT_OWNER}:{project_id}:{PORTABLE_KEY_ASSET_CHANNEL}"
        ))
    }

    /// The key for one of a project's declared asset roots.
    #[must_use]
    pub fn project_asset_root(project_id: &str, root_id: &str) -> Self {
        if root_id == PORTABLE_KEY_PROJECT_OWNER {
            Self::project_assets(project_id)
        } else {
            Self(format!("{}:{root_id}", Self::project_assets(project_id).0))
        }
    }

    /// The key for a gem's default asset contribution.
    #[must_use]
    pub fn gem_assets(gem_id: &str) -> Self {
        Self(format!(
            "{PORTABLE_KEY_GEM_OWNER}:{gem_id}:{PORTABLE_KEY_ASSET_CHANNEL}"
        ))
    }

    /// The key for one named asset contribution of a gem.
    #[must_use]
    pub fn gem_asset_contribution(gem_id: &str, contribution_id: &str) -> Self {
        if contribution_id == PORTABLE_KEY_ASSET_CHANNEL {
            Self::gem_assets(gem_id)
        } else {
            Self(format!("{}:{contribution_id}", Self::gem_assets(gem_id).0))
        }
    }

    /// Validate a portable key read from a lock, a database, or an RPC.
    ///
    /// # Errors
    ///
    /// Returns [`PortableKeyError`] when the segment count, the owner
    /// channel, the asset channel, or any id segment breaks the grammar.
    pub fn parse(value: &str) -> Result<Self, PortableKeyError> {
        Self::validate(value)?;
        Ok(Self(value.to_string()))
    }

    fn validate(value: &str) -> Result<(), PortableKeyError> {
        let segments = value.split(':').collect::<Vec<_>>();
        if !matches!(segments.len(), 3 | 4) {
            return Err(PortableKeyError::SegmentCount {
                key: value.to_string(),
                segments: segments.len(),
            });
        }
        if !matches!(
            segments[0],
            PORTABLE_KEY_PROJECT_OWNER | PORTABLE_KEY_GEM_OWNER
        ) {
            return Err(PortableKeyError::Owner {
                key: value.to_string(),
                owner: segments[0].to_string(),
            });
        }
        if segments[2] != PORTABLE_KEY_ASSET_CHANNEL {
            return Err(PortableKeyError::Channel {
                key: value.to_string(),
                channel: segments[2].to_string(),
            });
        }
        for segment in [Some(segments[1]), segments.get(3).copied()]
            .into_iter()
            .flatten()
        {
            if let Some(reason) = portable_package_id_error(segment) {
                return Err(PortableKeyError::Segment {
                    key: value.to_string(),
                    segment: segment.to_string(),
                    reason,
                });
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PortableKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl AsRef<str> for PortableKey {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl TryFrom<String> for PortableKey {
    type Error = PortableKeyError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::validate(&value)?;
        Ok(Self(value))
    }
}

impl From<PortableKey> for String {
    fn from(key: PortableKey) -> Self {
        key.0
    }
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum PortableKeyError {
    #[error(
        "portable key `{key}` has {segments} `:`-separated segments; expected `<owner>:<owner-id>:assets[:<root>]`"
    )]
    SegmentCount { key: String, segments: usize },
    #[error(
        "portable key `{key}` names owner channel `{owner}`; expected `{PORTABLE_KEY_PROJECT_OWNER}` or `{PORTABLE_KEY_GEM_OWNER}`"
    )]
    Owner { key: String, owner: String },
    #[error(
        "portable key `{key}` names channel `{channel}`; expected `{PORTABLE_KEY_ASSET_CHANNEL}`"
    )]
    Channel { key: String, channel: String },
    #[error("portable key `{key}` segment `{segment}` is not portable: {reason}")]
    Segment {
        key: String,
        segment: String,
        reason: String,
    },
}

/// A normalized path relative to a native asset source root.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NativeAssetPath(String);

impl NativeAssetPath {
    /// Validate and normalize a source path to Unicode NFC, lowercase, and
    /// forward-slash separators.
    ///
    /// # Errors
    ///
    /// Returns [`NativeAssetPathError`] when the input cannot be represented
    /// safely and consistently on every supported host filesystem.
    pub fn new(path: &str) -> Result<Self, NativeAssetPathError> {
        if path.is_empty() {
            return Err(NativeAssetPathError::Empty);
        }
        if path.contains('\0') {
            return Err(NativeAssetPathError::Nul);
        }

        let slash_path = path.replace('\\', "/");
        if slash_path.starts_with('/') || has_drive_prefix(&slash_path) {
            return Err(NativeAssetPathError::Absolute);
        }

        let mut components = Vec::new();
        for component in slash_path.split('/') {
            if component.is_empty() {
                return Err(NativeAssetPathError::EmptyComponent);
            }
            if component == "." {
                return Err(NativeAssetPathError::CurrentDirectory);
            }
            if component == ".." {
                return Err(NativeAssetPathError::ParentDirectory);
            }
            if component.ends_with(['.', ' ']) {
                return Err(NativeAssetPathError::TrailingDotOrSpace {
                    component: component.to_string(),
                });
            }

            let normalized = component
                .nfc()
                .collect::<String>()
                .to_lowercase()
                .nfc()
                .collect::<String>();
            if is_platform_reserved_component(&normalized) {
                return Err(NativeAssetPathError::ReservedComponent {
                    component: component.to_string(),
                });
            }
            components.push(normalized);
        }

        Ok(Self(components.join("/")))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn virtual_path(&self) -> String {
        format!("{ASSET_NAMESPACE_MOUNT}/{}", self.0)
    }
}

impl fmt::Display for NativeAssetPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl AsRef<str> for NativeAssetPath {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum NativeAssetPathError {
    #[error("path cannot be empty")]
    Empty,
    #[error("path contains a NUL character")]
    Nul,
    #[error("absolute, drive-prefixed, and UNC paths are not allowed")]
    Absolute,
    #[error("path contains an empty component")]
    EmptyComponent,
    #[error("path contains a `.` component")]
    CurrentDirectory,
    #[error("path contains a `..` component")]
    ParentDirectory,
    #[error("component `{component}` ends with a dot or space")]
    TrailingDotOrSpace { component: String },
    #[error("component `{component}` is reserved by a supported platform")]
    ReservedComponent { component: String },
}

fn has_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn is_platform_reserved_component(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    matches!(stem, "con" | "prn" | "aux" | "nul")
        || stem
            .strip_prefix("com")
            .is_some_and(is_reserved_device_number)
        || stem
            .strip_prefix("lpt")
            .is_some_and(is_reserved_device_number)
}

fn is_reserved_device_number(value: &str) -> bool {
    matches!(value, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_asset_path_normalizes_case_slashes_and_unicode() {
        let path = NativeAssetPath::new("Rendering\\Textures\\CAF\u{00c9}.PNG").unwrap();

        assert_eq!(path.as_str(), "rendering/textures/caf\u{00e9}.png");
        assert_eq!(
            path.virtual_path(),
            "@assets@/rendering/textures/caf\u{00e9}.png"
        );
    }

    #[test]
    fn native_asset_path_rejects_cross_platform_unsafe_components() {
        let absolute = std::env::temp_dir().join("assets/a.png");
        let absolute = absolute.to_string_lossy();
        for path in [
            absolute.as_ref(),
            "//server/share/a.png",
            "world/../a.ron",
            "world//a.ron",
            "world/con.txt",
            "world/name. ",
        ] {
            assert!(NativeAssetPath::new(path).is_err(), "{path} must fail");
        }
    }

    #[test]
    fn asset_root_tiers_order_project_content_above_engine_content() {
        let mut tiers = vec![
            AssetRootTier::EngineGem,
            AssetRootTier::Project,
            AssetRootTier::RegistryGem,
            AssetRootTier::ProjectGem,
        ];
        tiers.sort_unstable();

        assert_eq!(
            tiers,
            vec![
                AssetRootTier::Project,
                AssetRootTier::ProjectGem,
                AssetRootTier::RegistryGem,
                AssetRootTier::EngineGem,
            ]
        );
    }

    #[test]
    fn asset_mount_accepts_only_the_published_namespace() {
        assert_eq!(
            AssetMount::parse(ASSET_NAMESPACE_MOUNT).unwrap(),
            AssetMount::assets()
        );
        assert_eq!(AssetMount::default().as_str(), ASSET_NAMESPACE_MOUNT);

        assert!(matches!(
            AssetMount::parse("assets"),
            Err(AssetMountError::Delimiters { .. })
        ));
        assert!(matches!(
            AssetMount::parse("@@"),
            Err(AssetMountError::EmptyNamespace)
        ));
        assert!(matches!(
            AssetMount::parse("@Assets@"),
            Err(AssetMountError::UnsupportedCharacter { character: 'A', .. })
        ));
        assert!(matches!(
            AssetMount::parse("@products@"),
            Err(AssetMountError::UnknownNamespace { .. })
        ));
    }

    #[test]
    fn portable_key_constructors_agree_with_the_grammar() {
        for key in [
            PortableKey::project_assets("local.test"),
            PortableKey::project_asset_root("local.test", "project"),
            PortableKey::project_asset_root("local.test", "dlc-1"),
            PortableKey::gem_assets("azoth.animation"),
            PortableKey::gem_asset_contribution("azoth.animation", "assets"),
            PortableKey::gem_asset_contribution("azoth.animation", "clips"),
        ] {
            PortableKey::parse(key.as_str())
                .unwrap_or_else(|error| panic!("`{key}` must parse: {error}"));
        }

        assert_eq!(
            PortableKey::project_asset_root("local.test", "project").as_str(),
            "project:local.test:assets"
        );
        assert_eq!(
            PortableKey::gem_asset_contribution("azoth.animation", "clips").as_str(),
            "gem:azoth.animation:assets:clips"
        );
    }

    #[test]
    fn portable_key_rejects_malformed_values() {
        assert!(matches!(
            PortableKey::parse("gem:azoth.animation"),
            Err(PortableKeyError::SegmentCount { segments: 2, .. })
        ));
        assert!(matches!(
            PortableKey::parse("engine:azoth.animation:assets"),
            Err(PortableKeyError::Owner { .. })
        ));
        assert!(matches!(
            PortableKey::parse("gem:azoth.animation:products"),
            Err(PortableKeyError::Channel { .. })
        ));
        assert!(matches!(
            PortableKey::parse("gem:azoth animation:assets"),
            Err(PortableKeyError::Segment { .. })
        ));
    }
}
