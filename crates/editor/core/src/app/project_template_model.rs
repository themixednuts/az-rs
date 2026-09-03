// Expansion drops `cfg(test)`-only names and adds unused ones; it does not compile.
#[allow(clippy::wildcard_imports)]
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProjectTemplateStep {
    Template,
    Configure,
    Gems,
}

impl ProjectTemplateStep {
    pub(super) const ALL: [Self; 3] = [Self::Template, Self::Configure, Self::Gems];

    pub(super) const fn number(self) -> u8 {
        match self {
            Self::Template => 1,
            Self::Configure => 2,
            Self::Gems => 3,
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Template => "Template",
            Self::Configure => "Configure",
            Self::Gems => "Gems",
        }
    }

    pub(super) const fn next(self) -> Self {
        match self {
            Self::Template => Self::Configure,
            Self::Configure | Self::Gems => Self::Gems,
        }
    }

    pub(super) const fn previous(self) -> Self {
        match self {
            Self::Template | Self::Configure => Self::Template,
            Self::Gems => Self::Configure,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProjectTemplateKind {
    Empty,
    ThreeD,
    FirstPerson,
    ThirdPerson,
    TopDown,
    TwoD,
    VrXr,
    Mobile,
    Multiplayer,
}

impl ProjectTemplateKind {
    pub(super) const ALL: [Self; 9] = [
        Self::Empty,
        Self::ThreeD,
        Self::FirstPerson,
        Self::ThirdPerson,
        Self::TopDown,
        Self::TwoD,
        Self::VrXr,
        Self::Mobile,
        Self::Multiplayer,
    ];

    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Empty => "Empty",
            Self::ThreeD => "3D",
            Self::FirstPerson => "First Person",
            Self::ThirdPerson => "Third Person",
            Self::TopDown => "Top-Down",
            Self::TwoD => "2D",
            Self::VrXr => "VR / XR",
            Self::Mobile => "Mobile",
            Self::Multiplayer => "Multiplayer",
        }
    }

    pub(super) const fn description(self) -> &'static str {
        match self {
            Self::Empty => "Core project shell with only required systems.",
            Self::ThreeD => "Lit 3D scene with physics, terrain, and gameplay basics.",
            Self::FirstPerson => "First-person controls, camera, input, and interaction hooks.",
            Self::ThirdPerson => "Character controller, follow camera, animation, and input.",
            Self::TopDown => "Top-down camera, click-to-move structure, and world systems.",
            Self::TwoD => "2D-oriented camera, UI, sprites, and lightweight gameplay.",
            Self::VrXr => "OpenXR-oriented shell with rendering and input placeholders.",
            Self::Mobile => "Mobile-friendly render path, input, and packaging defaults.",
            Self::Multiplayer => "Network-ready project shell with server-friendly defaults.",
        }
    }

    pub(super) const fn icon(self) -> IconName {
        match self {
            Self::Empty => IconName::File,
            Self::ThreeD | Self::TwoD => IconName::LayoutDashboard,
            Self::FirstPerson => IconName::Play,
            Self::ThirdPerson => IconName::Bot,
            Self::TopDown => IconName::GridView,
            Self::VrXr => IconName::Eye,
            Self::Mobile | Self::Multiplayer => IconName::Network,
        }
    }

    /// Every template ships the Vulkan backend today. The arm is exhaustive on
    /// purpose: a new template variant must name its backend rather than
    /// inheriting one from a wildcard.
    pub(super) const fn renderer(self) -> ProjectRendererBackend {
        match self {
            Self::Empty
            | Self::ThreeD
            | Self::FirstPerson
            | Self::ThirdPerson
            | Self::TopDown
            | Self::TwoD
            | Self::VrXr
            | Self::Mobile
            | Self::Multiplayer => ProjectRendererBackend::Vulkan,
        }
    }

