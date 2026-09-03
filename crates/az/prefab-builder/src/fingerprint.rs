//! Deterministic semantic fingerprint for Prefab processing inputs.
//!
//! A Prefab product embeds reflected Rust type paths and native component
//! lowering identities, then frames them into AZSCENE bytes. Neither half is
//! visible in the source file, so source bytes alone cannot prove that an
//! existing product is current. This fingerprint covers both.
//!
//! **The processing registry** is hashed directly, because reflected types and
//! component lowerings are contributed by crates this one does not depend on.
//! No source hash can see them.
//!
//! **The code framing those types into bytes** is covered by build-time source
//! fingerprints of `az-scene` (which owns `encode_scene_asset` and the AZSCENE
//! format version), `az-prefab`, and this crate. Changing an encoder leaves
//! every registry untouched, so this half is what notices that a different
//! codec produced an existing product.
//!
//! Until the codec half existed, a hand-maintained `PREFAB_BUILD_RULE_VERSION`
//! counter was the manual compensation for the gap. Both halves above now
//! derive what it approximated, so the counter is gone and the rule declares a
//! constant `1` version.

use az_asset_builder::JobContext;
use az_asset_builder::fingerprint::AnalysisFingerprint;
use az_core::component::ComponentLoweringRegistration;
use az_core::component::lowering::ComponentExportPolicy;
use az_prefab::{PrefabConstruction, PrefabTypeData};
use bevy::reflect::{TypeInfo, TypeRegistry, enums::VariantInfo};

/// Bumped to `v2` when codec identity joined the registry hash, so a stored
/// fingerprint still says what it covers.
const DOMAIN: &str = "azoth.prefab-analysis/v2:";

/// Domain for the registry component, kept separate so the registry digest
/// stays comparable to what it hashed before codec identity was added.
const REGISTRY_DOMAIN: &str = "azoth.prefab-registry/v1:";

/// The analysis fingerprint for a Prefab rule composed by this host.
///
/// The registry half covers every Prefab type the host composed, not just the
/// engine's: a gem-contributed type changes this value, so AZSCENE products
/// built before it was composed are re-processed.
#[must_use]
pub fn prefab_cook_analysis_fingerprint(context: &JobContext<'_>) -> String {
    let registry = crate::job_type_registry(context);
    let registry = registry.read();
    let lowerings = crate::job_lowerings(context);
    AnalysisFingerprint::new(DOMAIN)
        .field("prefab-builder-sources", crate::SOURCE_FINGERPRINT)
        .field("prefab-sources", az_prefab::SOURCE_FINGERPRINT)
        .field("azscene-codec-sources", az_scene::SOURCE_FINGERPRINT)
        .field(
            "type-registry",
            registry_fingerprint(&registry, &lowerings)
                .to_hex()
                .as_ref(),
        )
        .finish()
}

