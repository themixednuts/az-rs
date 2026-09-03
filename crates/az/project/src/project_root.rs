use std::path::{Path, PathBuf};

use crate::{ProjectManifestError, load_project_manifest, project_manifest_path};

/// Locate and validate the nearest Azoth project root at or above `start`.
///
/// Cargo build scripts and nested project tools must not encode assumptions
/// such as `../../..` about a package's role directory. The project manifest is
/// the durable boundary, so discovery stops at the first `azoth.toml` and
/// validates that it is a project manifest before returning it.
///
/// # Errors
///
/// Returns [`ProjectManifestError::ProjectRootNotFound`] if no ancestor of
/// `start` holds an `azoth.toml`, [`ProjectManifestError::Read`] if the located
/// root cannot be canonicalized, or any error [`load_project_manifest`] returns
/// when the first `azoth.toml` found is not a valid project manifest.
pub fn find_project_root(start: impl AsRef<Path>) -> Result<PathBuf, ProjectManifestError> {
    let start = start.as_ref();
    let start_directory = if start.is_file() {
        start.parent().unwrap_or(start)
    } else {
        start
    };

    for candidate in start_directory.ancestors() {
        if project_manifest_path(candidate).is_file() {
            load_project_manifest(candidate)?;
            return candidate
                .canonicalize()
                .map_err(|source| ProjectManifestError::Read {
                    path: candidate.to_path_buf(),
                    source,
                });
        }
    }

    Err(ProjectManifestError::ProjectRootNotFound {
        start: start.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_project_from_nested_role_package() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        let nested = root.join("gems/example/runtime/src");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(
            root.join("azoth.toml"),
            r#"[manifest]
kind = "project"
schema = "azoth.project/v1"

[project]
id = "local.example"
name = "Example"
version = "0.1.0"
engine_version = "0.1.0"
"#,
        )
        .unwrap();

        assert_eq!(
            find_project_root(&nested).unwrap(),
            root.canonicalize().unwrap()
        );
    }

    #[test]
    fn reports_the_original_start_when_no_project_exists() {
        let temp = tempfile::tempdir().unwrap();
        let error = find_project_root(temp.path()).unwrap_err();
        assert!(matches!(
            error,
            ProjectManifestError::ProjectRootNotFound { start } if start == temp.path()
        ));
    }
}
