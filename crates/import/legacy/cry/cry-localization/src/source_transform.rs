//! Legacy localization XML import transform.

use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, normalize_source_path,
};
use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize};

use crate::{
    LocalizationDocument, LocalizationEntry, LocalizationParseError, LocalizationString,
    source_schemas,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalizationSource {
    pub source_path: String,
    pub locale: String,
    pub namespace: String,
    pub entries: Vec<LocalizationSourceEntry>,
}

impl LocalizationSource {
    #[must_use]
    pub fn from_legacy(source_path: &str, document: &LocalizationDocument<'_>) -> Self {
        let source_path = normalize_source_path(source_path);
        let (locale, namespace) = localization_identity(&source_path);
        Self {
            source_path,
            locale,
            namespace,
            entries: document
                .entries()
                .iter()
                .map(LocalizationSourceEntry::from)
                .collect(),
        }
    }

    /// Serialises this source to pretty-printed RON bytes with a trailing
    /// newline.
    ///
    /// # Errors
    ///
    /// Returns the [`ron::Error`] the serializer reports. The fields here are
    /// plain strings and enums that RON always accepts, so this is reachable
    /// only through a serializer-internal failure.
    pub fn to_ron_bytes(&self) -> Result<Vec<u8>, ron::Error> {
        let ron = ron::ser::to_string_pretty(self, PrettyConfig::default())?;
        Ok(format!("{ron}\n").into_bytes())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LocalizationSourceEntry {
    Text {
        key: String,
        text: String,
        attributes: Vec<LocalizationSourceAttribute>,
    },
    Nil {
        text: String,
        attributes: Vec<LocalizationSourceAttribute>,
    },
}

impl From<&LocalizationEntry<'_>> for LocalizationSourceEntry {
    fn from(entry: &LocalizationEntry<'_>) -> Self {
        match entry {
            LocalizationEntry::String(entry) => Self::from_string(entry),
            LocalizationEntry::Nil(entry) => Self::Nil {
                text: entry.value().to_string(),
                attributes: attributes(entry.attributes()),
            },
        }
    }
}

impl LocalizationSourceEntry {
    fn from_string(entry: &LocalizationString<'_>) -> Self {
        Self::Text {
            key: entry.key().to_string(),
            text: entry.value().to_string(),
            attributes: attributes(entry.attributes()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalizationSourceAttribute {
    pub name: String,
    pub value: String,
}

fn attributes(attributes: &[crate::LocalizationAttribute<'_>]) -> Vec<LocalizationSourceAttribute> {
    attributes
        .iter()
        .map(|attribute| LocalizationSourceAttribute {
            name: attribute.name().to_string(),
            value: attribute.value().to_string(),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LocalizationSourceTransform;

impl LegacySourceTransform for LocalizationSourceTransform {
    type Error = LocalizationSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        let document = LocalizationDocument::parse_bytes(input.bytes).map_err(|source| {
            LocalizationSourceTransformError::Parse {
                path: input.source_path.to_string(),
                source,
            }
        })?;
        let source = LocalizationSource::from_legacy(&input.source_path, &document);
        Ok(LegacySourceOutput::authoring_source(
            localization_source_path(&input.source_path),
            source_schemas::LOCALIZATION,
            source.to_ron_bytes()?,
        ))
    }
}

#[must_use]
pub fn localization_source_path(source_path: &str) -> String {
    let normalized = normalize_source_path(source_path);
    let stem = strip_localization_suffix(&normalized);
    format!("{stem}.loc.ron")
}

fn localization_identity(source_path: &str) -> (String, String) {
    let stem = strip_localization_suffix(source_path);
    let rest = stem.strip_prefix("localization/").unwrap_or(stem);
    let Some((locale, namespace)) = rest.split_once('/') else {
        return (String::new(), rest.to_string());
    };
    (locale.to_string(), namespace.to_string())
}

fn strip_localization_suffix(path: &str) -> &str {
    path.strip_suffix(".loc.xml")
        .or_else(|| path.strip_suffix(".loc"))
        .unwrap_or(path)
}

#[derive(Debug, thiserror::Error)]
pub enum LocalizationSourceTransformError {
    #[error("parse localization XML {path:?}: {source}")]
    Parse {
        path: String,
        #[source]
        source: LocalizationParseError,
    },
    #[error("serialize localization source RON: {0}")]
    Serialize(#[from] ron::Error),
}

#[cfg(test)]
mod tests {
    use az_asset_builder::{LegacySourceInput, LegacySourceTransform};

    use super::*;

    #[test]
    fn source_transform_emits_loc_ron_authoring_source() {
        let legacy = br#"<resources>
  <string key="Quest_1" speaker="Grace" rel_version="Launch">Hello &amp; goodbye</string>
  <string rel_version="PTR" xsi:nil="true"/>
</resources>"#;

        let output = LocalizationSourceTransform
            .transform(LegacySourceInput::new(
                "Localization/en-us/Quests/Main.loc.xml",
                legacy,
            ))
            .unwrap();

        let artifact = output.artifact().expect("authoring artifact");
        assert_eq!(artifact.path, "localization/en-us/quests/main.loc.ron");
        assert_eq!(artifact.schema, source_schemas::LOCALIZATION);
        let source: LocalizationSource = ron::de::from_bytes(&artifact.bytes).unwrap();
        assert_eq!(source.source_path, "localization/en-us/quests/main.loc.xml");
        assert_eq!(source.locale, "en-us");
        assert_eq!(source.namespace, "quests/main");
        assert_eq!(
            source.entries,
            vec![
                LocalizationSourceEntry::Text {
                    key: "Quest_1".to_string(),
                    text: "Hello & goodbye".to_string(),
                    attributes: vec![
                        LocalizationSourceAttribute {
                            name: "speaker".to_string(),
                            value: "Grace".to_string(),
                        },
                        LocalizationSourceAttribute {
                            name: "rel_version".to_string(),
                            value: "Launch".to_string(),
                        },
                    ],
                },
                LocalizationSourceEntry::Nil {
                    text: String::new(),
                    attributes: vec![
                        LocalizationSourceAttribute {
                            name: "rel_version".to_string(),
                            value: "PTR".to_string(),
                        },
                        LocalizationSourceAttribute {
                            name: "xsi:nil".to_string(),
                            value: "true".to_string(),
                        },
                    ],
                },
            ]
        );
    }

    #[test]
    fn localization_source_paths_replace_legacy_extension() {
        assert_eq!(
            localization_source_path("Localization/en-us/Main.loc.xml"),
            "localization/en-us/main.loc.ron"
        );
        assert_eq!(
            localization_source_path("Localization/en-us/Main.loc"),
            "localization/en-us/main.loc.ron"
        );
    }
}
