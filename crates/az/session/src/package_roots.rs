use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use az_asset::{
    AZPACK_INDEX_FILE_NAME, format_package_release_id_hex, package_manifest_release_id,
    package_payload_layout, read_package_manifest,
};
use az_proto_runtime::{RuntimeAssetPackageContainer, RuntimeAssetPackageRoot};

use crate::{SessionError, SessionManifest};

pub const PACKAGE_MANIFEST_FILE_NAME: &str = "package-manifest.azpkg";

#[must_use]
pub fn package_outputs_root(project_path: &Path) -> PathBuf {
    project_path.join("target").join("azoth").join("packages")
}

#[must_use]
pub fn package_output_dir(project_path: &Path, profile_name: &str, session: &str) -> PathBuf {
    package_outputs_root(project_path)
        .join(safe_package_path_component(profile_name))
        .join(safe_package_path_component(session))
}

#[must_use]
pub fn safe_package_path_component(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            output.push(ch);
        } else {
            output.push('_');
        }
    }
    if output.is_empty() {
        "_".to_string()
    } else {
        output
    }
}

/// Collects the packaged asset roots this session published, one per build
/// profile that has a package manifest under the session's output directory.
///
/// # Errors
///
/// Returns [`SessionError::Io`] if the packages root or a profile directory
/// cannot be enumerated or a package manifest cannot be opened,
/// [`SessionError::PackageManifest`] if a package manifest cannot be parsed or
/// its release id cannot be computed, and [`SessionError::InvalidPackageOutput`]
/// if a manifest names a container this runtime does not know or declares a
/// payload layout that does not resolve.
pub fn runtime_asset_package_roots_for_session(
    manifest: &SessionManifest,
) -> Result<Vec<RuntimeAssetPackageRoot>, SessionError> {
    let packages_root = package_outputs_root(&manifest.project_root);
    if !packages_root.is_dir() {
        return Ok(Vec::new());
    }

    let session_component = safe_package_path_component(&manifest.slug);
    let mut roots = Vec::new();
    for profile_entry in packages_root.read_dir()? {
        let profile_entry = profile_entry?;
        let profile_dir = profile_entry.path();
        if !profile_dir.is_dir() {
            continue;
        }

        let session_root = profile_dir.join(&session_component);
        if !session_root.is_dir() {
            continue;
        }

        let manifest_path = session_root.join(PACKAGE_MANIFEST_FILE_NAME);
        if !manifest_path.is_file() {
            continue;
        }

        let mut reader = BufReader::new(File::open(&manifest_path)?);
        let package_manifest = read_package_manifest(&mut reader)?;
        let release_id =
            format_package_release_id_hex(&package_manifest_release_id(&package_manifest)?);
        let container =
            RuntimeAssetPackageContainer::from_name(&package_manifest.profile.container)
                .ok_or_else(|| SessionError::InvalidPackageOutput {
                    session: manifest.slug.clone(),
                    profile: package_manifest.profile.name.clone(),
                    reason: format!(
                        "unsupported container `{}` in `{}`",
                        package_manifest.profile.container,
                        manifest_path.display()
                    ),
                })?;
        roots.push(runtime_asset_package_root_from_manifest(
            &manifest.slug,
            &session_root,
            &package_manifest.profile,
            container,
            &manifest_path,
            release_id,
        )?);
    }

    roots.sort_by(|left, right| {
        left.profile
            .cmp(&right.profile)
            .then_with(|| left.asset_platform.cmp(&right.asset_platform))
            .then_with(|| left.container.cmp(&right.container))
            .then_with(|| left.mount_root.cmp(&right.mount_root))
    });
    Ok(roots)
}

