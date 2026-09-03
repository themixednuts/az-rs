use std::path::{Path, PathBuf};

use az_project::{ProjectManifest, load_project_manifest};
use az_secrets::{LOCAL_BACKEND_ID, LocalSecretStore, ProvisionSecrets, SecretError, SecretRef};
use tracing::info;
use zeroize::Zeroizing;

use crate::error::{CliError, CliResult};

/// # Errors
///
/// Returns [`CliError::InvalidSecretReference`] if `reference` is not a valid
/// secret reference, [`CliError::ProjectManifest`] if the project manifest under
/// `project_path` cannot be loaded, [`CliError::Io`] if `from_file` cannot be
/// read, and [`CliError::Secret`] if the selected local store rejects the
/// material.
pub fn set(reference: &str, from_file: &Path, project_path: Option<PathBuf>) -> CliResult<()> {
    let project_root = project_path.unwrap_or_else(|| PathBuf::from("."));
    let manifest = load_project_manifest(&project_root)?;
    let secret = parse_reference(reference)?;
    ensure_local_mount(&manifest, &secret)?;
    let material = Zeroizing::new(std::fs::read(from_file)?);
    let store = LocalSecretStore::for_project(&manifest.project.name, &project_root)?;
    store.provision(&secret, &material)?;
    info!(reference = %secret.as_str(), project = %manifest.project.name, "provisioned project secret");
    println!(
        "provisioned {} for project {}",
        secret.as_str(),
        manifest.project.name
    );
    Ok(())
}

/// # Errors
///
/// Returns [`CliError::InvalidSecretReference`] if `reference` is not a valid
/// secret reference, [`CliError::ProjectManifest`] if the project manifest under
/// `project_path` cannot be loaded, and [`CliError::Secret`] if the secret has
/// no location in the project store.
pub fn path(reference: &str, project_path: Option<PathBuf>) -> CliResult<()> {
    let project_root = project_path.unwrap_or_else(|| PathBuf::from("."));
    let manifest = load_project_manifest(&project_root)?;
    let secret = parse_reference(reference)?;
    ensure_local_mount(&manifest, &secret)?;
    let store = LocalSecretStore::for_project(&manifest.project.name, &project_root)?;
    println!("{}", store.location(&secret)?.display());
    Ok(())
}

fn parse_reference(value: &str) -> CliResult<SecretRef> {
    SecretRef::parse(value).map_err(|error| CliError::InvalidSecretReference {
        message: error.to_string(),
    })
}

fn ensure_local_mount(manifest: &ProjectManifest, secret: &SecretRef) -> CliResult<()> {
    let configured = manifest.secrets.mounts.get(secret.mount());
    if configured.is_none() && secret.mount() != "project" {
        return Err(SecretError::MountNotConfigured {
            mount: secret.mount().to_owned().into_boxed_str(),
        }
        .into());
    }
    if configured
        .is_some_and(|mount| mount.backend != LOCAL_BACKEND_ID || !mount.options.is_empty())
    {
        return Err(SecretError::InvalidBackendConfiguration.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provisioning_honors_exact_mount_routing() {
        let mut manifest = ProjectManifest::new("sample", "Sample", "0.1.0");
        let project = SecretRef::parse("secret://project/key").unwrap();
        let shared = SecretRef::parse("secret://shared/key").unwrap();

        ensure_local_mount(&manifest, &project).expect("project defaults to local");
        assert!(matches!(
            ensure_local_mount(&manifest, &shared),
            Err(CliError::Secret(_))
        ));

        manifest.secrets.mounts.insert(
            "shared".to_owned(),
            az_secrets::SecretMountConfig::new("aws-secrets-manager"),
        );
        assert!(matches!(
            ensure_local_mount(&manifest, &shared),
            Err(CliError::Secret(_))
        ));

        manifest.secrets.mounts.insert(
            "shared".to_owned(),
            az_secrets::SecretMountConfig::new(LOCAL_BACKEND_ID),
        );
        ensure_local_mount(&manifest, &shared).expect("explicit local mount");

        manifest
            .secrets
            .mounts
            .get_mut("shared")
            .unwrap()
            .options
            .insert(
                "unexpected".to_owned(),
                serde_json::Value::String("value".to_owned()),
            );
        assert!(matches!(
            ensure_local_mount(&manifest, &shared),
            Err(CliError::Secret(_))
        ));
    }
}
