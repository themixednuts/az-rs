use std::fmt;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// Executables that form the Azoth host-tool bundle.
///
/// These are engine tools, not project packages. Production distributions
/// install them beside `azoth`; source-checkout bootstrapping must produce the
/// same layout before project work begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HostTool {
    Daemon,
    SessionSupervisor,
    /// Engine-owned asset-processor host (scheduler/DB/watch/dispatch).
    /// Project builders live only in the per-project `asset-worker`.
    AssetProcessor,
    /// Offline, single-owner asset registry maintenance tool.
    AssetDbMaintenance,
}

impl HostTool {
    #[must_use]
    pub const fn cargo_package(self) -> &'static str {
        match self {
            Self::Daemon => "az-daemon",
            Self::SessionSupervisor => "az-sessiond",
            // The maintenance tool is a second binary target of the host crate,
            // not a crate of its own, so both variants build from one package.
            Self::AssetProcessor | Self::AssetDbMaintenance => "az-asset-processor-host",
        }
    }

    #[must_use]
    pub const fn cargo_binary(self) -> &'static str {
        match self {
            Self::Daemon => "azd",
            Self::SessionSupervisor => "az-sessiond",
            Self::AssetProcessor => "asset-processor",
            Self::AssetDbMaintenance => "assetdb-maintenance",
        }
    }

    #[must_use]
    pub fn executable_name(self) -> String {
        if cfg!(windows) {
            format!("{}.exe", self.cargo_binary())
        } else {
            self.cargo_binary().to_string()
        }
    }
}

impl fmt::Display for HostTool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.cargo_binary())
    }
}

/// Directory containing a complete, directly executable Azoth tool bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostToolBundle {
    directory: PathBuf,
}

impl HostToolBundle {
    /// Resolve the bundle adjacent to an Azoth executable.
    ///
    /// # Errors
    ///
    /// Returns [`HostToolError::MissingBundleDirectory`] when the executable
    /// path has no parent directory.
    pub fn adjacent_to(executable: impl AsRef<Path>) -> Result<Self, HostToolError> {
        let executable = executable.as_ref();
        let directory = executable
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .ok_or_else(|| HostToolError::MissingBundleDirectory {
                executable: executable.to_path_buf(),
            })?;
        Ok(Self {
            directory: directory.to_path_buf(),
        })
    }

    /// Resolve the bundle adjacent to the running executable.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the current executable cannot be located or
    /// has no parent directory.
    pub fn current() -> Result<Self, HostToolError> {
        let executable = std::env::current_exe()
            .map_err(|source| HostToolError::CurrentExecutable { source })?;
        Self::adjacent_to(executable)
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    #[must_use]
    pub fn path(&self, tool: HostTool) -> PathBuf {
        self.directory.join(tool.executable_name())
    }

    /// Resolve one required executable from this bundle.
    ///
    /// # Errors
    ///
    /// Returns [`HostToolError::IncompleteBundle`] rather than falling back to
    /// Cargo when the executable is absent.
    pub fn resolve(&self, tool: HostTool) -> Result<PathBuf, HostToolError> {
        let path = self.path(tool);
        if path.is_file() {
            Ok(path)
        } else {
            Err(HostToolError::IncompleteBundle { tool, path })
        }
    }

    #[must_use]
    pub fn missing(&self, tools: &[HostTool]) -> Vec<HostTool> {
        tools
            .iter()
            .copied()
            .filter(|tool| !self.path(*tool).is_file())
            .collect()
    }
}

#[derive(Debug, Error)]
pub enum HostToolError {
    #[error("failed to locate the running Azoth executable: {source}")]
    CurrentExecutable {
        #[source]
        source: std::io::Error,
    },

    #[error("Azoth executable path has no bundle directory: {executable:?}")]
    MissingBundleDirectory { executable: PathBuf },

    #[error(
        "Azoth toolkit bundle is incomplete: required host tool `{tool}` is missing at {path:?}"
    )]
    IncompleteBundle { tool: HostTool, path: PathBuf },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_paths_are_adjacent_to_the_launcher() {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("bin");
        let bundle = HostToolBundle::adjacent_to(bin.join("azoth.exe")).unwrap();

        assert_eq!(bundle.directory(), bin);
        assert_eq!(
            bundle.path(HostTool::SessionSupervisor),
            bin.join(HostTool::SessionSupervisor.executable_name())
        );
    }

    #[test]
    fn incomplete_bundle_is_a_typed_error() {
        let temp = tempfile::tempdir().unwrap();
        let bundle = HostToolBundle::adjacent_to(temp.path().join("azoth.exe")).unwrap();

        let error = bundle.resolve(HostTool::Daemon).unwrap_err();

        assert!(matches!(
            error,
            HostToolError::IncompleteBundle {
                tool: HostTool::Daemon,
                path,
            } if path == temp.path().join(HostTool::Daemon.executable_name())
        ));
    }

    #[test]
    fn existing_host_tool_resolves_directly() {
        let temp = tempfile::tempdir().unwrap();
        let expected = temp.path().join(HostTool::Daemon.executable_name());
        std::fs::write(&expected, b"fixture").unwrap();
        let bundle = HostToolBundle::adjacent_to(temp.path().join("azoth.exe")).unwrap();

        assert_eq!(bundle.resolve(HostTool::Daemon).unwrap(), expected);
    }
}
