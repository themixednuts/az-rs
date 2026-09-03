//! Legacy `CrySystem` config import transform.

use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, normalize_source_path,
};
use cry_system::{CommentMarker as LegacyCommentMarker, ConfigFile, ConfigLine, ConfigParseError};
use serde::{Deserialize, Serialize};

use crate::source_schemas;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigSource {
    pub source_path: String,
    pub lines: Vec<ConfigSourceLine>,
}

impl ConfigSource {
    #[must_use]
    pub fn from_legacy(source_path: &str, config: &ConfigFile<'_>) -> Self {
        Self {
            source_path: normalize_source_path(source_path),
            lines: config.lines().iter().map(ConfigSourceLine::from).collect(),
        }
    }

    /// Serialises this source to pretty-printed TOML bytes.
    ///
    /// # Errors
    ///
    /// Returns the [`toml::ser::Error`] the serializer reports. TOML cannot
    /// represent every Rust shape, so this is reachable if a line variant ever
    /// gains a field TOML has no encoding for; the current string-and-enum
    /// fields always serialize.
    pub fn to_toml_bytes(&self) -> Result<Vec<u8>, toml::ser::Error> {
        toml::to_string_pretty(self).map(String::into_bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConfigSourceLine {
    Blank,
    Comment {
        marker: ConfigCommentMarker,
        text: String,
    },
    Section {
        name: String,
    },
    Assignment {
        key: String,
        value: String,
    },
    Command {
        name: String,
        args: String,
    },
}

impl From<&ConfigLine<'_>> for ConfigSourceLine {
    fn from(line: &ConfigLine<'_>) -> Self {
        match line {
            ConfigLine::Blank => Self::Blank,
            ConfigLine::Comment(comment) => Self::Comment {
                marker: ConfigCommentMarker::from(comment.marker()),
                text: comment.text().to_string(),
            },
            ConfigLine::Section(section) => Self::Section {
                name: section.name().to_string(),
            },
            ConfigLine::Assignment(assignment) => Self::Assignment {
                key: assignment.key().to_string(),
                value: assignment.value().to_string(),
            },
            ConfigLine::Command(command) => Self::Command {
                name: command.name().to_string(),
                args: command.args().to_string(),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigCommentMarker {
    Semicolon,
    DoubleDash,
}

impl From<LegacyCommentMarker> for ConfigCommentMarker {
    fn from(marker: LegacyCommentMarker) -> Self {
        match marker {
            LegacyCommentMarker::Semicolon => Self::Semicolon,
            LegacyCommentMarker::DoubleDash => Self::DoubleDash,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConfigSourceTransform;

impl LegacySourceTransform for ConfigSourceTransform {
    type Error = ConfigSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        let config = ConfigFile::parse_bytes(input.bytes).map_err(|source| {
            ConfigSourceTransformError::Parse {
                path: input.source_path.to_string(),
                source,
            }
        })?;
        let source = ConfigSource::from_legacy(&input.source_path, &config);
        let toml = source
            .to_toml_bytes()
            .map_err(ConfigSourceTransformError::SerializeToml)?;
        Ok(LegacySourceOutput::authoring_source(
            config_source_path(&input.source_path),
            source_schemas::CONFIG,
            toml,
        ))
    }
}

#[must_use]
pub fn config_source_path(source_path: &str) -> String {
    let normalized = normalize_source_path(source_path);
    normalized
        .strip_suffix(".cfg")
        .or_else(|| normalized.strip_suffix(".ini"))
        .map_or_else(
            || format!("{normalized}.config.toml"),
            |stem| format!("{stem}.config.toml"),
        )
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigSourceTransformError {
    #[error("parse CrySystem config {path:?}")]
    Parse {
        path: String,
        #[source]
        source: ConfigParseError,
    },
    #[error("serialize config source TOML: {0}")]
    SerializeToml(toml::ser::Error),
}

#[cfg(test)]
mod tests {
    use az_asset_builder::{LegacySourceInput, LegacySourceTransform};

    use super::*;

    #[test]
    fn source_transform_emits_config_toml_authoring_source() {
        let legacy = b"; boot comment
[default]
sys_game_name = \"Example Game\"
bind o \"r_measureOverdraw 0\"
-- trailing comment
";

        let output = ConfigSourceTransform
            .transform(LegacySourceInput::new("Config/Game.cfg", legacy))
            .unwrap();

        let artifact = output.artifact().expect("authoring artifact");
        assert_eq!(artifact.path, "config/game.config.toml");
        assert_eq!(artifact.schema, source_schemas::CONFIG);
        let source = std::str::from_utf8(&artifact.bytes).unwrap();
        assert!(source.contains(r#"source_path = "config/game.cfg""#));
        assert!(source.contains("kind = \"section\""));
        assert!(source.contains("name = \"default\""));
        assert!(source.contains("kind = \"assignment\""));
        assert!(source.contains("key = \"sys_game_name\""));
        assert!(source.contains("kind = \"command\""));

        let parsed: ConfigSource = toml::from_str(source).unwrap();
        assert_eq!(
            parsed.lines,
            vec![
                ConfigSourceLine::Comment {
                    marker: ConfigCommentMarker::Semicolon,
                    text: "boot comment".to_string(),
                },
                ConfigSourceLine::Section {
                    name: "default".to_string(),
                },
                ConfigSourceLine::Assignment {
                    key: "sys_game_name".to_string(),
                    value: "\"Example Game\"".to_string(),
                },
                ConfigSourceLine::Command {
                    name: "bind".to_string(),
                    args: "o \"r_measureOverdraw 0\"".to_string(),
                },
                ConfigSourceLine::Comment {
                    marker: ConfigCommentMarker::DoubleDash,
                    text: "trailing comment".to_string(),
                },
            ]
        );
    }

    #[test]
    fn config_source_paths_replace_legacy_extension() {
        assert_eq!(
            config_source_path("Config/Game.cfg"),
            "config/game.config.toml"
        );
        assert_eq!(
            config_source_path("Config/CVarGroups/sys_spec.ini"),
            "config/cvargroups/sys_spec.config.toml"
        );
    }
}
