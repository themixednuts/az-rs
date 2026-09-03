use std::fmt;

/// Runtime session mode. Renamed from `RuntimeRole` (ADR 0041).
///
/// Mode never changes composition validity; it is wire/runtime policy only.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum SessionMode {
    Live,
    EditorWorld,
    PlayPreview,
    ServerPreview,
    Validation,
    Thumbnail,
    Bake,
}

impl fmt::Display for SessionMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Live => "live",
            Self::EditorWorld => "editor-world",
            Self::PlayPreview => "play-preview",
            Self::ServerPreview => "server-preview",
            Self::Validation => "validation",
            Self::Thumbnail => "thumbnail",
            Self::Bake => "bake",
        })
    }
}