    pub(super) const fn topology(self) -> ProjectTopologyChoice {
        match self {
            Self::Multiplayer => ProjectTopologyChoice::MultiplayerClientServer,
            _ => ProjectTopologyChoice::SinglePlayer,
        }
    }

    pub(super) const fn pipeline(self) -> ProjectRenderPipeline {
        match self {
            Self::Mobile => ProjectRenderPipeline::Mobile,
            _ => ProjectRenderPipeline::ForwardPlus,
        }
    }

    pub(super) const fn platforms(self) -> &'static [ProjectTargetPlatform] {
        match self {
            Self::Empty | Self::TopDown | Self::VrXr => &[ProjectTargetPlatform::Windows],
            Self::ThreeD | Self::ThirdPerson => &[
                ProjectTargetPlatform::Windows,
                ProjectTargetPlatform::Linux,
                ProjectTargetPlatform::MacOs,
            ],
            Self::FirstPerson | Self::Multiplayer => {
                &[ProjectTargetPlatform::Windows, ProjectTargetPlatform::Linux]
            }
            Self::TwoD => &[ProjectTargetPlatform::Windows, ProjectTargetPlatform::Web],
            Self::Mobile => &[ProjectTargetPlatform::Android, ProjectTargetPlatform::Ios],
        }
    }

    pub(super) const fn gem_categories(self) -> &'static [&'static str] {
        match self {
            Self::Empty => &["System", "Data"],
            Self::ThreeD | Self::FirstPerson | Self::ThirdPerson | Self::TopDown => &[
                "System",
                "Data",
                "Rendering",
                "Physics",
                "World",
                "Gameplay",
                "Audio",
            ],
            Self::TwoD => &["System", "Data", "Rendering", "UI", "Gameplay", "Audio"],
            Self::VrXr => &["System", "Data", "Rendering", "Physics", "Platform"],
            Self::Mobile => &["System", "Data", "Rendering", "UI", "Platform", "Audio"],
            Self::Multiplayer => &[
                "System",
                "Data",
                "Rendering",
                "Physics",
                "Platform",
                "Gameplay",
                "Audio",
            ],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProjectTopologyChoice {
    SinglePlayer,
    MultiplayerClientServer,
    MultiplayerPeerToPeer,
}

impl ProjectTopologyChoice {
    pub(super) const ALL: [Self; 3] = [
        Self::SinglePlayer,
        Self::MultiplayerClientServer,
        Self::MultiplayerPeerToPeer,
    ];

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::SinglePlayer => "Single-player",
            Self::MultiplayerClientServer => "Multiplayer — client/server",
            Self::MultiplayerPeerToPeer => "Multiplayer — peer-to-peer",
        }
    }

    pub(super) const fn description(self) -> &'static str {
        match self {
            Self::SinglePlayer => "Runtime, authoring, and builders; one game target.",
            Self::MultiplayerClientServer => {
                "Runtime, client, server, authoring, and builders; client/server products."
            }
            Self::MultiplayerPeerToPeer => {
                "Runtime, P2P, authoring, and builders; one host-or-join game target."
            }
        }
    }

    pub(super) const fn as_manifest_id(self) -> &'static str {
        match self {
            Self::SinglePlayer => "single-player",
            Self::MultiplayerClientServer => "multiplayer-client-server",
            Self::MultiplayerPeerToPeer => "multiplayer-peer-to-peer",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProjectRendererBackend {
    Vulkan,
    DirectX12,
    Metal,
}

impl ProjectRendererBackend {
    pub(super) const ALL: [Self; 3] = [Self::Vulkan, Self::DirectX12, Self::Metal];

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Vulkan => "Vulkan",
            Self::DirectX12 => "DirectX 12",
            Self::Metal => "Metal",
        }
    }

    /// Stable lowercase identifier persisted to the project manifest `[render]`
    /// section (independent of the display label).
    pub(super) const fn as_manifest_id(self) -> &'static str {
        match self {
            Self::Vulkan => "vulkan",
            Self::DirectX12 => "directx12",
            Self::Metal => "metal",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProjectRenderPipeline {
    ForwardPlus,
    Deferred,
    Mobile,
}

impl ProjectRenderPipeline {
    pub(super) const ALL: [Self; 3] = [Self::ForwardPlus, Self::Deferred, Self::Mobile];

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::ForwardPlus => "Forward+",
            Self::Deferred => "Deferred",
            Self::Mobile => "Mobile",
        }
    }

    pub(super) const fn as_manifest_id(self) -> &'static str {
        match self {
            Self::ForwardPlus => "forward_plus",
            Self::Deferred => "deferred",
            Self::Mobile => "mobile",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProjectColorSpace {
    Linear,
    Gamma,
}

impl ProjectColorSpace {
    pub(super) const ALL: [Self; 2] = [Self::Linear, Self::Gamma];

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Gamma => "Gamma",
        }
    }

    pub(super) const fn as_manifest_id(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Gamma => "gamma",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ProjectTargetPlatform {
    Windows,
    Linux,
    MacOs,
    Android,
    Ios,
    Web,
}

impl ProjectTargetPlatform {
    pub(super) const ALL: [Self; 6] = [
        Self::Windows,
        Self::Linux,
        Self::MacOs,
        Self::Android,
        Self::Ios,
        Self::Web,
    ];

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::Linux => "Linux",
            Self::MacOs => "macOS",
            Self::Android => "Android",
            Self::Ios => "iOS",
            Self::Web => "Web",
        }
    }

    pub(super) const fn as_manifest_id(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::MacOs => "macos",
            Self::Android => "android",
            Self::Ios => "ios",
            Self::Web => "web",
        }
    }
}