fn runtime_asset_package_root_from_manifest(
    session: &str,
    output_root: &Path,
    profile: &az_asset::PackageManifestProfile,
    container: RuntimeAssetPackageContainer,
    manifest_path: &Path,
    release_id: String,
) -> Result<RuntimeAssetPackageRoot, SessionError> {
    let layout = package_payload_layout(output_root, profile).map_err(|error| {
        SessionError::InvalidPackageOutput {
            session: session.to_string(),
            profile: profile.name.clone(),
            reason: format!(
                "container `{}` declared by `{}` has invalid package layout: {error}",
                container.as_str(),
                manifest_path.display()
            ),
        }
    })?;

    let (mount_root, payload_path, catalog_path) = match container {
        RuntimeAssetPackageContainer::Loose => {
            let root = layout.mount_root;
            let payload = layout.payload_path;
            let catalog = layout.catalog_path;
            ensure_runtime_package_dir(session, &root, profile, container, manifest_path)?;
            ensure_runtime_package_file(
                session,
                &catalog,
                profile,
                container,
                "asset catalog",
                manifest_path,
            )?;
            (root, payload, catalog)
        }
        RuntimeAssetPackageContainer::AzPack => {
            let root = layout.mount_root;
            let payload = layout.payload_path;
            let catalog = layout.catalog_path;
            let index = root.join(AZPACK_INDEX_FILE_NAME);
            ensure_runtime_package_dir(session, &root, profile, container, manifest_path)?;
            ensure_runtime_package_file(
                session,
                &index,
                profile,
                container,
                "azpack index",
                manifest_path,
            )?;
            ensure_runtime_package_file(
                session,
                &catalog,
                profile,
                container,
                "asset catalog",
                manifest_path,
            )?;
            (root, payload, catalog)
        }
        RuntimeAssetPackageContainer::Pak => {
            let root = layout.mount_root;
            let payload = layout.payload_path;
            let catalog = layout.catalog_path;
            ensure_runtime_package_file(
                session,
                &payload,
                profile,
                container,
                "pak payload",
                manifest_path,
            )?;
            ensure_runtime_package_file(
                session,
                &catalog,
                profile,
                container,
                "asset catalog",
                manifest_path,
            )?;
            (root, payload, catalog)
        }
    };

    Ok(RuntimeAssetPackageRoot {
        profile: profile.name.clone(),
        asset_platform: profile.asset_platform.clone(),
        container,
        mount_root: mount_root.to_string_lossy().into_owned(),
        payload_path: payload_path.to_string_lossy().into_owned(),
        catalog_path: catalog_path.to_string_lossy().into_owned(),
        release_id,
    })
}

fn ensure_runtime_package_dir(
    session: &str,
    path: &Path,
    profile: &az_asset::PackageManifestProfile,
    container: RuntimeAssetPackageContainer,
    manifest_path: &Path,
) -> Result<(), SessionError> {
    if path.is_dir() {
        return Ok(());
    }

    Err(SessionError::InvalidPackageOutput {
        session: session.to_string(),
        profile: profile.name.clone(),
        reason: format!(
            "container `{}` declared by `{}` is missing package directory `{}`",
            container.as_str(),
            manifest_path.display(),
            path.display()
        ),
    })
}

