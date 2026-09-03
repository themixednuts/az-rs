//! Legacy `CrySystem` config-script source transform and builder.

pub mod builder;
pub mod source_transform;

pub use source_transform::{
    ConfigCommentMarker, ConfigSource, ConfigSourceLine, ConfigSourceTransform,
    ConfigSourceTransformError, config_source_path,
};

use az_asset_builder::{
    BuildRuleRegistration, ProductFormat, ProductFormatRegistration, SourceFormat,
    SourceSchemaRegistration, product_format_id, source_schema_type,
};

#[derive(SourceFormat)]
#[source(schema = "azoth.compat.crysystem.ConfigSource", ext = "config.toml")]
pub struct ConfigSourceFormat;

#[derive(ProductFormat)]
#[product_format(
    id = "azoth.compat.crysystem.config",
    version = 1,
    asset = az_core::ConfigAsset
)]
pub struct ConfigProductFormat;

pub mod source_schemas {
    use super::{ConfigSourceFormat, source_schema_type};
    use az_asset_builder::SourceSchemaType;

    pub const CONFIG: SourceSchemaType = source_schema_type::<ConfigSourceFormat>();
}

pub mod product_formats {
    use super::{ConfigProductFormat, product_format_id};
    use az_asset_builder::ProductFormatId;

    pub const CRY_SYSTEM_CONFIG: ProductFormatId = product_format_id::<ConfigProductFormat>();
}

/// The product formats this crate owns, for a host contribution to register.
#[must_use]
pub const fn product_formats() -> [ProductFormatRegistration; 1] {
    [ProductFormatRegistration::for_format::<ConfigProductFormat>()]
}

/// The source schemas this crate owns, for a host contribution to register.
#[must_use]
pub const fn source_schemas() -> [SourceSchemaRegistration; 1] {
    [SourceSchemaRegistration::for_source::<ConfigSourceFormat>()
        .with_category("Cry/Lumberyard Compatibility")
        .with_import_file("config", &["config.toml"])]
}

/// The build rules this crate owns, for a host contribution to register.
#[must_use]
pub fn build_rules() -> [BuildRuleRegistration; 1] {
    [BuildRuleRegistration::new(
        builder::NAME,
        builder::ID,
        builder::desc,
    )]
}

/// Register this crate's asset-pipeline contributions into a composing host.
pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<ProductFormatRegistration>()
        .register_many(product_formats());
    ctx.registrar::<SourceSchemaRegistration>()
        .register_many(source_schemas());
    ctx.registrar::<BuildRuleRegistration>()
        .register_many(build_rules());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registration is keyed on the builder id and ordered by the name, so
    /// a registration that disagrees with the rule it resolves would file job
    /// attempts under an identity the dispatcher never reports.
    #[test]
    fn every_registration_matches_the_rule_it_resolves() {
        let registries = az_gem_contract::Registries::new();
        let context = az_asset_builder::JobContext::new(&registries);

        for registration in build_rules() {
            let rule = registration.rule(&context);
            assert_eq!(registration.name(), rule.name);
            assert_eq!(registration.id(), rule.id);
        }
    }
}
