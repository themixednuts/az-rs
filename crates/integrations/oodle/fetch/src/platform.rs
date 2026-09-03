//! The desktop platforms this tool fetches for, and how `--platform` resolves.

use std::fmt;

use thiserror::Error;

/// The desktop platforms Azoth targets, named as the manifest names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Platform {
    Win64,
    Linux,
    LinuxArm64,
    Mac,
}

impl Platform {
    pub const ALL: [Self; 4] = [Self::Win64, Self::Linux, Self::LinuxArm64, Self::Mac];

    /// The platform's directory under `lib/` and `redist/`.
    #[must_use]
    pub const fn directory(self) -> &'static str {
        match self {
            Self::Win64 => "Win64",
            Self::Linux => "Linux",
            Self::LinuxArm64 => "LinuxArm64",
            Self::Mac => "Mac",
        }
    }

    /// The CLI spelling.
    #[must_use]
    pub const fn as_argument(self) -> &'static str {
        match self {
            Self::Win64 => "win64",
            Self::Linux => "linux",
            Self::LinuxArm64 => "linux-arm64",
            Self::Mac => "mac",
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_argument())
    }
}

/// What `--platform` asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformRequest {
    /// Every platform the manifest carries libraries for.
    All,
    One(Platform),
}

#[derive(Debug, Error)]
#[error("this tool has no Oodle platform for {target_os}/{target_arch}; pass --platform")]
pub struct UnknownHostError {
    pub target_os: String,
    pub target_arch: String,
}

/// The platform whose libraries a build on `target_os`/`target_arch` needs.
///
/// Takes the target triple's parts rather than reading [`std::env::consts`] so
/// the mapping stays testable from a host it does not describe.
#[must_use]
pub fn host_platform(target_os: &str, target_arch: &str) -> Option<Platform> {
    match (target_os, target_arch) {
        ("windows", "x86_64") => Some(Platform::Win64),
        ("linux", "x86_64") => Some(Platform::Linux),
        ("linux", "aarch64") => Some(Platform::LinuxArm64),
        ("macos", _) => Some(Platform::Mac),
        _ => None,
    }
}

/// Expand `--platform` into the platforms to fetch, defaulting to the host.
///
/// # Errors
///
/// Returns [`UnknownHostError`] when nothing was requested and the host has no
/// Oodle platform.
pub fn resolve_platforms(
    requests: &[PlatformRequest],
    target_os: &str,
    target_arch: &str,
) -> Result<Vec<Platform>, UnknownHostError> {
    if requests.is_empty() {
        let Some(platform) = host_platform(target_os, target_arch) else {
            return Err(UnknownHostError {
                target_os: target_os.to_owned(),
                target_arch: target_arch.to_owned(),
            });
        };
        return Ok(vec![platform]);
    }

    let requested = requests.iter().flat_map(|request| match request {
        PlatformRequest::All => Platform::ALL.to_vec(),
        PlatformRequest::One(platform) => vec![*platform],
    });
    let mut platforms: Vec<Platform> = Vec::new();
    for platform in requested {
        if !platforms.contains(&platform) {
            platforms.push(platform);
        }
    }
    Ok(platforms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_platform_maps_the_desktop_targets() {
        assert_eq!(host_platform("windows", "x86_64"), Some(Platform::Win64));
        assert_eq!(host_platform("linux", "x86_64"), Some(Platform::Linux));
        assert_eq!(
            host_platform("linux", "aarch64"),
            Some(Platform::LinuxArm64)
        );
        assert_eq!(host_platform("macos", "aarch64"), Some(Platform::Mac));
        assert_eq!(host_platform("android", "aarch64"), None);
    }

    #[test]
    fn resolve_platforms_defaults_to_the_host_and_expands_all() {
        assert_eq!(
            resolve_platforms(&[], "linux", "aarch64").expect("the host resolves"),
            [Platform::LinuxArm64]
        );
        assert_eq!(
            resolve_platforms(&[PlatformRequest::All], "windows", "x86_64").expect("all resolves"),
            Platform::ALL
        );
        assert_eq!(
            resolve_platforms(
                &[
                    PlatformRequest::One(Platform::Mac),
                    PlatformRequest::One(Platform::Mac),
                    PlatformRequest::One(Platform::Win64),
                ],
                "linux",
                "x86_64"
            )
            .expect("explicit platforms resolve"),
            [Platform::Mac, Platform::Win64]
        );
        assert!(resolve_platforms(&[], "haiku", "x86_64").is_err());
    }
}