fn ensure_runtime_package_file(
    session: &str,
    path: &Path,
    profile: &az_asset::PackageManifestProfile,
    container: RuntimeAssetPackageContainer,
    label: &'static str,
    manifest_path: &Path,
) -> Result<(), SessionError> {
    if path.is_file() {
        return Ok(());
    }

    Err(SessionError::InvalidPackageOutput {
        session: session.to_string(),
        profile: profile.name.clone(),
        reason: format!(
            "container `{}` declared by `{}` is missing {label} `{}`",
            container.as_str(),
            manifest_path.display(),
            path.display()
        ),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use uuid::Uuid;

    use super::*;
    use crate::SessionId;

    fn manifest(root: &Path) -> SessionManifest {
        SessionManifest::new(
            SessionId::new(),
            "local.package_test".to_string(),
            "editor-work".to_string(),
            root.join("project"),
            root.join("workspace"),
            root.join("run"),
            0,
        )
    }

    fn write_test_package_manifest(
        output_root: &Path,
        profile_name: &str,
        asset_platform: &str,
        container: &str,
    ) {
        let profile = az_asset::PackageManifestProfile {
            name: profile_name.to_string(),
            asset_platform: asset_platform.to_string(),
            cargo_profile: "dev".to_string(),
            container: container.to_string(),
            compression: "none".to_string(),
            oodle_compressor: None,
            oodle_effort: None,
        };
        write_test_package_manifest_with_profile(output_root, profile);
    }

    fn write_test_package_manifest_with_profile(
        output_root: &Path,
        profile: az_asset::PackageManifestProfile,
    ) {
        fs::create_dir_all(output_root).unwrap();
        let entry = az_asset::PackageManifestEntry::new(
            "textures/test.azbin",
            Uuid::from_bytes([0x22; 16]),
            0,
            "az.test.raw",
            1,
            [0x33; az_asset::PACKAGE_CONTENT_HASH_BYTES],
            4,
            Uuid::from_bytes([0x44; 16]),
            "textures/test.ron",
            "test-job",
        );
        let manifest = az_asset::PackageManifest::new(profile, vec![entry]).unwrap();
        let mut file = fs::File::create(output_root.join(PACKAGE_MANIFEST_FILE_NAME)).unwrap();
        az_asset::write_package_manifest(&manifest, &mut file).unwrap();
    }

    #[test]
    fn discovers_built_azpack_mounts_for_session() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = manifest(temp.path());
        let output_root = package_output_dir(&manifest.project_root, "pc-dev", &manifest.slug);
        let azpack_root = output_root.join("azpack");
        fs::create_dir_all(azpack_root.join("chunks")).unwrap();
        write_test_package_manifest(&output_root, "pc-dev", "pc", "azpack");
        fs::write(azpack_root.join(AZPACK_INDEX_FILE_NAME), b"index").unwrap();
        fs::write(
            azpack_root.join(az_asset::ASSET_CATALOG_FILE_NAME),
            b"catalog",
        )
        .unwrap();

        let roots = runtime_asset_package_roots_for_session(&manifest).unwrap();

        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].profile, "pc-dev");
        assert_eq!(roots[0].asset_platform, "pc");
        assert_eq!(roots[0].container, RuntimeAssetPackageContainer::AzPack);
        assert_eq!(roots[0].mount_root, azpack_root.to_string_lossy());
        assert_eq!(roots[0].payload_path, azpack_root.to_string_lossy());
        assert_eq!(
            roots[0].catalog_path,
            azpack_root
                .join(az_asset::ASSET_CATALOG_FILE_NAME)
                .to_string_lossy()
        );
        assert_eq!(
            roots[0].release_id.len(),
            az_proto_runtime::RUNTIME_PACKAGE_RELEASE_ID_HEX_LEN
        );
        assert!(az_proto_runtime::is_runtime_package_release_id(
            &roots[0].release_id
        ));
    }

    #[test]
    fn discovers_built_compatible_pak_mounts_for_session() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = manifest(temp.path());
        let output_root = package_output_dir(&manifest.project_root, "pc-release", &manifest.slug);
        write_test_package_manifest(&output_root, "pc-release", "pc", "pak");
        let layout = az_asset::package_payload_layout(
            &output_root,
            &az_asset::PackageManifestProfile {
                name: "pc-release".to_string(),
                asset_platform: "pc".to_string(),
                cargo_profile: "dev".to_string(),
                container: "pak".to_string(),
                compression: "none".to_string(),
                oodle_compressor: None,
                oodle_effort: None,
            },
        )
        .unwrap();
        fs::write(&layout.payload_path, b"pak").unwrap();
        fs::write(&layout.catalog_path, b"catalog").unwrap();

        let roots = runtime_asset_package_roots_for_session(&manifest).unwrap();

        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].profile, "pc-release");
        assert_eq!(roots[0].container, RuntimeAssetPackageContainer::Pak);
        assert_eq!(roots[0].mount_root, output_root.to_string_lossy());
        assert_eq!(roots[0].payload_path, layout.payload_path.to_string_lossy());
        assert_eq!(roots[0].catalog_path, layout.catalog_path.to_string_lossy());
    }

    #[test]
    fn rejects_incomplete_package_outputs() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = manifest(temp.path());
        let output_root = package_output_dir(&manifest.project_root, "pc-dev", &manifest.slug);
        let azpack_root = output_root.join("azpack");
        fs::create_dir_all(&azpack_root).unwrap();
        write_test_package_manifest(&output_root, "pc-dev", "pc", "azpack");
        fs::write(azpack_root.join(AZPACK_INDEX_FILE_NAME), b"index").unwrap();

        assert!(matches!(
            runtime_asset_package_roots_for_session(&manifest),
            Err(SessionError::InvalidPackageOutput { reason, .. })
                if reason.contains("asset catalog")
        ));
    }
}
