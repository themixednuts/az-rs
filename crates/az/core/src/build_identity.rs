//! Runtime/product build identity shared by launch profiles and protocols.

use std::fmt;

/// Version tuple for a runnable product target.
///
/// These fields are strings because compatibility protocols often preserve
/// native token text exactly instead of parsing it as semantic version numbers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BuildVersion {
    pub major: String,
    pub minor: String,
    pub build: String,
}

impl BuildVersion {
    #[must_use]
    pub fn new(
        major: impl Into<String>,
        minor: impl Into<String>,
        build: impl Into<String>,
    ) -> Self {
        Self {
            major: major.into(),
            minor: minor.into(),
            build: build.into(),
        }
    }
}

/// Identity of a runnable client/server/editor product build.
///
/// Protocols and launchers adapt this into their native handshake or command
/// line shape. The identity lives above any one protocol message so manifests,
/// editor profiles, CLI targets, and compatibility handshakes can agree on one
/// source of truth.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BuildIdentity {
    pub channel: String,
    pub product: String,
    pub version: BuildVersion,
    pub revision: String,
}

impl BuildIdentity {
    #[must_use]
    pub fn new(
        channel: impl Into<String>,
        product: impl Into<String>,
        version: BuildVersion,
        revision: impl Into<String>,
    ) -> Self {
        Self {
            channel: channel.into(),
            product: product.into(),
            version,
            revision: revision.into(),
        }
    }

    #[must_use]
    pub fn display_string(&self) -> String {
        format!(
            "{}.{}.{}.{}.{}.{}",
            self.channel,
            self.product,
            self.version.major,
            self.version.minor,
            self.version.build,
            self.revision
        )
    }
}

impl fmt::Display for BuildIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_string_preserves_native_tokens() {
        let identity = BuildIdentity::new(
            "[RETAIL]",
            "ExampleProduct",
            BuildVersion::new("1", "400", "6031"),
            "6004151",
        );

        assert_eq!(
            identity.display_string(),
            "[RETAIL].ExampleProduct.1.400.6031.6004151"
        );
    }
}
