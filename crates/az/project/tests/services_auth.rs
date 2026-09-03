//! `[services.auth]` manifest integration tests (ADR 0026).

use az_project::manifest::ProjectManifest;

const BASE: &str = r#"
[manifest]
kind = "project"
schema = "azoth.project/v1"

[project]
id = "local.test"
name = "test"
version = "0.1.0"
engine_version = "0.1.0"
"#;

fn parse(extra: &str) -> ProjectManifest {
    toml::from_str(&format!("{BASE}{extra}")).expect("manifest parses")
}

#[test]
fn services_auth_block_parses_and_enables() {
    let manifest = parse(
        r#"
[services.auth]
enabled = true
provider = "azoth.auth.steam"
adapter = "sample.auth-adapter"
audience = "sample-game"

[services.auth.session]
issuer = "sample-project"
access_ttl_seconds = 900
refresh_ttl_seconds = 2592000
signing_key = "secret://project/auth-signing-key"
"#,
    );
    assert!(manifest.services.auth_enabled());
    let auth = manifest.services.auth.as_ref().unwrap();
    assert_eq!(auth.provider.as_str(), "azoth.auth.steam");
    assert_eq!(auth.session.signing_key.path(), "project/auth-signing-key");
    manifest.validate().expect("valid manifest");
}

#[test]
fn absent_services_block_means_auth_disabled() {
    let manifest = parse("");
    assert!(!manifest.services.auth_enabled());
    assert!(manifest.services.is_empty());
    manifest.validate().expect("valid manifest");
}

#[test]
fn disabled_auth_is_not_enabled() {
    let manifest = parse(
        r#"
[services.auth]
enabled = false
provider = "azoth.auth.local"
audience = "game"

[services.auth.session]
issuer = "iss"
signing_key = "secret://k"
"#,
    );
    assert!(!manifest.services.auth_enabled());
    manifest.validate().expect("valid even when disabled");
}

#[test]
fn inline_signing_secret_is_rejected() {
    // A non-`secret://` signing key fails to deserialize (SecretRef guard).
    let result: Result<ProjectManifest, _> = toml::from_str(&format!(
        "{BASE}{}",
        r#"
[services.auth]
enabled = true
provider = "azoth.auth.local"
audience = "game"

[services.auth.session]
issuer = "iss"
signing_key = "hunter2"
"#,
    ));
    assert!(result.is_err(), "inline secret must not parse");
}

#[test]
fn empty_audience_fails_validation() {
    let manifest = parse(
        r#"
[services.auth]
enabled = true
provider = "azoth.auth.local"
audience = ""

[services.auth.session]
issuer = "iss"
signing_key = "secret://k"
"#,
    );
    let error = manifest.validate().unwrap_err();
    assert!(
        error.to_string().contains("services.auth"),
        "unexpected error: {error}"
    );
}

#[test]
fn provider_cannot_be_its_own_link_provider() {
    let manifest = parse(
        r#"
[services.auth]
enabled = true
provider = "azoth.auth.steam"
audience = "game"
link_providers = ["azoth.auth.steam"]

[services.auth.session]
issuer = "iss"
signing_key = "secret://project/key"
"#,
    );
    assert!(manifest.validate().is_err());
}

#[test]
fn session_authority_round_trips_and_rejects_blank_ids() {
    let valid = parse(
        r#"
[services.auth]
enabled = true
provider = "azoth.auth.local"
session_authority = "sample.auth-authority"
audience = "game"

[services.auth.session]
issuer = "iss"
signing_key = "secret://project/key"
"#,
    );
    valid.validate().expect("non-empty authority id");
    let serialized = toml::to_string(&valid).unwrap();
    assert!(serialized.contains("session_authority = \"sample.auth-authority\""));

    let invalid = parse(
        r#"
[services.auth]
enabled = true
provider = "azoth.auth.local"
session_authority = "  "
audience = "game"

[services.auth.session]
issuer = "iss"
signing_key = "secret://project/key"
"#,
    );
    assert!(invalid.validate().is_err());
}

#[test]
fn services_auth_round_trips_through_toml() {
    let manifest = parse(
        r#"
[services.auth]
enabled = true
provider = "azoth.auth.eos"
audience = "game"

[services.auth.session]
issuer = "iss"
signing_key = "secret://project/key"
"#,
    );
    let serialized = toml::to_string(&manifest).expect("serializes");
    let reparsed: ProjectManifest = toml::from_str(&serialized).expect("re-parses");
    assert_eq!(reparsed.services, manifest.services);
}

#[test]
fn secret_mounts_are_typed_and_round_trip_backend_options() {
    let manifest = parse(
        r#"
[secrets.mounts.shared]
backend = "aws-secrets-manager"
profile = "studio"
region = "us-east-1"
name_prefix = "sample/"

[secrets.mounts.ci]
backend = "env"
var_prefix = "AZ_SECRET_"
"#,
    );
    manifest.validate().expect("valid secret mounts");
    assert_eq!(
        manifest.secrets.mounts["shared"].backend,
        "aws-secrets-manager"
    );
    assert_eq!(
        manifest.secrets.mounts["ci"].options["var_prefix"],
        "AZ_SECRET_"
    );

    let serialized = toml::to_string(&manifest).expect("serializes");
    let reparsed: ProjectManifest = toml::from_str(&serialized).expect("re-parses");
    assert_eq!(reparsed.secrets, manifest.secrets);
}

#[test]
fn secret_mounts_reject_invalid_mounts_and_empty_backends() {
    for extra in [
        "\n[secrets.mounts.\"two words\"]\nbackend = \"local\"\n",
        "\n[secrets.mounts.project]\nbackend = \"\"\n",
    ] {
        let manifest = parse(extra);
        assert!(
            manifest.validate().is_err(),
            "accepted invalid config: {extra}"
        );
    }
}

#[test]
fn client_and_title_runtime_manifests_do_not_link_the_resolver() {
    for (name, manifest) in [
        (
            "auth contract",
            include_str!("../../../../gems/auth/Cargo.toml"),
        ),
        ("runtime app", include_str!("../../runtime-app/Cargo.toml")),
        (
            "engine runtime",
            include_str!("../../../engine/runtime/Cargo.toml"),
        ),
        ("framework", include_str!("../../framework/Cargo.toml")),
    ] {
        let parsed: toml::Value = toml::from_str(manifest).expect("valid manifest");
        let production = parsed
            .get("dependencies")
            .and_then(toml::Value::as_table)
            .expect("production dependencies");
        assert!(
            !production.contains_key("az-secrets"),
            "{name} must receive narrow capabilities, not the secret resolver"
        );
    }
}
