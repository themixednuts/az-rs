//! Bevy-native Prefab authoring model.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use az_core::EntityId;
use bevy_reflect::{PartialReflect, TypeInfo};
use thiserror::Error;

/// Current source-container version for typed Prefab documents.
pub const PREFAB_DOCUMENT_VERSION: u32 = 1;
/// Logical source type advertised to project workflow and asset-builder
/// routing. The source body is a [`PrefabDocument`], not an authored schema
/// object graph.
pub const PREFAB_SOURCE_TYPE: &str = "azoth.prefab.Prefab";

macro_rules! alias_type {
    ($name:ident, $label:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// Creates a stable, human-readable alias.
            ///
            /// # Errors
            ///
            /// Returns [`PrefabDocumentError::InvalidName`] when `value` is
            /// empty or carries leading/trailing whitespace. Control characters
            /// are accepted here, unlike in structural identifiers.
            pub fn new(value: impl Into<String>) -> Result<Self, PrefabDocumentError> {
                let value = value.into();
                // Aliases are free-text display names, not structural
                // identifiers like an asset path or TypePath — no native
                // constraint requires them to be single-line (shipped MTX
                // camp-skin entity aliases carry an authored newline), so
                // control characters are permitted here specifically.
                // `validate_name`'s other callers (asset path, TypePath)
                // keep the strict check.
                validate_name(&value, $label, true)?;
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            #[must_use]
            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl TryFrom<String> for $name {
            type Error = PrefabDocumentError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }
    };
}

alias_type!(EntityAlias, "entity alias");
alias_type!(InstanceAlias, "instance alias");

/// Stable source identity for a nested Prefab.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrefabAssetPath(String);

impl PrefabAssetPath {
    /// Creates a nested-Prefab source path.
    ///
    /// # Errors
    ///
    /// Returns [`PrefabDocumentError::InvalidName`] when `value` is empty,
    /// untrimmed, contains a control character, or contains a backslash
    /// instead of a forward slash.
    pub fn new(value: impl Into<String>) -> Result<Self, PrefabDocumentError> {
        let value = value.into();
        validate_name(&value, "Prefab asset path", false)?;
        if value.contains('\\') {
            return Err(PrefabDocumentError::InvalidName {
                kind: "Prefab asset path",
                value,
                reason: "use forward slashes in source asset paths",
            });
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PrefabAssetPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Additional runtime catalog lookup path for a cooked Prefab.
///
/// The path remains metadata on the canonical product. It does not name a
/// second output file or create another product identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrefabCatalogAlias(String);

impl PrefabCatalogAlias {
    /// Creates an additional runtime catalog lookup path.
    ///
    /// # Errors
    ///
    /// Returns [`PrefabDocumentError::InvalidName`] when `value` is empty,
    /// untrimmed, contains a control character, or contains a backslash
    /// instead of a forward slash.
    pub fn new(value: impl Into<String>) -> Result<Self, PrefabDocumentError> {
        let value = value.into();
        validate_name(&value, "Prefab catalog alias", false)?;
        if value.contains('\\') {
            return Err(PrefabDocumentError::InvalidName {
                kind: "Prefab catalog alias",
                value,
                reason: "use forward slashes in catalog paths",
            });
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PrefabCatalogAlias {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A reflected path made exclusively from stable named fields.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReflectedPath(Vec<String>);

impl ReflectedPath {
    #[must_use]
    pub const fn root() -> Self {
        Self(Vec::new())
    }

    /// Creates a reflected path from already-split named-field segments.
    ///
    /// # Errors
    ///
    /// Returns [`PrefabDocumentError::InvalidPathSegment`] for any segment that
    /// is empty or contains a character other than an underscore or an
    /// alphanumeric.
    pub fn new(
        segments: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, PrefabDocumentError> {
        let segments = segments
            .into_iter()
            .map(Into::into)
            .collect::<Vec<String>>();
        for segment in &segments {
            validate_path_segment(segment)?;
        }
        Ok(Self(segments))
    }

    /// Parses a dot-separated reflected path. An empty string and `"."` both
    /// denote the root path.
    ///
    /// # Errors
    ///
    /// Forwards [`PrefabDocumentError::InvalidPathSegment`] from
    /// [`Self::new`] for any malformed segment between the dots.
    pub fn parse(value: &str) -> Result<Self, PrefabDocumentError> {
        if value.is_empty() || value == "." {
            return Ok(Self::root());
        }
        let value = value.strip_prefix('.').unwrap_or(value);
        Self::new(value.split('.'))
    }

    #[must_use]
    pub fn segments(&self) -> &[String] {
        &self.0
    }

    #[must_use]
    pub const fn is_root(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for ReflectedPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return formatter.write_str(".");
        }
        for segment in &self.0 {
            write!(formatter, ".{segment}")?;
        }
        Ok(())
    }
}

/// A reflected source value that retains only explicitly represented fields.
pub struct SparseValue {
    value: Box<dyn PartialReflect>,
    type_info: &'static TypeInfo,
}

impl SparseValue {
    /// Wraps a reflected value that carries its represented type information.
    ///
    /// # Errors
    ///
    /// Returns [`PrefabDocumentError::MissingRepresentedTypeInfo`] when `value`
    /// has no represented `TypeInfo`, which is the case for bare dynamic values.
    pub fn try_new(value: Box<dyn PartialReflect>) -> Result<Self, PrefabDocumentError> {
        let type_info = value
            .get_represented_type_info()
            .ok_or(PrefabDocumentError::MissingRepresentedTypeInfo)?;
        Ok(Self { value, type_info })
    }

    /// Wraps a reflected value and asserts that it represents `expected`.
    ///
    /// # Errors
    ///
    /// Forwards [`PrefabDocumentError::MissingRepresentedTypeInfo`] from
    /// [`Self::try_new`], and returns
    /// [`PrefabDocumentError::ReflectedTypeMismatch`] when the represented type
    /// id differs from `expected`.
    pub fn for_type(
        value: Box<dyn PartialReflect>,
        expected: &'static TypeInfo,
    ) -> Result<Self, PrefabDocumentError> {
        let result = Self::try_new(value)?;
        if result.type_info.type_id() != expected.type_id() {
            return Err(PrefabDocumentError::ReflectedTypeMismatch {
                expected: expected.type_path().to_owned(),
                actual: result.type_info.type_path().to_owned(),
            });
        }
        Ok(result)
    }

    #[must_use]
    pub fn value(&self) -> &dyn PartialReflect {
        self.value.as_ref()
    }

    #[must_use]
    pub fn value_mut(&mut self) -> &mut dyn PartialReflect {
        self.value.as_mut()
    }

    #[must_use]
    pub const fn type_info(&self) -> &'static TypeInfo {
        self.type_info
    }

    #[must_use]
    pub fn type_path(&self) -> &'static str {
        self.type_info.type_path()
    }

    #[must_use]
    pub fn into_value(self) -> Box<dyn PartialReflect> {
        self.value
    }
}

impl Clone for SparseValue {
    fn clone(&self) -> Self {
        let value = self.value.reflect_clone().map_or_else(
            |_| self.value.to_dynamic(),
            bevy_reflect::PartialReflect::into_partial_reflect,
        );
        Self {
            value,
            type_info: self.type_info,
        }
    }
}

impl fmt::Debug for SparseValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SparseValue")
            .field("type_path", &self.type_info.type_path())
            .field("value", &format_args!("{:?}", self.value))
            .finish()
    }
}

impl PartialEq for SparseValue {
    fn eq(&self, other: &Self) -> bool {
        self.type_info.type_id() == other.type_info.type_id()
            && self.value.reflect_partial_eq(other.value.as_ref()) == Some(true)
    }
}

/// One authored entity and its sparse component bodies, keyed by canonical tag.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PrefabEntity {
    /// Optional source identity authored on the native AZ entity.
    pub entity_id: Option<EntityId>,
    pub parent: Option<EntityAlias>,
    pub components: BTreeMap<String, SparseValue>,
}

/// The address of an override inside a nested Prefab source.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OverrideTarget {
    pub instance_chain: Vec<InstanceAlias>,
    pub entity: EntityAlias,
    pub component: String,
    pub path: ReflectedPath,
}

impl OverrideTarget {
    /// Addresses one override inside a nested Prefab source.
    ///
    /// # Errors
    ///
    /// Returns [`PrefabDocumentError::InvalidName`] when `component` is not a
    /// usable `TypePath`: empty, untrimmed, or carrying control characters.
    pub fn new(
        instance_chain: Vec<InstanceAlias>,
        entity: EntityAlias,
        component: impl Into<String>,
        path: ReflectedPath,
    ) -> Result<Self, PrefabDocumentError> {
        let component = component.into();
        validate_name(&component, "component TypePath", false)?;
        Ok(Self {
            instance_chain,
            entity,
            component,
            path,
        })
    }
}

/// Structural action carried by a nested-instance override.
#[derive(Debug, Clone, PartialEq)]
pub enum OverrideAction {
    Set(SparseValue),
    Clear,
    Insert { index: usize, value: SparseValue },
    Remove { index: usize },
    Move { from: usize, to: usize },
}

/// One sparse nested-instance override.
#[derive(Debug, Clone, PartialEq)]
pub struct OverrideOperation {
    pub target: OverrideTarget,
    pub action: OverrideAction,
}

/// A source-retained nested Prefab instance.
#[derive(Debug, Clone, PartialEq)]
pub struct PrefabInstance {
    pub source: PrefabAssetPath,
    pub parent: Option<EntityAlias>,
    pub overrides: Vec<OverrideOperation>,
}

/// Typed, sparse Prefab source. Cooking, rather than this model, flattens nesting.
#[derive(Debug, Clone, PartialEq)]
pub struct PrefabDocument {
    pub version: u32,
    pub type_versions: BTreeMap<String, u32>,
    pub catalog_aliases: BTreeSet<PrefabCatalogAlias>,
    pub entities: BTreeMap<EntityAlias, PrefabEntity>,
    pub instances: BTreeMap<InstanceAlias, PrefabInstance>,
}

impl Default for PrefabDocument {
    fn default() -> Self {
        Self {
            version: PREFAB_DOCUMENT_VERSION,
            type_versions: BTreeMap::new(),
            catalog_aliases: BTreeSet::new(),
            entities: BTreeMap::new(),
            instances: BTreeMap::new(),
        }
    }
}

impl PrefabDocument {
    /// Assembles a document from already-validated entity and instance lists.
    ///
    /// # Errors
    ///
    /// Returns [`PrefabDocumentError::DuplicateAlias`] when `entities` or
    /// `instances` yields the same alias twice.
    pub fn try_from_parts(
        version: u32,
        type_versions: BTreeMap<String, u32>,
        entities: impl IntoIterator<Item = (EntityAlias, PrefabEntity)>,
        instances: impl IntoIterator<Item = (InstanceAlias, PrefabInstance)>,
    ) -> Result<Self, PrefabDocumentError> {
        let entities = collect_unique(entities, "entity")?;
        let instances = collect_unique(instances, "instance")?;
        Ok(Self {
            version,
            type_versions,
            catalog_aliases: BTreeSet::new(),
            entities,
            instances,
        })
    }

    pub fn direct_dependencies(&self) -> impl Iterator<Item = &PrefabAssetPath> {
        self.instances.values().map(|instance| &instance.source)
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PrefabDocumentError {
    #[error("invalid {kind} `{value}`: {reason}")]
    InvalidName {
        kind: &'static str,
        value: String,
        reason: &'static str,
    },
    #[error("invalid reflected path segment `{segment}`")]
    InvalidPathSegment { segment: String },
    #[error("duplicate {kind} alias `{alias}`")]
    DuplicateAlias { kind: &'static str, alias: String },
    #[error("sparse reflected value has no represented TypeInfo")]
    MissingRepresentedTypeInfo,
    #[error("sparse reflected value represents `{actual}`, expected `{expected}`")]
    ReflectedTypeMismatch { expected: String, actual: String },
}

fn validate_name(
    value: &str,
    kind: &'static str,
    allow_control_characters: bool,
) -> Result<(), PrefabDocumentError> {
    if value.is_empty() || value != value.trim() {
        return Err(PrefabDocumentError::InvalidName {
            kind,
            value: value.to_owned(),
            reason: "value must be non-empty and trimmed",
        });
    }
    // No native Lumberyard constraint requires a single-line name — this
    // check is purely self-imposed by our own document format. Structural
    // identifiers (asset paths, TypePaths) still forbid control characters;
    // free-text aliases (`alias_type!`) permit them, since shipped entity
    // aliases carry an authored newline (MTX camp-skin display names) that
    // is legitimate data, not corruption. Never sanitize/rewrite here —
    // that would risk silently colliding two distinct alias values.
    if !allow_control_characters && value.chars().any(char::is_control) {
        return Err(PrefabDocumentError::InvalidName {
            kind,
            value: value.to_owned(),
            reason: "control characters are not allowed",
        });
    }
    Ok(())
}

fn validate_path_segment(segment: &str) -> Result<(), PrefabDocumentError> {
    if segment.is_empty()
        || !segment
            .chars()
            .all(|character| character == '_' || character.is_alphanumeric())
    {
        return Err(PrefabDocumentError::InvalidPathSegment {
            segment: segment.to_owned(),
        });
    }
    Ok(())
}

fn collect_unique<K, V>(
    values: impl IntoIterator<Item = (K, V)>,
    kind: &'static str,
) -> Result<BTreeMap<K, V>, PrefabDocumentError>
where
    K: Ord + fmt::Display,
{
    let mut result = BTreeMap::new();
    for (key, value) in values {
        let alias = key.to_string();
        if result.insert(key, value).is_some() {
            return Err(PrefabDocumentError::DuplicateAlias { kind, alias });
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_alias_permits_control_characters() {
        // Shipped MTX camp-skin entities carry an authored newline in their
        // display name, e.g. "25 (MTX) CampSkin_Firelight01_B\n(Mar2023)" —
        // legitimate data, not corruption. No native constraint requires a
        // single-line alias.
        assert!(EntityAlias::new("25 (MTX) CampSkin_Firelight01_B\n(Mar2023)").is_ok());
    }

    #[test]
    fn instance_alias_permits_control_characters() {
        assert!(InstanceAlias::new("a\nb").is_ok());
    }

    #[test]
    fn entity_alias_still_rejects_empty_or_untrimmed() {
        assert!(EntityAlias::new("").is_err());
        assert!(EntityAlias::new(" padded ").is_err());
    }

    #[test]
    fn prefab_asset_path_still_rejects_control_characters() {
        // Structural identifiers keep the strict check — only free-text
        // aliases were relaxed.
        assert!(matches!(
            PrefabAssetPath::new("a\nb"),
            Err(PrefabDocumentError::InvalidName {
                reason: "control characters are not allowed",
                ..
            })
        ));
    }
}