fn registry_fingerprint(
    registry: &TypeRegistry,
    lowerings: &[ComponentLoweringRegistration],
) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new();
    hash_text(&mut hasher, REGISTRY_DOMAIN);

    let mut registrations = registry.iter().collect::<Vec<_>>();
    registrations.sort_by_key(|registration| registration.type_info().type_path());
    hash_len(&mut hasher, registrations.len());
    for registration in registrations {
        hash_type_info(&mut hasher, registration.type_info());
        match registration.data::<PrefabTypeData>() {
            Some(prefab) => {
                hasher.update(b"prefab");
                hash_text(&mut hasher, prefab.tag);
                hasher.update(&prefab.source_version.to_le_bytes());
                hash_len(&mut hasher, prefab.aliases.len());
                for alias in prefab.aliases {
                    hash_text(&mut hasher, alias.tag);
                    hasher.update(&alias.source_version.to_le_bytes());
                }
                hash_len(&mut hasher, prefab.migrations.len());
                for migration in prefab.migrations {
                    hasher.update(&migration.from_version.to_le_bytes());
                    hasher.update(&migration.to_version.to_le_bytes());
                }
                match prefab.construction {
                    PrefabConstruction::ReflectDefaultOrFromWorld => {
                        hasher.update(b"reflect-default-or-from-world");
                    }
                    PrefabConstruction::Template { template_type_info } => {
                        hasher.update(b"template");
                        hash_text(&mut hasher, template_type_info().type_path());
                    }
                }
            }
            None => {
                hasher.update(b"plain-reflect");
            }
        }
    }

    let mut lowerings = lowerings
        .iter()
        .filter(|lowering| {
            registry
                .get((lowering.type_registration.rust_type_id)())
                .is_some()
        })
        .collect::<Vec<_>>();
    lowerings.sort_by_key(|lowering| {
        (
            lowering.type_registration.native_type_id,
            lowering.type_registration.name,
        )
    });
    hash_len(&mut hasher, lowerings.len());
    for lowering in lowerings {
        let native = &lowering.type_registration;
        hash_text(&mut hasher, native.name);
        hasher.update(native.native_type_id.as_bytes());
        hash_len(&mut hasher, native.base_type_ids.len());
        for base in native.base_type_ids {
            hasher.update(base.as_bytes());
        }
        hasher.update(&[match lowering.export_policy {
            ComponentExportPolicy::RuntimeAndEditor => 0,
            ComponentExportPolicy::RuntimeOnly => 1,
            ComponentExportPolicy::EditorOnly => 2,
            ComponentExportPolicy::Excluded => 3,
        }]);
        if let Some(bevy) = lowering.bevy_component {
            hasher.update(&[
                1,
                u8::from(bevy.component_id.is_some()),
                u8::from(bevy.apply_values.is_some()),
                u8::from(bevy.finalize_entity_table.is_some()),
            ]);
        } else {
            hasher.update(&[0]);
        }
    }

    hasher.finalize()
}

fn hash_type_info(hasher: &mut blake3::Hasher, info: &TypeInfo) {
    hash_text(hasher, info.type_path());
    match info {
        TypeInfo::Struct(info) => {
            hasher.update(b"struct");
            hash_len(hasher, info.field_len());
            for field in info.iter() {
                hash_text(hasher, field.name());
                hash_text(hasher, field.type_path());
            }
        }
        TypeInfo::TupleStruct(info) => {
            hasher.update(b"tuple-struct");
            hash_len(hasher, info.field_len());
            for field in info.iter() {
                hash_text(hasher, field.type_path());
            }
        }
        TypeInfo::Tuple(info) => {
            hasher.update(b"tuple");
            hash_len(hasher, info.field_len());
            for field in info.iter() {
                hash_text(hasher, field.type_path());
            }
        }
        TypeInfo::List(info) => {
            hasher.update(b"list");
            hash_text(hasher, info.item_ty().path());
        }
        TypeInfo::Array(info) => {
            hasher.update(b"array");
            hash_text(hasher, info.item_ty().path());
            hash_len(hasher, info.capacity());
        }
        TypeInfo::Map(info) => {
            hasher.update(b"map");
            hash_text(hasher, info.key_ty().path());
            hash_text(hasher, info.value_ty().path());
        }
        TypeInfo::Set(info) => {
            hasher.update(b"set");
            hash_text(hasher, info.value_ty().path());
        }
        TypeInfo::Enum(info) => {
            hasher.update(b"enum");
            hash_len(hasher, info.variant_len());
            for variant in info.iter() {
                hash_text(hasher, variant.name());
                match variant {
                    VariantInfo::Struct(info) => {
                        hasher.update(b"struct-variant");
                        hash_len(hasher, info.field_len());
                        for field in info.iter() {
                            hash_text(hasher, field.name());
                            hash_text(hasher, field.type_path());
                        }
                    }
                    VariantInfo::Tuple(info) => {
                        hasher.update(b"tuple-variant");
                        hash_len(hasher, info.field_len());
                        for field in info.iter() {
                            hash_text(hasher, field.type_path());
                        }
                    }
                    VariantInfo::Unit(_) => {
                        hasher.update(b"unit-variant");
                    }
                }
            }
        }
        TypeInfo::Opaque(_) => {
            hasher.update(b"opaque");
        }
    }
}