pub(super) fn default_project_location() -> String {
    std::env::var_os("USERPROFILE")
        .map(std::path::PathBuf::from)
        .map_or_else(
            || std::path::PathBuf::from("projects"),
            |home| home.join("dev").join("azoth"),
        )
        .to_string_lossy()
        .to_string()
}

pub(super) fn template_enabled_gems(
    template: ProjectTemplateKind,
    cx: &Context<'_, ProjectManagerView>,
) -> BTreeSet<String> {
    let Some(catalog) = cx.try_global::<az_editor_ui::EditorGemCatalog>() else {
        return BTreeSet::new();
    };
    let categories = template.gem_categories();
    let selected = catalog
        .gems
        .iter()
        .filter(|gem| !gem.is_deprecated())
        .filter(|gem| categories.iter().any(|category| *category == gem.category))
        .map(|gem| gem.id.clone())
        .collect::<BTreeSet<_>>();
    if selected.is_empty() {
        catalog
            .gems
            .iter()
            .filter(|gem| !gem.is_deprecated())
            .map(|gem| gem.id.clone())
            .collect()
    } else {
        selected
    }
}

pub(super) fn template_project_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        "MyGame".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(super) fn template_project_path(name: &str, location: &str) -> String {
    let name = template_project_name(name);
    let location = location.trim();
    if location.is_empty() {
        return name;
    }
    std::path::PathBuf::from(location)
        .join(name)
        .to_string_lossy()
        .to_string()
}

pub(super) fn template_lore_url(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_playable_templates_include_data_and_audio_gems() {
        for template in [
            ProjectTemplateKind::ThreeD,
            ProjectTemplateKind::FirstPerson,
            ProjectTemplateKind::ThirdPerson,
            ProjectTemplateKind::TopDown,
            ProjectTemplateKind::TwoD,
            ProjectTemplateKind::Mobile,
            ProjectTemplateKind::Multiplayer,
        ] {
            let categories = template.gem_categories();
            assert!(
                categories.contains(&"Data"),
                "{} should select GameData by default",
                template.name()
            );
            assert!(
                categories.contains(&"Audio"),
                "{} should select AudioSystem by default",
                template.name()
            );
        }
    }
}
