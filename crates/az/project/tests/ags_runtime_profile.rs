use az_project::manifest::ProjectManifest;

const PROJECT: &str = r#"
[manifest]
kind = "project"
schema = "azoth.project/v1"

[project]
id = "local.test"
name = "test"
version = "0.1.0"
engine_version = "0.1.0"

[[runtime.profiles]]
name = "game"

[runtime.profiles.build]
channel = "retail"
product = "test"
revision = "1"

[runtime.profiles.build.version]
major = "1"
minor = "0"
build = "1"

[runtime.profiles.ags]
game = "test"
gateway_service_tag = "gateway"
steam_app_id = 480
steam_auth_product_id = 1
token_version = 1

[runtime.profiles.ags.steam_ticket_verification]
endpoint = "https://api.steampowered.com/ISteamUserAuth/AuthenticateUserTicket/v1/"
publisher_key = "secret://project/test/steam-publisher-key"

[runtime.profiles.ags.steam_ticket_verification.identity_defaults]
region = "US"
account_type = "full"
account_age_group = "UNKNOWN"
platform_age_group = "UNKNOWN"
"#;

#[test]
fn ags_profile_owns_typed_steam_ticket_verification_policy() {
    let manifest: ProjectManifest = toml::from_str(PROJECT).expect("manifest parses");
    manifest.validate().expect("manifest validates");

    let ags = manifest
        .runtime
        .profile("game")
        .unwrap()
        .ags
        .as_ref()
        .unwrap();
    let verification = ags.require_steam_ticket_verification("game").unwrap();
    assert_eq!(
        verification.publisher_key.as_str(),
        "secret://project/test/steam-publisher-key"
    );
    assert_eq!(verification.identity_defaults.region, "US");
}