fn hash_text(hasher: &mut blake3::Hasher, value: &str) {
    hash_len(hasher, value.len());
    hasher.update(value.as_bytes());
}

fn hash_len(hasher: &mut blake3::Hasher, value: usize) {
    hasher.update(&(value as u64).to_le_bytes());
}

#[cfg(test)]
mod tests {
    use bevy::reflect::{Reflect, TypeRegistry};

    use super::*;

    #[derive(Reflect)]
    struct FirstShape {
        value: u32,
    }

    #[derive(Reflect)]
    struct SecondShape {
        value: u32,
    }

    #[test]
    fn registry_fingerprint_is_independent_of_registration_order() {
        let mut left = TypeRegistry::default();
        left.register::<FirstShape>();
        left.register::<SecondShape>();
        let mut right = TypeRegistry::default();
        right.register::<SecondShape>();
        right.register::<FirstShape>();

        assert_eq!(
            registry_fingerprint(&left, &[]),
            registry_fingerprint(&right, &[])
        );
    }

    #[test]
    fn registry_fingerprint_tracks_reflected_type_identity() {
        let mut first = TypeRegistry::default();
        first.register::<FirstShape>();
        let mut second = TypeRegistry::default();
        second.register::<SecondShape>();

        assert_ne!(
            registry_fingerprint(&first, &[]),
            registry_fingerprint(&second, &[])
        );
    }

    #[test]
    fn prefab_rule_publishes_registry_fingerprint_without_version_bump() {
        let registries = az_gem_contract::Registries::new();
        let rule = crate::prefab_build_rule(&JobContext::new(&registries));
        assert!(rule.analysis_fingerprint.starts_with(DOMAIN));
    }

    #[test]
    fn codec_identity_participates_in_the_fingerprint() {
        // The gap this module previously documented: an AZSCENE encoder change
        // left the registry, and therefore the whole fingerprint, untouched.
        let registries = az_gem_contract::Registries::new();
        let context = JobContext::new(&registries);
        let fingerprint = prefab_cook_analysis_fingerprint(&context);
        let registry_only = {
            let registry = crate::job_type_registry(&context);
            let fingerprint = {
                let registry = registry.read();
                registry_fingerprint(&registry, &crate::job_lowerings(&context))
            };
            format!("{REGISTRY_DOMAIN}{fingerprint}")
        };
        assert_ne!(fingerprint, registry_only);

        let without_codec = AnalysisFingerprint::new(DOMAIN)
            .field("prefab-builder-sources", crate::SOURCE_FINGERPRINT)
            .field("prefab-sources", az_prefab::SOURCE_FINGERPRINT)
            .field("azscene-codec-sources", "not-the-real-codec")
            .field("type-registry", &{
                let registry = crate::job_type_registry(&context);
                let registry = registry.read();
                registry_fingerprint(&registry, &crate::job_lowerings(&context))
                    .to_hex()
                    .to_string()
            })
            .finish();
        assert_ne!(
            fingerprint, without_codec,
            "swapping the codec fingerprint must change the rule fingerprint"
        );
    }

    #[test]
    fn every_source_component_is_a_real_build_time_fingerprint() {
        for (label, value) in [
            ("prefab-builder", crate::SOURCE_FINGERPRINT),
            ("prefab", az_prefab::SOURCE_FINGERPRINT),
            ("scene", az_scene::SOURCE_FINGERPRINT),
        ] {
            assert!(
                value.starts_with("azoth.source-tree/v1:"),
                "{label} published `{value}` instead of a source-tree fingerprint"
            );
        }
    }

    #[test]
    fn the_collection_rule_shares_the_derived_fingerprint() {
        let registries = az_gem_contract::Registries::new();
        let context = JobContext::new(&registries);
        assert_eq!(
            crate::prefab_collection_build_rule(&context).analysis_fingerprint,
            crate::prefab_build_rule(&context).analysis_fingerprint,
        );
    }
}
